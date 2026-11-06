//! Tauri command surface. Kept thin: validation, threading and I/O live here; pixels live in
//! `pipeline`. Anything CPU-heavy runs off the async runtime so the webview's IPC stays live.

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::imageops::{self, FilterType};
use serde::Serialize;
use tauri::ipc::{Channel, Response};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::naming::unique_stem;
use crate::pipeline::{self, Settings, Source, PNG_EXPORT_SIDE};

struct Item {
    source: Arc<Source>,
    stem: String,
}

#[derive(Default)]
pub struct AppState {
    next_id: AtomicU32,
    items: Mutex<HashMap<u32, Item>>,
    batch_running: AtomicBool,
}

/// Commands borrow `State<'_>`, but blocking closures must be `'static` — so the state is an
/// `Arc` we clone into each worker rather than a borrowed reference.
pub type Shared = Arc<AppState>;

impl AppState {
    fn get(&self, id: u32) -> AppResult<(Arc<Source>, String)> {
        let items = self.items.lock().map_err(|_| AppError::Internal("state lock poisoned".into()))?;
        items.get(&id).map(|i| (i.source.clone(), i.stem.clone())).ok_or(AppError::UnknownItem)
    }
}

fn data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", B64.encode(png))
}

/// Run a CPU-bound closure on Tokio's blocking pool and flatten join errors (panics) into an
/// `AppError`, so a bug in one image surfaces as a message instead of a dead promise.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(format!("worker crashed: {e}")))?
}

/// Write via a temp file + rename so a failure (disk full, AV lock) never leaves a truncated
/// .ico where a good one used to be. `fs::rename` replaces atomically on NTFS.
fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let tmp = path.with_extension("simulacrum-tmp");
    std::fs::write(&tmp, bytes).map_err(|e| AppError::write(path, e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        AppError::write(path, e)
    })
}

/// Compose once, then write the .ico (and optional 512 PNG) from that same master.
fn export_one(source: &Source, s: &Settings, ico_path: &Path, also_png: bool) -> AppResult<Vec<String>> {
    let master = pipeline::compose(source, s);
    write_atomic(ico_path, &pipeline::build_ico(&master, &s.sizes)?)?;
    let mut written = vec![ico_path.display().to_string()];
    if also_png {
        let png_path = ico_path.with_extension("png");
        let png = pipeline::encode_png(&pipeline::render(&master, PNG_EXPORT_SIDE), false)?;
        write_atomic(&png_path, &png)?;
        written.push(png_path.display().to_string());
    }
    Ok(written)
}

// ---- load / remove ---------------------------------------------------------------------

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LoadResult {
    #[serde(rename_all = "camelCase")]
    Ok { id: u32, name: String, width: u32, height: u32, thumb: String, warning: Option<String> },
    #[serde(rename_all = "camelCase")]
    Error { name: String, error: String },
}

#[tauri::command]
pub async fn load_images(state: State<'_, Shared>, paths: Vec<String>) -> AppResult<Vec<LoadResult>> {
    let state = state.inner().clone();
    blocking(move || {
        // Decode in parallel (scoped threads, bounded by core count): big JPEGs are the slow
        // part of adding a batch, and each decode is independent.
        let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
        let mut out = Vec::with_capacity(paths.len());
        for chunk in paths.chunks(workers) {
            let results: Vec<_> = std::thread::scope(|sc| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|p| sc.spawn(move || (p, catch_unwind(|| load_one(Path::new(p))))))
                    .collect();
                handles.into_iter().filter_map(|h| h.join().ok()).collect()
            });
            for (p, res) in results {
                let path = Path::new(p);
                let name = path.file_name().map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned());
                let res = res.unwrap_or_else(|_| Err(AppError::Internal("decoder panicked".into())));
                out.push(match res {
                    Ok((source, thumb)) => {
                        let id = state.next_id.fetch_add(1, Ordering::Relaxed);
                        let stem = path.file_stem().map_or("icon".into(), |s| s.to_string_lossy().into_owned());
                        let r = LoadResult::Ok {
                            id,
                            name,
                            width: source.orig_w,
                            height: source.orig_h,
                            thumb,
                            warning: source.warning(),
                        };
                        state
                            .items
                            .lock()
                            .map_err(|_| AppError::Internal("state lock poisoned".into()))?
                            .insert(id, Item { source: Arc::new(source), stem });
                        r
                    }
                    Err(e) => LoadResult::Error { name, error: e.to_string() },
                });
            }
        }
        Ok(out)
    })
    .await
}

fn load_one(path: &Path) -> AppResult<(Source, String)> {
    let source = pipeline::load(path)?;
    // Thumbnail fidelity doesn't matter; Triangle is ~5x cheaper than Lanczos here.
    let thumb = imageops::resize(&source.img, 64, 64, FilterType::Triangle);
    Ok((source, data_url(&pipeline::encode_png(&thumb, true)?)))
}

#[tauri::command]
pub fn remove_image(state: State<'_, Shared>, id: u32) -> AppResult<()> {
    state.items.lock().map_err(|_| AppError::Internal("state lock poisoned".into()))?.remove(&id);
    Ok(())
}

// ---- preview ---------------------------------------------------------------------------

pub const PREVIEW_MAIN: u32 = 512;

/// Returns raw bytes, not JSON: `ipc::Response` reaches JS as an `ArrayBuffer` with no
/// serde/base64 step, and the frontend `putImageData`s straight into canvases — no PNG
/// encode here, no PNG decode there. Layout (little-endian u32s):
///
///   [count] then `count` × ([side] [side·side·4 bytes straight-alpha RGBA])
///
/// First entry is the 512 main preview, then every *selected* ICO size, ascending.
#[tauri::command]
pub async fn preview(state: State<'_, Shared>, id: u32, settings: Settings) -> AppResult<Response> {
    let s = settings.validated()?;
    let (source, _) = state.get(id)?;
    blocking(move || {
        // Same compose + render path as export → preview is pixel-identical to the output.
        let master = pipeline::compose(&source, &s);
        let sides: Vec<u32> = std::iter::once(PREVIEW_MAIN).chain(s.sizes.iter().copied()).collect();
        let total: usize = 4 + sides.iter().map(|&n| 4 + (n * n * 4) as usize).sum::<usize>();
        let mut buf = Vec::with_capacity(total);
        buf.extend_from_slice(&(sides.len() as u32).to_le_bytes());
        for n in sides {
            buf.extend_from_slice(&n.to_le_bytes());
            buf.extend_from_slice(pipeline::render(&master, n).as_raw());
        }
        Ok(Response::new(buf))
    })
    .await
}

// ---- export ----------------------------------------------------------------------------

#[tauri::command]
pub async fn export_single(
    state: State<'_, Shared>,
    id: u32,
    path: String,
    settings: Settings,
    also_png: bool,
) -> AppResult<Vec<String>> {
    let s = settings.validated()?;
    let (source, _) = state.get(id)?;
    let mut path = PathBuf::from(path);
    if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("ico")) {
        path.set_extension("ico");
    }
    blocking(move || export_one(&source, &s, &path, also_png)).await
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    name: String,
    error: String,
}

#[derive(Serialize, Clone)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum BatchEvent {
    #[serde(rename_all = "camelCase")]
    Progress { done: usize, total: usize, name: String },
    #[serde(rename_all = "camelCase")]
    Finished { ok: usize, failed: Vec<Failure>, out_dir: String },
}

/// Returns as soon as the worker thread starts; progress streams over `on_event`. A per-call
/// `Channel` (vs. global `emit`) is ordered and can't be heard by a stale listener.
#[tauri::command]
pub fn export_batch(
    state: State<'_, Shared>,
    ids: Vec<u32>,
    out_dir: String,
    settings: Settings,
    also_png: bool,
    on_event: Channel<BatchEvent>,
) -> AppResult<()> {
    let s = settings.validated()?;
    let dir = PathBuf::from(&out_dir);
    if !dir.is_dir() {
        return Err(AppError::Write {
            path: out_dir,
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "folder does not exist"),
        });
    }
    if state.batch_running.swap(true, Ordering::AcqRel) {
        return Err(AppError::Internal("a batch export is already running".into()));
    }
    // Snapshot the items now: removing one from the list mid-batch must not change this run.
    let jobs: Vec<_> = ids.iter().map(|&id| (id, state.get(id))).collect();
    let state = state.inner().clone();

    std::thread::spawn(move || {
        let total = jobs.len();
        let mut taken = HashSet::new();
        let (mut ok, mut failed) = (0, Vec::new());
        for (done, (id, job)) in jobs.into_iter().enumerate() {
            let label = job.as_ref().map_or_else(|_| format!("item #{id}"), |(_, stem)| stem.clone());
            let result = job.and_then(|(source, stem)| {
                let stem = unique_stem(&dir, &stem, also_png, &mut taken);
                let path = dir.join(format!("{stem}.ico"));
                // A panic in one image must not kill the thread and leave the UI spinning.
                catch_unwind(AssertUnwindSafe(|| export_one(&source, &s, &path, also_png)))
                    .unwrap_or_else(|_| Err(AppError::Internal("encoder panicked".into())))
            });
            match result {
                Ok(_) => ok += 1,
                Err(e) => failed.push(Failure { name: label.clone(), error: e.to_string() }),
            }
            let _ = on_event.send(BatchEvent::Progress { done: done + 1, total, name: label });
        }
        state.batch_running.store(false, Ordering::Release);
        let _ = on_event.send(BatchEvent::Finished { ok, failed, out_dir: dir.display().to_string() });
    });
    Ok(())
}

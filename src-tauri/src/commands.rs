//! Tauri command surface. Kept thin: validation, threading and I/O live here; pixels live in
//! `pipeline`. Anything CPU-heavy runs off the async runtime so the webview's IPC stays live.

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::imageops::{self, FilterType};
use image::RgbaImage;
use serde::Serialize;
use tauri::ipc::{Channel, Response};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::naming::unique_stem_opts;
use crate::pipeline::{
    self, IcoEntryInfo, PackKind, Settings, Shape, Source, ALLOWED_SIZES, PNG_1024_SIDE,
    PNG_EXPORT_SIDE,
};

struct Item {
    source: Arc<Source>,
    stem: String,
    overrides: HashMap<u32, RgbaImage>,
    /// Custom Android monochrome layer artwork, if the user picked one.
    android_mono: Option<Arc<Source>>,
}

#[derive(Default)]
pub struct AppState {
    next_id: AtomicU32,
    items: Mutex<HashMap<u32, Item>>,
    batch_running: AtomicBool,
    batch_cancel: AtomicBool,
}

/// Commands borrow `State<'_>`, but blocking closures must be `'static` — so the state is an
/// `Arc` we clone into each worker rather than a borrowed reference.
pub type Shared = Arc<AppState>;

impl AppState {
    fn get(&self, id: u32) -> AppResult<(Arc<Source>, String, HashMap<u32, RgbaImage>)> {
        let items = self.items.lock().map_err(|_| AppError::Internal("state lock poisoned".into()))?;
        items
            .get(&id)
            .map(|i| (i.source.clone(), i.stem.clone(), i.overrides.clone()))
            .ok_or(AppError::UnknownItem)
    }

    fn get_android(&self, id: u32) -> AppResult<(Arc<Source>, Option<Arc<Source>>)> {
        let items = self.items.lock().map_err(|_| AppError::Internal("state lock poisoned".into()))?;
        items
            .get(&id)
            .map(|i| (i.source.clone(), i.android_mono.clone()))
            .ok_or(AppError::UnknownItem)
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
/// file where a good one used to be. `fs::rename` replaces atomically on NTFS.
fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let tmp = path.with_extension("simulacrum-tmp");
    std::fs::write(&tmp, bytes).map_err(|e| AppError::write(path, e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        AppError::write(path, e)
    })
}

/// Compose once, then write the .ico (and optional 512 / 1024 PNGs) from that same master.
fn export_one(
    source: &Source,
    overrides: &HashMap<u32, RgbaImage>,
    s: &Settings,
    ico_path: &Path,
    also_png: bool,
    also_png_1024: bool,
) -> AppResult<Vec<String>> {
    let master = pipeline::compose_with_overrides(source, s, Some(overrides));
    write_atomic(ico_path, &pipeline::build_ico_with_options(&master, &s.sizes, s.legacy_bmp)?)?;
    let mut written = vec![ico_path.display().to_string()];
    if also_png {
        let png_path = ico_path.with_extension("png");
        let png = pipeline::encode_png(&pipeline::render(&master, PNG_EXPORT_SIDE), false)?;
        write_atomic(&png_path, &png)?;
        written.push(png_path.display().to_string());
    }
    if also_png_1024 {
        let stem = ico_path
            .file_stem()
            .and_then(|st| st.to_str())
            .unwrap_or("icon");
        let png_1024_path = ico_path.with_file_name(format!("{stem}-1024.png"));
        let png = pipeline::encode_png(&pipeline::render(&master, PNG_1024_SIDE), false)?;
        write_atomic(&png_1024_path, &png)?;
        written.push(png_1024_path.display().to_string());
    }
    Ok(written)
}

fn make_thumb(source: &Source) -> AppResult<String> {
    let thumb = imageops::resize(&source.img, 64, 64, FilterType::Triangle);
    Ok(data_url(&pipeline::encode_png(&thumb, true)?))
}

// ---- load / remove ---------------------------------------------------------------------

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LoadResult {
    #[serde(rename_all = "camelCase")]
    Ok {
        id: u32,
        name: String,
        width: u32,
        height: u32,
        thumb: String,
        warning: Option<String>,
        is_svg: bool,
        ico_entries: Option<Vec<IcoEntryInfo>>,
    },
    #[serde(rename_all = "camelCase")]
    Error { name: String, error: String },
}

fn register_source(state: &AppState, name: String, stem: String, source: Source, thumb: String) -> AppResult<LoadResult> {
    let id = state.next_id.fetch_add(1, Ordering::Relaxed);
    let r = LoadResult::Ok {
        id,
        name,
        width: source.orig_w,
        height: source.orig_h,
        thumb,
        warning: source.warning(),
        is_svg: source.is_svg,
        ico_entries: source.ico_entries.clone(),
    };
    state
        .items
        .lock()
        .map_err(|_| AppError::Internal("state lock poisoned".into()))?
        .insert(id, Item { source: Arc::new(source), stem, overrides: HashMap::new(), android_mono: None });
    Ok(r)
}

#[tauri::command]
pub async fn load_images(state: State<'_, Shared>, paths: Vec<String>) -> AppResult<Vec<LoadResult>> {
    let state = state.inner().clone();
    blocking(move || {
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
                        let stem = path.file_stem().map_or("icon".into(), |s| s.to_string_lossy().into_owned());
                        register_source(&state, name, stem, source, thumb)?
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
    let thumb = make_thumb(&source)?;
    Ok((source, thumb))
}

#[tauri::command]
pub async fn load_clipboard_image(
    state: State<'_, Shared>,
    name: String,
    bytes: Vec<u8>,
) -> AppResult<LoadResult> {
    let state = state.inner().clone();
    blocking(move || {
        let safe_name = if name.trim().is_empty() { "clipboard.png".to_string() } else { name };
        let stem = Path::new(&safe_name)
            .file_stem()
            .map_or("clipboard".into(), |s| s.to_string_lossy().into_owned());
        match pipeline::load_bytes(&bytes, &safe_name) {
            Ok(source) => {
                let thumb = make_thumb(&source)?;
                register_source(&state, safe_name, stem, source, thumb)
            }
            Err(e) => Ok(LoadResult::Error { name: safe_name, error: e.to_string() }),
        }
    })
    .await
}

#[tauri::command]
pub fn remove_image(state: State<'_, Shared>, id: u32) -> AppResult<()> {
    state.items.lock().map_err(|_| AppError::Internal("state lock poisoned".into()))?.remove(&id);
    Ok(())
}

#[tauri::command]
pub async fn set_size_override(
    state: State<'_, Shared>,
    id: u32,
    size: u32,
    path: Option<String>,
) -> AppResult<Vec<u32>> {
    if !ALLOWED_SIZES.contains(&size) {
        return Err(AppError::InvalidSettings(format!("unsupported override size {size}")));
    }
    let state = state.inner().clone();
    blocking(move || {
        let override_img = match path {
            Some(p) => {
                let src = pipeline::load(Path::new(&p))?;
                if src.img.dimensions() == (size, size) && !src.is_svg {
                    src.img
                } else {
                    let s = Settings {
                        shape: Shape::None,
                        padding: 0.0,
                        sizes: vec![size],
                        ..Settings::default()
                    };
                    pipeline::render(&pipeline::compose(&src, &s), size)
                }
            }
            None => {
                let mut items =
                    state.items.lock().map_err(|_| AppError::Internal("state lock poisoned".into()))?;
                let item = items.get_mut(&id).ok_or(AppError::UnknownItem)?;
                item.overrides.remove(&size);
                let mut keys: Vec<u32> = item.overrides.keys().copied().collect();
                keys.sort_unstable();
                return Ok(keys);
            }
        };

        let mut items =
            state.items.lock().map_err(|_| AppError::Internal("state lock poisoned".into()))?;
        let item = items.get_mut(&id).ok_or(AppError::UnknownItem)?;
        item.overrides.insert(size, override_img);
        let mut keys: Vec<u32> = item.overrides.keys().copied().collect();
        keys.sort_unstable();
        Ok(keys)
    })
    .await
}

// ---- preview ---------------------------------------------------------------------------

pub const PREVIEW_MAIN: u32 = 512;

/// Returns raw bytes, not JSON: `ipc::Response` reaches JS as an `ArrayBuffer` with no
/// serde/base64 step, and the frontend `putImageData`s straight into canvases.
#[tauri::command]
pub async fn preview(state: State<'_, Shared>, id: u32, settings: Settings) -> AppResult<Response> {
    let s = settings.validated()?;
    let (source, _, overrides) = state.get(id)?;
    blocking(move || {
        let master = pipeline::compose_with_overrides(&source, &s, Some(&overrides));
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

#[tauri::command]
pub async fn render_png_bytes(
    state: State<'_, Shared>,
    id: u32,
    settings: Settings,
    size: u32,
) -> AppResult<Response> {
    let s = settings.validated()?;
    let (source, _, overrides) = state.get(id)?;
    let side = size.clamp(16, 1024);
    blocking(move || {
        let master = pipeline::compose_with_overrides(&source, &s, Some(&overrides));
        let png = pipeline::encode_png(&pipeline::render(&master, side), false)?;
        Ok(Response::new(png))
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
    also_png_1024: Option<bool>,
) -> AppResult<Vec<String>> {
    let s = settings.validated()?;
    let (source, _, overrides) = state.get(id)?;
    let mut path = PathBuf::from(path);
    if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("ico")) {
        path.set_extension("ico");
    }
    let also_1024 = also_png_1024.unwrap_or(false);
    blocking(move || export_one(&source, &overrides, &s, &path, also_png, also_1024)).await
}

#[tauri::command]
pub async fn export_icns(
    state: State<'_, Shared>,
    id: u32,
    path: String,
    settings: Settings,
) -> AppResult<String> {
    let s = settings.validated()?;
    let (source, _, overrides) = state.get(id)?;
    let mut path = PathBuf::from(path);
    if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("icns")) {
        path.set_extension("icns");
    }
    blocking(move || {
        let master = pipeline::compose_with_overrides(&source, &s, Some(&overrides));
        let bytes = crate::icns::encode(&master)?;
        write_atomic(&path, &bytes)?;
        Ok(path.display().to_string())
    })
    .await
}

#[tauri::command]
pub async fn export_pack(
    state: State<'_, Shared>,
    id: u32,
    out_dir: String,
    pack: PackKind,
    settings: Settings,
) -> AppResult<Vec<String>> {
    let s = settings.validated()?;
    let (source, _, overrides) = state.get(id)?;
    let dir = PathBuf::from(&out_dir);
    blocking(move || {
        let master = pipeline::compose_with_overrides(&source, &s, Some(&overrides));
        pipeline::export_pack(&master, &dir, pack)
    })
    .await
}

// ---- Android adaptive icons ------------------------------------------------------------

/// Set (or with `None`, clear) the custom monochrome artwork for item `id`. Returns the file name.
#[tauri::command]
pub async fn set_android_mono(
    state: State<'_, Shared>,
    id: u32,
    path: Option<String>,
) -> AppResult<Option<String>> {
    let state = state.inner().clone();
    blocking(move || {
        let (src, name) = match path {
            Some(p) => {
                let p = PathBuf::from(p);
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
                (Some(Arc::new(pipeline::load(&p)?)), name)
            }
            None => (None, None),
        };
        let mut items =
            state.items.lock().map_err(|_| AppError::Internal("state lock poisoned".into()))?;
        items.get_mut(&id).ok_or(AppError::UnknownItem)?.android_mono = src;
        Ok(name)
    })
    .await
}

/// Binary payload, see `android::preview_payload`.
#[tauri::command]
pub async fn android_preview(state: State<'_, Shared>, id: u32, settings: Settings) -> AppResult<Response> {
    let s = settings.validated()?;
    let (source, mono) = state.get_android(id)?;
    blocking(move || Ok(Response::new(crate::android::preview_payload(&source, &s, mono.as_deref())?))).await
}

#[tauri::command]
pub async fn export_android(
    state: State<'_, Shared>,
    id: u32,
    out_dir: String,
    settings: Settings,
) -> AppResult<Vec<String>> {
    let s = settings.validated()?;
    let (source, mono) = state.get_android(id)?;
    let dir = PathBuf::from(&out_dir);
    blocking(move || crate::android::export(&source, &s, mono.as_deref(), &dir)).await
}

/// Directory reader: what kind of project `path` is and where its icons live.
#[tauri::command]
pub async fn scan_project(path: String) -> AppResult<crate::project::ProjectReport> {
    blocking(move || crate::project::scan(Path::new(&path))).await
}

/// Write the chosen targets into the project. Originals go to
/// `<app local data>/backups/<project>-<unix time>/`.
#[tauri::command]
pub async fn export_to_project(
    app: tauri::AppHandle,
    state: State<'_, Shared>,
    id: u32,
    root: String,
    target_ids: Vec<String>,
    settings: Settings,
) -> AppResult<crate::project::ExportOutcome> {
    use tauri::Manager;
    let s = settings.validated()?;
    let (source, _, overrides) = state.get(id)?;
    let (_, mono) = state.get_android(id)?;
    let data = app.path().app_local_data_dir().map_err(|e| AppError::Internal(format!("no app data folder: {e}")))?;
    let root = PathBuf::from(root);
    let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "project".into());
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let backup = data.join("backups").join(format!("{name}-{stamp}"));
    blocking(move || {
        let input = crate::project::ExportInput {
            source: &source,
            settings: &s,
            overrides: Some(&overrides),
            mono: mono.as_deref(),
        };
        crate::project::export(&root, &target_ids, &input, &backup)
    })
    .await
}

#[tauri::command]
pub async fn apply_folder_icon(
    state: State<'_, Shared>,
    id: u32,
    folder: String,
    settings: Settings,
) -> AppResult<String> {
    let s = settings.validated()?;
    let (source, _, overrides) = state.get(id)?;
    let dir = PathBuf::from(&folder);
    blocking(move || {
        let master = pipeline::compose_with_overrides(&source, &s, Some(&overrides));
        let ico_bytes = pipeline::build_ico_with_options(&master, &s.sizes, s.legacy_bmp)?;
        crate::folder_icon::apply_to_folder(&dir, &ico_bytes)
    })
    .await
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
    Finished { ok: usize, failed: Vec<Failure>, out_dir: String, cancelled: bool },
}

#[tauri::command]
pub fn cancel_batch(state: State<'_, Shared>) -> AppResult<()> {
    state.batch_cancel.store(true, Ordering::Release);
    Ok(())
}

/// Returns as soon as the worker coordinator starts; progress streams over `on_event`.
#[tauri::command]
pub fn export_batch(
    state: State<'_, Shared>,
    ids: Vec<u32>,
    out_dir: String,
    settings: Settings,
    also_png: bool,
    also_png_1024: Option<bool>,
    item_settings: Option<HashMap<u32, Settings>>,
    on_event: Channel<BatchEvent>,
) -> AppResult<()> {
    let default_s = settings.validated()?;
    let mut validated_overrides = HashMap::new();
    if let Some(map) = item_settings {
        for (id, raw_s) in map {
            validated_overrides.insert(id, raw_s.validated()?);
        }
    }
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
    state.batch_cancel.store(false, Ordering::Release);

    let also_1024 = also_png_1024.unwrap_or(false);
    let mut taken = HashSet::new();
    let jobs: Vec<_> = ids
        .iter()
        .map(|&id| {
            let item_res = state.get(id).map(|(source, stem, overrides)| {
                let unique = unique_stem_opts(&dir, &stem, also_png, also_1024, &mut taken);
                let path = dir.join(format!("{unique}.ico"));
                let item_s = validated_overrides.get(&id).cloned().unwrap_or_else(|| default_s.clone());
                (source, overrides, item_s, path, stem)
            });
            (id, item_res)
        })
        .collect();
    let state = state.inner().clone();

    std::thread::spawn(move || {
        let total = jobs.len();
        let done_ctr = AtomicUsize::new(0);
        let ok_ctr = AtomicUsize::new(0);
        let failed = Mutex::new(Vec::<Failure>::new());
        let event_lock = Mutex::new(());
        let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(4).max(1);
        let next_idx = AtomicUsize::new(0);

        std::thread::scope(|sc| {
            for _ in 0..workers {
                sc.spawn(|| {
                    loop {
                        if state.batch_cancel.load(Ordering::Acquire) {
                            break;
                        }
                        let idx = next_idx.fetch_add(1, Ordering::Relaxed);
                        let Some((id, job)) = jobs.get(idx) else {
                            break;
                        };
                        let label = job
                            .as_ref()
                            .map_or_else(|_| format!("item #{id}"), |(_, _, _, _, stem)| stem.clone());
                        let result = match job {
                            Ok((source, overrides, item_s, path, _)) => catch_unwind(AssertUnwindSafe(|| {
                                export_one(source, overrides, item_s, path, also_png, also_1024)
                            }))
                            .unwrap_or_else(|_| Err(AppError::Internal("encoder panicked".into()))),
                            Err(e) => Err(AppError::Internal(e.to_string())),
                        };
                        match result {
                            Ok(_) => {
                                ok_ctr.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(e) => {
                                if let Ok(mut f) = failed.lock() {
                                    f.push(Failure { name: label.clone(), error: e.to_string() });
                                }
                            }
                        }
                        let done = done_ctr.fetch_add(1, Ordering::AcqRel) + 1;
                        if let Ok(_g) = event_lock.lock() {
                            let _ = on_event.send(BatchEvent::Progress { done, total, name: label });
                        }
                    }
                });
            }
        });

        let cancelled = state.batch_cancel.load(Ordering::Acquire);
        let ok = ok_ctr.load(Ordering::Acquire);
        let failed = failed.into_inner().unwrap_or_default();
        state.batch_running.store(false, Ordering::Release);
        let _ = on_event.send(BatchEvent::Finished {
            ok,
            failed,
            out_dir: dir.display().to_string(),
            cancelled,
        });
    });
    Ok(())
}

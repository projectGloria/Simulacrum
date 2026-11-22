//! Directory reader: work out what kind of project a folder is and where its icons live, then
//! write every icon straight into it, backing up whatever gets replaced.
//!
//! Detection only reads small config files (manifests, build scripts, project files). Every
//! target path is resolved relative to the project root and rejected if it would land outside
//! it, so a hostile `manifest.json` with `../../` paths can't make an export write elsewhere.
//! Exports re-run detection on the Rust side; the UI only ever sends target ids.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::{ImageEncoder, RgbaImage};
use serde::Serialize;
use serde_json::Value;

use crate::android::{self, Plan, ResNames, ResSink};
use crate::error::{AppError, AppResult};
use crate::pipeline::{self, BadgeMode, BgMode, Master, Settings, Shape, Source};

const MAX_DEPTH: usize = 7;
const MAX_ENTRIES: usize = 20_000;
const MAX_TEXT: u64 = 1 << 20;
const FAVICON_SIZES: [u32; 3] = [16, 32, 48];

/// Build output, dependencies and caches. Dot-folders (`.git`, `.gradle`, `.dart_tool`, `.wxt`,
/// `.output`, `.next`, …) are skipped too.
const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "out", "gen", "pods", "deriveddata", "bin", "obj", "venv",
    "env", "__pycache__", "coverage", "vendor", "bower_components", "intermediates", "ephemeral",
    "site-packages",
];

pub const WEB_MANIFEST: &str = r##"{
  "name": "",
  "short_name": "",
  "icons": [
    { "src": "/android-chrome-192x192.png", "sizes": "192x192", "type": "image/png" },
    { "src": "/android-chrome-512x512.png", "sizes": "512x512", "type": "image/png" }
  ],
  "theme_color": "#ffffff",
  "background_color": "#ffffff",
  "display": "standalone"
}
"##;

// ---- report types ----------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Flutter,
    Android,
    Tauri,
    Electron,
    BrowserExtension,
    VsCode,
    Xcode,
    RustWindows,
    DotNet,
    PyInstaller,
    Web,
    RepoIcon,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Flutter => "Flutter",
            Kind::Android => "Android",
            Kind::Tauri => "Tauri",
            Kind::Electron => "Electron",
            Kind::BrowserExtension => "Browser extension",
            Kind::VsCode => "VS Code extension",
            Kind::Xcode => "Xcode",
            Kind::RustWindows => "Rust (Windows)",
            Kind::DotNet => ".NET",
            Kind::PyInstaller => "PyInstaller",
            Kind::Web => "Web / PWA",
            Kind::RepoIcon => "Repository icon",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Format {
    /// Windows icon. `favicon` → 16/32/48, otherwise the sizes chosen in the sidebar.
    Ico { favicon: bool },
    Png { side: u32 },
    Icns,
    /// An Android `res/` folder; `base` / `round` are the mipmap names from the manifest.
    AndroidRes { base: String, round: Option<String> },
    /// An Xcode `AppIcon.appiconset` folder, driven by its `Contents.json`.
    AppIconSet,
    /// A new `site.webmanifest`. Never written over an existing file.
    WebManifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Profile {
    /// The icon exactly as styled in the sidebar.
    AsIs,
    /// Square, no padding/shadow/badge, flattened onto the background (iOS, Apple touch).
    FullBleedOpaque,
    /// Like `FullBleedOpaque` but keeps ≥10 % padding for the maskable safe zone.
    Maskable,
    /// The Android adaptive-icon pipeline.
    Android,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: String,
    pub label: String,
    pub rel_path: String,
    pub format: Format,
    pub profile: Profile,
    pub exists: bool,
    pub default_on: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    pub kind: Kind,
    pub label: String,
    pub evidence: Vec<String>,
    pub targets: Vec<Target>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectReport {
    pub root: String,
    pub name: String,
    pub primary: Option<Kind>,
    pub primary_label: Option<String>,
    pub detections: Vec<Detection>,
    pub warnings: Vec<String>,
}

impl ProjectReport {
    pub fn targets(&self) -> impl Iterator<Item = &Target> {
        self.detections.iter().flat_map(|d| d.targets.iter())
    }

    pub fn target(&self, id: &str) -> Option<&Target> {
        self.targets().find(|t| t.id == id)
    }

    pub fn detection(&self, kind: Kind) -> Option<&Detection> {
        self.detections.iter().find(|d| d.kind == kind)
    }
}

// ---- small parsing helpers -------------------------------------------------------------

fn read_text(path: &Path) -> Option<String> {
    let m = fs::metadata(path).ok()?;
    if !m.is_file() || m.len() > MAX_TEXT {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string())
}

/// Every quoted string that follows `needle` (after optional `space = : ( [`).
/// `set_icon("a.ico")`, `icon=['a.ico']`, `ICON "a.ico"`, `icon: 'a.png'`.
fn quoted_after(text: &str, needle: &str, ignore_case: bool) -> Vec<String> {
    if needle.is_empty() {
        return Vec::new();
    }
    let hay = if ignore_case { text.to_ascii_lowercase() } else { text.to_string() };
    let needle = if ignore_case { needle.to_ascii_lowercase() } else { needle.to_string() };
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(pos) = hay[from..].find(&needle) {
        let mut i = from + pos + needle.len();
        from = i;
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'=' | b':' | b'(' | b'[') {
            i += 1;
        }
        if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
            let q = bytes[i];
            if let Some(end) = bytes[i + 1..].iter().position(|&b| b == q || b == b'\n') {
                if bytes[i + 1 + end] == q {
                    let v = &text[i + 1..i + 1 + end];
                    if !v.trim().is_empty() {
                        out.push(v.trim().to_string());
                    }
                }
            }
        }
    }
    out
}

/// Value of `name="…"` inside one XML start tag.
fn xml_attr(tag: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(pos) = tag[from..].find(name) {
        let start = from + pos;
        let mut i = start + name.len();
        from = i;
        let prev_ok = start == 0 || tag.as_bytes()[start - 1].is_ascii_whitespace();
        let rest = tag[i..].trim_start();
        i = tag.len() - rest.len();
        if !prev_ok || !rest.starts_with('=') {
            continue;
        }
        let rest = rest[1..].trim_start();
        let q = rest.chars().next()?;
        if q != '"' && q != '\'' {
            continue;
        }
        let body = &rest[1..];
        let _ = i;
        return body.find(q).map(|end| body[..end].to_string());
    }
    None
}

/// The first `<name …>` start tag in `text`.
fn start_tag<'t>(text: &'t str, name: &str) -> Option<&'t str> {
    let open = format!("<{name}");
    let mut from = 0;
    while let Some(pos) = text[from..].find(&open) {
        let s = from + pos;
        let after = text[s + open.len()..].chars().next();
        if matches!(after, Some(c) if c.is_whitespace() || c == '>' || c == '/') {
            let end = text[s..].find('>').map(|e| s + e + 1).unwrap_or(text.len());
            return Some(&text[s..end]);
        }
        from = s + open.len();
    }
    None
}

/// `32x32`, `128x128@2x`, `icon-192x192`, `Square44x44Logo` → the pixel side.
fn side_from_name(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_digit()) {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i < b.len() && b[i] == b'x' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
                let w: u32 = lower[start..i].parse().ok()?;
                let mut scale = 1;
                if let Some(at) = lower[i..].find('@') {
                    scale = lower[i + at + 1..].chars().next().and_then(|c| c.to_digit(10)).unwrap_or(1);
                }
                return Some(w * scale.max(1));
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Join `p` onto the root-relative folder `base`, resolving `.`/`..` lexically. `None` for
/// absolute paths, URLs, or anything that climbs above the root.
pub fn join_rel(base: &str, p: &str) -> Option<String> {
    let p = p.trim().replace('\\', "/");
    if p.is_empty() || p.starts_with('/') || p.contains(':') {
        return None;
    }
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty() && *s != ".").collect();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn parent_rel(rel: &str) -> &str {
    rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

fn file_name(rel: &str) -> &str {
    rel.rsplit_once('/').map(|(_, f)| f).unwrap_or(rel)
}

fn ext_lower(rel: &str) -> String {
    let f = file_name(rel);
    f.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// True when `path` (existing or not) resolves inside `canon_root`, following symlinks of
/// whatever part of it already exists.
fn contained(canon_root: &Path, path: &Path) -> bool {
    let mut probe = path.to_path_buf();
    let mut tail = Vec::new();
    loop {
        if let Ok(c) = probe.canonicalize() {
            let full = tail.iter().rev().fold(c, |acc: PathBuf, seg: &std::ffi::OsString| acc.join(seg));
            return full.starts_with(canon_root);
        }
        match (probe.file_name().map(|f| f.to_os_string()), probe.parent()) {
            (Some(f), Some(parent)) => {
                if f == ".." {
                    return false;
                }
                tail.push(f);
                probe = parent.to_path_buf();
            }
            _ => return false,
        }
    }
}

// ---- scan ------------------------------------------------------------------------------

struct Ctx {
    root: PathBuf,
    canon: PathBuf,
    /// Root-relative file paths with `/` separators.
    files: Vec<String>,
    seen: HashSet<String>,
    warnings: Vec<String>,
}

fn walk(root: &Path) -> (Vec<String>, bool) {
    let mut files = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new(), 0usize, false)];
    let mut entries = 0usize;
    while let Some((dir, rel, depth, files_only)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let mut subdirs = Vec::new();
        for entry in rd.flatten() {
            entries += 1;
            if entries > MAX_ENTRIES {
                return (files, true);
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_file() {
                files.push(child_rel);
            } else if ft.is_dir() && !files_only && depth + 1 < MAX_DEPTH {
                let lower = name.to_ascii_lowercase();
                if name.starts_with('.') || SKIP_DIRS.contains(&lower.as_str()) {
                    continue;
                }
                // Root-level `build/` holds Electron's buildResources; nested ones are output.
                if lower == "build" {
                    if depth == 0 {
                        subdirs.push((entry.path(), child_rel, depth + 1, true));
                    }
                    continue;
                }
                subdirs.push((entry.path(), child_rel, depth + 1, false));
            }
        }
        stack.extend(subdirs);
    }
    files.sort();
    (files, false)
}

impl Ctx {
    fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
    fn exists(&self, rel: &str) -> bool {
        self.abs(rel).exists()
    }
    fn is_file(&self, rel: &str) -> bool {
        self.abs(rel).is_file()
    }
    fn is_dir(&self, rel: &str) -> bool {
        self.abs(rel).is_dir()
    }
    fn text(&self, rel: &str) -> Option<String> {
        read_text(&self.abs(rel))
    }
    fn json(&self, rel: &str) -> Option<Value> {
        serde_json::from_str(&self.text(rel)?).ok()
    }
    fn first_existing<'s>(&self, rels: &[&'s str]) -> Option<&'s str> {
        rels.iter().copied().find(|r| self.is_file(r))
    }
    fn files_ending(&self, suffix: &str) -> Vec<String> {
        let suffix = suffix.to_ascii_lowercase();
        self.files.iter().filter(|f| f.to_ascii_lowercase().ends_with(&suffix)).cloned().collect()
    }
    fn files_with_ext(&self, ext: &str, max_depth: usize) -> Vec<String> {
        self.files
            .iter()
            .filter(|f| f.matches('/').count() < max_depth && ext_lower(f) == ext)
            .cloned()
            .collect()
    }
    fn safe(&self, rel: &str) -> bool {
        contained(&self.canon, &self.abs(rel))
    }
    fn png_side(&self, rel: &str, fallback: u32) -> u32 {
        if let Some(s) = side_from_name(file_name(rel)) {
            return s;
        }
        image::image_dimensions(self.abs(rel))
            .map(|(w, h)| w.max(h).clamp(16, 1024))
            .unwrap_or(fallback)
    }
}

struct Det {
    kind: Kind,
    evidence: Vec<String>,
    targets: Vec<Target>,
    notes: Vec<String>,
}

impl Det {
    fn new(kind: Kind, evidence: &[&str]) -> Det {
        Det { kind, evidence: evidence.iter().map(|s| s.to_string()).collect(), targets: Vec::new(), notes: Vec::new() }
    }

    fn add(&mut self, ctx: &mut Ctx, rel: &str, label: impl Into<String>, format: Format, profile: Profile, default_on: bool) {
        if !ctx.safe(rel) {
            self.notes.push(format!("Skipped \"{rel}\": it points outside the project folder."));
            return;
        }
        if !ctx.seen.insert(rel.to_ascii_lowercase()) {
            return;
        }
        let exists = match &format {
            Format::AndroidRes { base, .. } => {
                ctx.is_file(&format!("{rel}/mipmap-anydpi-v26/{base}.xml"))
                    || ["png", "webp"].iter().any(|e| ctx.is_file(&format!("{rel}/mipmap-hdpi/{base}.{e}")))
            }
            Format::AppIconSet => ctx.is_file(&format!("{rel}/Contents.json")),
            _ => ctx.is_file(rel),
        };
        self.targets.push(Target {
            id: rel.to_string(),
            label: label.into(),
            rel_path: rel.to_string(),
            format,
            profile,
            exists,
            default_on,
        });
    }

    /// Add a file target, picking the format from the extension.
    fn add_file(&mut self, ctx: &mut Ctx, rel: &str, label: &str, png_fallback: u32, default_on: bool) {
        match ext_lower(rel).as_str() {
            "ico" => self.add(ctx, rel, label, Format::Ico { favicon: false }, Profile::AsIs, default_on),
            "icns" => self.add(ctx, rel, label, Format::Icns, Profile::AsIs, default_on),
            "png" => {
                let side = ctx.png_side(rel, png_fallback);
                self.add(ctx, rel, label, Format::Png { side }, Profile::AsIs, default_on)
            }
            other => self.notes.push(format!(
                "\"{rel}\" is a .{other} file; Simulacrum writes .ico, .icns and .png, so it was left alone."
            )),
        }
    }

    fn into_detection(self) -> Detection {
        Detection { kind: self.kind, label: self.kind.label().to_string(), evidence: self.evidence, targets: self.targets, notes: self.notes }
    }
}

/// Launcher names from the `<application>` tag: `(base, round, note)`.
fn manifest_names(manifest: Option<&str>, manifest_rel: &str) -> (String, Option<String>, Option<String>) {
    let tag = manifest.and_then(|m| start_tag(m, "application"));
    let attr = |name: &str| tag.and_then(|t| xml_attr(t, name));
    let split = |v: &str| v.trim().strip_prefix('@').and_then(|r| r.split_once('/')).map(|(t, n)| (t.to_string(), n.to_string()));
    let mut note = None;
    let base = match attr("android:icon").as_deref().and_then(split) {
        Some((t, n)) if t == "mipmap" => n,
        Some((t, n)) => {
            note = Some(format!(
                "{manifest_rel} uses @{t}/{n} for android:icon. Simulacrum writes mipmap resources — change it to @mipmap/{n}."
            ));
            n
        }
        None => {
            note = Some(format!(
                "{manifest_rel} has no android:icon. Add android:icon=\"@mipmap/ic_launcher\" to <application>."
            ));
            "ic_launcher".to_string()
        }
    };
    let round = attr("android:roundIcon").as_deref().and_then(split).map(|(_, n)| n).filter(|n| *n != base);
    (base, round, note)
}

fn detect_flutter(ctx: &mut Ctx) -> Option<Det> {
    let pubspec = ctx.text("pubspec.yaml")?;
    if !pubspec.lines().any(|l| l.trim_end() == "flutter:" || l.trim() == "sdk: flutter") {
        return None;
    }
    let mut d = Det::new(Kind::Flutter, &["pubspec.yaml"]);
    if ctx.is_dir("android/app/src/main") {
        let m_rel = "android/app/src/main/AndroidManifest.xml";
        let (base, round, note) = manifest_names(ctx.text(m_rel).as_deref(), m_rel);
        d.notes.extend(note);
        d.add(ctx, "android/app/src/main/res", "Android launcher icon", Format::AndroidRes { base, round }, Profile::Android, true);
    }
    for (dir, label, profile) in [
        ("ios/Runner/Assets.xcassets/AppIcon.appiconset", "iOS app icon (opaque, full bleed)", Profile::FullBleedOpaque),
        ("macos/Runner/Assets.xcassets/AppIcon.appiconset", "macOS app icon", Profile::AsIs),
    ] {
        if ctx.is_file(&format!("{dir}/Contents.json")) {
            d.add(ctx, dir, label, Format::AppIconSet, profile, true);
        }
    }
    if ctx.is_dir("web") {
        d.add(ctx, "web/favicon.png", "Web favicon", Format::Png { side: 32 }, Profile::AsIs, true);
        if ctx.is_dir("web/icons") {
            for side in [192, 512] {
                d.add(ctx, &format!("web/icons/Icon-{side}.png"), "Web app icon", Format::Png { side }, Profile::AsIs, true);
            }
            for side in [192, 512] {
                d.add(ctx, &format!("web/icons/Icon-maskable-{side}.png"), "Web maskable icon", Format::Png { side }, Profile::Maskable, true);
            }
        }
    }
    if ctx.is_dir("windows/runner/resources") {
        d.add(ctx, "windows/runner/resources/app_icon.ico", "Windows app icon", Format::Ico { favicon: false }, Profile::AsIs, true);
    }
    Some(d)
}

fn detect_android(ctx: &mut Ctx) -> Option<Det> {
    let gradle = ["", "android/"].iter().find_map(|p| {
        ["settings.gradle", "settings.gradle.kts", "build.gradle", "build.gradle.kts"]
            .iter()
            .map(|f| format!("{p}{f}"))
            .find(|f| ctx.is_file(f))
            .map(|f| (p.to_string(), f))
    });
    let (prefix, gradle_file) = gradle?;
    let manifests: Vec<String> = ctx
        .files_ending("src/main/AndroidManifest.xml")
        .into_iter()
        .filter(|m| m.starts_with(&prefix))
        .collect();
    if manifests.is_empty() {
        return None;
    }
    let mut d = Det::new(Kind::Android, &[&gradle_file]);
    for m in &manifests {
        let text = ctx.text(m);
        let main = parent_rel(m).to_string();
        let module = main.trim_end_matches("/src/main");
        let module = if module == main { "" } else { module };
        let has_app = text.as_deref().and_then(|t| start_tag(t, "application")).is_some_and(|t| t.contains("android:icon"));
        let is_app_module = file_name(module) == "app";
        if !has_app && !is_app_module {
            continue; // library module
        }
        d.evidence.push(m.clone());
        let (base, round, note) = manifest_names(text.as_deref(), m);
        d.notes.extend(note);
        let label = if module.is_empty() { "Launcher icon".to_string() } else { format!("Launcher icon · {module}") };
        d.add(ctx, &format!("{main}/res"), label, Format::AndroidRes { base: base.clone(), round }, Profile::Android, true);
        let store = format!("{main}/{base}-playstore.png");
        let on = ctx.is_file(&store);
        d.add(ctx, &store, "Play Store icon (512 px, opaque)", Format::Png { side: android::PLAY_STORE_SIDE }, Profile::Android, on);
    }
    Some(d)
}

fn detect_tauri(ctx: &mut Ctx) -> Option<Det> {
    let conf = ctx
        .files
        .iter()
        .filter(|f| f.matches('/').count() <= 2 && file_name(f) == "tauri.conf.json")
        .min_by_key(|f| (!f.starts_with("src-tauri/"), f.len()))?
        .clone();
    let v = ctx.json(&conf);
    let base = parent_rel(&conf).to_string();
    let mut d = Det::new(Kind::Tauri, &[&conf]);
    let icons: Vec<String> = v
        .as_ref()
        .and_then(|v| v.pointer("/bundle/icon").or_else(|| v.pointer("/tauri/bundle/icon")))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let icons = if icons.is_empty() {
        d.notes.push(format!("{conf} has no bundle.icon list; using Tauri's default icon files."));
        ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.icns", "icons/icon.ico"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        icons
    };
    for icon in &icons {
        match join_rel(&base, icon) {
            Some(rel) => d.add_file(ctx, &rel, "App icon", 512, true),
            None => d.notes.push(format!("Skipped \"{icon}\": it points outside the project folder.")),
        }
    }
    // `tauri icon` also writes these; keep them in sync when they exist.
    let icons_dir = join_rel(&base, "icons").unwrap_or_else(|| "icons".into());
    let extras: Vec<String> = ctx
        .files
        .iter()
        .filter(|f| parent_rel(f) == icons_dir)
        .filter(|f| {
            let n = file_name(f);
            (n.starts_with("Square") && n.ends_with("Logo.png")) || n == "StoreLogo.png" || n == "icon.png"
        })
        .cloned()
        .collect();
    for rel in extras {
        let side = if file_name(&rel) == "StoreLogo.png" { 50 } else { ctx.png_side(&rel, 512) };
        d.add(ctx, &rel, "Windows Store logo", Format::Png { side }, Profile::AsIs, true);
    }
    Some(d)
}

fn json_icons(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, val) in m {
                if k == "icon" {
                    if let Some(s) = val.as_str() {
                        out.push(s.to_string());
                    }
                }
                json_icons(val, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| json_icons(x, out)),
        _ => {}
    }
}

fn yaml_value(line: &str, key: &str) -> Option<String> {
    let t = line.trim_start().strip_prefix(key)?.trim_start().strip_prefix(':')?;
    let t = t.split(" #").next()?.trim().trim_matches(|c| c == '"' || c == '\'');
    (!t.is_empty()).then(|| t.to_string())
}

fn has_dep(pkg: &Value, name: &str) -> bool {
    ["dependencies", "devDependencies"].iter().any(|k| pkg.get(k).and_then(|d| d.get(name)).is_some())
}

fn detect_electron(ctx: &mut Ctx, pkg: Option<&Value>) -> Option<Det> {
    let configs: Vec<&str> = [
        "electron-builder.yml", "electron-builder.yaml", "electron-builder.json", "electron-builder.json5",
        "electron-builder.config.js", "electron-builder.config.cjs", "electron-builder.config.mjs",
        "electron-builder.config.ts", "forge.config.js", "forge.config.cjs", "forge.config.mjs", "forge.config.ts",
    ]
    .into_iter()
    .filter(|f| ctx.is_file(f))
    .collect();
    let pkg_build = pkg.and_then(|p| p.get("build")).filter(|b| b.is_object());
    let dep = pkg.is_some_and(|p| has_dep(p, "electron"));
    if configs.is_empty() && !dep {
        return None;
    }
    let mut evidence: Vec<&str> = configs.clone();
    if dep || pkg_build.is_some() {
        evidence.push("package.json");
    }
    let mut d = Det::new(Kind::Electron, &evidence);

    let mut build_res = pkg_build
        .and_then(|b| b.pointer("/directories/buildResources"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut icons = Vec::new();
    if let Some(b) = pkg_build {
        json_icons(b, &mut icons);
    }
    for c in &configs {
        let Some(text) = ctx.text(c) else { continue };
        if c.ends_with(".yml") || c.ends_with(".yaml") {
            for line in text.lines() {
                if let Some(v) = yaml_value(line, "icon") {
                    icons.push(v);
                }
                if let Some(v) = yaml_value(line, "buildResources") {
                    build_res = Some(v);
                }
            }
        } else if c.ends_with(".json") || c.ends_with(".json5") {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                json_icons(&v, &mut icons);
                if let Some(br) = v.pointer("/directories/buildResources").and_then(Value::as_str) {
                    build_res = Some(br.to_string());
                }
            }
        } else {
            icons.extend(quoted_after(&text, "icon:", false));
            icons.extend(quoted_after(&text, "icon =", false));
            if let Some(br) = quoted_after(&text, "buildResources:", false).into_iter().next() {
                build_res = Some(br);
            }
        }
    }
    let br = build_res.unwrap_or_else(|| "build".into());
    let br = join_rel("", &br).unwrap_or_else(|| "build".into());

    let mut rels = Vec::new();
    for icon in icons {
        let candidates: Vec<String> = [join_rel(&br, &icon), join_rel("", &icon)].into_iter().flatten().collect();
        let Some(first) = candidates.first().cloned() else {
            d.notes.push(format!("Skipped \"{icon}\": it points outside the project folder."));
            continue;
        };
        let pick = candidates.iter().find(|c| ctx.exists(c)).cloned().unwrap_or_else(|| {
            if icon.contains('/') { candidates.last().cloned().unwrap_or(first.clone()) } else { first.clone() }
        });
        if ctx.is_dir(&pick) {
            d.notes.push(format!("\"{pick}\" is a folder of Linux icons; it was left alone."));
            continue;
        }
        if ext_lower(&pick).is_empty() || !["ico", "icns", "png"].contains(&ext_lower(&pick).as_str()) && !pick.contains('.') {
            // Forge style: no extension, the platform adds .ico/.icns/.png.
            let found: Vec<String> = ["ico", "icns", "png"].iter().map(|e| format!("{pick}.{e}")).filter(|p| ctx.is_file(p)).collect();
            if found.is_empty() {
                rels.push(format!("{pick}.ico"));
                rels.push(format!("{pick}.png"));
            } else {
                rels.extend(found);
            }
        } else {
            rels.push(pick);
        }
    }
    if rels.is_empty() {
        let found: Vec<String> = ["ico", "icns", "png"].iter().map(|e| format!("{br}/icon.{e}")).filter(|p| ctx.is_file(p)).collect();
        if found.is_empty() {
            rels.push(format!("{br}/icon.ico"));
            rels.push(format!("{br}/icon.png"));
        } else {
            rels.extend(found);
        }
    }
    for e in ["ico", "png"] {
        let r = format!("resources/icon.{e}");
        if ctx.is_file(&r) {
            rels.push(r);
        }
    }
    for rel in rels {
        d.add_file(ctx, &rel, "App icon", 512, true);
    }
    Some(d)
}

fn detect_extension(ctx: &mut Ctx, pkg: Option<&Value>) -> Option<Det> {
    let manifest = ["manifest.json", "src/manifest.json", "public/manifest.json", "extension/manifest.json", "app/manifest.json"]
        .into_iter()
        .find_map(|m| ctx.json(m).filter(|v| v.get("manifest_version").is_some()).map(|v| (m, v)));
    let wxt = ctx.first_existing(&["wxt.config.ts", "wxt.config.js", "wxt.config.mts", "wxt.config.mjs"]);
    let plasmo = pkg.is_some_and(|p| has_dep(p, "plasmo"));
    if manifest.is_none() && wxt.is_none() && !plasmo {
        return None;
    }
    let mut evidence = Vec::new();
    if let Some((m, _)) = &manifest {
        evidence.push(*m);
    }
    if let Some(w) = wxt {
        evidence.push(w);
    }
    if plasmo {
        evidence.push("package.json");
    }
    let mut d = Det::new(Kind::BrowserExtension, &evidence);

    let mut declared = false;
    if let Some((m, v)) = &manifest {
        let base = parent_rel(m).to_string();
        let mut by_path: Vec<(String, u32)> = Vec::new();
        let mut put = |p: &str, size: u32| {
            if let Some(e) = by_path.iter_mut().find(|(q, _)| q == p) {
                e.1 = e.1.max(size);
            } else {
                by_path.push((p.to_string(), size));
            }
        };
        let mut collect = |obj: &Value| match obj {
            Value::Object(map) => {
                for (k, p) in map {
                    if let (Ok(size), Some(p)) = (k.parse::<u32>(), p.as_str()) {
                        put(p, size);
                    }
                }
            }
            Value::String(p) => put(p, 128),
            _ => {}
        };
        if let Some(icons) = v.get("icons") {
            collect(icons);
        }
        for key in ["action", "browser_action", "page_action"] {
            if let Some(di) = v.get(key).and_then(|a| a.get("default_icon")) {
                collect(di);
            }
        }
        declared = !by_path.is_empty();
        for (p, size) in by_path {
            match join_rel(&base, p.trim_start_matches('/')) {
                Some(rel) if ext_lower(&rel) == "png" => d.add(ctx, &rel, format!("Extension icon {size}"), Format::Png { side: size }, Profile::AsIs, true),
                Some(rel) => d.notes.push(format!("\"{rel}\" is not a PNG; browsers need PNG extension icons, so it was left alone.")),
                None => d.notes.push(format!("Skipped \"{p}\": it points outside the project folder.")),
            }
        }
        if !declared && wxt.is_none() {
            let dir = join_rel(&base, "icons").unwrap_or_else(|| "icons".into());
            for size in [16, 32, 48, 128] {
                d.add(ctx, &format!("{dir}/icon{size}.png"), format!("Extension icon {size}"), Format::Png { side: size }, Profile::AsIs, true);
            }
            d.notes.push(format!(
                "{m} has no \"icons\" entry. Add \"icons\": {{\"16\": \"icons/icon16.png\", \"32\": \"icons/icon32.png\", \"48\": \"icons/icon48.png\", \"128\": \"icons/icon128.png\"}}."
            ));
        }
    }
    if wxt.is_some() && !declared {
        // WXT discovers public/icon/<size>.png or public/icon-<size>.png.
        let existing: Vec<String> = ctx
            .files
            .iter()
            .filter(|f| {
                let n = file_name(f);
                (parent_rel(f) == "public/icon" && n.trim_end_matches(".png").parse::<u32>().is_ok() && n.ends_with(".png"))
                    || (parent_rel(f) == "public" && n.starts_with("icon-") && n.ends_with(".png"))
            })
            .cloned()
            .collect();
        let rels: Vec<(String, u32)> = if existing.is_empty() {
            [16, 32, 48, 96, 128].iter().map(|s| (format!("public/icon/{s}.png"), *s)).collect()
        } else {
            existing
                .into_iter()
                .filter_map(|f| {
                    let n = file_name(&f).trim_end_matches(".png").trim_start_matches("icon-").to_string();
                    n.parse::<u32>().ok().map(|s| (f, s))
                })
                .collect()
        };
        for (rel, size) in rels {
            d.add(ctx, &rel, format!("Extension icon {size}"), Format::Png { side: size }, Profile::AsIs, true);
        }
    }
    if plasmo {
        d.add(ctx, "assets/icon.png", "Plasmo icon (sizes generated at build)", Format::Png { side: 512 }, Profile::AsIs, true);
    }
    Some(d)
}

fn detect_vscode(ctx: &mut Ctx, pkg: Option<&Value>) -> Option<Det> {
    let pkg = pkg.filter(|p| p.pointer("/engines/vscode").is_some())?;
    let mut d = Det::new(Kind::VsCode, &["package.json"]);
    let icon = pkg.get("icon").and_then(Value::as_str);
    match icon.and_then(|i| join_rel("", i)) {
        Some(rel) if ext_lower(&rel) == "png" => {
            d.add(ctx, &rel, "Marketplace icon", Format::Png { side: 256 }, Profile::AsIs, true);
        }
        _ => {
            let dir = if ctx.is_dir("media") { "media" } else { "images" };
            let rel = format!("{dir}/icon.png");
            d.add(ctx, &rel, "Marketplace icon", Format::Png { side: 256 }, Profile::AsIs, true);
            d.notes.push(match icon {
                Some(i) => format!("package.json \"icon\" is \"{i}\", but the Marketplace only accepts PNG. Point it at \"{rel}\"."),
                None => format!("package.json has no \"icon\" field. Add \"icon\": \"{rel}\"."),
            });
        }
    }
    Some(d)
}

fn detect_xcode(ctx: &mut Ctx) -> Option<Det> {
    let proj = ctx.files_ending(".xcodeproj/project.pbxproj").into_iter().next()?;
    let sets = ctx.files_ending(".appiconset/Contents.json");
    if sets.is_empty() {
        return None;
    }
    let mut d = Det::new(Kind::Xcode, &[parent_rel(&proj)]);
    for cj in sets {
        let dir = parent_rel(&cj).to_string();
        let mac = dir.to_ascii_lowercase().contains("mac");
        let (label, profile) = if mac { ("macOS app icon", Profile::AsIs) } else { ("iOS app icon (opaque, full bleed)", Profile::FullBleedOpaque) };
        d.add(ctx, &dir, label, Format::AppIconSet, profile, true);
    }
    Some(d)
}

fn detect_rust(ctx: &mut Ctx) -> Option<Det> {
    if !ctx.is_file("Cargo.toml") {
        return None;
    }
    let mut d = Det::new(Kind::RustWindows, &["Cargo.toml"]);
    let mut rels = Vec::new();
    let builds: Vec<String> = ctx.files.iter().filter(|f| f.matches('/').count() <= 2 && file_name(f) == "build.rs").cloned().collect();
    for build_rel in builds {
        let Some(build) = ctx.text(&build_rel) else { continue };
        let mut found = quoted_after(&build, "set_icon(", false);
        found.extend(quoted_after(&build, "set_icon_with_id(", false));
        let base = parent_rel(&build_rel).to_string();
        // Any other ".ico" literal (e.g. `root.join("assets/app.ico")`) counts only if that file is really there.
        for q in quoted_strings(&build) {
            if ext_lower(&q) == "ico" && !q.contains(' ') && !found.contains(&q) {
                let hit = [join_rel(&base, &q), join_rel("", &q)].into_iter().flatten().any(|r| ctx.is_file(&r));
                if hit {
                    found.push(q);
                }
            }
        }
        if found.is_empty() {
            if build.contains("set_icon") {
                d.evidence.push(build_rel.clone());
                d.notes.push(format!("{build_rel} generates its icon at compile time, so there is no icon file to replace."));
            }
            continue;
        }
        d.evidence.push(build_rel.clone());
        for f in found {
            let local = join_rel(&base, &f);
            let rel = local.clone().filter(|r| ctx.exists(r)).or_else(|| join_rel("", &f).filter(|r| ctx.exists(r))).or(local);
            rels.extend(rel);
        }
    }
    for rc in ctx.files_with_ext("rc", 3) {
        let Some(text) = ctx.text(&rc) else { continue };
        // Skip template placeholders like "@ICON_PATH@" or "${ICON}".
        let found: Vec<String> = quoted_after(&text, "ICON", true).into_iter().filter(|f| !f.contains(['@', '$', '{'])).collect();
        if found.is_empty() {
            continue;
        }
        d.evidence.push(rc.clone());
        let base = parent_rel(&rc).to_string();
        for f in found {
            let local = join_rel(&base, &f);
            let rel = local.clone().filter(|r| ctx.exists(r)).or_else(|| join_rel("", &f)).or(local);
            rels.extend(rel);
        }
    }
    if d.evidence.len() == 1 {
        return None; // a Rust crate with no Windows icon wiring
    }
    for rel in rels {
        d.add_file(ctx, &rel, "Windows executable icon", 256, true);
    }
    Some(d)
}

/// `/win32icon:<path>` in a build script; resolves `$var` from a `$var = … "path"` line.
/// The first `"…"` or `'…'` string on a line.
fn first_quoted(s: &str) -> Option<String> {
    let i = s.find(|c| c == '"' || c == '\'')?;
    let q = s[i..].chars().next()?;
    let rest = &s[i + 1..];
    let v = rest[..rest.find(q)?].trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// Every single-line `"…"` string literal in a source file.
fn quoted_strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("");
        for (i, part) in line.split('"').enumerate() {
            if i % 2 == 1 && !part.trim().is_empty() {
                out.push(part.trim().to_string());
            }
        }
    }
    out
}

/// `icon = "…"` values from a GloriaSetup `gloria.toml`.
fn gloria_icons(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim_start().strip_prefix("icon")?.trim_start().strip_prefix('=')?;
            first_quoted(rest)
        })
        .collect()
}

fn win32icon(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("/win32icon:") {
        let start = from + pos + "/win32icon:".len();
        from = start;
        let rest = &text[start..];
        let v: String = if let Some(stripped) = rest.strip_prefix('"') {
            stripped.split('"').next().unwrap_or("").to_string()
        } else {
            rest.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == ')').next().unwrap_or("").to_string()
        };
        if let Some(var) = v.strip_prefix('$') {
            let var = var.trim_matches(|c| c == '{' || c == '}');
            let assign = format!("${var}");
            for line in text.lines() {
                let t = line.trim_start();
                if t.len() > assign.len()
                    && t[..assign.len()].eq_ignore_ascii_case(&assign)
                    && t[assign.len()..].trim_start().starts_with('=')
                {
                    if let Some(q) = first_quoted(&t[assign.len()..]) {
                        out.push(q);
                    }
                }
            }
        } else if !v.is_empty() {
            out.push(v);
        }
    }
    out
}

fn detect_dotnet(ctx: &mut Ctx) -> Option<Det> {
    let projs: Vec<String> = ["csproj", "vbproj", "fsproj"].iter().flat_map(|e| ctx.files_with_ext(e, 3)).collect();
    let scripts: Vec<String> = ["ps1", "bat", "cmd"].iter().flat_map(|e| ctx.files_with_ext(e, 2)).collect();
    let mut d = Det::new(Kind::DotNet, &[]);
    let mut rels = Vec::new();
    for p in &projs {
        d.evidence.push(p.clone());
        let Some(text) = ctx.text(p) else { continue };
        let base = parent_rel(p).to_string();
        let mut from = 0;
        while let Some(pos) = text[from..].find("<ApplicationIcon") {
            let s = from + pos;
            let Some(gt) = text[s..].find('>') else { break };
            let body_start = s + gt + 1;
            let Some(end) = text[body_start..].find("</ApplicationIcon>") else { break };
            from = body_start + end;
            let v = text[body_start..body_start + end].trim();
            if v.contains("$(") {
                d.notes.push(format!("{p} builds its icon path from MSBuild properties ({v}); set it manually."));
            } else if let Some(rel) = join_rel(&base, v) {
                rels.push(rel);
            }
        }
    }
    for s in &scripts {
        let Some(text) = ctx.text(s) else { continue };
        let found = win32icon(&text);
        if found.is_empty() {
            continue;
        }
        d.evidence.push(s.clone());
        let base = parent_rel(s).to_string();
        rels.extend(found.iter().filter_map(|f| join_rel(&base, f)));
    }
    if d.evidence.is_empty() {
        return None;
    }
    for rel in rels {
        d.add_file(ctx, &rel, "Application icon", 256, true);
    }
    Some(d)
}

/// `--icon=x`, `--icon x`, `-i x` on a PyInstaller command line.
fn pyinstaller_flag(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines().filter(|l| l.to_ascii_lowercase().contains("pyinstaller")) {
        let mut words = line.split_whitespace().peekable();
        while let Some(w) = words.next() {
            let v = if let Some(v) = w.strip_prefix("--icon=") {
                Some(v.to_string())
            } else if w == "--icon" || w == "-i" {
                words.next().map(str::to_string)
            } else {
                None
            };
            if let Some(v) = v {
                let v = v.trim_matches(|c| c == '"' || c == '\'' || c == '`');
                if !v.is_empty() && v != "NONE" {
                    out.push(v.to_string());
                }
            }
        }
    }
    out
}

fn detect_pyinstaller(ctx: &mut Ctx) -> Option<Det> {
    let specs = ctx.files_with_ext("spec", 2);
    let scripts: Vec<String> = ["ps1", "bat", "cmd", "sh"].iter().flat_map(|e| ctx.files_with_ext(e, 2)).collect();
    let mut d = Det::new(Kind::PyInstaller, &[]);
    let mut rels = Vec::new();
    for s in &specs {
        let Some(text) = ctx.text(s) else { continue };
        if !text.contains("Analysis(") && !text.contains("EXE(") {
            continue;
        }
        d.evidence.push(s.clone());
        let base = parent_rel(s).to_string();
        let mut found = quoted_after(&text, "icon=", false);
        found.extend(quoted_after(&text, "icon =", false));
        rels.extend(found.iter().filter_map(|f| join_rel(&base, f)));
    }
    for s in &scripts {
        let Some(text) = ctx.text(s) else { continue };
        let found = pyinstaller_flag(&text);
        if found.is_empty() {
            continue;
        }
        d.evidence.push(s.clone());
        rels.extend(found.iter().filter_map(|f| join_rel(parent_rel(s), f)));
    }
    if d.evidence.is_empty() {
        return None;
    }
    for rel in rels {
        d.add_file(ctx, &rel, "Executable icon", 256, true);
    }
    Some(d)
}

fn detect_web(ctx: &mut Ctx, host_app: bool) -> Option<Det> {
    let index = ctx.first_existing(&["index.html", "public/index.html", "src/index.html"])?;
    let mut d = Det::new(Kind::Web, &[index]);
    let on = !host_app;
    let out = if ctx.is_dir("public") { "public".to_string() } else { parent_rel(index).to_string() };
    let at = |f: &str| if out.is_empty() { f.to_string() } else { format!("{out}/{f}") };

    d.add(ctx, &at("favicon.ico"), "Favicon", Format::Ico { favicon: true }, Profile::AsIs, on);
    d.add(ctx, &at("apple-touch-icon.png"), "Apple touch icon (opaque)", Format::Png { side: 180 }, Profile::FullBleedOpaque, on);

    // <link rel="icon" href="…"> already in index.html
    let html = ctx.text(index).unwrap_or_default();
    let mut from = 0;
    while let Some(pos) = html[from..].find("<link") {
        let s = from + pos;
        let end = html[s..].find('>').map(|e| s + e + 1).unwrap_or(html.len());
        from = end;
        let tag = &html[s..end];
        let rel_attr = xml_attr(tag, "rel").unwrap_or_default().to_ascii_lowercase();
        if !rel_attr.contains("icon") {
            continue;
        }
        let Some(href) = xml_attr(tag, "href") else { continue };
        let href = href.split(['?', '#']).next().unwrap_or("").to_string();
        let target = if href.starts_with('/') { join_rel(&out, href.trim_start_matches('/')) } else { join_rel(parent_rel(index), &href) };
        let Some(rel) = target else { continue };
        match ext_lower(&rel).as_str() {
            "svg" => d.notes.push(format!(
                "{index} links an SVG favicon ({href}). Simulacrum writes PNG and ICO, so that SVG was left alone."
            )),
            "png" => {
                let side = xml_attr(tag, "sizes").and_then(|s| side_from_name(&s)).unwrap_or_else(|| ctx.png_side(&rel, 32));
                let profile = if rel_attr.contains("apple") { Profile::FullBleedOpaque } else { Profile::AsIs };
                d.add(ctx, &rel, "Linked icon", Format::Png { side }, profile, on);
            }
            "ico" => d.add(ctx, &rel, "Linked favicon", Format::Ico { favicon: true }, Profile::AsIs, on),
            _ => {}
        }
    }

    // An existing web manifest decides the app icons; it is never rewritten.
    let manifest = ctx
        .files
        .iter()
        .filter(|f| parent_rel(f) == out && (f.ends_with(".webmanifest") || file_name(f) == "manifest.json"))
        .find(|f| ctx.json(f).is_some_and(|v| v.get("manifest_version").is_none()))
        .cloned();
    match manifest.and_then(|m| ctx.json(&m).map(|v| (m, v))) {
        Some((m, v)) => {
            d.evidence.push(m.clone());
            for icon in v.get("icons").and_then(Value::as_array).into_iter().flatten() {
                let Some(src) = icon.get("src").and_then(Value::as_str) else { continue };
                let src = src.split(['?', '#']).next().unwrap_or("");
                let rel = if src.starts_with('/') { join_rel(&out, src.trim_start_matches('/')) } else { join_rel(parent_rel(&m), src) };
                let Some(rel) = rel else {
                    d.notes.push(format!("Skipped \"{src}\" from {m}: it points outside the project folder."));
                    continue;
                };
                if ext_lower(&rel) != "png" {
                    continue;
                }
                let side = icon.get("sizes").and_then(Value::as_str).and_then(side_from_name).unwrap_or_else(|| ctx.png_side(&rel, 512));
                let maskable = icon.get("purpose").and_then(Value::as_str).is_some_and(|p| p.contains("maskable"));
                let (label, profile) = if maskable { ("Maskable app icon", Profile::Maskable) } else { ("App icon", Profile::AsIs) };
                d.add(ctx, &rel, label, Format::Png { side }, profile, on);
            }
        }
        None => {
            for (f, side) in [("favicon-16x16.png", 16), ("favicon-32x32.png", 32), ("android-chrome-192x192.png", 192), ("android-chrome-512x512.png", 512)] {
                d.add(ctx, &at(f), "Favicon / app icon", Format::Png { side }, Profile::AsIs, on);
            }
            d.add(ctx, &at("site.webmanifest"), "Web app manifest (new)", Format::WebManifest, Profile::AsIs, on);
            if !html.contains("site.webmanifest") {
                d.notes.push(format!(
                    "{index} doesn't link the favicon set yet. Add <link rel=\"icon\" href=\"/favicon.ico\" sizes=\"any\">, <link rel=\"apple-touch-icon\" href=\"/apple-touch-icon.png\"> and <link rel=\"manifest\" href=\"/site.webmanifest\">."
                ));
            }
        }
    }
    if host_app {
        d.notes.push("This index.html is the app's own frontend, so the favicon set is off by default.".into());
    }
    Some(d)
}

fn detect_repo_icon(ctx: &mut Ctx, folder: &str) -> Option<Det> {
    let root_files: Vec<String> = ctx.files.iter().filter(|f| !f.contains('/')).cloned().collect();
    let named: Vec<String> = root_files
        .iter()
        .filter(|f| {
            let lower = f.to_ascii_lowercase();
            let stem = folder.to_ascii_lowercase();
            lower == format!("{stem}.ico") || lower == format!("{stem}.png")
        })
        .cloned()
        .collect();
    let picks = if named.is_empty() {
        let icos: Vec<String> = root_files.iter().filter(|f| ext_lower(f) == "ico").cloned().collect();
        if icos.len() == 1 { icos } else { Vec::new() }
    } else {
        named
    };
    let picks: Vec<String> = picks.into_iter().filter(|p| !ctx.seen.contains(&p.to_ascii_lowercase())).collect();
    let installer: Vec<String> = ctx
        .text("gloria.toml")
        .map(|t| gloria_icons(&t))
        .unwrap_or_default()
        .iter()
        .filter_map(|f| join_rel("", f))
        .filter(|p| !ctx.seen.contains(&p.to_ascii_lowercase()))
        .collect();
    if picks.is_empty() && installer.is_empty() {
        return None;
    }
    let mut d = Det::new(Kind::RepoIcon, &[]);
    for p in &picks {
        d.evidence.push(p.clone());
        d.add_file(ctx, p, "Repository icon", 512, true);
    }
    if !installer.is_empty() {
        d.evidence.push("gloria.toml".into());
        for p in &installer {
            d.add_file(ctx, p, "Installer icon (gloria.toml)", 256, true);
        }
    }
    Some(d)
}

/// Read `root` and report what kind of project it is and every icon it uses.
pub fn scan(root: &Path) -> AppResult<ProjectReport> {
    let meta = fs::metadata(root).map_err(|e| AppError::Open { path: root.display().to_string(), source: e })?;
    if !meta.is_dir() {
        return Err(AppError::Open {
            path: root.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a folder"),
        });
    }
    let canon = root.canonicalize().map_err(|e| AppError::Open { path: root.display().to_string(), source: e })?;
    let (files, truncated) = walk(root);
    let mut ctx = Ctx { root: root.to_path_buf(), canon, files, seen: HashSet::new(), warnings: Vec::new() };
    if truncated {
        ctx.warnings.push(format!(
            "This folder has more than {MAX_ENTRIES} entries; only part of it was read. Open the project's own folder rather than a parent."
        ));
    }
    let folder = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.display().to_string());
    let pkg = ctx.json("package.json");

    let mut dets: Vec<Det> = Vec::new();
    let flutter = detect_flutter(&mut ctx);
    let is_flutter = flutter.is_some();
    dets.extend(flutter);
    if !is_flutter {
        dets.extend(detect_android(&mut ctx));
    }
    let tauri = detect_tauri(&mut ctx);
    let is_tauri = tauri.is_some();
    dets.extend(tauri);
    let electron = detect_electron(&mut ctx, pkg.as_ref());
    let is_electron = electron.is_some();
    dets.extend(electron);
    dets.extend(detect_extension(&mut ctx, pkg.as_ref()));
    dets.extend(detect_vscode(&mut ctx, pkg.as_ref()));
    if !is_flutter && !is_tauri {
        dets.extend(detect_xcode(&mut ctx));
    }
    dets.extend(detect_rust(&mut ctx));
    dets.extend(detect_dotnet(&mut ctx));
    dets.extend(detect_pyinstaller(&mut ctx));
    let is_ext = dets.iter().any(|d| d.kind == Kind::BrowserExtension || d.kind == Kind::VsCode);
    if !is_ext {
        dets.extend(detect_web(&mut ctx, is_tauri || is_electron));
    }
    dets.extend(detect_repo_icon(&mut ctx, &folder));

    let detections: Vec<Detection> = dets.into_iter().map(Det::into_detection).collect();
    let primary = detections.first().map(|d| d.kind);
    if detections.is_empty() {
        ctx.warnings.push(
            "No known project layout here (Android, Flutter, Tauri, Electron, browser or VS Code extension, Xcode, Rust, .NET, PyInstaller or web).".into(),
        );
    }
    Ok(ProjectReport {
        root: root.display().to_string(),
        name: folder,
        primary,
        primary_label: primary.map(|k| k.label().to_string()),
        detections,
        warnings: ctx.warnings,
    })
}

// ---- export ----------------------------------------------------------------------------

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutcome {
    /// Root-relative paths of every file written.
    pub written: Vec<String>,
    /// How many of `written` replaced an existing file.
    pub replaced: usize,
    /// Root-relative paths removed (duplicate-extension Android resources).
    pub removed: Vec<String>,
    /// Where the originals were copied, if anything was replaced or removed.
    pub backup_dir: Option<String>,
    pub warnings: Vec<String>,
}

pub struct ExportInput<'a> {
    pub source: &'a Source,
    pub settings: &'a Settings,
    pub overrides: Option<&'a HashMap<u32, RgbaImage>>,
    pub mono: Option<&'a Source>,
}

struct Writer {
    root: PathBuf,
    canon: PathBuf,
    backup_dir: PathBuf,
    backed: HashSet<PathBuf>,
    out: ExportOutcome,
}

impl Writer {
    fn rel(&self, path: &Path) -> String {
        path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().replace('\\', "/")
    }

    fn guard(&self, path: &Path) -> AppResult<()> {
        if contained(&self.canon, path) {
            Ok(())
        } else {
            Err(AppError::InvalidSettings(format!("refusing to write outside the project: {}", path.display())))
        }
    }

    fn backup(&mut self, path: &Path) -> AppResult<()> {
        if !self.backed.insert(path.to_path_buf()) {
            return Ok(());
        }
        let dest = self.backup_dir.join(self.rel(path));
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::write(parent, e))?;
        }
        fs::copy(path, &dest).map_err(|e| AppError::write(&dest, e))?;
        self.out.backup_dir = Some(self.backup_dir.display().to_string());
        Ok(())
    }

    fn put(&mut self, path: &Path, bytes: &[u8]) -> AppResult<()> {
        self.guard(path)?;
        if path.is_file() {
            if fs::read(path).is_ok_and(|old| old == bytes) {
                self.out.written.push(self.rel(path));
                return Ok(());
            }
            self.backup(path)?;
            self.out.replaced += 1;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::write(parent, e))?;
        }
        pipeline::write_atomic_file(path, bytes)?;
        self.out.written.push(self.rel(path));
        Ok(())
    }

    fn remove(&mut self, path: &Path) -> AppResult<()> {
        self.guard(path)?;
        self.backup(path)?;
        fs::remove_file(path).map_err(|e| AppError::write(path, e))?;
        self.out.removed.push(self.rel(path));
        Ok(())
    }
}

/// Android resource sink for project mode: backs up what it replaces, never deletes the
/// project's own files as "stale", and removes same-name resources with another extension
/// (`ic_launcher.webp` next to a new `ic_launcher.png` fails the Gradle build).
struct ProjectRes<'w>(&'w mut Writer);

const RES_EXTS: [&str; 6] = ["png", "webp", "jpg", "jpeg", "gif", "xml"];

impl ResSink for ProjectRes<'_> {
    fn write(&mut self, path: &Path, bytes: &[u8]) -> AppResult<()> {
        self.0.put(path, bytes)?;
        let (Some(dir), Some(stem), Some(ext)) = (path.parent(), path.file_stem(), path.extension()) else { return Ok(()) };
        let dir_name = dir.file_name().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
        if !dir_name.starts_with("mipmap") && !dir_name.starts_with("drawable") {
            return Ok(());
        }
        for e in RES_EXTS {
            if ext.eq_ignore_ascii_case(e) {
                continue;
            }
            let sibling = dir.join(format!("{}.{e}", stem.to_string_lossy()));
            if sibling.is_file() {
                self.0.remove(&sibling)?;
            }
        }
        Ok(())
    }
    fn remove_stale(&mut self, _path: &Path) -> AppResult<()> {
        Ok(())
    }
    fn mkdir(&mut self, dir: &Path) -> AppResult<()> {
        self.0.guard(dir)?;
        fs::create_dir_all(dir).map_err(|e| AppError::write(dir, e))
    }
}

/// True when another `values*/*.xml` in `res` already defines `<color name="{color}">`.
fn color_taken(res: &Path, color: &str) -> bool {
    let own = format!("{color}.xml");
    let needle = format!("name=\"{color}\"");
    let Ok(rd) = fs::read_dir(res) else { return false };
    rd.flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("values"))
        .filter_map(|e| fs::read_dir(e.path()).ok())
        .flat_map(|rd| rd.flatten())
        .filter(|f| f.file_name().to_string_lossy() != own)
        .filter_map(|f| read_text(&f.path()))
        .any(|t| t.contains(&needle) && t.contains("<color"))
}

fn encode_rgb_png(img: &RgbaImage) -> AppResult<Vec<u8>> {
    let rgb: Vec<u8> = img.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut buf = Vec::new();
    PngEncoder::new_with_quality(Cursor::new(&mut buf), CompressionType::Best, PngFilter::Adaptive)
        .write_image(&rgb, img.width(), img.height(), image::ExtendedColorType::Rgb8)
        .map_err(|e| AppError::Encode(e.to_string()))?;
    Ok(buf)
}

fn flatten(img: &mut RgbaImage, bg: [u8; 3]) {
    for p in img.pixels_mut() {
        let a = p[3] as f32 / 255.0;
        for c in 0..3 {
            p[c] = (p[c] as f32 * a + bg[c] as f32 * (1.0 - a)).round() as u8;
        }
        p[3] = 255;
    }
}

/// Masters for each look, composed on first use.
struct Renders<'a> {
    input: &'a ExportInput<'a>,
    as_is: Option<Master>,
    bleed: Option<Master>,
    maskable: Option<Master>,
    bg: Option<[u8; 3]>,
    cache: HashMap<(Profile, u32), Vec<u8>>,
}

impl<'a> Renders<'a> {
    fn new(input: &'a ExportInput<'a>) -> Self {
        Renders { input, as_is: None, bleed: None, maskable: None, bg: None, cache: HashMap::new() }
    }

    fn bg(&mut self) -> [u8; 3] {
        let s = self.input.settings;
        *self.bg.get_or_insert_with(|| {
            if s.bg_mode == BgMode::Transparent {
                Plan::new(self.input.source, s, None).bg_rgb()
            } else {
                pipeline::parse_hex_rgb(&s.bg_color1).map(|c| (c * 255.0).round() as u8)
            }
        })
    }

    fn bleed_settings(&mut self, maskable: bool) -> Settings {
        let bg = self.bg();
        let s = self.input.settings;
        let mut b = s.clone();
        b.shape = Shape::None;
        b.padding = if maskable { s.padding.max(10.0) } else { 0.0 };
        b.shadow = false;
        b.badge_mode = BadgeMode::None;
        b.border_width = 0.0;
        if s.bg_mode == BgMode::Transparent {
            b.bg_mode = BgMode::Solid;
            b.bg_color1 = format!("#{:02x}{:02x}{:02x}", bg[0], bg[1], bg[2]);
        }
        b
    }

    fn master(&mut self, p: Profile) -> &Master {
        let input = self.input;
        match p {
            Profile::FullBleedOpaque => {
                if self.bleed.is_none() {
                    let s = self.bleed_settings(false);
                    self.bleed = Some(pipeline::compose(input.source, &s));
                }
                self.bleed.as_ref().unwrap()
            }
            Profile::Maskable => {
                if self.maskable.is_none() {
                    let s = self.bleed_settings(true);
                    self.maskable = Some(pipeline::compose(input.source, &s));
                }
                self.maskable.as_ref().unwrap()
            }
            Profile::AsIs | Profile::Android => self
                .as_is
                .get_or_insert_with(|| pipeline::compose_with_overrides(input.source, input.settings, input.overrides)),
        }
    }

    fn png(&mut self, p: Profile, side: u32) -> AppResult<Vec<u8>> {
        if let Some(b) = self.cache.get(&(p, side)) {
            return Ok(b.clone());
        }
        let opaque = matches!(p, Profile::FullBleedOpaque | Profile::Maskable);
        let bg = if opaque { Some(self.bg()) } else { None };
        let mut img = pipeline::render(self.master(p), side);
        let bytes = match bg {
            Some(bg) => {
                flatten(&mut img, bg);
                encode_rgb_png(&img)?
            }
            None => pipeline::encode_png(&img, false)?,
        };
        self.cache.insert((p, side), bytes.clone());
        Ok(bytes)
    }
}

fn parse_scale(v: Option<&str>) -> f32 {
    v.and_then(|s| s.trim().trim_end_matches('x').parse().ok()).unwrap_or(1.0)
}

fn write_appiconset(w: &mut Writer, r: &mut Renders, dir: &Path) -> AppResult<()> {
    let cj = dir.join("Contents.json");
    let text = read_text(&cj).ok_or_else(|| AppError::Write {
        path: cj.display().to_string(),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "Contents.json is missing"),
    })?;
    let mut v: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Encode(format!("{} is not valid JSON: {e}", cj.display())))?;
    let Some(images) = v.get_mut("images").and_then(Value::as_array_mut) else {
        return Err(AppError::Encode(format!("{} has no \"images\" list", cj.display())));
    };
    let mut changed = false;
    let mut done = HashSet::new();
    for img in images.iter_mut() {
        if img.get("appearances").is_some() {
            continue; // dark / tinted variants stay as they are
        }
        let side: f32 = img
            .get("size")
            .and_then(Value::as_str)
            .and_then(|s| s.split('x').next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        let px = (side * parse_scale(img.get("scale").and_then(Value::as_str))).round() as u32;
        if px == 0 {
            continue;
        }
        let mac = img.get("idiom").and_then(Value::as_str) == Some("mac");
        let name = match img.get("filename").and_then(Value::as_str) {
            Some(f) if !f.contains('/') && !f.contains('\\') && !f.contains("..") => f.to_string(),
            Some(_) => continue,
            None => {
                let f = format!("icon-{px}.png");
                img["filename"] = Value::String(f.clone());
                changed = true;
                f
            }
        };
        if !done.insert(name.clone()) {
            continue;
        }
        let profile = if mac { Profile::AsIs } else { Profile::FullBleedOpaque };
        let bytes = r.png(profile, px)?;
        w.put(&dir.join(&name), &bytes)?;
    }
    if changed {
        let mut json = serde_json::to_string_pretty(&v).map_err(|e| AppError::Encode(e.to_string()))?;
        json.push('\n');
        w.put(&cj, json.as_bytes())?;
    }
    Ok(())
}

/// Write the selected targets of the project at `root`. Targets are re-detected here; ids the
/// scan no longer finds are skipped with a warning.
pub fn export(root: &Path, ids: &[String], input: &ExportInput, backup_dir: &Path) -> AppResult<ExportOutcome> {
    let report = scan(root)?;
    let mut targets = Vec::new();
    let mut missing = Vec::new();
    for id in ids {
        match report.target(id) {
            Some(t) => targets.push(t.clone()),
            None => missing.push(id.clone()),
        }
    }
    if targets.is_empty() {
        return Err(AppError::InvalidSettings("nothing selected to export — tick at least one target".into()));
    }
    let mut w = Writer {
        root: root.to_path_buf(),
        canon: root.canonicalize().map_err(|e| AppError::Open { path: root.display().to_string(), source: e })?,
        backup_dir: backup_dir.to_path_buf(),
        backed: HashSet::new(),
        out: ExportOutcome::default(),
    };
    for m in missing {
        w.out.warnings.push(format!("\"{m}\" is no longer in the project; skipped."));
    }
    let s = input.settings;
    let mut android_settings = s.clone();
    android_settings.android.legacy = true; // pre-API-26 devices still need ic_launcher.png
    let plan = targets
        .iter()
        .any(|t| t.profile == Profile::Android)
        .then(|| Plan::new(input.source, &android_settings, input.mono));
    let mut r = Renders::new(input);
    let mut first_err = None;
    let mut ok = 0usize;

    for t in &targets {
        let path = root.join(&t.rel_path);
        let res: AppResult<()> = (|| match (&t.format, t.profile) {
            (Format::AndroidRes { base, round }, _) => {
                let plan = plan.as_ref().expect("android plan");
                let mut names = ResNames::new(base, round.as_deref());
                if plan.uses_color_resource() && color_taken(&path, &names.color) {
                    let alt = format!("{base}_background_color");
                    w.out.warnings.push(format!(
                        "{} already defines a color named {}; the launcher background uses {alt} instead.",
                        t.rel_path, names.color
                    ));
                    names.color = alt;
                }
                w.guard(&path)?;
                write_res(plan, &path, &names, &mut w)
            }
            (Format::Png { .. }, Profile::Android) => {
                let plan = plan.as_ref().expect("android plan");
                w.put(&path, &android::play_store_png(plan)?)
            }
            (Format::Png { side }, p) => {
                let bytes = r.png(p, *side)?;
                w.put(&path, &bytes)
            }
            (Format::Ico { favicon }, _) => {
                let sizes: &[u32] = if *favicon { &FAVICON_SIZES } else { &s.sizes };
                let bytes = pipeline::build_ico_with_options(r.master(Profile::AsIs), sizes, s.legacy_bmp && !*favicon)?;
                w.put(&path, &bytes)
            }
            (Format::Icns, _) => {
                let bytes = crate::icns::encode(r.master(Profile::AsIs))?;
                w.put(&path, &bytes)
            }
            (Format::AppIconSet, _) => write_appiconset(&mut w, &mut r, &path),
            (Format::WebManifest, _) => {
                if path.exists() {
                    Ok(())
                } else {
                    w.put(&path, WEB_MANIFEST.as_bytes())
                }
            }
        })();
        match res {
            Ok(()) => ok += 1,
            Err(e) => {
                w.out.warnings.push(format!("{}: {e}", t.rel_path));
                first_err.get_or_insert(e);
            }
        }
    }
    if ok == 0 {
        if let Some(e) = first_err {
            return Err(e);
        }
    }
    Ok(w.out)
}

fn write_res(plan: &Plan, res: &Path, names: &ResNames, w: &mut Writer) -> AppResult<()> {
    let mut sink = ProjectRes(w);
    android::write_res(plan, res, names, &mut sink)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_rel_rejects_escapes() {
        assert_eq!(join_rel("app", "../icon.ico").as_deref(), Some("icon.ico"));
        assert_eq!(join_rel("", "../x.ico"), None);
        assert_eq!(join_rel("a", "../../x.ico"), None);
        assert_eq!(join_rel("", "C:/x.ico"), None);
        assert_eq!(join_rel("", "/etc/x"), None);
        assert_eq!(join_rel("src-tauri", "icons\\32x32.png").as_deref(), Some("src-tauri/icons/32x32.png"));
    }

    #[test]
    fn sizes_from_names() {
        assert_eq!(side_from_name("32x32.png"), Some(32));
        assert_eq!(side_from_name("128x128@2x.png"), Some(256));
        assert_eq!(side_from_name("Square44x44Logo.png"), Some(44));
        assert_eq!(side_from_name("android-chrome-192x192.png"), Some(192));
        assert_eq!(side_from_name("icon.png"), None);
    }

    #[test]
    fn quoted_values() {
        assert_eq!(quoted_after("res.set_icon(\"Captura.ico\");", "set_icon(", false), vec!["Captura.ico"]);
        assert_eq!(quoted_after("icon=['icon.ico'],", "icon=", false), vec!["icon.ico"]);
        assert_eq!(quoted_after("IDI_ICON1 ICON \"app.ico\"", "ICON", true), vec!["app.ico"]);
        assert_eq!(win32icon("$iconPath = Join-Path $PSScriptRoot \"B.ico\"\ncsc /win32icon:$iconPath x.cs"), vec!["B.ico"]);
        assert_eq!(gloria_icons("name = \"x\"
icon          = \"assets/icon.ico\"
iconic = \"no\""), vec!["assets/icon.ico"]);
        assert_eq!(quoted_strings("let p = root.join(\"assets/a.ico\"); // \"b.ico\""), vec!["assets/a.ico"]);
        assert_eq!(pyinstaller_flag("pyinstaller --onefile --icon=app.ico main.py"), vec!["app.ico"]);
    }

    #[test]
    fn manifest_launcher_names() {
        let m = r#"<manifest><application android:icon="@mipmap/my_icon" android:roundIcon="@mipmap/my_icon_round" android:label="x"></application></manifest>"#;
        assert_eq!(manifest_names(Some(m), "m"), ("my_icon".into(), Some("my_icon_round".into()), None));
        let same = r#"<application android:icon="@mipmap/ic_launcher" android:roundIcon="@mipmap/ic_launcher">"#;
        assert_eq!(manifest_names(Some(same), "m").1, None);
    }
}

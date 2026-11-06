# Simulacrum — Icon Maker

**Version 1.0.0-beta.1**

Windows desktop app (Tauri v2, Rust + vanilla TypeScript) that turns images into shaped,
multi-size `.ico` files.

- Load PNG / JPG / WEBP / BMP via file picker or drag & drop (multiple files at once).
- Non-square images are centre-cropped to a square. JPEG EXIF rotation is honoured.
- Shapes: **Rounded square** (corner radius 0–50 %, default 22 %), **Circle**, **None**.
- **Padding** 0–30 % per edge: shrinks the artwork inside the canvas; the mask still covers the full canvas.
- Live preview on a checkerboard, plus a preview of **every selected size** at true device pixels (sizes below ~96 px also get a pixelated zoom).
- Pick any of 16, 24, 32, 48, 64, 128, 256 (at least one stays selected).
- Single export (save dialog) or batch export to a folder (progress bar, success/failure summary, `name-1.ico` suffixes instead of overwriting).
- Optional 512×512 PNG next to every `.ico`.
- Settings persist between launches.

## How the pixels are made

All image work runs in Rust (`src-tauri/src/pipeline.rs`). Preview and export use the **same** functions, so the preview matches the output pixel for pixel.

1. Decode → centre-crop → if the side is > 1024, downscale **once** to 1024.
2. Compose a 1024² *master*: Lanczos3-scale the art to `side × (1 − 2·padding)`, centre it on a transparent canvas, then multiply by the shape mask.
3. Every output size (ICO entries, the 512 PNG, the previews) is a **single** Lanczos3 resample of that master. Nothing is ever downscaled from an already downscaled size.

Resampling uses a dedicated separable Lanczos3 (`resample.rs`): weights precomputed per axis, rows split across cores, no clamping (overshoot is resolved once in `from_premul`). Previews go to the webview as raw RGBA over binary IPC and are painted straight into canvases, so there's no PNG encode/decode or base64 on the slider-drag path.

Details:

- **Mask** (`mask.rs`): one rounded-rectangle signed-distance function with a 1-px linear alpha falloff (analytic anti-aliasing). Circle = rounded square at 50 % radius.
- **Premultiplied resampling**: resizing happens in premultiplied float RGBA. With straight alpha, the black RGB of fully transparent pixels leaks into the anti-aliased edge and gives you a dark halo. There's a test for this (`no_dark_fringe_on_edges`).
- **ICO** (`ico.rs`): hand-written container. Every entry is a 32-bit RGBA PNG, sizes are sorted ascending, only the selected sizes are written, nothing above 256 is allowed, and 256 is stored as `0` in the directory.

## Prerequisites (Windows 10/11)

| Tool | Notes |
|---|---|
| **Rust** (stable, MSVC toolchain) | `rustup default stable-x86_64-pc-windows-msvc` |
| **Microsoft C++ Build Tools** | Visual Studio Build Tools → workload *Desktop development with C++* (MSVC + Windows SDK) |
| **Node.js** ≥ 18 | npm is used for the frontend and the Tauri CLI |
| **WebView2 Runtime** | Preinstalled on Windows 11 and current Windows 10. The installer bootstraps it if it's missing. |

The first `tauri build` downloads NSIS and WiX automatically to produce the installers.

## Run & build

```bash
npm install
```

```bash
npm run tauri dev
```

```bash
npm run tauri build
```

Build outputs (under `src-tauri/target/release/`):

- `simulacrum.exe`: standalone app
- `bundle/nsis/Simulacrum_1.0.0-beta.1_x64-setup.exe`: NSIS installer
- `bundle/msi/Simulacrum_1.0.0-beta.1_x64_en-US.msi`: MSI installer

## Tests & verification

```bash
cd src-tauri && cargo test --release
```

The headless CLI runs the same pipeline the app uses:

```bash
cd src-tauri && cargo run --release --example make_ico -- ../test-out/sample.png ../test-out/out.ico --shape rounded --radius 22 --sizes 16,32,256 --png
```

Independent check with Pillow. It reads the raw ICO directory, then asserts the exact size set, 32 bpp, PNG payloads, transparent corners and an opaque centre:

```bash
python -I scripts/verify_ico.py test-out/out.ico --expect 16,32,256
```

With ImageMagick installed:

```bash
magick identify test-out/out.ico
```

`scripts/make_sample.py out.png [w h]` generates a non-square test image.

## Project layout

```
index.html, src/          UI: main.ts (wiring), api.ts (typed invoke), settings.ts (persisted prefs), styles.css
src-tauri/src/
  lib.rs                  Tauri builder, plugin + command registration
  commands.rs             IPC commands, blocking pool / batch thread, progress Channel
  pipeline.rs             load → crop → compose → render
  mask.rs  ico.rs  naming.rs  error.rs
src-tauri/examples/       make_ico CLI
src-tauri/tests/          end-to-end pipeline tests
scripts/                  sample generator + Pillow verifier
```

## App icon & version

- `assets/simulacrum-icon-2048.png` is the master artwork. `src-tauri/icons/icon.ico` was exported from it with Simulacrum itself. The bundle PNGs (`32x32`, `128x128`, `128x128@2x`, `icon.png`) are Lanczos downscales of the master.
- The version is set in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. MSI (WiX) only accepts numeric versions, so `bundle.windows.wix.version` maps `1.0.0-beta.1` to `1.0.0.1` for the MSI's internal ProductVersion; the file name keeps the beta tag. Bump both together.
- The dev server uses port **1430** (not Tauri's default 1420), so it can run next to other Tauri projects.

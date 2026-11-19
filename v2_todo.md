# Simulacrum v2 — Expansion & Improvement Roadmap

This document outlines the architecture, features, and quality-of-life improvements implemented in **Simulacrum v2.0.0**, organized by subsystem and phase.

---

## Phase 1: Input Formats, Ingestion & Clipboard Workflow

- [x] **Native Vector (SVG / SVGZ) Rasterization**
  - Integrated `resvg` / `usvg` (`0.50`) in `src-tauri/Cargo.toml`.
  - Rasterizes SVG/SVGZ inputs directly at target framed scale (`pipeline.rs`) so vector curves remain sharp across zoom and padding adjustments.
- [x] **Expanded Raster Format Support**
  - Enabled `tiff`, `gif`, and `ico` decoding in `image` features and `pipeline::load_bytes` *(AVIF decoding omitted because `image 0.25` requires external native `dav1d`/nasm)*.
- [x] **Existing `.ico` Inspector & Repacker**
  - Dropping an `.ico` inspects embedded directory entries (dimensions, format, bpp) in the stage metadata bar, extracts the highest-resolution frame as the source, and supports per-size slot overrides.
- [x] **Clipboard Paste & Quick Copy (`Ctrl+V` / `Ctrl+C`)**
  - Paste image data directly from the clipboard (`Ctrl+V`) via `load_clipboard_image`.
  - Copy the styled `512×512` PNG directly to the clipboard via `Copy PNG` / `Ctrl+C` (`render_png_bytes`).

---

## Phase 2: Interactive Crop, Framing & Transform Studio

- [x] **Fit vs. Fill (Non-Destructive Aspect Ratio Modes)**
  - **Crop (Fill)** with X/Y focal point panning and zoom (`0.5×`–`4.0×`).
  - **Contain (Fit)** preserving the full non-square image inside the square icon canvas with transparent or colored backdrop letterboxing.
- [x] **Interactive Canvas Manipulation**
  - Drag directly on the preview canvas to pan, scroll wheel to zoom in/out, and double-click to reset framing.
- [x] **Geometric Transforms**
  - Added 90° CW/CCW rotation buttons and horizontal/vertical flip toggles backed by non-destructive `Source::raw` storage (`pipeline.rs`).

---

## Phase 3: SDF Mask Engine & Layer Styling (`mask.rs` & `pipeline.rs`)

- [x] **Continuous-Curvature Squircle & New Analytic Shapes**
  - Added $L_4$ **Squircle**, **Hexagon**, **Octagon**, and **Shield** SDF masks in `mask.rs` with 1-pixel analytic anti-aliasing.
- [x] **Background Fill & Gradient Backdrop**
  - Added `Transparent`, `Solid`, `Linear` (angle-controlled), and `Radial` two-stop backdrops composited inside the SDF shape mask behind artwork.
- [x] **SDF Inner Border / Stroke**
  - Analytic inner border rendered directly from the signed distance field with configurable width (`0–10%`), color, and opacity (`0–100%`).
- [x] **Outer Drop Shadow (Within Canvas Bounds)**
  - Analytic soft drop shadow below the masked shape with automatic inner margin scaling so shadows never clip flat at canvas boundaries.
- [x] **Corner Badge / Overlay Slot**
  - Added `Dot` and `Ring` corner overlays (`Bottom-Right`, `Top-Right`, `Bottom-Left`, `Top-Left`) with configurable color, size, and transparent cutout ring.

---

## Phase 4: Per-Size Optical Tuning & Container Fidelity (`resample.rs` & `ico.rs`)

- [x] **Resolution-Adaptive Micro-Sharpening (Unsharp Mask)**
  - Added `resample::sharpen_premul` weighted by sample opacity and scaled inversely for small icon sizes (`≤ 64px`) via the **Small-size crispness** slider.
- [x] **Per-Size Custom Artwork Override**
  - Click any size in the preview tray to assign a custom image override for that exact slot (`set_size_override`), with a badge and one-click clear button.
- [x] **Hybrid BMP + PNG `.ico` Encoding Option (`ico.rs`)**
  - Added `encode_bmp_dib` (32-bit BGRA `BITMAPINFOHEADER` + bottom-up XOR + 1-bpp AND mask) and `decode_entry`, controlled by **Legacy Win32 BMP+PNG .ico**.
- [x] **Extended Size Selection**
  - Added `20px`, `40px`, `60px`, and `96px` size checkboxes alongside `1024×1024` companion PNG export.

---

## Phase 5: Multi-Platform & Developer Bundle Exports

- [x] **Developer Icon Pack Generator (Tauri / Web / PWA)**
  - **Tauri bundle**: `icon.ico`, `icon.icns`, `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.png`.
  - **Web / PWA bundle**: `favicon.ico`, `favicon-16x16.png`, `favicon-32x32.png`, `apple-touch-icon.png`, `android-chrome-192x192.png`, `android-chrome-512x512.png`, and `site.webmanifest`.
- [x] **macOS `.icns` Export**
  - Implemented zero-dependency `.icns` container encoder/parser in `src-tauri/src/icns.rs` (`icp4` through `ic10`).
- [x] **Apply Icon Directly to Windows Folder**
  - Implemented `src-tauri/src/folder_icon.rs` writing `folder.ico` + `desktop.ini`, setting Windows Hidden/System/ReadOnly attributes, and invoking `SHChangeNotify`.

---

## Phase 6: UI/UX, Context Previews & Batch Power Tools (`src/`)

- [x] **Real-World Context Preview Switcher**
  - Switchable stage & tray backdrop (`Checker`, `Win11 Dark` + live taskbar strip, `Win11 Light` + live taskbar strip, and custom solid color).
- [x] **Before / After & Pixel Grid Inspector**
  - Hold `Space` or the **Compare** button to preview the unstyled source; toggle **Grid** for a `1px` cell overlay on zoomed icon canvases.
- [x] **Named User Presets**
  - Save and delete custom presets in `settings.ts` alongside built-in presets (*Default Rounded*, *Windows 11 App*, *macOS Squircle*, *Circle Avatar*, *Hexagon Badge*, *Raw Pixel Art*).
- [x] **Per-Item Settings Override in Batch Mode**
  - Toggle **Custom for this item** for per-image settings in batch mode, or **Apply to all** to sync settings across the queue.
- [x] **Parallel Batch Worker & Cancellation (`commands.rs`)**
  - Parallelized `export_batch` across worker threads with live progress and a **Cancel batch** button backed by an `AtomicBool`.
- [x] **Extended CLI Utility (`examples/make_ico.rs`)**
  - Updated `make_ico` with `--shape squircle|hexagon|octagon|shield`, `--sharpen`, `--legacy-bmp`, `--icns`, and `--pack tauri|web`.

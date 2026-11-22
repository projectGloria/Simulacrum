//! Android adaptive-icon pack: file layout, pixel sizes, XML validity and framing geometry.

use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use simulacrum_lib::android::{AndroidBg, AndroidMono, DENSITIES, LEGACY_DENSITIES};
use simulacrum_lib::pipeline::{self, Settings, Source};

fn out_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("simulacrum-android-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn solid(side: u32, rgb: [u8; 3]) -> Source {
    pipeline::from_rgba(RgbaImage::from_pixel(side, side, Rgba([rgb[0], rgb[1], rgb[2], 255]))).unwrap()
}

/// A dark round glyph on a flat light tile — the typical "logo on colour" icon.
fn logo_on_tile() -> Source {
    pipeline::from_rgba(RgbaImage::from_fn(512, 512, |x, y| {
        if (x as i32 - 256).pow(2) + (y as i32 - 256).pow(2) < 200 * 200 {
            Rgba([20, 20, 60, 255])
        } else {
            Rgba([240, 200, 40, 255])
        }
    }))
    .unwrap()
}

fn gradient() -> Source {
    pipeline::from_rgba(RgbaImage::from_fn(512, 512, |x, y| Rgba([(x / 2) as u8, (y / 2) as u8, 128, 255])))
        .unwrap()
}

fn settings(f: impl FnOnce(&mut Settings)) -> Settings {
    let mut s = Settings::default();
    f(&mut s);
    s.validated().unwrap()
}

fn dims(p: &Path) -> (u32, u32) {
    image::image_dimensions(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn read(p: &Path) -> RgbaImage {
    image::open(p).unwrap().into_rgba8()
}

/// Every `android:drawable` reference must resolve to files we wrote.
fn assert_xml_refs_exist(res: &Path, xml: &str) {
    let doc = roxmltree::Document::parse(xml).expect("well-formed XML");
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "adaptive-icon");
    let ns = "http://schemas.android.com/apk/res/android";
    let mut seen = Vec::new();
    for node in root.children().filter(|n| n.is_element()) {
        seen.push(node.tag_name().name().to_string());
        let r = node.attribute((ns, "drawable")).expect("android:drawable attribute");
        if let Some(name) = r.strip_prefix("@mipmap/") {
            for (d, _) in DENSITIES {
                let p = res.join(format!("mipmap-{d}/{name}.png"));
                assert!(p.is_file(), "{r} → missing {}", p.display());
            }
        } else if let Some(name) = r.strip_prefix("@color/") {
            let values = std::fs::read_to_string(res.join("values/ic_launcher_background.xml")).unwrap();
            let vdoc = roxmltree::Document::parse(&values).unwrap();
            assert!(
                vdoc.descendants().any(|n| n.has_tag_name("color") && n.attribute("name") == Some(name)),
                "{r} not defined"
            );
        } else {
            panic!("unexpected reference {r}");
        }
    }
    assert_eq!(&seen[..2], ["background", "foreground"]);
}

#[test]
fn color_background_pack_layout_sizes_and_xml() {
    let dir = out_dir("color");
    let written = simulacrum_lib::android::export(&logo_on_tile(), &settings(|_| {}), None, &dir).unwrap();
    let root = dir.join("android");
    let res = root.join("res");
    assert!(written.iter().all(|w| Path::new(w).starts_with(&root)));

    for (d, n) in DENSITIES {
        assert_eq!(dims(&res.join(format!("mipmap-{d}/ic_launcher_foreground.png"))), (n, n), "{d}");
        assert!(!res.join(format!("mipmap-{d}/ic_launcher_background.png")).exists());
        assert!(!res.join(format!("mipmap-{d}/ic_launcher_monochrome.png")).exists());
        assert!(!res.join(format!("mipmap-{d}/ic_launcher.png")).exists(), "legacy is off by default");
    }

    let xml = std::fs::read_to_string(res.join("mipmap-anydpi-v26/ic_launcher.xml")).unwrap();
    let round = std::fs::read_to_string(res.join("mipmap-anydpi-v26/ic_launcher_round.xml")).unwrap();
    assert_eq!(xml, round);
    assert_eq!(
        xml,
        concat!(
            "<adaptive-icon xmlns:android=\"http://schemas.android.com/apk/res/android\">\n",
            "    <background android:drawable=\"@color/ic_launcher_background\" />\n",
            "    <foreground android:drawable=\"@mipmap/ic_launcher_foreground\" />\n",
            "</adaptive-icon>\n",
        )
    );
    assert_xml_refs_exist(&res, &xml);

    // Auto mode samples the flat tile colour.
    let values = std::fs::read_to_string(res.join("values/ic_launcher_background.xml")).unwrap();
    assert!(values.contains("<color name=\"ic_launcher_background\">#FFF0C828</color>"), "{values}");

    let store = read(&root.join("play_store_512.png"));
    assert_eq!(store.dimensions(), (512, 512));
    assert!(store.pixels().all(|p| p[3] == 255), "store icon must be opaque");
    assert_eq!(store.get_pixel(0, 0).0, [240, 200, 40, 255], "no rounded corners");
    assert!(std::fs::read_to_string(root.join("README.txt")).unwrap().contains("app/src/main/"));
}

#[test]
fn image_background_mono_and_legacy() {
    let dir = out_dir("image");
    let s = settings(|s| {
        s.android.mono = AndroidMono::Auto;
        s.android.legacy = true;
    });
    let src = gradient();
    simulacrum_lib::android::export(&src, &s, None, &dir).unwrap();
    let res = dir.join("android/res");

    for (d, n) in DENSITIES {
        for layer in ["foreground", "background", "monochrome"] {
            let p = res.join(format!("mipmap-{d}/ic_launcher_{layer}.png"));
            assert_eq!(dims(&p), (n, n), "{}", p.display());
        }
        let bg = read(&res.join(format!("mipmap-{d}/ic_launcher_background.png")));
        assert!(bg.pixels().all(|p| p[3] == 255), "background layer must be opaque");
    }
    for (d, n) in LEGACY_DENSITIES {
        for name in ["ic_launcher", "ic_launcher_round"] {
            let p = res.join(format!("mipmap-{d}/{name}.png"));
            assert_eq!(dims(&p), (n, n), "{}", p.display());
            let img = read(&p);
            assert_eq!(img.get_pixel(0, 0)[3], 0, "legacy icons are pre-masked");
            assert_eq!(img.get_pixel(n / 2, n / 2)[3], 255);
        }
    }
    assert!(!res.join("values/ic_launcher_background.xml").exists());

    let xml = std::fs::read_to_string(res.join("mipmap-anydpi-v26/ic_launcher.xml")).unwrap();
    assert!(xml.contains("@mipmap/ic_launcher_background"));
    assert!(xml.contains("<monochrome android:drawable=\"@mipmap/ic_launcher_monochrome\" />"));
    assert_xml_refs_exist(&res, &xml);

    let mono = read(&res.join("mipmap-xxxhdpi/ic_launcher_monochrome.png"));
    assert!(mono.pixels().all(|p| p[3] == 0 || (p[0], p[1], p[2]) == (255, 255, 255)), "white silhouette");

    // Re-export as a plain colour pack: stale image / mono / legacy files must disappear.
    let s2 = settings(|s| s.android.bg_mode = AndroidBg::Color);
    simulacrum_lib::android::export(&src, &s2, None, &dir).unwrap();
    for (d, _) in DENSITIES {
        for stale in ["ic_launcher_background", "ic_launcher_monochrome", "ic_launcher", "ic_launcher_round"] {
            assert!(!res.join(format!("mipmap-{d}/{stale}.png")).exists(), "{d}/{stale}");
        }
    }
    let xml = std::fs::read_to_string(res.join("mipmap-anydpi-v26/ic_launcher.xml")).unwrap();
    assert!(!xml.contains("monochrome"));
    assert_xml_refs_exist(&res, &xml);
}

/// Opaque-art bounding box on the exported xxxhdpi foreground.
fn fg_bbox(tag: &str, s: &Settings) -> (u32, u32, u32, u32) {
    let dir = out_dir(tag);
    simulacrum_lib::android::export(&solid(600, [200, 40, 40]), s, None, &dir).unwrap();
    let fg = read(&dir.join("android/res/mipmap-xxxhdpi/ic_launcher_foreground.png"));
    assert_eq!(fg.dimensions(), (432, 432));
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in fg.enumerate_pixels() {
        if p[3] >= 128 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x0, y0, x1, y1)
}

#[test]
fn fit_visible_fills_exactly_the_72dp_area() {
    let (x0, y0, x1, y1) = fg_bbox("fit", &settings(|s| s.android.bg_mode = AndroidBg::Color));
    let (w, h) = (x1 - x0, y1 - y0);
    assert!(w.abs_diff(288) <= 2 && h.abs_diff(288) <= 2, "bbox {w}×{h}");
    assert!(x0.abs_diff(72) <= 1 && y0.abs_diff(72) <= 1, "centred at ({x0},{y0})");

    // Safe zone preset: 66/72 of the visible area → 264 px on 432.
    let (x0, _, x1, _) = fg_bbox("safe", &settings(|s| {
        s.android.bg_mode = AndroidBg::Color;
        s.android.scale = 66.0 / 72.0 * 100.0;
    }));
    assert!((x1 - x0).abs_diff(264) <= 2, "safe-zone width {}", x1 - x0);
}

fn analysis(src: &Source, s: &Settings) -> serde_json::Value {
    let buf = simulacrum_lib::android::preview_payload(src, s, None).unwrap();
    let len = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
    let v: serde_json::Value = serde_json::from_slice(&buf[4..4 + len]).unwrap();
    let side = u32::from_le_bytes(buf[4 + len..8 + len].try_into().unwrap()) as usize;
    let mono_side = u32::from_le_bytes(buf[8 + len + side * side * 4..][..4].try_into().unwrap()) as usize;
    assert_eq!(buf.len(), 12 + len + side * side * 4 + mono_side * mono_side * 4);
    v
}

fn has_warning(v: &serde_json::Value, needle: &str) -> bool {
    v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains(needle))
}

#[test]
fn analysis_warns_about_clipping_and_small_art() {
    let ok = analysis(&logo_on_tile(), &settings(|_| {}));
    assert_eq!(ok["bgMode"], "color");
    assert_eq!(ok["edge"], "flat");
    assert!(ok["warnings"].as_array().unwrap().is_empty(), "{ok}");

    // Round glyph, 72 × 400/512 ≈ 56 dp wide at 100 % (inside the circle); push it off-centre.
    let off = analysis(&logo_on_tile(), &settings(|s| {
        s.android.scale = 130.0;
        s.android.offset_x = 18.0;
        s.android.offset_y = 18.0;
    }));
    assert!(has_warning(&off, "72 dp circle"), "{off}");

    let small = analysis(&logo_on_tile(), &settings(|s| s.android.scale = 60.0));
    assert!(has_warning(&small, "visible width"), "{small}");

    let photo = analysis(&gradient(), &settings(|_| {}));
    assert_eq!(photo["bgMode"], "image");
    assert!(photo["clippedPct"].as_f64().unwrap() > 0.5, "full-bleed art is cut by round masks: {photo}");

    let mono = analysis(&logo_on_tile(), &settings(|s| s.android.mono = AndroidMono::Custom));
    assert!(has_warning(&mono, "No monochrome image"), "{mono}");
}

#[test]
fn settings_without_android_block_use_defaults() {
    let s: Settings = serde_json::from_str(r#"{"shape":"circle"}"#).unwrap();
    assert_eq!(s.android.scale, 100.0);
    let s: Settings =
        serde_json::from_str(r#"{"android":{"scale":500,"offsetX":-99,"bgColor":"zz"}}"#).unwrap();
    let v = s.validated().unwrap();
    assert_eq!((v.android.scale, v.android.offset_x, v.android.bg_color.as_str()), (130.0, -18.0, "#ffffff"));
}

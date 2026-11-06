//! End-to-end: file on disk → load → compose → ICO bytes → parse → decode each entry.

use image::{Rgba, RgbaImage};
use simulacrum_lib::error::AppError;
use simulacrum_lib::ico;
use simulacrum_lib::pipeline::{self, Settings, Shape};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("simulacrum-it-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

fn sample_png(name: &str) -> std::path::PathBuf {
    // Non-square gradient so cropping and resampling both have real work to do.
    let img = RgbaImage::from_fn(640, 400, |x, y| Rgba([(x % 256) as u8, (y % 256) as u8, 180, 255]));
    let p = tmp(name); // per-test file: tests run in parallel
    img.save(&p).unwrap();
    p
}

#[test]
fn ico_contains_exactly_selected_sizes_with_transparent_corners() {
    let src = pipeline::load(&sample_png("sample-a.png")).unwrap();
    let s = Settings { shape: Shape::Rounded, radius: 22.0, padding: 0.0, sizes: vec![256, 16, 32] }
        .validated()
        .unwrap();
    let bytes = pipeline::build_ico(&pipeline::compose(&src, &s), &s.sizes).unwrap();
    let dir = ico::parse(&bytes).unwrap();

    assert_eq!(dir.iter().map(|e| e.width).collect::<Vec<_>>(), vec![16, 32, 256]);
    for e in &dir {
        assert!(e.is_png && e.bpp == 32);
        let img = image::load_from_memory(&bytes[e.data.clone()]).unwrap().into_rgba8();
        assert_eq!(img.dimensions(), (e.width, e.height));
        let n = e.width;
        for (x, y) in [(0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)] {
            assert_eq!(img.get_pixel(x, y)[3], 0, "corner ({x},{y}) at {n}px");
        }
        assert_eq!(img.get_pixel(n / 2, n / 2)[3], 255);
    }
}

#[test]
fn unreadable_and_corrupt_files_error_cleanly() {
    assert!(matches!(pipeline::load(&tmp("does-not-exist.png")), Err(AppError::Open { .. })));

    let txt = tmp("notes.png");
    std::fs::write(&txt, b"definitely not an image").unwrap();
    assert!(matches!(pipeline::load(&txt), Err(AppError::UnsupportedFormat { .. })));

    let broken = tmp("broken.png");
    let mut bytes = std::fs::read(sample_png("sample-b.png")).unwrap();
    bytes.truncate(60); // valid signature, truncated body
    std::fs::write(&broken, bytes).unwrap();
    let r = pipeline::load(&broken);
    assert!(matches!(r, Err(AppError::Decode { .. } | AppError::Open { .. })), "{:?}", r.err());
}

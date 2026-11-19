//! Headless front-end to the app's pipeline, for verification and scripting.
//!
//! cargo run --release --example make_ico -- <input> <out.ico>
//!     [--shape rounded|squircle|circle|hexagon|octagon|shield|none]
//!     [--radius 22] [--padding 0] [--sizes 16,24,32,48,64,128,256]
//!     [--sharpen 0] [--legacy-bmp] [--png] [--icns] [--pack tauri|web]

use std::path::PathBuf;
use std::process::ExitCode;

use simulacrum_lib::pipeline::{self, PackKind, Settings, Shape, DEFAULT_SIZES, PNG_EXPORT_SIDE};
use simulacrum_lib::{error::AppResult, icns, ico};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(args.next().ok_or("missing <input>")?);
    let output = PathBuf::from(args.next().ok_or("missing <out.ico>")?);
    let mut s = Settings { sizes: DEFAULT_SIZES.to_vec(), ..Settings::default() };
    let mut png = false;
    let mut emit_icns = false;
    let mut pack: Option<PackKind> = None;

    while let Some(flag) = args.next() {
        let mut val = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--shape" => {
                s.shape = match val()?.as_str() {
                    "rounded" => Shape::Rounded,
                    "squircle" => Shape::Squircle,
                    "circle" => Shape::Circle,
                    "hexagon" => Shape::Hexagon,
                    "octagon" => Shape::Octagon,
                    "shield" => Shape::Shield,
                    "none" => Shape::None,
                    o => return Err(format!("unknown shape {o}")),
                }
            }
            "--radius" => s.radius = val()?.parse().map_err(|e| format!("--radius: {e}"))?,
            "--padding" => s.padding = val()?.parse().map_err(|e| format!("--padding: {e}"))?,
            "--sharpen" => s.sharpen = val()?.parse().map_err(|e| format!("--sharpen: {e}"))?,
            "--legacy-bmp" => s.legacy_bmp = true,
            "--sizes" => {
                s.sizes = val()?
                    .split(',')
                    .map(|v| v.trim().parse().map_err(|e| format!("--sizes: {e}")))
                    .collect::<Result<_, _>>()?
            }
            "--png" => png = true,
            "--icns" => emit_icns = true,
            "--pack" => {
                pack = Some(match val()?.as_str() {
                    "tauri" => PackKind::Tauri,
                    "web" => PackKind::Web,
                    o => return Err(format!("unknown pack {o}")),
                })
            }
            o => return Err(format!("unknown flag {o}")),
        }
    }

    export(&input, &output, &s, png, emit_icns, pack).map_err(|e| e.to_string())?;

    if pack.is_none() {
        let bytes = std::fs::read(&output).map_err(|e| e.to_string())?;
        for e in ico::parse(&bytes)? {
            println!("{}x{} bpp={} png={} bytes={}", e.width, e.height, e.bpp, e.is_png, e.data.len());
        }
    }
    Ok(())
}

fn export(
    input: &std::path::Path,
    output: &std::path::Path,
    s: &Settings,
    png: bool,
    emit_icns: bool,
    pack: Option<PackKind>,
) -> AppResult<()> {
    let s = s.validated()?;
    let src = pipeline::load(input)?;
    if let Some(w) = src.warning() {
        eprintln!("warning: {w}");
    }
    let master = pipeline::compose(&src, &s);

    if let Some(kind) = pack {
        std::fs::create_dir_all(output)
            .map_err(|e| simulacrum_lib::error::AppError::write(output, e))?;
        for path in pipeline::export_pack(&master, output, kind)? {
            println!("wrote {path}");
        }
        return Ok(());
    }

    std::fs::write(output, pipeline::build_ico_with_options(&master, &s.sizes, s.legacy_bmp)?)
        .map_err(|e| simulacrum_lib::error::AppError::write(output, e))?;
    if png {
        let p = output.with_extension("png");
        std::fs::write(&p, pipeline::encode_png(&pipeline::render(&master, PNG_EXPORT_SIDE), false)?)
            .map_err(|e| simulacrum_lib::error::AppError::write(&p, e))?;
    }
    if emit_icns {
        let p = output.with_extension("icns");
        std::fs::write(&p, icns::encode(&master)?)
            .map_err(|e| simulacrum_lib::error::AppError::write(&p, e))?;
    }
    Ok(())
}

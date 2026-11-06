//! Headless front-end to the app's pipeline, for verification and scripting.
//!
//! cargo run --release --example make_ico -- <input> <out.ico>
//!     [--shape rounded|circle|none] [--radius 22] [--padding 0]
//!     [--sizes 16,24,32,48,64,128,256] [--png]

use std::path::PathBuf;
use std::process::ExitCode;

use simulacrum_lib::pipeline::{self, Settings, Shape, ALLOWED_SIZES, PNG_EXPORT_SIDE};
use simulacrum_lib::{error::AppResult, ico};

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
    let mut s = Settings { shape: Shape::Rounded, radius: 22.0, padding: 0.0, sizes: ALLOWED_SIZES.to_vec() };
    let mut png = false;
    while let Some(flag) = args.next() {
        let mut val = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--shape" => {
                s.shape = match val()?.as_str() {
                    "rounded" => Shape::Rounded,
                    "circle" => Shape::Circle,
                    "none" => Shape::None,
                    o => return Err(format!("unknown shape {o}")),
                }
            }
            "--radius" => s.radius = val()?.parse().map_err(|e| format!("--radius: {e}"))?,
            "--padding" => s.padding = val()?.parse().map_err(|e| format!("--padding: {e}"))?,
            "--sizes" => {
                s.sizes = val()?
                    .split(',')
                    .map(|v| v.trim().parse().map_err(|e| format!("--sizes: {e}")))
                    .collect::<Result<_, _>>()?
            }
            "--png" => png = true,
            o => return Err(format!("unknown flag {o}")),
        }
    }
    export(&input, &output, &s, png).map_err(|e| e.to_string())?;

    let bytes = std::fs::read(&output).map_err(|e| e.to_string())?;
    for e in ico::parse(&bytes)? {
        println!("{}x{} bpp={} png={} bytes={}", e.width, e.height, e.bpp, e.is_png, e.data.len());
    }
    Ok(())
}

fn export(input: &std::path::Path, output: &std::path::Path, s: &Settings, png: bool) -> AppResult<()> {
    let s = s.validated()?;
    let src = pipeline::load(input)?;
    if let Some(w) = src.warning() {
        eprintln!("warning: {w}");
    }
    let master = pipeline::compose(&src, &s);
    std::fs::write(output, pipeline::build_ico(&master, &s.sizes)?)
        .map_err(|e| simulacrum_lib::error::AppError::write(output, e))?;
    if png {
        let p = output.with_extension("png");
        std::fs::write(&p, pipeline::encode_png(&pipeline::render(&master, PNG_EXPORT_SIDE), false)?)
            .map_err(|e| simulacrum_lib::error::AppError::write(&p, e))?;
    }
    Ok(())
}

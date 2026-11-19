//! Zero-dependency macOS `.icns` container encoder/parser.
//!
//! Packs PNG payloads at 16, 32, 64, 128, 256, 512, and 1024 px using standard Apple
//! icon type OSTypes (`icp4` through `ic10`).

use crate::error::AppResult;
use crate::pipeline::{self, Master};

const CHUNKS: &[(u32, [u8; 4])] = &[
    (16, *b"icp4"),
    (32, *b"icp5"),
    (64, *b"icp6"),
    (128, *b"ic07"),
    (256, *b"ic08"),
    (512, *b"ic09"),
    (1024, *b"ic10"),
];

pub fn encode(master: &Master) -> AppResult<Vec<u8>> {
    let mut chunks = Vec::with_capacity(CHUNKS.len());
    let mut total = 8usize; // "icns" + u32 total size
    for &(side, tag) in CHUNKS {
        let img = pipeline::render(master, side);
        let png = pipeline::encode_png(&img, false)?;
        total += 8 + png.len();
        chunks.push((tag, png));
    }

    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(total as u32).to_be_bytes());
    for (tag, png) in chunks {
        out.extend_from_slice(&tag);
        out.extend_from_slice(&((8 + png.len()) as u32).to_be_bytes());
        out.extend_from_slice(&png);
    }
    Ok(out)
}

/// Parse an `.icns` container, returning `(chunk_type, payload_len)` pairs.
pub fn parse(bytes: &[u8]) -> Result<Vec<([u8; 4], usize)>, String> {
    if bytes.len() < 8 || &bytes[..4] != b"icns" {
        return Err("not an ICNS file".into());
    }
    let total = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    if total > bytes.len() || total < 8 {
        return Err("invalid ICNS length".into());
    }
    let mut out = Vec::new();
    let mut off = 8usize;
    while off + 8 <= total {
        let tag = [bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]];
        let len = u32::from_be_bytes([bytes[off + 4], bytes[off + 5], bytes[off + 6], bytes[off + 7]]) as usize;
        if len < 8 || off + len > total {
            return Err("truncated ICNS chunk".into());
        }
        out.push((tag, len - 8));
        off += len;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn encodes_and_parses_all_standard_chunks() {
        let src = pipeline::from_rgba(RgbaImage::from_pixel(256, 256, Rgba([80, 140, 220, 255]))).unwrap();
        let master = pipeline::compose(&src, &pipeline::Settings::default());
        let bytes = encode(&master).unwrap();
        let parsed = parse(&bytes).unwrap();
        let tags: Vec<[u8; 4]> = parsed.iter().map(|(t, _)| *t).collect();
        assert_eq!(tags, vec![*b"icp4", *b"icp5", *b"icp6", *b"ic07", *b"ic08", *b"ic09", *b"ic10"]);
        assert!(parsed.iter().all(|(_, len)| *len > 8));
    }
}

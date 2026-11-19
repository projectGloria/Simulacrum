//! Minimal ICO container writer/reader. Supports both Vista+ PNG payloads and legacy
//! Win32 32-bit BGRA `BITMAPINFOHEADER` DIB payloads (XOR + 1-bpp AND mask).

use image::{ImageFormat, Rgba, RgbaImage};

const HEADER: usize = 6;
const ENTRY: usize = 16;

/// Encode a 32-bit BGRA DIB (`BITMAPINFOHEADER` + bottom-up XOR pixels + bottom-up 1-bpp AND mask)
/// for legacy Win32 `.ico` compatibility (typically 16–48 px).
pub fn encode_bmp_dib(img: &RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let and_stride = ((w as usize) + 31) / 32 * 4;
    let xor_size = (w as usize) * (h as usize) * 4;
    let and_size = and_stride * (h as usize);
    let image_size = (xor_size + and_size) as u32;

    let mut out = Vec::with_capacity(40 + xor_size + and_size);
    // BITMAPINFOHEADER (40 bytes)
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth
    out.extend_from_slice(&((h * 2) as i32).to_le_bytes()); // biHeight (XOR + AND)
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // biCompression (BI_RGB)
    out.extend_from_slice(&image_size.to_le_bytes()); // biSizeImage
    out.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    out.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

    // Bottom-up BGRA XOR mask.
    for y in (0..h).rev() {
        for x in 0..w {
            let p = img.get_pixel(x, y).0;
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }

    // Bottom-up 1-bpp AND mask (bit=1 means transparent).
    for y in (0..h).rev() {
        let mut row = vec![0u8; and_stride];
        for x in 0..w {
            if img.get_pixel(x, y).0[3] == 0 {
                row[(x as usize) / 8] |= 1 << (7 - ((x as usize) % 8));
            }
        }
        out.extend_from_slice(&row);
    }

    out
}

/// `entries`: (side, payload_bytes). Caller guarantees 1 ≤ side ≤ 256.
pub fn encode(entries: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let payload: usize = entries.iter().map(|(_, p)| p.len()).sum();
    let mut out = Vec::with_capacity(HEADER + ENTRY * entries.len() + payload);
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type 1 = icon (2 = cursor)
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());

    let mut offset = (HEADER + ENTRY * entries.len()) as u32;
    for (side, data) in entries {
        debug_assert!((1..=256).contains(side));
        // Width/height are a single byte each; 0 means 256.
        let dim = if *side >= 256 { 0u8 } else { *side as u8 };
        out.push(dim); // width
        out.push(dim); // height
        out.push(0); // palette colours (0 = no palette)
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in entries {
        out.extend_from_slice(data);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub width: u32,
    pub height: u32,
    pub bpp: u16,
    pub is_png: bool,
    pub data: std::ops::Range<usize>,
}

/// Parse the directory and bounds-check each payload.
pub fn parse(bytes: &[u8]) -> Result<Vec<DirEntry>, String> {
    let u16_at = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    if bytes.len() < HEADER || u16_at(0) != 0 || u16_at(2) != 1 {
        return Err("not an ICO file".into());
    }
    let count = u16_at(4) as usize;
    if count == 0 {
        return Err("ICO contains zero entries".into());
    }
    if bytes.len() < HEADER + count * ENTRY {
        return Err("truncated directory".into());
    }
    (0..count)
        .map(|i| {
            let e = HEADER + i * ENTRY;
            let dim = |b: u8| if b == 0 { 256 } else { b as u32 };
            let (size, off) = (u32_at(e + 8) as usize, u32_at(e + 12) as usize);
            let end = off.checked_add(size).ok_or_else(|| format!("entry {i} overflow"))?;
            if end > bytes.len() {
                return Err(format!("entry {i} points past end of file"));
            }
            let data = off..end;
            let payload = &bytes[data.clone()];
            let is_png = payload.starts_with(b"\x89PNG\r\n\x1a\n");
            let mut bpp = u16_at(e + 6);
            if !is_png && payload.len() >= 16 {
                let dib_bpp = u16::from_le_bytes([payload[14], payload[15]]);
                if dib_bpp > 0 {
                    bpp = dib_bpp;
                }
            }
            Ok(DirEntry {
                width: dim(bytes[e]),
                height: dim(bytes[e + 1]),
                bpp,
                is_png,
                data,
            })
        })
        .collect()
}

/// Decode a single `DirEntry` payload (PNG or 32-bit DIB BMP) into straight-alpha `RgbaImage`.
pub fn decode_entry(bytes: &[u8], entry: &DirEntry) -> Result<RgbaImage, String> {
    let slice = bytes.get(entry.data.clone()).ok_or_else(|| "entry out of range".to_string())?;
    if entry.is_png {
        return image::load_from_memory_with_format(slice, ImageFormat::Png)
            .map(|d| d.into_rgba8())
            .map_err(|e| e.to_string());
    }

    if slice.len() < 40 {
        return Err("truncated BMP DIB header".into());
    }
    let u16_at = |o: usize| u16::from_le_bytes([slice[o], slice[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([slice[o], slice[o + 1], slice[o + 2], slice[o + 3]]);
    let i32_at = |o: usize| i32::from_le_bytes([slice[o], slice[o + 1], slice[o + 2], slice[o + 3]]);

    let header_size = u32_at(0) as usize;
    if header_size < 40 || header_size > slice.len() {
        return Err("unsupported BMP DIB header".into());
    }
    let w = i32_at(4);
    let h_dbl = i32_at(8);
    let bpp = u16_at(14);
    let compression = u32_at(16);
    if w <= 0 || h_dbl == 0 || compression != 0 {
        return Err(format!("unsupported DIB geometry ({w}x{h_dbl}, comp={compression})"));
    }
    let w = w as u32;
    let h = (h_dbl.unsigned_abs() / 2).max(1);
    if bpp != 32 {
        return Err(format!("unsupported DIB bit depth {bpp} (only 32-bit supported directly)"));
    }
    let need = header_size + (w as usize) * (h as usize) * 4;
    if slice.len() < need {
        return Err("truncated 32-bit DIB pixel data".into());
    }

    let bottom_up = h_dbl > 0;
    let mut img = RgbaImage::new(w, h);
    let px = &slice[header_size..need];
    for y in 0..h {
        let src_y = if bottom_up { h - 1 - y } else { y };
        let row = &px[(src_y as usize) * (w as usize) * 4..];
        for x in 0..w {
            let o = (x as usize) * 4;
            img.put_pixel(x, y, Rgba([row[o + 2], row[o + 1], row[o], row[o + 3]]));
        }
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_256_as_zero() {
        let png = b"\x89PNG\r\n\x1a\nfake".to_vec();
        let bytes = encode(&[(16, png.clone()), (256, png.clone())]);
        assert_eq!(bytes[6], 16); // first entry width byte
        assert_eq!(bytes[6 + 16], 0); // second entry: 256 stored as 0
        let dir = parse(&bytes).unwrap();
        assert_eq!(dir.iter().map(|e| e.width).collect::<Vec<_>>(), vec![16, 256]);
        assert!(dir.iter().all(|e| e.is_png && e.bpp == 32));
        assert_eq!(&bytes[dir[1].data.clone()], &png[..]);
    }

    #[test]
    fn bmp_dib_round_trip() {
        let img = RgbaImage::from_fn(16, 16, |x, y| {
            if x == 0 && y == 0 {
                Rgba([0, 0, 0, 0])
            } else {
                Rgba([12, 120, 240, 255])
            }
        });
        let dib = encode_bmp_dib(&img);
        let bytes = encode(&[(16, dib)]);
        let dir = parse(&bytes).unwrap();
        assert_eq!(dir.len(), 1);
        assert!(!dir[0].is_png);
        assert_eq!(dir[0].bpp, 32);
        let decoded = decode_entry(&bytes, &dir[0]).unwrap();
        assert_eq!(decoded, img);
    }
}

//! Minimal ICO container writer/reader. Every entry is a PNG blob (Vista+ reads PNG entries at
//! any size; 256 is effectively required to be PNG).

const HEADER: usize = 6;
const ENTRY: usize = 16;

/// `entries`: (side, png_bytes). Caller guarantees 1 ≤ side ≤ 256.
pub fn encode(entries: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let payload: usize = entries.iter().map(|(_, p)| p.len()).sum();
    let mut out = Vec::with_capacity(HEADER + ENTRY * entries.len() + payload);
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type 1 = icon (2 = cursor)
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());

    let mut offset = (HEADER + ENTRY * entries.len()) as u32;
    for (side, png) in entries {
        debug_assert!((1..=256).contains(side));
        // Width/height are a single byte each; 0 means 256.
        let dim = if *side >= 256 { 0u8 } else { *side as u8 };
        out.push(dim); // width
        out.push(dim); // height
        out.push(0); // palette colours (0 = no palette)
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in entries {
        out.extend_from_slice(png);
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

/// Parse the directory and bounds-check each payload — used by tests and the example CLI.
pub fn parse(bytes: &[u8]) -> Result<Vec<DirEntry>, String> {
    let u16_at = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    if bytes.len() < HEADER || u16_at(0) != 0 || u16_at(2) != 1 {
        return Err("not an ICO file".into());
    }
    let count = u16_at(4) as usize;
    if bytes.len() < HEADER + count * ENTRY {
        return Err("truncated directory".into());
    }
    (0..count)
        .map(|i| {
            let e = HEADER + i * ENTRY;
            let dim = |b: u8| if b == 0 { 256 } else { b as u32 };
            let (size, off) = (u32_at(e + 8) as usize, u32_at(e + 12) as usize);
            let data = off..off + size;
            if data.end > bytes.len() {
                return Err(format!("entry {i} points past end of file"));
            }
            Ok(DirEntry {
                width: dim(bytes[e]),
                height: dim(bytes[e + 1]),
                bpp: u16_at(e + 6),
                is_png: bytes[data.clone()].starts_with(b"\x89PNG\r\n\x1a\n"),
                data,
            })
        })
        .collect()
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
}

//! Collision-safe output naming for batch export.

use std::collections::HashSet;
use std::path::Path;

/// Windows device names that can't be used as file stems even with an extension.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];
const ILLEGAL: &str = "<>:\"/\\|?*";

pub fn sanitize_stem(stem: &str) -> String {
    let cleaned: String =
        stem.chars().map(|c| if c.is_control() || ILLEGAL.contains(c) { '_' } else { c }).collect();
    // Windows silently strips trailing dots/spaces, which would make "a." and "a" collide.
    let cleaned = cleaned.trim_end_matches(['.', ' ']).trim_start().to_string();
    if cleaned.is_empty() {
        return "icon".into();
    }
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&cleaned)) {
        return format!("{cleaned}_");
    }
    cleaned
}

/// Returns a stem such that `<stem>.ico` (and `<stem>.png` if `with_png`) neither exists in
/// `dir` nor was already handed out in this batch. `taken` is lowercase because NTFS is
/// case-insensitive. Appends `-1`, `-2`, … on collision — never overwrites.
pub fn unique_stem(dir: &Path, stem: &str, with_png: bool, taken: &mut HashSet<String>) -> String {
    let base = sanitize_stem(stem);
    let free = |s: &str| {
        !taken.contains(&s.to_lowercase())
            && !dir.join(format!("{s}.ico")).exists()
            && !(with_png && dir.join(format!("{s}.png")).exists())
    };
    let chosen = std::iter::once(base.clone())
        .chain((1u32..).map(|i| format!("{base}-{i}")))
        .find(|s| free(s))
        .expect("unbounded iterator always yields a free name");
    taken.insert(chosen.to_lowercase());
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize_stem("a:b*c?. "), "a_b_c_");
        assert_eq!(sanitize_stem("con"), "con_");
        assert_eq!(sanitize_stem("..."), "icon");
    }

    #[test]
    fn suffixes_on_collision() {
        let dir = std::env::temp_dir().join(format!("simulacrum-naming-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("logo.ico"), b"x").unwrap();
        let mut taken = HashSet::new();
        assert_eq!(unique_stem(&dir, "logo", false, &mut taken), "logo-1");
        assert_eq!(unique_stem(&dir, "Logo", false, &mut taken), "Logo-2");
        assert_eq!(unique_stem(&dir, "other", false, &mut taken), "other");
        std::fs::write(dir.join("p.png"), b"x").unwrap();
        assert_eq!(unique_stem(&dir, "p", true, &mut taken), "p-1");
        assert_eq!(unique_stem(&dir, "q", false, &mut taken), "q");
        std::fs::remove_dir_all(&dir).ok();
    }
}

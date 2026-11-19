//! Windows Explorer folder icon applicator (`folder.ico` + `desktop.ini` + shell attributes).

use std::path::Path;

use crate::error::{AppError, AppResult};

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    const FILE_ATTRIBUTE_READONLY: u32 = 0x01;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x02;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x04;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
    const SHCNF_IDLIST: u32 = 0x0000;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileAttributesW(lp_file_name: *const u16, dw_file_attributes: u32) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHChangeNotify(w_event_id: i32, u_flags: u32, dw_item1: *const c_void, dw_item2: *const c_void);
    }

    fn to_wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn clear_attrs(path: &Path) {
        if path.exists() {
            let w = to_wide(path);
            unsafe {
                let _ = SetFileAttributesW(w.as_ptr(), FILE_ATTRIBUTE_NORMAL);
            }
        }
    }

    pub fn mark_hidden_system(path: &Path) {
        let w = to_wide(path);
        unsafe {
            let _ = SetFileAttributesW(w.as_ptr(), FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM);
        }
    }

    pub fn mark_folder_readonly(folder: &Path) {
        let w = to_wide(folder);
        unsafe {
            let _ = SetFileAttributesW(w.as_ptr(), FILE_ATTRIBUTE_READONLY);
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, std::ptr::null(), std::ptr::null());
        }
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let tmp = path.with_extension("simulacrum-tmp");
    std::fs::write(&tmp, bytes).map_err(|e| AppError::write(path, e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        AppError::write(path, e)
    })
}

pub fn apply_to_folder(folder: &Path, ico_bytes: &[u8]) -> AppResult<String> {
    if !folder.is_dir() {
        return Err(AppError::Write {
            path: folder.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "folder does not exist"),
        });
    }

    let ico_path = folder.join("folder.ico");
    let ini_path = folder.join("desktop.ini");

    #[cfg(windows)]
    {
        win::clear_attrs(&ico_path);
        win::clear_attrs(&ini_path);
    }

    write_atomic(&ico_path, ico_bytes)?;
    let ini = "[.ShellClassInfo]\r\nIconResource=folder.ico,0\r\n";
    write_atomic(&ini_path, ini.as_bytes())?;

    #[cfg(windows)]
    {
        win::mark_hidden_system(&ico_path);
        win::mark_hidden_system(&ini_path);
        win::mark_folder_readonly(folder);
    }

    Ok(folder.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_folder_ico_and_desktop_ini() {
        let dir = std::env::temp_dir().join(format!("simulacrum-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let fake_ico = b"\x00\x00\x01\x00\x01\x00";
        let out = apply_to_folder(&dir, fake_ico).unwrap();
        assert_eq!(out, dir.display().to_string());
        assert_eq!(std::fs::read(dir.join("folder.ico")).unwrap(), fake_ico);
        let ini = std::fs::read_to_string(dir.join("desktop.ini")).unwrap();
        assert!(ini.contains("[.ShellClassInfo]"));
        assert!(ini.contains("IconResource=folder.ico,0"));

        // Re-applying over hidden/system files must also succeed.
        apply_to_folder(&dir, fake_ico).unwrap();

        #[cfg(windows)]
        {
            win::clear_attrs(&dir.join("folder.ico"));
            win::clear_attrs(&dir.join("desktop.ini"));
            win::clear_attrs(&dir);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

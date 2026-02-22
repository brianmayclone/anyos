use alloc::string::String;
use crate::lib;

pub struct FileDialog;

impl FileDialog {
    /// Show an Open Folder dialog. Returns `Some(path)` or `None` if cancelled.
    pub fn open_folder() -> Option<String> {
        let mut buf = [0u8; 257];
        let len = (lib().open_folder_fn)(buf.as_mut_ptr(), buf.len() as u32);
        if len == 0 { return None; }
        let s = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        Some(String::from(s))
    }

    /// Show an Open File dialog. Returns `Some(path)` or `None` if cancelled.
    pub fn open_file() -> Option<String> {
        let mut buf = [0u8; 257];
        let len = (lib().open_file_fn)(buf.as_mut_ptr(), buf.len() as u32);
        if len == 0 { return None; }
        let s = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        Some(String::from(s))
    }

    /// Show a Save File dialog. Returns `Some(path)` or `None` if cancelled.
    pub fn save_file(default_name: &str) -> Option<String> {
        let mut buf = [0u8; 257];
        let len = (lib().save_file_fn)(
            buf.as_mut_ptr(),
            buf.len() as u32,
            default_name.as_ptr(),
            default_name.len() as u32,
        );
        if len == 0 { return None; }
        let s = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        Some(String::from(s))
    }

    /// Show a Create Folder dialog. Returns `Some(path)` of created folder or `None` if cancelled.
    pub fn create_folder() -> Option<String> {
        let mut buf = [0u8; 257];
        let len = (lib().create_folder_fn)(buf.as_mut_ptr(), buf.len() as u32);
        if len == 0 { return None; }
        let s = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        Some(String::from(s))
    }
}

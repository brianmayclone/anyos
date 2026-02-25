#![no_std]
#![no_main]

use anyos_std::{fs, process};
use anyos_std::icons::MimeDb;

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let path = raw.trim();

    if path.is_empty() {
        anyos_std::println!("Usage: open <file or application>");
        anyos_std::println!("  open Something.app       Launch an application");
        anyos_std::println!("  open file.txt            Open file with associated app");
        anyos_std::println!("  open /path/to/file       Open by full path");
        return;
    }

    // Check if it's a .app bundle (name ends with .app)
    if path.ends_with(".app") {
        launch_app(path);
        return;
    }

    // Stat the target to see what it is
    let mut stat_buf = [0u32; 7];
    let ret = fs::stat(path, &mut stat_buf);
    if ret != 0 {
        // Maybe it's a bare app name? Try /Applications/<name>.app
        let mut try_path = alloc::format!("/Applications/{}.app", path);
        if fs::stat(&try_path, &mut stat_buf) == 0 && stat_buf[0] == 1 {
            launch_app(&try_path);
            return;
        }
        // Also try as-is with .app suffix
        try_path = alloc::format!("{}.app", path);
        if fs::stat(&try_path, &mut stat_buf) == 0 && stat_buf[0] == 1 {
            launch_app(&try_path);
            return;
        }
        anyos_std::println!("open: {}: No such file or directory", path);
        return;
    }

    // It's a directory — check if it's a .app bundle by name
    if stat_buf[0] == 1 {
        // Directory — could be a .app bundle or a regular directory
        // For now, just report it
        anyos_std::println!("open: {}: is a directory", path);
        return;
    }

    // It's a regular file — look up by extension
    let ext = match path.rfind('.') {
        Some(dot) => &path[dot + 1..],
        None => {
            anyos_std::println!("open: {}: no file extension, cannot determine application", path);
            return;
        }
    };

    let db = MimeDb::load();
    match db.app_for_ext(ext) {
        Some(app_path) => {
            let args = alloc::format!("\"{}\" {}", app_path, path);
            let tid = process::spawn(app_path, &args);
            if tid == u32::MAX {
                anyos_std::println!("open: failed to launch {} for {}", app_path, path);
            }
        }
        None => {
            anyos_std::println!("open: no application associated with .{} files", ext);
        }
    }
}

fn launch_app(path: &str) {
    let tid = process::spawn(path, "");
    if tid == u32::MAX {
        anyos_std::println!("open: failed to launch {}", path);
    }
}

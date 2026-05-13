use alloc::string::{String, ToString};
use alloc::vec::Vec;
use anyos_std::{format, fs};

#[derive(Clone)]
pub struct DirEntry {
    pub entry_type: u8,
    pub name: String,
}

pub fn join_root(root: &str, path: &str) -> String {
    if root == "/" {
        return path.to_string();
    }

    if path.starts_with('/') {
        format!("{}/{}", root.trim_end_matches('/'), &path[1..])
    } else {
        format!("{}/{}", root.trim_end_matches('/'), path)
    }
}

pub fn strip_leading_slash(path: &str) -> &str {
    if path.starts_with('/') {
        &path[1..]
    } else {
        path
    }
}

pub fn ensure_parent_dirs(path: &str) {
    let bytes = path.as_bytes();
    for pos in 0..bytes.len() {
        if bytes[pos] == b'/' && pos > 0 {
            let _ = fs::mkdir(&path[..pos]);
        }
    }
}

pub fn ensure_dir(path: &str) {
    ensure_parent_dirs(path);
    let _ = fs::mkdir(path);
}

pub fn path_exists(path: &str) -> bool {
    let mut stat = [0u32; 7];
    fs::stat(path, &mut stat) == 0
}

pub fn file_size(path: &str) -> Option<u64> {
    let mut stat = [0u32; 7];
    if fs::stat(path, &mut stat) != 0 {
        return None;
    }
    Some(stat[1] as u64)
}

pub fn is_config_path(path: &str) -> bool {
    path.ends_with(".json")
        || path.ends_with(".conf")
        || path.ends_with(".cfg")
        || path.ends_with(".ini")
        || path.ends_with(".env")
        || path.contains("/System/etc/")
        || path.ends_with("/boot.cfg")
}

pub fn is_kv_config_path(path: &str) -> bool {
    path.ends_with(".conf")
        || path.ends_with(".cfg")
        || path.ends_with(".ini")
        || path.ends_with(".env")
        || path.ends_with("/boot.cfg")
}

pub fn is_boot_critical_path(path: &str) -> bool {
    path == "/System/krnl64"
        || path.starts_with("/boot/")
        || path.starts_with("/System/Drivers/")
        || path.starts_with("/Libraries/")
        || path.ends_with(".so")
}

pub fn read_dir(path: &str) -> Result<Vec<DirEntry>, String> {
    let mut out = Vec::new();
    let dir = fs::read_dir(path).map_err(|_| format!("could not read directory {}", path))?;
    for entry in dir {
        if entry.name != "." && entry.name != ".." {
            out.push(DirEntry {
                entry_type: entry.file_type,
                name: entry.name,
            });
        }
    }
    Ok(out)
}

pub fn remove_tree_files(path: &str) {
    let entries = match read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries {
        let child = format!("{}/{}", path.trim_end_matches('/'), entry.name);
        if entry.entry_type == 1 {
            remove_tree_files(&child);
            let _ = fs::unlink(&child);
        } else {
            let _ = fs::unlink(&child);
        }
    }
}

pub fn copy_file(src: &str, dst: &str) -> Result<(), String> {
    let src_fd = fs::open(src, 0);
    if src_fd == u32::MAX {
        return Err(format!("failed to open {}", src));
    }

    let dst_fd = fs::open(dst, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if dst_fd == u32::MAX {
        fs::close(src_fd);
        return Err(format!("failed to open {}", dst));
    }

    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = fs::read(src_fd, &mut buf);
        if n == 0 {
            break;
        }
        if n == u32::MAX {
            fs::close(src_fd);
            fs::close(dst_fd);
            return Err(format!("failed to read {}", src));
        }

        if fs::write(dst_fd, &buf[..n as usize]) == u32::MAX {
            fs::close(src_fd);
            fs::close(dst_fd);
            return Err(format!("failed to write {}", dst));
        }
    }

    let _ = fs::fsync(dst_fd as i32);
    fs::close(src_fd);
    fs::close(dst_fd);
    Ok(())
}

pub fn append_line(path: &str, line: &str) -> Result<(), String> {
    ensure_parent_dirs(path);
    let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_APPEND | fs::O_SYNC);
    if fd == u32::MAX {
        return Err(format!("failed to open {}", path));
    }

    let mut bytes = line.as_bytes();
    while !bytes.is_empty() {
        let written = fs::write(fd, bytes);
        if written == u32::MAX || written == 0 {
            fs::close(fd);
            return Err(format!("failed to append {}", path));
        }
        bytes = &bytes[written as usize..];
    }

    if fs::write(fd, b"\n") == u32::MAX {
        fs::close(fd);
        return Err(format!("failed to append {}", path));
    }

    let _ = fs::fsync(fd as i32);
    fs::close(fd);
    Ok(())
}

// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Host-mode filesystem — delegates to std::fs.

use alloc::string::String;
use alloc::vec::Vec;
use std::collections::HashMap;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read as IoRead, Seek as IoSeek, SeekFrom, Write as IoWrite};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

/// Open flags (compatible with anyOS API).
pub const O_WRITE: u32 = 1;
pub const O_APPEND: u32 = 2;
pub const O_CREATE: u32 = 4;
pub const O_TRUNC: u32 = 8;
pub const O_SYNC: u32 = 0x20;

enum HostHandle {
    File(File),
}

fn handles() -> &'static Mutex<HashMap<u32, HostHandle>> {
    static HANDLES: OnceLock<Mutex<HashMap<u32, HostHandle>>> = OnceLock::new();
    HANDLES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_fd() -> u32 {
    static NEXT_FD: AtomicU32 = AtomicU32::new(3);
    NEXT_FD.fetch_add(1, Ordering::Relaxed)
}

pub fn open(path: &str, flags: u32) -> u32 {
    let mut options = OpenOptions::new();
    let wants_write = (flags & (O_WRITE | O_APPEND | O_CREATE | O_TRUNC | O_SYNC)) != 0;
    options.read(!wants_write || (flags & O_WRITE) == 0);
    options.write(wants_write);
    options.append((flags & O_APPEND) != 0);
    options.create((flags & O_CREATE) != 0);
    options.truncate((flags & O_TRUNC) != 0);

    match options.open(path) {
        Ok(mut file) => {
            if (flags & O_APPEND) != 0 {
                let _ = file.seek(SeekFrom::End(0));
            }
            let fd = next_fd();
            handles().lock().unwrap().insert(fd, HostHandle::File(file));
            fd
        }
        Err(_) => u32::MAX,
    }
}

pub fn close(fd: u32) -> u32 {
    handles().lock().unwrap().remove(&fd);
    0
}

pub fn read(fd: u32, buf: &mut [u8]) -> u32 {
    let mut guard = handles().lock().unwrap();
    let Some(HostHandle::File(file)) = guard.get_mut(&fd) else {
        return u32::MAX;
    };
    match file.read(buf) {
        Ok(n) => n as u32,
        Err(_) => u32::MAX,
    }
}

pub fn write(fd: u32, buf: &[u8]) -> u32 {
    if fd == 1 {
        let _ = std::io::stdout().write_all(buf);
    } else if fd == 2 {
        let _ = std::io::stderr().write_all(buf);
    } else {
        let mut guard = handles().lock().unwrap();
        let Some(HostHandle::File(file)) = guard.get_mut(&fd) else {
            return u32::MAX;
        };
        if file.write_all(buf).is_err() {
            return u32::MAX;
        }
    }
    buf.len() as u32
}

pub fn read_to_string(path: &str) -> Result<String, ()> {
    std::fs::read_to_string(path).map_err(|_| ())
}

pub fn read_to_vec(path: &str) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

pub fn write_bytes(path: &str, data: &[u8]) -> Result<(), ()> {
    std::fs::write(path, data).map_err(|_| ())
}

fn file_type_id(metadata: &Metadata) -> u32 {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        2
    } else if file_type.is_dir() {
        1
    } else {
        0
    }
}

fn mode(metadata: &Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode()
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0
    }
}

fn mtime(metadata: &Metadata) -> u32 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs().min(u32::MAX as u64) as u32)
        .unwrap_or(0)
}

fn fill_stat(metadata: Metadata, stat_buf: &mut [u32; 7]) {
    let ty = file_type_id(&metadata);
    stat_buf[0] = ty;
    stat_buf[1] = metadata.len().min(u32::MAX as u64) as u32;
    stat_buf[2] = if ty == 2 { 1 } else { 0 };
    stat_buf[3] = 0;
    stat_buf[4] = 0;
    stat_buf[5] = mode(&metadata);
    stat_buf[6] = mtime(&metadata);
}

pub fn stat(path: &str, stat_buf: &mut [u32; 7]) -> u32 {
    match fs::metadata(path) {
        Ok(metadata) => {
            fill_stat(metadata, stat_buf);
            0
        }
        Err(_) => u32::MAX,
    }
}

pub fn lstat(path: &str, stat_buf: &mut [u32; 7]) -> u32 {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            fill_stat(metadata, stat_buf);
            0
        }
        Err(_) => u32::MAX,
    }
}

pub fn fstat(_fd: u32, _stat_buf: &mut [u32; 4]) -> u32 {
    u32::MAX
}

pub fn readdir(_path: &str, _buf: &mut [u8]) -> u32 {
    let path = _path;
    let buf = _buf;
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return u32::MAX,
    };

    let entry_size = 64usize;
    let max_entries = buf.len() / entry_size;
    let mut written = 0usize;

    for entry in entries.flatten().take(max_entries) {
        let metadata = entry.metadata().ok();
        let file_type = match entry.file_type() {
            Ok(ft) => {
                if ft.is_symlink() {
                    2u8
                } else if ft.is_dir() {
                    1u8
                } else {
                    0u8
                }
            }
            Err(_) => 0u8,
        };
        let size = metadata
            .as_ref()
            .map(|metadata| metadata.len().min(u32::MAX as u64) as u32)
            .unwrap_or(0);
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(56);
        let off = written * entry_size;
        buf[off] = file_type;
        buf[off + 1] = name_len as u8;
        buf[off + 2] = 0;
        buf[off + 3] = 0;
        for b in &mut buf[off + 4..off + entry_size] {
            *b = 0;
        }
        buf[off + 4..off + 8].copy_from_slice(&size.to_le_bytes());
        buf[off + 8..off + 8 + name_len].copy_from_slice(&name_bytes[..name_len]);
        written += 1;
    }

    written as u32
}

pub fn mkdir(path: &str) -> u32 {
    if std::fs::create_dir_all(path).is_ok() {
        0
    } else {
        u32::MAX
    }
}

pub fn unlink(path: &str) -> u32 {
    if std::fs::remove_file(path).is_ok() || std::fs::remove_dir(path).is_ok() {
        0
    } else {
        u32::MAX
    }
}

pub fn getcwd(buf: &mut [u8]) -> u32 {
    if let Ok(cwd) = std::env::current_dir() {
        let s = cwd.to_string_lossy();
        let bytes = s.as_bytes();
        let len = bytes.len().min(buf.len() - 1);
        buf[..len].copy_from_slice(&bytes[..len]);
        buf[len] = 0;
        len as u32
    } else {
        u32::MAX
    }
}

pub fn isatty(fd: u32) -> u32 {
    if fd <= 2 {
        1
    } else {
        0
    }
}

pub fn rename(old: &str, new: &str) -> u32 {
    if std::fs::rename(old, new).is_ok() {
        0
    } else {
        u32::MAX
    }
}

//! App bundle discovery — find bundle path and resources at runtime.
//!
//! For `.app` bundle programs, `argv[0]` is the bundle directory path
//! (e.g., `/Applications/Terminal.app`).  This module caches `argv[0]`
//! on first access and derives bundle paths from it.

use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};

static mut CACHED_ARGV0: [u8; 256] = [0u8; 256];
static mut CACHED_ARGV0_LEN: usize = 0;
static ARGV0_INIT: AtomicBool = AtomicBool::new(false);

fn argv0() -> &'static str {
    if !ARGV0_INIT.load(Ordering::Acquire) {
        let mut buf = [0u8; 256];
        let len = crate::process::getargs(&mut buf);
        let all = core::str::from_utf8(&buf[..len]).unwrap_or("");
        // argv[0] is everything before the first space
        let a0 = match all.find(' ') {
            Some(idx) => &all[..idx],
            None => all,
        };
        let copy_len = a0.len().min(255);
        unsafe {
            CACHED_ARGV0[..copy_len].copy_from_slice(&a0.as_bytes()[..copy_len]);
            CACHED_ARGV0_LEN = copy_len;
        }
        ARGV0_INIT.store(true, Ordering::Release);
    }
    unsafe {
        core::str::from_utf8_unchecked(&CACHED_ARGV0[..CACHED_ARGV0_LEN])
    }
}

/// Returns the bundle directory path if the running process is an `.app` bundle.
///
/// Example: `Some("/Applications/Terminal.app")`
pub fn bundle_path() -> Option<&'static str> {
    let a0 = argv0();
    if a0.ends_with(".app") {
        Some(a0)
    } else {
        None
    }
}

/// Returns the full path to a resource inside the bundle.
///
/// Example: `resource_path("id1/pak0.pak")` → `Some("/Applications/Quake.app/id1/pak0.pak")`
pub fn resource_path(name: &str) -> Option<String> {
    bundle_path().map(|bp| alloc::format!("{}/{}", bp, name))
}

/// Returns the display name from the bundle's Info.conf.
pub fn bundle_name() -> Option<String> {
    bundle_info("name")
}

/// Read any key from the bundle's Info.conf file.
///
/// Example: `bundle_info("version")` → `Some("1.0")`
pub fn bundle_info(key: &str) -> Option<String> {
    let bp = bundle_path()?;
    let conf_path = alloc::format!("{}/Info.conf", bp);
    // Read the Info.conf file
    let fd = crate::fs::open(&conf_path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut buf = [0u8; 1024];
    let n = crate::fs::read(fd, &mut buf);
    crate::fs::close(fd);
    if n == 0 || n == u32::MAX {
        return None;
    }
    let text = core::str::from_utf8(&buf[..n as usize]).ok()?;
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(idx) = line.find('=') {
            let k = line[..idx].trim();
            let v = line[idx + 1..].trim();
            if k == key && !v.is_empty() {
                return Some(String::from(v));
            }
        }
    }
    None
}

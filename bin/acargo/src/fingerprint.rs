//! Incremental compilation support via mtime-based fingerprinting.
//!
//! Tracks modification times of source files and build artifacts to skip
//! recompilation of crates whose inputs haven't changed.

use crate::fs;
use crate::prelude::*;
use anyos_std::println;

/// A fingerprint entry for a single crate.
#[derive(Debug, Clone)]
pub struct Fingerprint {
    pub crate_name: String,
    /// Modification times of source files (path -> mtime).
    pub src_mtimes: Vec<(String, u64)>,
    /// Hash of compile options (opt_level, cfg flags, etc.).
    pub options_hash: u64,
    /// Path to the output artifact.
    pub output_path: String,
}

/// Check if a crate needs recompilation.
/// Returns true if the crate is up-to-date and can be skipped.
pub fn is_fresh(
    fingerprint_dir: &str,
    crate_name: &str,
    src_file: &str,
    output_path: &str,
    src_dir: &str,
    options_hash: u64,
) -> bool {
    // Check if output exists
    if !fs::file_exists(output_path) {
        return false;
    }

    // Get output mtime
    let out_mtime = get_mtime(output_path);
    if out_mtime == 0 {
        return false;
    }

    // Check main source file mtime
    let src_mtime = get_mtime(src_file);
    if src_mtime == 0 || src_mtime >= out_mtime {
        return false;
    }

    // Check all .rs files in src_dir (rough but effective)
    if any_source_newer(src_dir, out_mtime) {
        return false;
    }

    // Check fingerprint file for option changes
    let fp_path = format!("{}/{}.fp", fingerprint_dir, crate_name);
    if !fs::file_exists(&fp_path) {
        return false; // first build
    }
    let Some(fp) = fs::read_file(&fp_path) else {
        return false;
    };
    let expected = format!("hash={}", options_hash);
    if !fp.lines().any(|line| line == expected) {
        return false;
    }

    true
}

/// Write a fingerprint after successful compilation.
pub fn write_fingerprint(
    fingerprint_dir: &str,
    crate_name: &str,
    src_file: &str,
    output_path: &str,
    options_hash: u64,
) {
    fs::mkdir_p(fingerprint_dir);
    let fp_path = format!("{}/{}.fp", fingerprint_dir, crate_name);
    let content = format!(
        "src={}\nout={}\nhash={}\n",
        src_file, output_path, options_hash
    );
    fs::write_file(&fp_path, content.as_bytes());
}

/// Simple hash of compile options for change detection.
pub fn hash_options(
    opt_level: u32,
    cfg_flags: &[String],
    features: &[String],
    env_vars: &[(String, String)],
    release: bool,
) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325; // FNV offset basis
    let mix = |h: &mut u64, b: u8| {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x100000001b3); // FNV prime
    };

    mix(&mut h, opt_level as u8);
    mix(&mut h, release as u8);
    for flag in cfg_flags {
        for b in flag.bytes() {
            mix(&mut h, b);
        }
        mix(&mut h, 0);
    }
    for feat in features {
        for b in feat.bytes() {
            mix(&mut h, b);
        }
        mix(&mut h, 0);
    }
    for (key, value) in env_vars {
        for b in key.bytes() {
            mix(&mut h, b);
        }
        mix(&mut h, b'=');
        for b in value.bytes() {
            mix(&mut h, b);
        }
        mix(&mut h, 0);
    }
    h
}

/// Get the modification time of a file (seconds since epoch).
/// Returns 0 if the file doesn't exist or mtime can't be determined.
fn get_mtime(path: &str) -> u64 {
    // anyOS stat: [type:u32, size:u32, flags:u32, uid:u32, gid:u32, mode:u32, mtime:u32]
    let mut stat_buf = [0u32; 7];
    let result = anyos_std::fs::stat(path, &mut stat_buf);
    if result == u32::MAX {
        return 0;
    }
    stat_buf[6] as u64 // mtime field
}

/// Check if any .rs file in a directory tree is newer than the given mtime.
fn any_source_newer(dir: &str, threshold: u64) -> bool {
    let mut buf = [0u8; 64 * 128];
    let count = anyos_std::fs::readdir(dir, &mut buf);
    if count == u32::MAX {
        return false;
    }

    let entry_size = 64;
    for i in 0..count as usize {
        let off = i * entry_size;
        let file_type = buf[off];
        let name_len = buf[off + 1] as usize;
        if name_len == 0 {
            continue;
        }
        let name_bytes = &buf[off + 8..off + 8 + name_len];
        if let Ok(name) = core::str::from_utf8(name_bytes) {
            if name == "." || name == ".." {
                continue;
            }
            let child_path = format!("{}/{}", dir, name);
            if file_type == 1 {
                // Directory: recurse
                if any_source_newer(&child_path, threshold) {
                    return true;
                }
            } else if name.ends_with(".rs") {
                let mtime = get_mtime(&child_path);
                if mtime >= threshold {
                    return true;
                }
            }
        }
    }
    false
}

//! Download files using curl as a subprocess.
//!
//! Uses `/System/bin/curl -o <output> <url>` for HTTPS-capable downloads.

use alloc::format;
use anyos_std::{println, process};

/// Download a file from `url` to `output_path` using curl.
/// Returns true on success.
pub fn download(url: &str, output_path: &str) -> bool {
    download_inner(url, output_path, false)
}

/// Download a file, showing progress information.
pub fn download_verbose(url: &str, output_path: &str) -> bool {
    download_inner(url, output_path, true)
}

/// Internal download implementation.
fn download_inner(url: &str, output_path: &str, verbose: bool) -> bool {
    let args = if verbose {
        format!("-f -L -o {} {}", output_path, url)
    } else {
        format!("-f -s -L -o {} {}", output_path, url)
    };

    let tid = process::spawn("/System/bin/curl", &args);
    if tid == u32::MAX {
        println!("apkg: failed to execute curl");
        return false;
    }

    let exit_code = process::waitpid(tid);
    if exit_code != 0 {
        if verbose {
            println!("apkg: download failed (curl exit code {})", exit_code);
        }
        return false;
    }
    true
}

//! Download files using libhttp (HTTP/HTTPS with BearSSL TLS).
//!
//! Uses `libhttp.so` shared library for native HTTP/HTTPS downloads
//! with automatic redirect following and gzip decompression.
//! Verbose mode shows a live progress bar on the console.

use alloc::string::String;
use anyos_std::{print, println};

/// Download a file from `url` to `output_path`.
/// Returns true on success.
pub fn download(url: &str, output_path: &str) -> bool {
    download_inner(url, output_path, false)
}

/// Download a file, showing a progress bar on the console.
pub fn download_verbose(url: &str, output_path: &str) -> bool {
    download_inner(url, output_path, true)
}

/// Progress callback for the console progress bar.
/// Builds the entire line in a `String` and prints it in a single syscall.
/// Format: `\r  [===============>              ] 45% (1.2/2.6 MB)   `
extern "C" fn progress_callback(received: u32, total: u32, _userdata: u64) {
    let mut buf = String::with_capacity(80);
    buf.push('\r');
    buf.push_str("  ");

    if total == 0 {
        push_bytes(&mut buf, received);
        buf.push_str(" received   ");
        print!("{}", buf);
        return;
    }

    let pct = ((received as u64 * 100) / total as u64).min(100) as u32;
    let bar_width: u32 = 30;
    let filled = (pct * bar_width / 100) as usize;

    buf.push('[');
    let mut i = 0;
    while i < bar_width as usize {
        if i < filled {
            buf.push('=');
        } else if i == filled {
            buf.push('>');
        } else {
            buf.push(' ');
        }
        i += 1;
    }
    buf.push_str("] ");
    push_u32(&mut buf, pct);
    buf.push_str("% (");
    push_bytes(&mut buf, received);
    buf.push('/');
    push_bytes(&mut buf, total);
    buf.push_str(")   ");
    print!("{}", buf);
}

/// Append a byte count in human-readable format to a string buffer.
fn push_bytes(buf: &mut String, bytes: u32) {
    if bytes >= 1_048_576 {
        push_u32(buf, bytes / 1_048_576);
        buf.push('.');
        push_u32(buf, (bytes % 1_048_576) * 10 / 1_048_576);
        buf.push_str(" MB");
    } else if bytes >= 1_024 {
        push_u32(buf, bytes / 1_024);
        buf.push_str(" KB");
    } else {
        push_u32(buf, bytes);
        buf.push_str(" B");
    }
}

/// Append a u32 as decimal digits to a string buffer.
fn push_u32(buf: &mut String, val: u32) {
    if val == 0 {
        buf.push('0');
        return;
    }
    let mut digits = [0u8; 10];
    let mut n = val;
    let mut i = 0;
    while n > 0 {
        digits[i] = (n % 10) as u8 + b'0';
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        buf.push(digits[i] as char);
    }
}

/// Internal download implementation using libhttp.
fn download_inner(url: &str, output_path: &str, verbose: bool) -> bool {
    if verbose {
        println!("  downloading {}", url);
    }

    let result = if verbose {
        libhttp_client::download_progress(url, output_path, progress_callback, 0)
    } else {
        libhttp_client::download(url, output_path)
    };

    if verbose {
        println!(); // finalize progress bar line
    }

    if !result {
        let err = libhttp_client::last_error();
        let status = libhttp_client::last_status();
        if verbose {
            let err_msg = match err {
                1 => "invalid URL",
                2 => "DNS resolution failed",
                3 => "connection failed",
                4 => "send failed",
                5 => "no response",
                6 => "too many redirects",
                7 => "TLS handshake failed",
                9 => "file write error",
                _ => "unknown error",
            };
            if status > 0 {
                println!("apkg: download failed: HTTP {} ({})", status, err_msg);
            } else {
                println!("apkg: download failed: {}", err_msg);
            }
        }
        return false;
    }

    true
}

//! Download files using libhttp (HTTP/HTTPS with BearSSL TLS).
//!
//! Uses `libhttp.so` shared library for native HTTP/HTTPS downloads
//! with automatic redirect following and gzip decompression.

use anyos_std::println;

/// Download a file from `url` to `output_path`.
/// Returns true on success.
pub fn download(url: &str, output_path: &str) -> bool {
    download_inner(url, output_path, false)
}

/// Download a file, showing progress information.
pub fn download_verbose(url: &str, output_path: &str) -> bool {
    download_inner(url, output_path, true)
}

/// Internal download implementation using libhttp.
fn download_inner(url: &str, output_path: &str, verbose: bool) -> bool {
    if verbose {
        println!("  downloading {}", url);
    }

    let result = libhttp_client::download(url, output_path);

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

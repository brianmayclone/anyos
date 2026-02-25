#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!("Usage: gzip [-d] [-k] file [file...]");
    anyos_std::println!("       gunzip [-k] file [file...]");
    anyos_std::println!("  -d  Decompress (same as gunzip)");
    anyos_std::println!("  -k  Keep original file");
}

fn main() {
    if !libzip_client::init() {
        anyos_std::println!("gzip: failed to load libzip.so");
        return;
    }

    // Get full args including argv[0] to detect "gunzip" invocation
    let mut full_buf = [0u8; 256];
    let full_len = anyos_std::process::getargs(&mut full_buf);
    let full_args = core::str::from_utf8(&full_buf[..full_len]).unwrap_or("");
    let argv0 = match full_args.find(' ') {
        Some(idx) => &full_args[..idx],
        None => full_args,
    };
    let is_gunzip = argv0.contains("gunzip");

    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if args.pos_count < 1 {
        usage();
        return;
    }

    let decompress = is_gunzip || args.has(b'd');
    let keep = args.has(b'k');

    for i in 0..args.pos_count {
        let path = args.positional[i];

        if decompress {
            // Decompress: file.gz → file
            let out_path = if path.ends_with(".gz") {
                String::from(&path[..path.len() - 3])
            } else if path.ends_with(".tgz") {
                let mut s = String::from(&path[..path.len() - 4]);
                s.push_str(".tar");
                s
            } else {
                anyos_std::println!("gzip: '{}': unknown suffix -- ignored", path);
                continue;
            };

            if libzip_client::gzip_decompress_file(path, &out_path) {
                anyos_std::println!("{} -> {}", path, out_path);
                if !keep {
                    anyos_std::fs::unlink(path);
                }
            } else {
                anyos_std::println!("gzip: '{}': decompression failed", path);
            }
        } else {
            // Compress: file → file.gz
            let out_path = format!("{}.gz", path);

            if libzip_client::gzip_compress_file(path, &out_path) {
                anyos_std::println!("{} -> {}", path, out_path);
                if !keep {
                    anyos_std::fs::unlink(path);
                }
            } else {
                anyos_std::println!("gzip: '{}': compression failed", path);
            }
        }
    }
}

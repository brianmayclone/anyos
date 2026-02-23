#![no_std]
#![no_main]

use alloc::format;

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!("Usage: unzip [-l] [-d dir] archive.zip");
    anyos_std::println!("  -l       List contents only");
    anyos_std::println!("  -d dir   Extract to directory");
}

fn method_name(method: u32) -> &'static str {
    match method {
        0 => "Stored",
        8 => "Deflate",
        _ => "Unknown",
    }
}

/// Ensure all parent directories for a path exist.
fn ensure_parent_dirs(path: &str) {
    let bytes = path.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] == b'/' && pos > 0 {
            let dir = &path[..pos];
            anyos_std::fs::mkdir(dir);
        }
        pos += 1;
    }
}

fn main() {
    if !libzip_client::init() {
        anyos_std::println!("unzip: failed to load libzip.so");
        return;
    }

    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"d");

    if args.pos_count < 1 {
        usage();
        return;
    }

    let list_only = args.has(b'l');
    let dest_dir = args.opt(b'd').unwrap_or(".");
    let archive_path = args.positional[0];

    let reader = match libzip_client::ZipReader::open(archive_path) {
        Some(r) => r,
        None => {
            anyos_std::println!("unzip: cannot open '{}'", archive_path);
            return;
        }
    };

    let count = reader.entry_count();

    if list_only {
        anyos_std::println!("Archive: {}", archive_path);
        anyos_std::println!("  Length   Method    Compr  Name");
        anyos_std::println!("--------  --------  -----  ----");

        let mut total_size: u32 = 0;
        for i in 0..count {
            let name = reader.entry_name(i);
            let size = reader.entry_size(i);
            let compressed = reader.entry_compressed_size(i);
            let method = reader.entry_method(i);

            let ratio = if size > 0 {
                100 - ((compressed as u64 * 100) / size as u64) as u32
            } else {
                0
            };

            anyos_std::println!("{:>8}  {:>8}  {:>4}%  {}",
                size, method_name(method), ratio, name);
            total_size += size;
        }

        anyos_std::println!("--------                    ----");
        anyos_std::println!("{:>8}                    {} files", total_size, count);
        return;
    }

    // Extract
    anyos_std::println!("Archive: {}", archive_path);

    // Create destination directory
    if dest_dir != "." {
        anyos_std::fs::mkdir(dest_dir);
    }

    let mut extracted = 0u32;
    let mut errors = 0u32;

    for i in 0..count {
        let name = reader.entry_name(i);
        let is_dir = reader.entry_is_dir(i);

        let full_path = if dest_dir == "." {
            name.clone()
        } else {
            format!("{}/{}", dest_dir, name)
        };

        if is_dir {
            anyos_std::println!("   creating: {}", name);
            ensure_parent_dirs(&full_path);
            anyos_std::fs::mkdir(&full_path);
        } else {
            anyos_std::println!("  inflating: {}", name);
            ensure_parent_dirs(&full_path);
            if reader.extract_to_file(i, &full_path) {
                extracted += 1;
            } else {
                anyos_std::println!("    error extracting: {}", name);
                errors += 1;
            }
        }
    }

    if errors > 0 {
        anyos_std::println!("unzip: {} files extracted, {} errors", extracted, errors);
    }
}

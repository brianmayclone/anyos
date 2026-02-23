#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!("Usage: zip [-0] [-r] archive.zip file [file...]");
    anyos_std::println!("  -0  Store only (no compression)");
    anyos_std::println!("  -r  Recurse into directories");
}

/// Add a file to the writer.
fn add_file(writer: &libzip_client::ZipWriter, path: &str, archive_name: &str, compress: bool) {
    match anyos_std::fs::read_to_vec(path) {
        Ok(data) => {
            if writer.add_file(archive_name, &data, compress) {
                anyos_std::println!("  adding: {}", archive_name);
            } else {
                anyos_std::println!("  error adding: {}", archive_name);
            }
        }
        Err(_) => {
            anyos_std::println!("zip: cannot read '{}'", path);
        }
    }
}

/// Add a directory recursively.
fn add_dir_recursive(writer: &libzip_client::ZipWriter, path: &str, prefix: &str, compress: bool) {
    let dir_name = if prefix.is_empty() {
        format!("{}/", basename(path))
    } else {
        format!("{}{}/", prefix, basename(path))
    };

    writer.add_dir(&dir_name);
    anyos_std::println!("  adding: {}", dir_name);

    if let Ok(entries) = anyos_std::fs::read_dir(path) {
        for entry in entries {
            let child_path = format!("{}/{}", path, entry.name);
            let archive_name = format!("{}{}", dir_name, entry.name);

            if entry.is_dir() {
                add_dir_recursive(writer, &child_path, &dir_name, compress);
            } else {
                add_file(writer, &child_path, &archive_name, compress);
            }
        }
    }
}

/// Get the last path component.
fn basename(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    match path.rfind('/') {
        Some(pos) => &path[pos + 1..],
        None => path,
    }
}

fn main() {
    if !libzip_client::init() {
        anyos_std::println!("zip: failed to load libzip.so");
        return;
    }

    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if args.pos_count < 2 {
        usage();
        return;
    }

    let store_only = args.has(b'0');
    let recursive = args.has(b'r');
    let compress = !store_only;

    let archive_path = args.positional[0];

    let writer = match libzip_client::ZipWriter::new() {
        Some(w) => w,
        None => {
            anyos_std::println!("zip: failed to create archive");
            return;
        }
    };

    for i in 1..args.pos_count {
        let path = args.positional[i];

        // Check if it's a directory
        let mut stat_buf = [0u32; 7];
        if anyos_std::fs::stat(path, &mut stat_buf) == 0 && stat_buf[0] == 1 {
            // Directory
            if recursive {
                add_dir_recursive(&writer, path, "", compress);
            } else {
                anyos_std::println!("zip: '{}' is a directory (use -r to recurse)", path);
            }
        } else {
            add_file(&writer, path, basename(path), compress);
        }
    }

    if writer.write_to_file(archive_path) {
        anyos_std::println!("zip: created '{}'", archive_path);
    } else {
        anyos_std::println!("zip: failed to write '{}'", archive_path);
    }
}

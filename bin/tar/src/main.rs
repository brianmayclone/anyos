#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!("Usage: tar [-c|-x|-t] [-z] [-f archive] [files...]");
    anyos_std::println!("  -c  Create archive");
    anyos_std::println!("  -x  Extract archive");
    anyos_std::println!("  -t  List archive contents");
    anyos_std::println!("  -z  Filter through gzip (.tar.gz)");
    anyos_std::println!("  -f  Archive file (required)");
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

/// Get the last path component.
fn basename(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    match path.rfind('/') {
        Some(pos) => &path[pos + 1..],
        None => path,
    }
}

/// Add a file to the tar writer.
fn add_file(writer: &libzip_client::TarWriter, path: &str, archive_name: &str) {
    match anyos_std::fs::read_to_vec(path) {
        Ok(data) => {
            if writer.add_file(archive_name, &data) {
                anyos_std::println!("a {}", archive_name);
            } else {
                anyos_std::println!("tar: error adding '{}'", archive_name);
            }
        }
        Err(_) => {
            anyos_std::println!("tar: cannot read '{}'", path);
        }
    }
}

/// Add a directory recursively.
fn add_dir_recursive(writer: &libzip_client::TarWriter, path: &str, prefix: &str) {
    let dir_name = if prefix.is_empty() {
        format!("{}/", basename(path))
    } else {
        format!("{}{}/", prefix, basename(path))
    };

    writer.add_dir(&dir_name);
    anyos_std::println!("a {}", dir_name);

    if let Ok(entries) = anyos_std::fs::read_dir(path) {
        for entry in entries {
            let child_path = format!("{}/{}", path, entry.name);
            let archive_name = format!("{}{}", dir_name, entry.name);

            if entry.is_dir() {
                add_dir_recursive(writer, &child_path, &dir_name);
            } else {
                add_file(writer, &child_path, &archive_name);
            }
        }
    }
}

fn main() {
    if !libzip_client::init() {
        anyos_std::println!("tar: failed to load libzip.so");
        return;
    }

    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"f");

    let create = args.has(b'c');
    let extract = args.has(b'x');
    let list = args.has(b't');
    let gzip = args.has(b'z');
    let archive = args.opt(b'f');

    if !create && !extract && !list {
        usage();
        return;
    }

    let archive_path = match archive {
        Some(p) => p,
        None => {
            anyos_std::println!("tar: -f archive required");
            return;
        }
    };

    if create {
        // Create archive
        let writer = match libzip_client::TarWriter::new() {
            Some(w) => w,
            None => {
                anyos_std::println!("tar: failed to create archive");
                return;
            }
        };

        for i in 0..args.pos_count {
            let path = args.positional[i];

            // Check if it's a directory
            let mut stat_buf = [0u32; 7];
            if anyos_std::fs::stat(path, &mut stat_buf) == 0 && stat_buf[0] == 1 {
                add_dir_recursive(&writer, path, "");
            } else {
                add_file(&writer, path, basename(path));
            }
        }

        if writer.write_to_file(archive_path, gzip) {
            anyos_std::println!("tar: created '{}'", archive_path);
        } else {
            anyos_std::println!("tar: failed to write '{}'", archive_path);
        }
    } else {
        // List or extract — open archive
        let reader = match libzip_client::TarReader::open(archive_path) {
            Some(r) => r,
            None => {
                anyos_std::println!("tar: cannot open '{}'", archive_path);
                return;
            }
        };

        let count = reader.entry_count();

        if list {
            for i in 0..count {
                let name = reader.entry_name(i);
                let size = reader.entry_size(i);
                let is_dir = reader.entry_is_dir(i);
                if is_dir {
                    anyos_std::println!("drwxr-xr-x  0 {}", name);
                } else {
                    anyos_std::println!("-rw-r--r--  {} {}", size, name);
                }
            }
        } else {
            // Extract
            let mut extracted = 0u32;
            let mut errors = 0u32;

            for i in 0..count {
                let name = reader.entry_name(i);
                let is_dir = reader.entry_is_dir(i);

                if is_dir {
                    anyos_std::println!("x {}", name);
                    ensure_parent_dirs(&name);
                    anyos_std::fs::mkdir(&name);
                } else {
                    anyos_std::println!("x {}", name);
                    ensure_parent_dirs(&name);
                    if reader.extract_to_file(i, &name) {
                        extracted += 1;
                    } else {
                        anyos_std::println!("tar: error extracting '{}'", name);
                        errors += 1;
                    }
                }
            }

            if errors > 0 {
                anyos_std::println!("tar: {} extracted, {} errors", extracted, errors);
            }
        }
    }
}

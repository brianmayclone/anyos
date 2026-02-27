//! `apkg install` — install one or more packages with dependency resolution.

use alloc::string::String;
use alloc::format;
use anyos_std::{println, fs, crypto};
use crate::{config, db, download, index, resolve, archive};
use crate::db::InstalledPackage;

/// Execute `apkg install <names>`.
pub fn run(names: &[&str], yes: bool) {
    if names.is_empty() {
        println!("Usage: apkg install <package> [package...]");
        return;
    }

    let idx = match index::Index::load() {
        Some(i) => i,
        None => {
            println!("apkg: no package index found. Run 'apkg update' first.");
            return;
        }
    };

    let mut database = db::Database::load();

    // Resolve dependencies
    let plan = match resolve::resolve(names, &idx, &database) {
        Ok(p) => p,
        Err(e) => {
            println!("apkg: {}", e);
            return;
        }
    };

    if plan.is_empty() {
        println!("All requested packages are already installed and up to date.");
        return;
    }

    // Show install plan
    println!("The following packages will be installed:");
    let mut total_download: u64 = 0;
    let mut total_installed: u64 = 0;
    for item in &plan {
        if let Some(pkg) = idx.find(&item.name) {
            let auto_str = if item.auto { " [dependency]" } else { "" };
            println!("  {}-{} ({}){}", pkg.name, pkg.version_str,
                     format_size(pkg.size), auto_str);
            total_download += pkg.size;
            total_installed += pkg.size_installed;
        }
    }
    println!("Total download size: {}", format_size(total_download));
    println!("Total installed size: {}", format_size(total_installed));

    // Confirm
    if !yes {
        println!("Proceed? [Y/n] ");
        let mut buf = [0u8; 16];
        let n = fs::read(0, &mut buf);
        if n > 0 && n != u32::MAX {
            let answer = core::str::from_utf8(&buf[..n as usize]).unwrap_or("y").trim();
            if answer == "n" || answer == "N" || answer == "no" {
                println!("Aborted.");
                return;
            }
        }
    }

    let mirrors = config::read_mirrors();
    if mirrors.is_empty() {
        println!("apkg: no mirrors configured");
        return;
    }

    let arch = config::arch();
    let mut installed_count = 0u32;

    for item in &plan {
        let pkg = match idx.find(&item.name) {
            Some(p) => p,
            None => continue,
        };

        println!("Installing {}...", pkg.name);

        // Download
        let cache_path = format!("{}/{}", config::CACHE_DIR, pkg.filename);
        if !file_exists(&cache_path) {
            let mut downloaded = false;
            for mirror_url in &mirrors {
                let url = config::package_url(mirror_url, arch, &pkg.filename);
                if download::download_verbose(&url, &cache_path) {
                    downloaded = true;
                    break;
                }
            }
            if !downloaded {
                println!("apkg: failed to download {}", pkg.name);
                continue;
            }
        }

        // Verify MD5 checksum
        if !pkg.md5.is_empty() {
            if let Ok(data) = fs::read_to_vec(&cache_path) {
                let hash = crypto::md5_hex(&data);
                let hash_str = core::str::from_utf8(&hash).unwrap_or("");
                if hash_str != pkg.md5 {
                    println!("apkg: checksum mismatch for {}", pkg.name);
                    println!("  expected: {}", pkg.md5);
                    println!("  got:      {}", hash_str);
                    // Remove corrupted file
                    fs::unlink(&cache_path);
                    continue;
                }
            }
        }

        // Extract
        let result = match archive::extract_package(&cache_path) {
            Some(r) => r,
            None => {
                println!("apkg: failed to extract {}", pkg.name);
                continue;
            }
        };

        // Record in database
        let depends: alloc::vec::Vec<String> = pkg.depends.clone();
        database.add(InstalledPackage {
            name: String::from(&pkg.name),
            version: String::from(&pkg.version_str),
            files: result.installed_files,
            depends,
            pkg_type: String::from(&pkg.pkg_type),
            auto: item.auto,
        });

        installed_count += 1;
    }

    database.save();
    println!("{} package(s) installed.", installed_count);
}

/// Check if a file exists.
fn file_exists(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    fs::stat(path, &mut stat_buf) == 0
}

/// Format a byte size into a human-readable string.
fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{}.{} MiB", bytes / (1024 * 1024), (bytes % (1024 * 1024)) * 10 / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{}.{} KiB", bytes / 1024, (bytes % 1024) * 10 / 1024)
    } else {
        format!("{} B", bytes)
    }
}

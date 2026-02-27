//! `apkg upgrade` — upgrade installed packages to latest versions.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use anyos_std::{println, fs, crypto};
use crate::{config, db, download, index, archive, version::Version};
use crate::db::InstalledPackage;

/// Execute `apkg upgrade [name]`.
/// If no name is given, upgrades all packages with newer versions available.
pub fn run(name: Option<&str>, yes: bool) {
    let idx = match index::Index::load() {
        Some(i) => i,
        None => {
            println!("apkg: no package index found. Run 'apkg update' first.");
            return;
        }
    };

    let mut database = db::Database::load();

    // Collect packages to upgrade
    let mut upgrades: Vec<(String, String, String)> = Vec::new(); // (name, old_ver, new_ver)

    if let Some(pkg_name) = name {
        // Upgrade a specific package
        let installed = match database.get(pkg_name) {
            Some(p) => p,
            None => {
                println!("apkg: package '{}' is not installed", pkg_name);
                return;
            }
        };
        if let Some(avail) = idx.find(pkg_name) {
            let inst_ver = Version::parse(&installed.version)
                .unwrap_or(Version { major: 0, minor: 0, patch: 0 });
            if avail.version > inst_ver {
                upgrades.push((
                    String::from(pkg_name),
                    String::from(&installed.version),
                    String::from(&avail.version_str),
                ));
            } else {
                println!("{} is already at the latest version ({}).", pkg_name, installed.version);
                return;
            }
        } else {
            println!("apkg: package '{}' not found in index", pkg_name);
            return;
        }
    } else {
        // Upgrade all packages
        for installed in &database.packages {
            if let Some(avail) = idx.find(&installed.name) {
                let inst_ver = Version::parse(&installed.version)
                    .unwrap_or(Version { major: 0, minor: 0, patch: 0 });
                if avail.version > inst_ver {
                    upgrades.push((
                        String::from(&installed.name),
                        String::from(&installed.version),
                        String::from(&avail.version_str),
                    ));
                }
            }
        }
    }

    if upgrades.is_empty() {
        println!("All packages are up to date.");
        return;
    }

    // Show upgrade plan
    println!("The following packages will be upgraded:");
    for (name, old_ver, new_ver) in &upgrades {
        println!("  {} {} -> {}", name, old_ver, new_ver);
    }

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
    let mut upgraded_count = 0u32;

    for (pkg_name, _old_ver, _new_ver) in &upgrades {
        let pkg = match idx.find(pkg_name) {
            Some(p) => p,
            None => continue,
        };

        println!("Upgrading {}...", pkg.name);

        // Backup system packages
        if pkg.pkg_type == "system" {
            if let Some(installed) = database.get(pkg_name) {
                for file_path in &installed.files {
                    let backup_path = format!("{}/{}.{}", config::BACKUP_DIR,
                        file_path.rsplit('/').next().unwrap_or("unknown"),
                        installed.version);
                    // Copy file to backup (read + write)
                    if let Ok(data) = fs::read_to_vec(file_path) {
                        if let Ok(mut f) = fs::File::create(&backup_path) {
                            use anyos_std::fs::Write;
                            let _ = f.write_all(&data);
                        }
                    }
                }
            }
        }

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

        // Verify MD5
        if !pkg.md5.is_empty() {
            if let Ok(data) = fs::read_to_vec(&cache_path) {
                let hash = crypto::md5_hex(&data);
                let hash_str = core::str::from_utf8(&hash).unwrap_or("");
                if hash_str != pkg.md5 {
                    println!("apkg: checksum mismatch for {}", pkg.name);
                    fs::unlink(&cache_path);
                    continue;
                }
            }
        }

        // Extract (overwrites existing files)
        let result = match archive::extract_package(&cache_path) {
            Some(r) => r,
            None => {
                println!("apkg: failed to extract {}", pkg.name);
                continue;
            }
        };

        // Update database record
        let auto = database.get(pkg_name).map(|p| p.auto).unwrap_or(false);
        let depends: Vec<String> = pkg.depends.clone();
        database.add(InstalledPackage {
            name: String::from(&pkg.name),
            version: String::from(&pkg.version_str),
            files: result.installed_files,
            depends,
            pkg_type: String::from(&pkg.pkg_type),
            auto,
        });

        upgraded_count += 1;

        if pkg.pkg_type == "system" {
            println!("  {} updated. Reboot required to apply changes.", pkg.name);
        }
    }

    database.save();
    println!("{} package(s) upgraded.", upgraded_count);
}

/// Check if a file exists.
fn file_exists(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    fs::stat(path, &mut stat_buf) == 0
}

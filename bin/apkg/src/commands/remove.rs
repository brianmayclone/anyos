//! `apkg remove` — remove installed packages.

use anyos_std::{println, fs};
use crate::db;

/// Execute `apkg remove <names>`.
pub fn run(names: &[&str], yes: bool, force: bool) {
    if names.is_empty() {
        println!("Usage: apkg remove <package> [package...]");
        return;
    }

    let mut database = db::Database::load();
    let mut to_remove = alloc::vec::Vec::new();

    for &name in names {
        let pkg = match database.get(name) {
            Some(p) => p,
            None => {
                println!("apkg: package '{}' is not installed", name);
                continue;
            }
        };

        // System packages cannot be removed
        if pkg.pkg_type == "system" && !force {
            println!("apkg: '{}' is a system package and cannot be removed", name);
            println!("  use --force to override");
            continue;
        }

        // Check reverse dependencies
        if !force {
            let rdeps = database.reverse_deps(name);
            if !rdeps.is_empty() {
                println!("apkg: the following packages depend on '{}':", name);
                for rd in &rdeps {
                    println!("  {}", rd.name);
                }
                println!("  use --force to remove anyway");
                continue;
            }
        }

        to_remove.push(alloc::string::String::from(name));
    }

    if to_remove.is_empty() {
        return;
    }

    // Show removal plan
    println!("The following packages will be removed:");
    for name in &to_remove {
        if let Some(pkg) = database.get(name) {
            println!("  {}-{}", pkg.name, pkg.version);
        }
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

    let mut removed_count = 0u32;

    for name in &to_remove {
        if let Some(pkg) = database.remove(name) {
            // Delete all installed files
            for file_path in &pkg.files {
                if fs::unlink(file_path) == 0 {
                    println!("  removed {}", file_path);
                }
            }
            removed_count += 1;
        }
    }

    database.save();
    println!("{} package(s) removed.", removed_count);
}

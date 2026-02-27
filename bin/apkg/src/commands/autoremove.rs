//! `apkg autoremove` — remove orphaned auto-installed dependencies.

use anyos_std::{println, fs};
use crate::db;

/// Execute `apkg autoremove`.
pub fn run(yes: bool) {
    let mut database = db::Database::load();
    let orphans: alloc::vec::Vec<alloc::string::String> = database
        .orphan_auto_packages()
        .iter()
        .map(|p| alloc::string::String::from(&p.name))
        .collect();

    if orphans.is_empty() {
        println!("No orphaned packages to remove.");
        return;
    }

    println!("The following auto-installed packages are no longer needed:");
    for name in &orphans {
        if let Some(pkg) = database.get(name) {
            println!("  {}-{}", pkg.name, pkg.version);
        }
    }

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
    for name in &orphans {
        if let Some(pkg) = database.remove(name) {
            for file_path in &pkg.files {
                fs::unlink(file_path);
            }
            removed_count += 1;
        }
    }

    database.save();
    println!("{} package(s) removed.", removed_count);
}

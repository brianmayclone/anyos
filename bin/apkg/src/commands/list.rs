//! `apkg list` — list installed or available packages.

use anyos_std::println;
use crate::{config, db, index};

/// Execute `apkg list [-a]`.
/// `-a` = list all available packages instead of installed ones.
pub fn run(show_available: bool) {
    if show_available {
        list_available();
    } else {
        list_installed();
    }
}

/// List installed packages.
fn list_installed() {
    let database = db::Database::load();
    if database.packages.is_empty() {
        println!("No packages installed.");
        return;
    }

    println!("{:<20} {:<10} {:<10} {}", "Package", "Version", "Type", "Auto");
    println!("{}", "-".repeat(55));
    for pkg in &database.packages {
        let auto_str = if pkg.auto { "yes" } else { "" };
        println!("{:<20} {:<10} {:<10} {}", pkg.name, pkg.version, pkg.pkg_type, auto_str);
    }
    println!("{} package(s) installed.", database.packages.len());
}

/// List all available packages from the index.
fn list_available() {
    let idx = match index::Index::load() {
        Some(i) => i,
        None => {
            println!("apkg: no package index found. Run 'apkg update' first.");
            return;
        }
    };

    let arch = config::arch();
    let packages = idx.list_for_arch(arch);

    if packages.is_empty() {
        println!("No packages available for {}.", arch);
        return;
    }

    let db = db::Database::load();

    println!("{:<20} {:<10} {:<10} {}", "Package", "Version", "Type", "Status");
    println!("{}", "-".repeat(60));
    for pkg in &packages {
        let status = if db.is_installed(&pkg.name) { "installed" } else { "" };
        println!("{:<20} {:<10} {:<10} {}", pkg.name, pkg.version_str, pkg.pkg_type, status);
    }
    println!("{} package(s) available.", packages.len());
}

/// Helper to repeat a character.
trait RepeatStr {
    fn repeat(self, n: usize) -> alloc::string::String;
}

impl RepeatStr for &str {
    fn repeat(self, n: usize) -> alloc::string::String {
        let mut s = alloc::string::String::with_capacity(self.len() * n);
        for _ in 0..n {
            s.push_str(self);
        }
        s
    }
}

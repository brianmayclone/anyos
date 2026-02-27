//! `apkg search` — search available packages by name or description.

use anyos_std::println;
use crate::index;

/// Execute `apkg search <term>`.
pub fn run(term: &str) {
    if term.is_empty() {
        println!("Usage: apkg search <term>");
        return;
    }

    let idx = match index::Index::load() {
        Some(i) => i,
        None => {
            println!("apkg: no package index found. Run 'apkg update' first.");
            return;
        }
    };

    let results = idx.search(term);
    if results.is_empty() {
        println!("No packages found matching '{}'.", term);
        return;
    }

    for pkg in &results {
        println!("{:<20} {:<10} {}", pkg.name, pkg.version_str, pkg.description);
    }
    println!("{} package(s) found.", results.len());
}

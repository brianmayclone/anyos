//! `apkg update` — download the latest index.json from mirrors.

use anyos_std::println;
use crate::{config, download};

/// Execute `apkg update`.
pub fn run() {
    let mirrors = config::read_mirrors();
    if mirrors.is_empty() {
        println!("apkg: no mirrors configured");
        println!("  use 'apkg mirror add <url>' to add a mirror");
        return;
    }

    for mirror_url in &mirrors {
        let url = config::index_url(mirror_url);
        println!("Fetching index from {}...", mirror_url);

        if download::download(&url, config::INDEX_PATH) {
            println!("Package index updated successfully.");
            return;
        }
        println!("  mirror unavailable, trying next...");
    }

    println!("apkg: failed to fetch index from any mirror");
}

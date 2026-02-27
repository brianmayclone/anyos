//! `apkg clean` — remove cached package downloads.

use anyos_std::{println, fs};
use crate::config;

/// Execute `apkg clean`.
pub fn run() {
    let entries = match fs::read_dir(config::CACHE_DIR) {
        Ok(e) => e,
        Err(_) => {
            println!("Cache is empty.");
            return;
        }
    };

    let mut removed = 0u32;
    let mut freed: u64 = 0;

    for entry in entries {
        if entry.is_dir() {
            continue;
        }
        let path = alloc::format!("{}/{}", config::CACHE_DIR, entry.name);
        freed += entry.size as u64;
        if fs::unlink(&path) == 0 {
            removed += 1;
        }
    }

    if removed == 0 {
        println!("Cache is empty.");
    } else {
        println!("Removed {} cached file(s), freed {} bytes.", removed, freed);
    }
}

//! `apkg mirror` — manage the mirror URL list.

use anyos_std::println;
use crate::config;

/// Execute `apkg mirror <subcommand> [args]`.
pub fn run(subcmd: &str, arg: &str) {
    match subcmd {
        "list" => list(),
        "add" => add(arg),
        "remove" => remove(arg),
        _ => {
            println!("Usage: apkg mirror <list|add|remove> [url]");
        }
    }
}

/// List all configured mirrors.
fn list() {
    let mirrors = config::read_mirrors();
    if mirrors.is_empty() {
        println!("No mirrors configured.");
        println!("  use 'apkg mirror add <url>' to add a mirror");
        return;
    }

    println!("Configured mirrors:");
    for (i, url) in mirrors.iter().enumerate() {
        println!("  {}. {}", i + 1, url);
    }
}

/// Add a mirror URL.
fn add(url: &str) {
    if url.is_empty() {
        println!("Usage: apkg mirror add <url>");
        return;
    }
    if config::add_mirror(url) {
        println!("Mirror added: {}", url);
    } else {
        println!("Mirror already configured: {}", url);
    }
}

/// Remove a mirror URL.
fn remove(url: &str) {
    if url.is_empty() {
        println!("Usage: apkg mirror remove <url>");
        return;
    }
    if config::remove_mirror(url) {
        println!("Mirror removed: {}", url);
    } else {
        println!("Mirror not found: {}", url);
    }
}

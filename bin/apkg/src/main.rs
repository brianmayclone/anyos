//! apkg — anyOS Package Manager
//!
//! Downloads, installs, updates, and removes software packages over the
//! internet. Uses curl for HTTPS downloads and libzip for tar.gz extraction.
//!
//! # Usage
//! ```text
//! apkg update                          Download latest package index
//! apkg install <pkg> [pkg...]          Install package(s) with dependencies
//! apkg remove <pkg> [pkg...]           Remove package(s)
//! apkg upgrade [pkg]                   Upgrade package(s)
//! apkg search <term>                   Search available packages
//! apkg list [-a]                       List installed (or -a available) packages
//! apkg info <pkg>                      Show package details
//! apkg clean                           Remove cached downloads
//! apkg autoremove                      Remove unused auto-dependencies
//! apkg mirror list|add|remove [url]    Manage mirror URLs
//! ```

#![no_std]
#![no_main]

use anyos_std::println;

anyos_std::entry!(main);

mod version;
mod config;
mod index;
mod db;
mod download;
mod archive;
mod resolve;
mod commands;

fn main() {
    // Initialize libzip for archive operations
    if !libzip_client::init() {
        println!("apkg: failed to load libzip.so");
        return;
    }

    // Ensure directories exist
    config::ensure_dirs();

    // Parse arguments
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    let yes = args.has(b'y');
    let force = args.has(b'f');

    // Get command (first positional argument)
    let command = match args.pos(0) {
        Some(cmd) => cmd,
        None => {
            print_usage();
            return;
        }
    };

    match command {
        "update" => {
            commands::update::run();
        }
        "install" => {
            let names: alloc::vec::Vec<&str> = (1..args.pos_count)
                .filter_map(|i| args.pos(i))
                .collect();
            commands::install::run(&names, yes);
        }
        "remove" => {
            let names: alloc::vec::Vec<&str> = (1..args.pos_count)
                .filter_map(|i| args.pos(i))
                .collect();
            commands::remove::run(&names, yes, force);
        }
        "upgrade" => {
            let name = args.pos(1);
            commands::upgrade::run(name, yes);
        }
        "search" => {
            let term = args.pos(1).unwrap_or("");
            commands::search::run(term);
        }
        "list" => {
            let show_available = args.has(b'a');
            commands::list::run(show_available);
        }
        "info" => {
            let name = args.pos(1).unwrap_or("");
            commands::info::run(name);
        }
        "clean" => {
            commands::clean::run();
        }
        "autoremove" => {
            commands::autoremove::run(yes);
        }
        "mirror" => {
            let subcmd = args.pos(1).unwrap_or("");
            let arg = args.pos(2).unwrap_or("");
            commands::mirror::run(subcmd, arg);
        }
        "help" | "--help" | "-h" => {
            print_usage();
        }
        "version" | "--version" | "-V" => {
            println!("apkg {}", env!("ANYOS_VERSION"));
        }
        _ => {
            println!("apkg: unknown command '{}'", command);
            print_usage();
        }
    }
}

fn print_usage() {
    println!("apkg - anyOS Package Manager");
    println!();
    println!("Usage: apkg <command> [options] [arguments]");
    println!();
    println!("Commands:");
    println!("  update                 Download latest package index from mirrors");
    println!("  install <pkg> [...]    Install package(s) with dependencies");
    println!("  remove <pkg> [...]     Remove package(s)");
    println!("  upgrade [pkg]          Upgrade one or all installed packages");
    println!("  search <term>          Search available packages");
    println!("  list [-a]              List installed packages (-a for all available)");
    println!("  info <pkg>             Show detailed package information");
    println!("  clean                  Remove cached package downloads");
    println!("  autoremove             Remove unused auto-installed dependencies");
    println!("  mirror list            Show configured mirrors");
    println!("  mirror add <url>       Add a package mirror");
    println!("  mirror remove <url>    Remove a package mirror");
    println!();
    println!("Options:");
    println!("  -y    Skip confirmation prompts");
    println!("  -f    Force operation (ignore dependency checks)");
}

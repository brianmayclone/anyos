//! bcedit — Boot Configuration Editor for anyOS
//!
//! Parses and modifies /boot/boot.cfg semantically.
//! Validates changes, warns about dangerous operations, and detects
//! duplicate entries or conflicting settings.
//!
//! USAGE
//! ─────
//!   bcedit                              Compact entry list
//!   bcedit list                         List all entries with their keys
//!   bcedit list-flags                   Show global flags (timeout, default)
//!   bcedit show <name>                  Show all keys of a single entry
//!
//!   bcedit set-flag <key> <value>       Set a global flag
//!   bcedit del-flag <key>               Remove a global flag
//!
//!   bcedit add <name>                   Add a new entry
//!   bcedit remove <name>                Remove an entry
//!   bcedit rename <name> <newname>      Rename an entry
//!   bcedit duplicate <name> <newname>   Clone an entry under a new name
//!
//!   bcedit set <name> <key> <value>     Set a key inside an entry
//!   bcedit del <name> <key>             Remove a key from an entry
//!
//!   bcedit init                         Restore default boot.cfg
//!   bcedit check                        Validate the config and report issues
//!   bcedit help                         Show this help

#![no_std]
#![no_main]

mod config;
mod io;
mod commands;
mod validation;

anyos_std::entry!(main);

fn print_usage() {
    anyos_std::println!("bcedit — Boot Configuration Editor for anyOS");
    anyos_std::println!("");
    anyos_std::println!("Usage:");
    anyos_std::println!("  bcedit                           Compact entry list");
    anyos_std::println!("  bcedit list                      Verbose entry list");
    anyos_std::println!("  bcedit list-flags                Show global flags");
    anyos_std::println!("  bcedit show <name>               Show a single entry");
    anyos_std::println!("  bcedit check                     Validate config");
    anyos_std::println!("");
    anyos_std::println!("  bcedit set-flag <key> <value>    Set a global flag");
    anyos_std::println!("  bcedit del-flag <key>            Remove a global flag");
    anyos_std::println!("");
    anyos_std::println!("  bcedit add <name>                Add a new entry");
    anyos_std::println!("  bcedit remove <name>             Remove an entry");
    anyos_std::println!("  bcedit rename <name> <new>       Rename an entry");
    anyos_std::println!("  bcedit duplicate <name> <new>    Clone an entry");
    anyos_std::println!("");
    anyos_std::println!("  bcedit set <name> <key> <val>    Set a key in an entry");
    anyos_std::println!("  bcedit del <name> <key>          Remove a key from an entry");
    anyos_std::println!("");
    anyos_std::println!("  bcedit init                      Restore factory defaults");
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if args.pos_count == 0 {
        let mut cfg = config::Config::new();
        if io::read_file(&mut cfg) { commands::list(&cfg, false); }
        return;
    }

    let cmd = args.positional[0];

    // Read-only / special commands
    match cmd {
        "list" => {
            let mut cfg = config::Config::new();
            if io::read_file(&mut cfg) { commands::list(&cfg, true); }
            return;
        }
        "list-flags" => {
            let mut cfg = config::Config::new();
            if io::read_file(&mut cfg) { commands::list_flags(&cfg); }
            return;
        }
        "show" => {
            if args.pos_count < 2 {
                anyos_std::println!("Usage: bcedit show <name>");
                return;
            }
            let mut cfg = config::Config::new();
            if io::read_file(&mut cfg) { commands::show(&cfg, args.positional[1]); }
            return;
        }
        "check" => {
            let mut cfg = config::Config::new();
            if io::read_file(&mut cfg) { validation::check(&cfg); }
            return;
        }
        "init" => {
            io::write_default();
            return;
        }
        "help" | "--help" | "-h" => {
            print_usage();
            return;
        }
        _ => {}
    }

    // Mutating commands — read, modify, write back
    let mut cfg = config::Config::new();
    if !io::read_file(&mut cfg) { return; }

    let modified = match cmd {
        "set-flag" => {
            if args.pos_count < 3 {
                anyos_std::println!("Usage: bcedit set-flag <key> <value>");
                false
            } else {
                commands::set_flag(&mut cfg, args.positional[1], args.positional[2])
            }
        }
        "del-flag" => {
            if args.pos_count < 2 {
                anyos_std::println!("Usage: bcedit del-flag <key>");
                false
            } else {
                commands::del_flag(&mut cfg, args.positional[1])
            }
        }
        "add" => {
            if args.pos_count < 2 {
                anyos_std::println!("Usage: bcedit add <name>");
                false
            } else {
                commands::add(&mut cfg, args.positional[1])
            }
        }
        "remove" => {
            if args.pos_count < 2 {
                anyos_std::println!("Usage: bcedit remove <name>");
                false
            } else {
                commands::remove(&mut cfg, args.positional[1])
            }
        }
        "rename" => {
            if args.pos_count < 3 {
                anyos_std::println!("Usage: bcedit rename <name> <newname>");
                false
            } else {
                commands::rename(&mut cfg, args.positional[1], args.positional[2])
            }
        }
        "duplicate" => {
            if args.pos_count < 3 {
                anyos_std::println!("Usage: bcedit duplicate <name> <newname>");
                false
            } else {
                commands::duplicate(&mut cfg, args.positional[1], args.positional[2])
            }
        }
        "set" => {
            if args.pos_count < 4 {
                anyos_std::println!("Usage: bcedit set <name> <key> <value>");
                false
            } else {
                commands::set(&mut cfg, args.positional[1], args.positional[2], args.positional[3])
            }
        }
        "del" => {
            if args.pos_count < 3 {
                anyos_std::println!("Usage: bcedit del <name> <key>");
                false
            } else {
                commands::del(&mut cfg, args.positional[1], args.positional[2])
            }
        }
        _ => {
            anyos_std::println!("bcedit: unknown command '{}'", cmd);
            anyos_std::println!("Run 'bcedit help' for usage.");
            false
        }
    };

    if modified {
        io::write_file(&cfg);
    }
}

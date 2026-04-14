// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefs-snapshot` — snapshot management (stub).
//!
//! Snapshot operations (`list`, `create`, `delete`, `restore`) rely on
//! the mutable `OdfDeviceSession` service which currently lives only
//! in the `std`-gated main `corefs` crate.  The anyOS userspace
//! counterpart is queued — until then the tool parses its
//! sub-commands and exits with [`ExitCode::Unsupported`].

#![no_std]
#![no_main]

use libcorefs_tools::args;
use libcorefs_tools::error::ExitCode;

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!(
        "Usage: corefs-snapshot --device <id> --capacity <bytes> <list|create|delete|restore>\n                       \
         [--name <name>] [--id <id>] [--scope <path>] [--json]"
    );
}

fn main() -> u32 {
    let mut buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut buf);
    let args = args::parse(raw);

    if args.has("help") {
        usage();
        return ExitCode::Success.as_u32();
    }

    if args::parse_device_id(&args).is_none() {
        anyos_std::println!("corefs-snapshot: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    }

    let Some(sub) = args.positional_at(0) else {
        anyos_std::println!("corefs-snapshot: missing subcommand");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };

    match sub {
        "list" | "create" | "delete" | "restore" => {
            anyos_std::println!(
                "corefs-snapshot '{}': planned, not yet implemented in anyOS userspace.\n\
                 The snapshot service depends on the std-only \
                 OdfDeviceSession API in the main corefs crate.",
                sub
            );
            ExitCode::Unsupported.as_u32()
        }
        other => {
            anyos_std::println!("corefs-snapshot: unknown subcommand '{}'", other);
            usage();
            ExitCode::InvalidArgument.as_u32()
        }
    }
}

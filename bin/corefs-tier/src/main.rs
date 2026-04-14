// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefs-tier` — hot/cold storage tiering (stub).
//!
//! The tiering service in `corefs-core::storage::ondisk::tiering` is
//! feature-complete, but user-driven tier moves rely on a writable
//! session API (`OdfDeviceSession`) that currently lives only in the
//! std-gated main `corefs` crate.  This tool parses its arguments and
//! exits with [`ExitCode::Unsupported`].

#![no_std]
#![no_main]

use libcorefs_tools::args;
use libcorefs_tools::error::ExitCode;

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!(
        "Usage: corefs-tier --device <id> --capacity <bytes> <promote|demote|status>\n                  \
         [--slot <n>] [--tier hot|cold] [--json]"
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
        anyos_std::println!("corefs-tier: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    }

    let Some(sub) = args.positional_at(0) else {
        anyos_std::println!("corefs-tier: missing subcommand");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };

    match sub {
        "promote" | "demote" | "status" => {
            anyos_std::println!(
                "corefs-tier '{}': planned, not yet implemented in anyOS userspace.\n\
                 The storage-tiering mutation path depends on the std-only \
                 OdfDeviceSession API in the main corefs crate.",
                sub
            );
            ExitCode::Unsupported.as_u32()
        }
        other => {
            anyos_std::println!("corefs-tier: unknown subcommand '{}'", other);
            usage();
            ExitCode::InvalidArgument.as_u32()
        }
    }
}

// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefs-resize` — grow / shrink a CoreFS volume (stub).
//!
//! The resize operation is listed as planned in CoreFS
//! (`corefs-tools::resize::grow` / `shrink` are stubs).  This tool
//! parses its arguments and exits with [`ExitCode::Unsupported`].

#![no_std]
#![no_main]

use libcorefs_tools::args;
use libcorefs_tools::error::ExitCode;

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!(
        "Usage: corefs-resize --device <id> --capacity <bytes> --new-size <bytes> [--json]"
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
        anyos_std::println!("corefs-resize: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    }
    if args.get_u64("new-size").is_none() {
        anyos_std::println!("corefs-resize: missing or invalid --new-size <bytes>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    }

    anyos_std::println!(
        "corefs-resize: planned, not yet implemented.\n\
         The grow/shrink path requires allocator-aware block migration \
         which is not yet available in corefs-core."
    );
    ExitCode::Unsupported.as_u32()
}

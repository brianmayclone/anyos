// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefs-defrag` — defragmentation tool (stub).
//!
//! The defragmentation service lives in the main `corefs` crate
//! (`OdfDeviceSession::mutate` + `defragment()`), which is `std`-only.
//! Porting it to the AnyOS userspace is tracked in CoreFS backlog
//! (see `features_corefs.md`); until then this tool parses its
//! arguments, validates that the device can be opened, and exits
//! with [`ExitCode::Unsupported`].

#![no_std]
#![no_main]

use libcorefs_tools::args;
use libcorefs_tools::block_device::AnyOsBlockDevice;
use libcorefs_tools::error::{exit_code_for, ExitCode};

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!(
        "Usage: corefs-defrag --device <id> --capacity <bytes> [--json]"
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

    let Some(device_id) = args::parse_device_id(&args) else {
        anyos_std::println!("corefs-defrag: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(capacity) = args::parse_capacity(&args) else {
        anyos_std::println!("corefs-defrag: missing or invalid --capacity <bytes>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };

    // Probe the device so we fail fast when the handle is invalid.
    if let Err(e) = AnyOsBlockDevice::open(device_id, capacity) {
        anyos_std::println!("corefs-defrag: cannot open device {}: {}", device_id, e);
        return exit_code_for(&e).as_u32();
    }

    anyos_std::println!(
        "corefs-defrag: planned, not yet implemented in anyOS userspace.\n\
         The defragmentation service depends on the std-only \
         OdfDeviceSession API in the main corefs crate.\n\
         Track: features_corefs.md § defrag."
    );
    ExitCode::Unsupported.as_u32()
}

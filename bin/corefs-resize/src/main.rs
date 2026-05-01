// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefs-resize` — grow a CoreFS volume to a new capacity.
//!
//! ```text
//! corefs-resize --device <id> --capacity <bytes> --new-size <bytes> [--json]
//! ```
//!
//! Ruft [`corefs_core::storage::ondisk::resize::grow_device`] auf. Shrink
//! wird bewusst nicht angeboten — dafür müsste zunächst Dateninhalt aus
//! dem obersten Volume-Bereich evakuiert werden (separates Feature).

#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;

use corefs_core::storage::ondisk::layout::BLOCK_SIZE;
use corefs_core::storage::ondisk::resize::grow_device;
use libcorefs_tools::args;
use libcorefs_tools::block_device::AnyOsBlockDevice;
use libcorefs_tools::error::{exit_code_for, ExitCode};
use libcorefs_tools::report::{JsonBuilder, Report};

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!(
        "Usage: corefs-resize --device <id> --capacity <bytes> --new-size <bytes> [--json]"
    );
}

struct GrowReportView {
    old_total_blocks: u64,
    new_total_blocks: u64,
    added_blocks: u64,
    new_size_bytes: u64,
}

impl Report for GrowReportView {
    fn summary(&self) -> String {
        format!(
            "grow: {} -> {} blocks (+{} blocks, {} bytes)",
            self.old_total_blocks, self.new_total_blocks, self.added_blocks, self.new_size_bytes
        )
    }
    fn render_text(&self) -> String {
        format!(
            "volume grow\n-----------\n\
             old total blocks : {}\n\
             new total blocks : {}\n\
             added blocks     : {}\n\
             new size bytes   : {}\n",
            self.old_total_blocks, self.new_total_blocks, self.added_blocks, self.new_size_bytes
        )
    }
    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_u64("old_total_blocks", self.old_total_blocks);
        b.kv_u64("new_total_blocks", self.new_total_blocks);
        b.kv_u64("added_blocks", self.added_blocks);
        b.kv_u64("new_size_bytes", self.new_size_bytes);
        b.end_object();
        b.finish()
    }
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
        anyos_std::println!("corefs-resize: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(capacity) = args::parse_capacity(&args) else {
        anyos_std::println!("corefs-resize: missing or invalid --capacity <bytes>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(new_size) = args.get_u64("new-size") else {
        anyos_std::println!("corefs-resize: missing or invalid --new-size <bytes>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let json = args::flag_present(&args, "json");

    if new_size % BLOCK_SIZE != 0 {
        anyos_std::println!(
            "corefs-resize: --new-size {} must be a multiple of block size {}",
            new_size,
            BLOCK_SIZE
        );
        return ExitCode::InvalidArgument.as_u32();
    }
    let new_capacity_blocks = new_size / BLOCK_SIZE;

    // Open device spanning the full target capacity so grow_device can
    // address blocks beyond the current (logical) volume size.
    let target_capacity = core::cmp::max(capacity, new_size);
    let mut device = match AnyOsBlockDevice::open(device_id, target_capacity) {
        Ok(d) => d,
        Err(e) => {
            anyos_std::println!("corefs-resize: cannot open device {}: {}", device_id, e);
            return exit_code_for(&e).as_u32();
        }
    };

    match grow_device(&mut device, new_capacity_blocks) {
        Ok(report) => {
            libcorefs_tools::report::print_report(
                &GrowReportView {
                    old_total_blocks: report.old_total_blocks,
                    new_total_blocks: report.new_total_blocks,
                    added_blocks: report.added_blocks,
                    new_size_bytes: new_size,
                },
                json,
            );
            ExitCode::Success.as_u32()
        }
        Err(e) => {
            anyos_std::println!("corefs-resize: grow failed: {}", e);
            exit_code_for(&e).as_u32()
        }
    }
}

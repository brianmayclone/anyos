// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `mkfs.corefs` — create a fresh CoreFS volume on a block device.
//!
//! ```text
//! mkfs.corefs --device <id> --capacity <bytes> [--label <name>]
//!             [--inodes <count>] [--journal-blocks <n>] [--json]
//! ```

#![no_std]
#![no_main]

use alloc::string::{String, ToString};

use alloc::boxed::Box;

use corefs_core::config::CoreFsConfig;
use corefs_core::platform::Timestamp;
use corefs_core::storage::ondisk::session::{OdfDeviceSession, OdfSessionOptions};
use corefs_core::storage::ondisk::volume::FormatOptions;
use libcorefs_tools::args;
use libcorefs_tools::block_device::AnyOsBlockDevice;
use libcorefs_tools::error::{exit_code_for, ExitCode};
use libcorefs_tools::report::{JsonBuilder, Report};

anyos_std::entry!(main);

struct MkfsReport {
    device_id: u32,
    label: String,
    capacity_bytes: u64,
    total_blocks: u64,
    inode_count: u64,
    journal_blocks: u64,
    generation: u64,
}

impl Report for MkfsReport {
    fn summary(&self) -> String {
        anyos_std::format!(
            "formatted device {} ({} bytes, {} blocks, {} inode slots)",
            self.device_id,
            self.capacity_bytes,
            self.total_blocks,
            self.inode_count,
        )
    }

    fn render_text(&self) -> String {
        anyos_std::format!(
            "mkfs.corefs report\n\
             ------------------\n\
             device id       : {}\n\
             label           : {}\n\
             capacity        : {} bytes\n\
             total blocks    : {}\n\
             inode slots     : {}\n\
             journal blocks  : {}\n\
             superblock gen  : {}\n",
            self.device_id,
            self.label,
            self.capacity_bytes,
            self.total_blocks,
            self.inode_count,
            self.journal_blocks,
            self.generation,
        )
    }

    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_u64("device", self.device_id as u64);
        b.kv_string("label", &self.label);
        b.kv_u64("capacity_bytes", self.capacity_bytes);
        b.kv_u64("total_blocks", self.total_blocks);
        b.kv_u64("inode_slots", self.inode_count);
        b.kv_u64("journal_blocks", self.journal_blocks);
        b.kv_u64("generation", self.generation);
        b.end_object();
        b.finish()
    }
}

fn usage() {
    anyos_std::println!(
        "Usage: mkfs.corefs --device <id> --capacity <bytes> [--label <name>]\n\
                            [--inodes <count>] [--journal-blocks <n>] [--json]"
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
        anyos_std::println!("mkfs.corefs: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(capacity) = args::parse_capacity(&args) else {
        anyos_std::println!("mkfs.corefs: missing or invalid --capacity <bytes>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };

    let label = args.get("label").unwrap_or("corefs").to_string();
    let json = args::flag_present(&args, "json");

    let device = match AnyOsBlockDevice::open(device_id, capacity) {
        Ok(d) => d,
        Err(e) => {
            anyos_std::println!("mkfs.corefs: cannot open device {}: {}", device_id, e);
            return exit_code_for(&e).as_u32();
        }
    };

    // Default-FormatOptions als Inspirationsquelle für inode_count/journal_blocks.
    let defaults = FormatOptions::default();
    let inode_count = args.get_u64("inodes").unwrap_or(defaults.inode_count);
    let journal_blocks = args
        .get_u64("journal-blocks")
        .unwrap_or(defaults.journal_blocks);

    let session_opts = OdfSessionOptions {
        capacity_bytes: capacity,
        label: label.clone(),
        uuid: [0u8; 16], // Pseudo-UUID aus dem Timestamp
        inode_count,
        journal_blocks,
        config: CoreFsConfig::default(),
    };

    // Frische Session — formatiert + persistiert PersistedState::empty_at(...)
    // im NATIVE-Layout. Damit ist das Volume direkt von OdfReader/fsck/scrub
    // konsumierbar (ohne den vorherigen Blob→Native-Migrationsschritt).
    let session = match OdfDeviceSession::format_new_at(
        Box::new(device),
        &session_opts,
        Timestamp::EPOCH,
    ) {
        Ok(s) => s,
        Err(e) => {
            anyos_std::println!("mkfs.corefs: format failed: {}", e);
            return exit_code_for(&e).as_u32();
        }
    };

    let out = MkfsReport {
        device_id,
        label,
        capacity_bytes: capacity,
        // Geometrie-Felder werden nach dem format_new_at noch aus dem
        // PersistedState gelesen — `state.volume.block_size` etc.
        total_blocks: capacity / session.device().sector_size() as u64,
        inode_count,
        journal_blocks,
        // Doppel-Save (format_device + save_state_native) → Generation 2.
        generation: 2,
    };
    libcorefs_tools::report::print_report(&out, json);

    ExitCode::Success.as_u32()
}

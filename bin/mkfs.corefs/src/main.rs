// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `mkfs.corefs` — create a fresh CoreFS volume on a block device.
//!
//! ```text
//! mkfs.corefs --device <id> [--capacity <bytes>] [--label <name>]
//!             [--inodes <count>] [--journal-blocks <n>] [--json]
//! ```
//!
//! When `--capacity` is omitted the full device/partition size is used
//! (queried from the kernel via `disk_list`).

#![no_std]
#![no_main]

use alloc::string::{String, ToString};

use alloc::boxed::Box;

use corefs_core::config::CoreFsConfig;
use corefs_core::platform::Timestamp;
use corefs_core::storage::block_device::BlockDevice;
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
        "Usage: mkfs.corefs --device <id> [--capacity <bytes>] [--label <name>]\n\
                            [--inodes <count>] [--journal-blocks <n>] [--json]\n\
         \n\
         If --capacity is omitted the full device size is used."
    );
}

/// If `device_id` refers to a partition, update its MBR type byte to 0xCF
/// (CoreFS) so that fdisk and the kernel recognise the filesystem.
fn set_partition_type_corefs(device_id: u32) {
    const COREFS_MBR_TYPE: u8 = 0xCF;
    const ENTRY: usize = 64;
    let mut buf = [0u8; ENTRY * 16];
    let count = anyos_std::sys::disk_list(&mut buf);
    if count == 0 || count == u32::MAX {
        return;
    }
    for i in 0..count as usize {
        let base = i * ENTRY;
        if base + ENTRY > buf.len() { break; }
        let id = buf[base];
        if id as u32 != device_id { continue; }
        let disk_id = buf[base + 1];
        let part = buf[base + 2];
        if part == 0xFF { return; } // whole disk, nothing to set
        let start_lba = u64::from_le_bytes([
            buf[base + 8], buf[base + 9], buf[base + 10], buf[base + 11],
            buf[base + 12], buf[base + 13], buf[base + 14], buf[base + 15],
        ]);
        let size_sectors = u64::from_le_bytes([
            buf[base + 16], buf[base + 17], buf[base + 18], buf[base + 19],
            buf[base + 20], buf[base + 21], buf[base + 22], buf[base + 23],
        ]);
        // Build the 16-byte MBR entry expected by partition_create.
        let mut entry = [0u8; 16];
        entry[0] = part;               // partition index (0-based)
        entry[1] = COREFS_MBR_TYPE;    // type byte
        entry[2] = 0;                  // not bootable
        entry[4..8].copy_from_slice(&(start_lba as u32).to_le_bytes());
        entry[8..12].copy_from_slice(&(size_sectors as u32).to_le_bytes());
        anyos_std::sys::partition_create(disk_id as u32, &entry);
        anyos_std::sys::partition_rescan(disk_id as u32);
        return;
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
        anyos_std::println!("mkfs.corefs: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };

    let label = args.get("label").unwrap_or("corefs").to_string();
    let json = args::flag_present(&args, "json");

    // --capacity is optional: when omitted, query the full device size.
    let device = if let Some(capacity) = args::parse_capacity(&args) {
        match AnyOsBlockDevice::open(device_id, capacity) {
            Ok(d) => d,
            Err(e) => {
                anyos_std::println!("mkfs.corefs: cannot open device {}: {}", device_id, e);
                return exit_code_for(&e).as_u32();
            }
        }
    } else {
        match AnyOsBlockDevice::open_auto(device_id) {
            Ok(d) => d,
            Err(e) => {
                anyos_std::println!(
                    "mkfs.corefs: cannot determine size of device {}: {}",
                    device_id, e
                );
                return exit_code_for(&e).as_u32();
            }
        }
    };
    let capacity = device.geometry().capacity_bytes;

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

    // If the device is a partition, set its MBR type to 0xCF (CoreFS).
    set_partition_type_corefs(device_id);

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

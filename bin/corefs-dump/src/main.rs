// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefs-dump` — read-only inspection of CoreFS volumes.
//!
//! ```text
//! corefs-dump --device <id> --capacity <bytes> superblock [--json]
//! corefs-dump --device <id> --capacity <bytes> inode <slot> [--json]
//! ```

#![no_std]
#![no_main]

use alloc::string::String;

use corefs_core::storage::ondisk::reader::OdfReader;
use corefs_core::storage::ondisk::volume::{inspect, VolumeInfo};
use libcorefs_tools::args;
use libcorefs_tools::block_device::AnyOsBlockDevice;
use libcorefs_tools::error::{exit_code_for, ExitCode};
use libcorefs_tools::report::{JsonBuilder, Report};

anyos_std::entry!(main);

// ---------------------------------------------------------------------------
// Subcommand: superblock
// ---------------------------------------------------------------------------

struct SuperblockView {
    device_id: u32,
    info: VolumeInfo,
}

fn uuid_to_hex(uuid: &[u8; 16]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut s = String::with_capacity(32);
    for &b in uuid {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xF) as usize] as char);
    }
    s
}

impl Report for SuperblockView {
    fn summary(&self) -> String {
        anyos_std::format!(
            "superblock of device {} — {} blocks, {} inodes, gen {}",
            self.device_id, self.info.total_blocks, self.info.total_inodes, self.info.generation
        )
    }

    fn render_text(&self) -> String {
        anyos_std::format!(
            "corefs-dump superblock\n\
             ----------------------\n\
             device id        : {}\n\
             label            : {}\n\
             uuid             : {}\n\
             total blocks     : {}\n\
             free blocks      : {}\n\
             total inodes     : {}\n\
             free inodes      : {}\n\
             generation       : {}\n\
             state            : {}\n\
             primary sb       : {}\n\
             tertiary sb      : {}\n\
             secondary sb     : {}\n",
            self.device_id,
            self.info.label,
            uuid_to_hex(&self.info.uuid),
            self.info.total_blocks,
            self.info.free_blocks,
            self.info.total_inodes,
            self.info.free_inodes,
            self.info.generation,
            self.info.state,
            if self.info.primary_ok { "ok" } else { "unreadable" },
            if self.info.tertiary_ok { "ok" } else { "unreadable" },
            if self.info.secondary_ok { "ok" } else { "unreadable" },
        )
    }

    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_u64("device", self.device_id as u64);
        b.kv_string("label", &self.info.label);
        b.kv_string("uuid", &uuid_to_hex(&self.info.uuid));
        b.kv_u64("total_blocks", self.info.total_blocks);
        b.kv_u64("free_blocks", self.info.free_blocks);
        b.kv_u64("total_inodes", self.info.total_inodes);
        b.kv_u64("free_inodes", self.info.free_inodes);
        b.kv_u64("generation", self.info.generation);
        b.kv_u64("state", self.info.state as u64);
        b.kv_bool("primary_ok", self.info.primary_ok);
        b.kv_bool("tertiary_ok", self.info.tertiary_ok);
        b.kv_bool("secondary_ok", self.info.secondary_ok);
        b.end_object();
        b.finish()
    }
}

// ---------------------------------------------------------------------------
// Subcommand: inode
// ---------------------------------------------------------------------------

struct InodeView {
    slot: u64,
    domain_id: u64,
    kind: &'static str,
    size_bytes: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    link_count: u32,
    flags: u32,
    blocks_allocated: u64,
    generation: u64,
    data_crc: u64,
}

impl Report for InodeView {
    fn summary(&self) -> String {
        anyos_std::format!(
            "inode slot {} (domain {}): {} {} bytes",
            self.slot, self.domain_id, self.kind, self.size_bytes
        )
    }

    fn render_text(&self) -> String {
        anyos_std::format!(
            "corefs-dump inode\n\
             -----------------\n\
             slot             : {}\n\
             domain inode id  : {}\n\
             kind             : {}\n\
             size             : {} bytes\n\
             mode             : 0o{:o}\n\
             uid / gid        : {} / {}\n\
             link count       : {}\n\
             flags            : 0x{:08x}\n\
             blocks allocated : {}\n\
             generation       : {}\n\
             data crc         : 0x{:016x}\n",
            self.slot,
            self.domain_id,
            self.kind,
            self.size_bytes,
            self.mode,
            self.uid,
            self.gid,
            self.link_count,
            self.flags,
            self.blocks_allocated,
            self.generation,
            self.data_crc,
        )
    }

    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_u64("slot", self.slot);
        b.kv_u64("domain_id", self.domain_id);
        b.kv_string("kind", self.kind);
        b.kv_u64("size", self.size_bytes);
        b.kv_u64("mode", self.mode as u64);
        b.kv_u64("uid", self.uid as u64);
        b.kv_u64("gid", self.gid as u64);
        b.kv_u64("link_count", self.link_count as u64);
        b.kv_u64("flags", self.flags as u64);
        b.kv_u64("blocks_allocated", self.blocks_allocated);
        b.kv_u64("generation", self.generation);
        b.kv_u64("data_crc", self.data_crc);
        b.end_object();
        b.finish()
    }
}

fn kind_str(k: corefs_core::storage::ondisk::inode::OnDiskKind) -> &'static str {
    use corefs_core::storage::ondisk::inode::OnDiskKind;
    match k {
        OnDiskKind::Unused => "unused",
        OnDiskKind::File => "file",
        OnDiskKind::Directory => "directory",
        OnDiskKind::Symlink => "symlink",
        OnDiskKind::SystemPayload => "system-payload",
    }
}

fn usage() {
    anyos_std::println!(
        "Usage:\n  \
         corefs-dump --device <id> --capacity <bytes> superblock [--json]\n  \
         corefs-dump --device <id> --capacity <bytes> inode <slot> [--json]"
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
        anyos_std::println!("corefs-dump: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(capacity) = args::parse_capacity(&args) else {
        anyos_std::println!("corefs-dump: missing or invalid --capacity <bytes>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };

    let json = args::flag_present(&args, "json");
    let Some(subcommand) = args.positional_at(0) else {
        anyos_std::println!("corefs-dump: missing subcommand (superblock | inode)");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };

    let device = match AnyOsBlockDevice::open(device_id, capacity) {
        Ok(d) => d,
        Err(e) => {
            anyos_std::println!("corefs-dump: cannot open device {}: {}", device_id, e);
            return exit_code_for(&e).as_u32();
        }
    };

    match subcommand {
        "superblock" | "sb" => {
            let info = match inspect(&device) {
                Ok(i) => i,
                Err(e) => {
                    anyos_std::println!("corefs-dump: superblock: {}", e);
                    return exit_code_for(&e).as_u32();
                }
            };
            let view = SuperblockView {
                device_id,
                info,
            };
            libcorefs_tools::report::print_report(&view, json);
            ExitCode::Success.as_u32()
        }
        "inode" => {
            let Some(slot_str) = args.positional_at(1) else {
                anyos_std::println!("corefs-dump: inode subcommand requires <slot>");
                return ExitCode::InvalidArgument.as_u32();
            };
            let Some(slot) = args::parse_u64(slot_str) else {
                anyos_std::println!("corefs-dump: invalid slot number: {}", slot_str);
                return ExitCode::InvalidArgument.as_u32();
            };
            let reader = match OdfReader::open(&device) {
                Ok(r) => r,
                Err(e) => {
                    anyos_std::println!("corefs-dump: open reader: {}", e);
                    return exit_code_for(&e).as_u32();
                }
            };
            let rec = match reader.read_on_disk_inode(slot) {
                Ok(r) => r,
                Err(e) => {
                    anyos_std::println!("corefs-dump: read inode {}: {}", slot, e);
                    return exit_code_for(&e).as_u32();
                }
            };
            let view = InodeView {
                slot,
                domain_id: rec.domain_inode_id,
                kind: kind_str(rec.kind),
                size_bytes: rec.size_bytes,
                mode: rec.mode,
                uid: rec.uid,
                gid: rec.gid,
                link_count: rec.link_count,
                flags: rec.flags,
                blocks_allocated: rec.blocks_allocated,
                generation: rec.generation,
                data_crc: rec.data_crc,
            };
            libcorefs_tools::report::print_report(&view, json);
            ExitCode::Success.as_u32()
        }
        other => {
            anyos_std::println!("corefs-dump: unknown subcommand '{}'", other);
            usage();
            ExitCode::InvalidArgument.as_u32()
        }
    }
}

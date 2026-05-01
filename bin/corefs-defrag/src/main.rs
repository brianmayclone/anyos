// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefs-defrag` — defragment the block store of a CoreFS volume.
//!
//! ```text
//! corefs-defrag --device <id> --capacity <bytes> [--json]
//! ```
//!
//! Hydriert eine [`OdfDeviceSession`] aus dem Volume, ruft
//! `PersistedState::defragment_in_place` auf und persistiert das Ergebnis
//! atomar via `session.flush()`.
//!
//! Die eigentliche Defragmentierung passiert über
//! `corefs_core::storage::block_store::BlockStore::defragment`, das belegte
//! Extents kompaktiert und freie Lücken zusammenfasst. Die On-Disk-Reichweite
//! ist auf den `block_records`-Anteil des Zustands beschränkt — Snapshot-Daten
//! und Versions-Historie bleiben unberührt.

#![no_std]
#![no_main]

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;

use corefs_core::storage::block_store::DefragmentationReport;
use corefs_core::storage::ondisk::session::OdfDeviceSession;
use libcorefs_tools::args;
use libcorefs_tools::block_device::AnyOsBlockDevice;
use libcorefs_tools::error::{exit_code_for, ExitCode};
use libcorefs_tools::report::{JsonBuilder, Report};

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!("Usage: corefs-defrag --device <id> --capacity <bytes> [--json]");
}

struct DefragReport {
    device_id: u32,
    moved_entries: usize,
    reclaimed_gaps: usize,
    final_device_blocks: u64,
}

impl DefragReport {
    fn from_inner(device_id: u32, inner: &DefragmentationReport) -> Self {
        Self {
            device_id,
            moved_entries: inner.moved_entries,
            reclaimed_gaps: inner.reclaimed_gaps,
            final_device_blocks: inner.final_device_blocks,
        }
    }
}

impl Report for DefragReport {
    fn summary(&self) -> String {
        if self.moved_entries == 0 && self.reclaimed_gaps == 0 {
            format!(
                "defrag: nothing to do (device {}, {} blocks in use)",
                self.device_id, self.final_device_blocks
            )
        } else {
            format!(
                "defrag ok (device {}, {} entries moved, {} gaps reclaimed, {} blocks in use)",
                self.device_id, self.moved_entries, self.reclaimed_gaps, self.final_device_blocks
            )
        }
    }

    fn render_text(&self) -> String {
        format!(
            "corefs-defrag report\n--------------------\n\
             device id            : {}\n\
             moved entries        : {}\n\
             reclaimed gaps       : {}\n\
             final device blocks  : {}\n",
            self.device_id, self.moved_entries, self.reclaimed_gaps, self.final_device_blocks
        )
    }

    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_u64("device", self.device_id as u64);
        b.kv_u64("moved_entries", self.moved_entries as u64);
        b.kv_u64("reclaimed_gaps", self.reclaimed_gaps as u64);
        b.kv_u64("final_device_blocks", self.final_device_blocks);
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
        anyos_std::println!("corefs-defrag: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(capacity) = args::parse_capacity(&args) else {
        anyos_std::println!("corefs-defrag: missing or invalid --capacity <bytes>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let json = args::flag_present(&args, "json");

    let device = match AnyOsBlockDevice::open(device_id, capacity) {
        Ok(d) => d,
        Err(e) => {
            anyos_std::println!("corefs-defrag: cannot open device {}: {}", device_id, e);
            return exit_code_for(&e).as_u32();
        }
    };
    let mut session = match OdfDeviceSession::open(Box::new(device)) {
        Ok(s) => s,
        Err(e) => {
            anyos_std::println!("corefs-defrag: cannot hydrate volume: {}", e);
            return exit_code_for(&e).as_u32();
        }
    };

    let result = session.mutate(|state| Ok(state.defragment_in_place()));
    match result {
        Ok((inner, _flush)) => {
            libcorefs_tools::report::print_report(
                &DefragReport::from_inner(device_id, &inner),
                json,
            );
            ExitCode::Success.as_u32()
        }
        Err(e) => {
            anyos_std::println!("corefs-defrag: defragment failed: {}", e);
            exit_code_for(&e).as_u32()
        }
    }
}

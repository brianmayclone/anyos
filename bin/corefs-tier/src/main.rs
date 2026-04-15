// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefs-tier` — hot/cold storage tiering inspection for CoreFS volumes.
//!
//! ```text
//! corefs-tier --device <id> --capacity <bytes> status [--json] [--top <n>]
//! ```
//!
//! Der `status`-Subcommand aggregiert den im `PersistedState` persistierten
//! Tiering-Zustand: Anzahl aktiver Datei-Inodes je `StorageTier` sowie die
//! heißesten Pfade laut Hot-Path-Telemetrie.
//!
//! `promote` und `demote` bleiben absichtlich `Unsupported` — echte Tier-
//! Migration benötigt zwei physische Devices und läuft über die std-
//! gebundene `storage::ondisk::tiering::Migrator`-Pipeline, die nicht im
//! AnyOS-Userspace verfügbar ist.

#![no_std]
#![no_main]

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use corefs_core::storage::ondisk::session::OdfDeviceSession;
use corefs_core::storage::ondisk::tier::tier_status_from_state;
use libcorefs_tools::args;
use libcorefs_tools::block_device::AnyOsBlockDevice;
use libcorefs_tools::error::{exit_code_for, ExitCode};
use libcorefs_tools::report::{JsonBuilder, Report};

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!(
        "Usage: corefs-tier --device <id> --capacity <bytes> status [--json] [--top <n>]\n       \
         corefs-tier --device <id> --capacity <bytes> <promote|demote>  (unsupported)"
    );
}

struct StatusReportView {
    total_inodes: usize,
    hot_inodes: usize,
    warm_inodes: usize,
    cold_inodes: usize,
    hottest: Vec<(String, u64)>,
}

impl Report for StatusReportView {
    fn summary(&self) -> String {
        format!(
            "tier: files hot={} warm={} cold={} (total inodes={})",
            self.hot_inodes, self.warm_inodes, self.cold_inodes, self.total_inodes
        )
    }
    fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str("tier status\n-----------\n");
        out.push_str(&format!("total inodes : {}\n", self.total_inodes));
        out.push_str(&format!("hot files    : {}\n", self.hot_inodes));
        out.push_str(&format!("warm files   : {}\n", self.warm_inodes));
        out.push_str(&format!("cold files   : {}\n", self.cold_inodes));
        out.push_str("hottest paths:\n");
        if self.hottest.is_empty() {
            out.push_str("  (no hot-path telemetry recorded)\n");
        } else {
            for (path, score) in &self.hottest {
                out.push_str(&format!("  score={:<8} {}\n", score, path));
            }
        }
        out
    }
    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_u64("total_inodes", self.total_inodes as u64);
        b.kv_u64("hot_inodes", self.hot_inodes as u64);
        b.kv_u64("warm_inodes", self.warm_inodes as u64);
        b.kv_u64("cold_inodes", self.cold_inodes as u64);
        b.key("hottest");
        b.begin_array();
        for (path, score) in &self.hottest {
            b.begin_object();
            b.kv_string("path", path);
            b.kv_u64("score", *score);
            b.end_object();
        }
        b.end_array();
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
        anyos_std::println!("corefs-tier: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(capacity) = args::parse_capacity(&args) else {
        anyos_std::println!("corefs-tier: missing or invalid --capacity <bytes>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(sub) = args.positional_at(0) else {
        anyos_std::println!("corefs-tier: missing subcommand");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let json = args::flag_present(&args, "json");
    let top_n = args.get_u64("top").unwrap_or(10) as usize;

    match sub {
        "status" => {
            let device = match AnyOsBlockDevice::open(device_id, capacity) {
                Ok(d) => d,
                Err(e) => {
                    anyos_std::println!("corefs-tier: cannot open device {}: {}", device_id, e);
                    return exit_code_for(&e).as_u32();
                }
            };
            let session = match OdfDeviceSession::open(Box::new(device)) {
                Ok(s) => s,
                Err(e) => {
                    anyos_std::println!("corefs-tier: cannot hydrate volume: {}", e);
                    return exit_code_for(&e).as_u32();
                }
            };
            let report = tier_status_from_state(session.state(), top_n);
            libcorefs_tools::report::print_report(
                &StatusReportView {
                    total_inodes: report.total_inodes,
                    hot_inodes: report.hot_inodes,
                    warm_inodes: report.warm_inodes,
                    cold_inodes: report.cold_inodes,
                    hottest: report
                        .hottest
                        .into_iter()
                        .map(|(p, s)| (p.to_string(), s))
                        .collect(),
                },
                json,
            );
            ExitCode::Success.as_u32()
        }
        "promote" | "demote" => {
            anyos_std::println!(
                "corefs-tier {}: requires two physical devices and the std-bound \
                 Migrator pipeline — not available in AnyOS userspace.\n\
                 Use 'corefs-tier status' to inspect hot-path telemetry and \
                 per-file storage_tier assignments instead.",
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

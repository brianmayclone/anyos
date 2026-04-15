// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefs-tier` — hot/cold storage tiering inspection and migration for
//! CoreFS volumes.
//!
//! ## Single-device mode (inspection)
//!
//! ```text
//! corefs-tier --device <id> --capacity <bytes> status [--json] [--top <n>]
//! ```
//!
//! Aggregiert den im `PersistedState` persistierten Tiering-Zustand: Anzahl
//! aktiver Datei-Inodes je `StorageTier` sowie die heissesten Pfade laut
//! Hot-Path-Telemetrie.
//!
//! ## Multi-device mode (block migration)
//!
//! ```text
//! corefs-tier --hot-device <id> --hot-capacity <bytes> \
//!             --cold-device <id> --cold-capacity <bytes> \
//!             rebalance [--max-migrations <N>] [--json]
//! corefs-tier --hot-device <id> --hot-capacity <bytes> \
//!             --cold-device <id> --cold-capacity <bytes> \
//!             promote --block <N>
//! corefs-tier --hot-device <id> --hot-capacity <bytes> \
//!             --cold-device <id> --cold-capacity <bytes> \
//!             demote --block <N>
//! ```
//!
//! `rebalance` benutzt einen leeren [`HotnessTracker`] — in der aktuellen
//! AnyOS-Integration existiert noch keine persistierte Telemetrie, die die
//! tatsächliche Zugriffsfrequenz pro Block liefert. Die Operation ist
//! deshalb ein No-op-Gerüst; explizite `promote`/`demote`-Aufrufe sind der
//! praktikable Weg, einzelne Blöcke gezielt zwischen Tiers zu verschieben.

#![no_std]
#![no_main]

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use corefs_core::storage::block_device::BlockDevice;
use corefs_core::storage::ondisk::session::OdfDeviceSession;
use corefs_core::storage::ondisk::tier::tier_status_from_state;
use corefs_core::storage::ondisk::tiering::{
    HotnessTracker, Migrator, Tier, TierMap, TierPolicy, TieredDevice,
};
use libcorefs_tools::args::{self, LongArgs};
use libcorefs_tools::block_device::{AnyOsBlockDevice, SyscallBackend};
use libcorefs_tools::error::{exit_code_for, ExitCode};
use libcorefs_tools::report::{JsonBuilder, Report};

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!(
        "Usage:\n  \
         corefs-tier --device <id> --capacity <bytes> status [--json] [--top <n>]\n  \
         corefs-tier --hot-device <id> --hot-capacity <bytes> --cold-device <id> --cold-capacity <bytes> rebalance [--max-migrations <N>] [--json]\n  \
         corefs-tier --hot-device <id> --hot-capacity <bytes> --cold-device <id> --cold-capacity <bytes> promote --block <N>\n  \
         corefs-tier --hot-device <id> --hot-capacity <bytes> --cold-device <id> --cold-capacity <bytes> demote --block <N>"
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

struct MigrationReportView {
    action: &'static str,
    promoted: usize,
    demoted: usize,
    considered: usize,
    block: Option<u64>,
}

impl Report for MigrationReportView {
    fn summary(&self) -> String {
        match self.block {
            Some(b) => format!("{}: block={} ok", self.action, b),
            None => format!(
                "{}: promoted={} demoted={} considered={}",
                self.action, self.promoted, self.demoted, self.considered
            ),
        }
    }
    fn render_text(&self) -> String {
        self.summary() + "\n"
    }
    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_string("action", self.action);
        if let Some(blk) = self.block {
            b.kv_u64("block", blk);
        }
        b.kv_u64("promoted", self.promoted as u64);
        b.kv_u64("demoted", self.demoted as u64);
        b.kv_u64("considered", self.considered as u64);
        b.end_object();
        b.finish()
    }
}

// ---------------------------------------------------------------------------
// Dual-device helpers
// ---------------------------------------------------------------------------

fn parse_u32(args: &LongArgs, name: &str) -> Option<u32> {
    args.get_u32(name)
}

fn parse_u64(args: &LongArgs, name: &str) -> Option<u64> {
    args.get_u64(name)
}

fn open_tiered(
    args: &LongArgs,
) -> Result<TieredDevice<AnyOsBlockDevice<SyscallBackend>, AnyOsBlockDevice<SyscallBackend>>, u32> {
    let Some(hot_id) = parse_u32(args, "hot-device") else {
        anyos_std::println!("corefs-tier: missing or invalid --hot-device <id>");
        return Err(ExitCode::InvalidArgument.as_u32());
    };
    let Some(hot_cap) = parse_u64(args, "hot-capacity") else {
        anyos_std::println!("corefs-tier: missing or invalid --hot-capacity <bytes>");
        return Err(ExitCode::InvalidArgument.as_u32());
    };
    let Some(cold_id) = parse_u32(args, "cold-device") else {
        anyos_std::println!("corefs-tier: missing or invalid --cold-device <id>");
        return Err(ExitCode::InvalidArgument.as_u32());
    };
    let Some(cold_cap) = parse_u64(args, "cold-capacity") else {
        anyos_std::println!("corefs-tier: missing or invalid --cold-capacity <bytes>");
        return Err(ExitCode::InvalidArgument.as_u32());
    };

    let hot = match AnyOsBlockDevice::open(hot_id, hot_cap) {
        Ok(d) => d,
        Err(e) => {
            anyos_std::println!("corefs-tier: cannot open hot device {}: {}", hot_id, e);
            return Err(exit_code_for(&e).as_u32());
        }
    };
    let cold = match AnyOsBlockDevice::open(cold_id, cold_cap) {
        Ok(d) => d,
        Err(e) => {
            anyos_std::println!("corefs-tier: cannot open cold device {}: {}", cold_id, e);
            return Err(exit_code_for(&e).as_u32());
        }
    };

    // Sanity: TieredDevice::new requires matching capacities. Surface a
    // specific, actionable error instead of a generic I/O failure.
    if hot.capacity() != cold.capacity() {
        anyos_std::println!(
            "corefs-tier: hot capacity {} != cold capacity {} — \
             tiered device requires equal logical sizes",
            hot.capacity(),
            cold.capacity()
        );
        return Err(ExitCode::InvalidArgument.as_u32());
    }

    // The in-tree `Tier` enum exposes only `Hot` and `Cold`. We initialise
    // the map with `Cold` as the conservative default — explicit promote
    // operations move individual blocks to the hot tier.
    TieredDevice::new(hot, cold, TierMap::new(Tier::Cold)).map_err(|e| {
        anyos_std::println!("corefs-tier: tiered device init failed: {}", e);
        exit_code_for(&e).as_u32()
    })
}

fn main() -> u32 {
    let mut buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut buf);
    let args = args::parse(raw);

    if args.has("help") {
        usage();
        return ExitCode::Success.as_u32();
    }

    let Some(sub) = args.positional_at(0) else {
        anyos_std::println!("corefs-tier: missing subcommand");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let json = args::flag_present(&args, "json");

    match sub {
        "status" => run_status(&args, json),
        "rebalance" => run_rebalance(&args, json),
        "promote" => run_migrate(&args, Tier::Hot, "promote", json),
        "demote" => run_migrate(&args, Tier::Cold, "demote", json),
        other => {
            anyos_std::println!("corefs-tier: unknown subcommand '{}'", other);
            usage();
            ExitCode::InvalidArgument.as_u32()
        }
    }
}

fn run_status(args: &LongArgs, json: bool) -> u32 {
    let Some(device_id) = args::parse_device_id(args) else {
        anyos_std::println!("corefs-tier: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(capacity) = args::parse_capacity(args) else {
        anyos_std::println!("corefs-tier: missing or invalid --capacity <bytes>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let top_n = args.get_u64("top").unwrap_or(10) as usize;

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

fn run_rebalance(args: &LongArgs, json: bool) -> u32 {
    let mut tiered = match open_tiered(args) {
        Ok(t) => t,
        Err(code) => return code,
    };
    let max_migrations = args.get_u64("max-migrations").unwrap_or(64) as usize;
    let mut policy = TierPolicy::balanced();
    policy.max_migrations_per_pass = max_migrations;

    // No persisted access-frequency feed yet → empty tracker. rebalance()
    // still enforces the demote pass over explicit Hot entries; promotion
    // is a no-op until telemetry is wired in.
    let tracker = HotnessTracker::new();
    let report = match Migrator::rebalance(&mut tiered, &tracker, &policy) {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("corefs-tier: rebalance failed: {}", e);
            return exit_code_for(&e).as_u32();
        }
    };
    libcorefs_tools::report::print_report(
        &MigrationReportView {
            action: "rebalance",
            promoted: report.promoted,
            demoted: report.demoted,
            considered: report.considered,
            block: None,
        },
        json,
    );
    ExitCode::Success.as_u32()
}

fn run_migrate(args: &LongArgs, target: Tier, action: &'static str, json: bool) -> u32 {
    let Some(block) = args.get_u64("block") else {
        anyos_std::println!("corefs-tier: missing or invalid --block <N>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let mut tiered = match open_tiered(args) {
        Ok(t) => t,
        Err(code) => return code,
    };
    if let Err(e) = tiered.migrate_block(block, target) {
        anyos_std::println!("corefs-tier: {} block {} failed: {}", action, block, e);
        return exit_code_for(&e).as_u32();
    }
    libcorefs_tools::report::print_report(
        &MigrationReportView {
            action,
            promoted: if matches!(target, Tier::Hot) { 1 } else { 0 },
            demoted: if matches!(target, Tier::Cold) { 1 } else { 0 },
            considered: 1,
            block: Some(block),
        },
        json,
    );
    ExitCode::Success.as_u32()
}

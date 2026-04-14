// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `fsck.corefs` — structural check (and optional repair) of a CoreFS volume.

#![no_std]
#![no_main]

use alloc::string::String;

use corefs_core::storage::ondisk::fsck::{self, FsckIssue, FsckReport, Severity};
use corefs_core::storage::ondisk::fsck_repair;
use libcorefs_tools::args;
use libcorefs_tools::block_device::AnyOsBlockDevice;
use libcorefs_tools::error::{exit_code_for, ExitCode};
use libcorefs_tools::report::{JsonBuilder, Report};

anyos_std::entry!(main);

struct CombinedReport {
    device_id: u32,
    fsck: FsckReport,
    repair: Option<fsck_repair::RepairReport>,
}

fn severity_str(sev: Severity) -> &'static str {
    match sev {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

fn issues_to_text(issues: &[FsckIssue]) -> String {
    let mut out = String::new();
    for issue in issues {
        out.push_str(&anyos_std::format!(
            "  [{:>7}] {}: {}\n",
            severity_str(issue.severity),
            issue.code,
            issue.message,
        ));
    }
    if out.is_empty() {
        out.push_str("  (no issues)\n");
    }
    out
}

impl Report for CombinedReport {
    fn summary(&self) -> String {
        let clean = self.fsck.is_clean();
        anyos_std::format!(
            "device {} : {} issues ({} errors, {} warnings) — {}",
            self.device_id,
            self.fsck.issues.len(),
            self.fsck.count(Severity::Error),
            self.fsck.count(Severity::Warning),
            if clean { "CLEAN" } else { "DIRTY" },
        )
    }

    fn render_text(&self) -> String {
        let mut out = anyos_std::format!(
            "fsck.corefs report\n\
             ------------------\n\
             device id        : {}\n\
             inodes checked   : {}\n\
             extents checked  : {}\n\
             blocks referenced: {}\n\
             issues           : {} ({} errors, {} warnings, {} info)\n\
             status           : {}\n\nIssues:\n",
            self.device_id,
            self.fsck.inodes_checked,
            self.fsck.extents_checked,
            self.fsck.blocks_referenced,
            self.fsck.issues.len(),
            self.fsck.count(Severity::Error),
            self.fsck.count(Severity::Warning),
            self.fsck.count(Severity::Info),
            if self.fsck.is_clean() { "CLEAN" } else { "DIRTY" },
        );
        out.push_str(&issues_to_text(&self.fsck.issues));
        if let Some(r) = &self.repair {
            out.push_str(&anyos_std::format!(
                "\nRepair pass:\n  ops committed : {}\n  fixed codes   : {}\n  unfixable     : {}\n",
                r.ops_committed,
                r.fixed.len(),
                r.unfixable.len(),
            ));
            for code in &r.fixed {
                out.push_str(&anyos_std::format!("    fixed: {}\n", code));
            }
            for issue in &r.unfixable {
                out.push_str(&anyos_std::format!(
                    "    unfixable [{:>7}] {}: {}\n",
                    severity_str(issue.severity),
                    issue.code,
                    issue.message,
                ));
            }
        }
        out
    }

    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_u64("device", self.device_id as u64);
        b.kv_u64("inodes_checked", self.fsck.inodes_checked);
        b.kv_u64("extents_checked", self.fsck.extents_checked);
        b.kv_u64("blocks_referenced", self.fsck.blocks_referenced);
        b.kv_bool("clean", self.fsck.is_clean());
        b.kv_u64("error_count", self.fsck.count(Severity::Error) as u64);
        b.kv_u64("warning_count", self.fsck.count(Severity::Warning) as u64);
        b.kv_u64("info_count", self.fsck.count(Severity::Info) as u64);
        b.key("issues");
        b.begin_array();
        for issue in &self.fsck.issues {
            b.begin_object();
            b.kv_string("severity", severity_str(issue.severity));
            b.kv_string("code", issue.code);
            b.kv_string("message", &issue.message);
            b.end_object();
        }
        b.end_array();
        if let Some(r) = &self.repair {
            b.key("repair");
            b.begin_object();
            b.kv_u64("ops_committed", r.ops_committed as u64);
            b.key("fixed");
            b.begin_array();
            for code in &r.fixed {
                b.string(code);
            }
            b.end_array();
            b.key("unfixable");
            b.begin_array();
            for issue in &r.unfixable {
                b.begin_object();
                b.kv_string("severity", severity_str(issue.severity));
                b.kv_string("code", issue.code);
                b.kv_string("message", &issue.message);
                b.end_object();
            }
            b.end_array();
            b.end_object();
        }
        b.end_object();
        b.finish()
    }
}

fn usage() {
    anyos_std::println!(
        "Usage: fsck.corefs --device <id> --capacity <bytes> [--repair] [--json]"
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
        anyos_std::println!("fsck.corefs: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(capacity) = args::parse_capacity(&args) else {
        anyos_std::println!("fsck.corefs: missing or invalid --capacity <bytes>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };

    let do_repair = args::flag_present(&args, "repair");
    let json = args::flag_present(&args, "json");

    let mut device = match AnyOsBlockDevice::open(device_id, capacity) {
        Ok(d) => d,
        Err(e) => {
            anyos_std::println!("fsck.corefs: cannot open device {}: {}", device_id, e);
            return exit_code_for(&e).as_u32();
        }
    };

    let fsck_report = match fsck::check(&device) {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fsck.corefs: check failed: {}", e);
            return exit_code_for(&e).as_u32();
        }
    };

    let repair_report = if do_repair {
        match fsck_repair::repair(&mut device, &fsck_report) {
            Ok(r) => Some(r),
            Err(e) => {
                anyos_std::println!("fsck.corefs: repair failed: {}", e);
                return exit_code_for(&e).as_u32();
            }
        }
    } else {
        None
    };

    let clean = fsck_report.is_clean()
        && repair_report.as_ref().map_or(true, |r| r.unfixable.is_empty());

    let combined = CombinedReport {
        device_id,
        fsck: fsck_report,
        repair: repair_report,
    };
    libcorefs_tools::report::print_report(&combined, json);

    if clean {
        ExitCode::Success.as_u32()
    } else {
        ExitCode::Corruption.as_u32()
    }
}

// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefs-scrub` — background scrub pass against a CoreFS volume.
//!
//! ```text
//! corefs-scrub --device <id> --capacity <bytes>
//!              [--mode full|structural|read-only] [--json]
//! ```

#![no_std]
#![no_main]

use alloc::string::String;

use corefs_core::storage::ondisk::fsck::Severity;
use corefs_core::storage::ondisk::scrub::{self, ScrubPlan, ScrubReport};
use libcorefs_tools::args;
use libcorefs_tools::block_device::AnyOsBlockDevice;
use libcorefs_tools::error::{exit_code_for, ExitCode};
use libcorefs_tools::report::{JsonBuilder, Report};

anyos_std::entry!(main);

struct ScrubView {
    device_id: u32,
    mode: &'static str,
    report: ScrubReport,
}

fn severity_str(sev: Severity) -> &'static str {
    match sev {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

impl Report for ScrubView {
    fn summary(&self) -> String {
        anyos_std::format!(
            "scrub({}) device {} — {} extents, {} blocks, {} data corruptions, {} residual issues",
            self.mode,
            self.device_id,
            self.report.extents_verified,
            self.report.blocks_verified,
            self.report.data_corruptions.len(),
            self.report.residual_issues.len(),
        )
    }

    fn render_text(&self) -> String {
        let mut out = anyos_std::format!(
            "corefs-scrub report\n\
             -------------------\n\
             device id             : {}\n\
             mode                  : {}\n\
             extents verified      : {}\n\
             blocks verified       : {}\n\
             fsck issues (initial) : {}\n\
             repair ops committed  : {}\n\
             residual issues       : {}\n\
             data corruptions      : {}\n\
             status                : {}\n",
            self.device_id,
            self.mode,
            self.report.extents_verified,
            self.report.blocks_verified,
            self.report.fsck_issues_before,
            self.report.repair_ops_committed,
            self.report.residual_issues.len(),
            self.report.data_corruptions.len(),
            if self.report.is_clean() { "CLEAN" } else { "DIRTY" },
        );
        for (slot, domain) in &self.report.data_corruptions {
            out.push_str(&anyos_std::format!(
                "  DATA CORRUPTION slot={} domain_id={}\n",
                slot, domain
            ));
        }
        for issue in &self.report.residual_issues {
            out.push_str(&anyos_std::format!(
                "  [{:>7}] {}: {}\n",
                severity_str(issue.severity),
                issue.code,
                issue.message,
            ));
        }
        out
    }

    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_u64("device", self.device_id as u64);
        b.kv_string("mode", self.mode);
        b.kv_u64("extents_verified", self.report.extents_verified);
        b.kv_u64("blocks_verified", self.report.blocks_verified);
        b.kv_u64("fsck_issues_before", self.report.fsck_issues_before as u64);
        b.kv_u64("repair_ops_committed", self.report.repair_ops_committed as u64);
        b.kv_bool("clean", self.report.is_clean());
        b.key("data_corruptions");
        b.begin_array();
        for (slot, domain) in &self.report.data_corruptions {
            b.begin_object();
            b.kv_u64("slot", *slot);
            b.kv_u64("domain_id", *domain);
            b.end_object();
        }
        b.end_array();
        b.key("residual_issues");
        b.begin_array();
        for issue in &self.report.residual_issues {
            b.begin_object();
            b.kv_string("severity", severity_str(issue.severity));
            b.kv_string("code", issue.code);
            b.kv_string("message", &issue.message);
            b.end_object();
        }
        b.end_array();
        b.end_object();
        b.finish()
    }
}

fn usage() {
    anyos_std::println!(
        "Usage: corefs-scrub --device <id> --capacity <bytes>\n                    \
         [--mode full|structural|read-only] [--json]"
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
        anyos_std::println!("corefs-scrub: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(capacity) = args::parse_capacity(&args) else {
        anyos_std::println!("corefs-scrub: missing or invalid --capacity <bytes>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };

    let json = args::flag_present(&args, "json");
    let mode_raw = args.get("mode").unwrap_or("full");
    let (plan, mode_name) = match mode_raw {
        "full" => (ScrubPlan::full(), "full"),
        "structural" | "structural-only" => (ScrubPlan::structural_only(), "structural"),
        "read-only" | "readonly" | "ro" => (ScrubPlan::read_only(), "read-only"),
        other => {
            anyos_std::println!(
                "corefs-scrub: unknown --mode '{}', expected full|structural|read-only",
                other
            );
            return ExitCode::InvalidArgument.as_u32();
        }
    };

    let mut device = match AnyOsBlockDevice::open(device_id, capacity) {
        Ok(d) => d,
        Err(e) => {
            anyos_std::println!("corefs-scrub: cannot open device {}: {}", device_id, e);
            return exit_code_for(&e).as_u32();
        }
    };

    let report = match scrub::run(&mut device, &plan) {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("corefs-scrub: scrub failed: {}", e);
            return exit_code_for(&e).as_u32();
        }
    };

    let clean = report.is_clean();
    let view = ScrubView {
        device_id,
        mode: mode_name,
        report,
    };
    libcorefs_tools::report::print_report(&view, json);

    if clean {
        ExitCode::Success.as_u32()
    } else {
        ExitCode::Corruption.as_u32()
    }
}

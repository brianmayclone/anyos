use alloc::string::{String, ToString};
use alloc::vec::Vec;
use anyos_std::format;
use anyos_std::fs;

use crate::detect::detect_existing_installation;
use crate::model::{UpgradeList, UpgradeOperation};
use crate::transaction::has_pending_upgrade;
use crate::util::{file_size, is_config_path, join_root, path_exists, read_dir};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PreflightCheck {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PreflightCode {
    ExistingInstallMissing,
    PendingUpgrade,
    SourceMissing,
    SourceTreeEmpty,
    VersionOutsidePlan,
    SpaceUnknown,
    InsufficientSpace,
    CriticalSourceMissing,
}

#[derive(Clone)]
pub struct PreflightIssue {
    pub level: PreflightCheck,
    pub code: PreflightCode,
    pub message: String,
}

pub struct PreflightReport {
    pub ok: bool,
    pub estimated_required_bytes: u64,
    pub available_bytes: Option<u64>,
    pub issues: Vec<PreflightIssue>,
}

pub fn preflight_upgrade(root: &str, list: &UpgradeList) -> PreflightReport {
    let mut report = PreflightReport {
        ok: true,
        estimated_required_bytes: 0,
        available_bytes: fs::statfs(root).map(|st| st.free_bytes),
        issues: Vec::new(),
    };

    let existing = match detect_existing_installation(root) {
        Some(existing) => existing,
        None => {
            push_issue(
                &mut report,
                PreflightCheck::Error,
                PreflightCode::ExistingInstallMissing,
                "No existing anyOS installation found on target",
            );
            return report;
        }
    };

    if has_pending_upgrade(root) {
        push_issue(
            &mut report,
            PreflightCheck::Error,
            PreflightCode::PendingUpgrade,
            "A previous upgrade is still pending recovery",
        );
    }

    validate_version(&existing.version, list, &mut report);

    let critical_sources = ["/System/krnl64", "/boot/boot.cfg"];
    for path in critical_sources {
        if !path_exists(path) {
            push_issue(
                &mut report,
                PreflightCheck::Error,
                PreflightCode::CriticalSourceMissing,
                &format!("Critical upgrade source is missing: {}", path),
            );
        }
    }

    let mut estimated = 0u64;
    for op in &list.operations {
        match op {
            UpgradeOperation::EnsureDir { .. } => {}
            UpgradeOperation::SyncTree {
                source,
                target,
                merge_configs,
                preserve_existing,
            } => {
                if !path_exists(source) {
                    push_issue(
                        &mut report,
                        PreflightCheck::Error,
                        PreflightCode::SourceMissing,
                        &format!("Upgrade source missing: {}", source),
                    );
                    continue;
                }

                match estimate_sync_bytes(root, source, target, *merge_configs, *preserve_existing)
                {
                    Ok(0) => {
                        push_issue(
                            &mut report,
                            PreflightCheck::Warning,
                            PreflightCode::SourceTreeEmpty,
                            &format!("Upgrade source is empty: {}", source),
                        );
                    }
                    Ok(bytes) => estimated = estimated.saturating_add(bytes),
                    Err(err) => push_issue(
                        &mut report,
                        PreflightCheck::Error,
                        PreflightCode::SourceMissing,
                        &err,
                    ),
                }
            }
        }
    }

    report.estimated_required_bytes = estimated.saturating_add(16 * 1024 * 1024);
    match report.available_bytes {
        Some(free) if free < report.estimated_required_bytes => {
            let required = report.estimated_required_bytes;
            push_issue(
                &mut report,
                PreflightCheck::Error,
                PreflightCode::InsufficientSpace,
                &format!(
                    "Insufficient free space: need about {} bytes, have {} bytes",
                    required, free
                ),
            )
        }
        Some(_) => {}
        None => push_issue(
            &mut report,
            PreflightCheck::Warning,
            PreflightCode::SpaceUnknown,
            "Could not determine free disk space on target",
        ),
    }

    report
}

fn validate_version(version: &str, list: &UpgradeList, report: &mut PreflightReport) {
    if list.from_version == "0.0.0" {
        return;
    }

    if compare_versions(version, &list.from_version) < 0 {
        push_issue(
            report,
            PreflightCheck::Error,
            PreflightCode::VersionOutsidePlan,
            &format!(
                "Installed version {} is older than supported upgrade baseline {}",
                version, list.from_version
            ),
        );
    }
}

fn estimate_sync_bytes(
    root: &str,
    src: &str,
    target: &str,
    merge_configs: bool,
    preserve_existing: bool,
) -> Result<u64, String> {
    let entries = read_dir(src)?;
    let mut total = 0u64;

    for entry in entries {
        let child_src = format!("{}/{}", src.trim_end_matches('/'), entry.name);
        let child_target_rel = format!("{}/{}", target.trim_end_matches('/'), entry.name);
        let child_dst = join_root(root, &child_target_rel);

        if entry.entry_type == 1 {
            total = total.saturating_add(estimate_sync_bytes(
                root,
                &child_src,
                &child_target_rel,
                merge_configs,
                preserve_existing,
            )?);
            continue;
        }

        let src_size = file_size(&child_src).unwrap_or(0);
        let dst_size = file_size(&child_dst).unwrap_or(0);

        if path_exists(&child_dst) && preserve_existing {
            continue;
        }

        if path_exists(&child_dst) {
            total = total.saturating_add(dst_size);
            if merge_configs && is_config_path(&child_target_rel) {
                total = total.saturating_add(src_size.max(dst_size));
            } else {
                total = total.saturating_add(src_size);
            }
        } else {
            total = total.saturating_add(src_size);
        }
    }

    Ok(total)
}

fn compare_versions(left: &str, right: &str) -> i32 {
    let mut left_parts = left.split('.');
    let mut right_parts = right.split('.');

    loop {
        let a = left_parts.next();
        let b = right_parts.next();
        if a.is_none() && b.is_none() {
            return 0;
        }

        let av = a.and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        let bv = b.and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        if av < bv {
            return -1;
        }
        if av > bv {
            return 1;
        }
    }
}

fn push_issue(
    report: &mut PreflightReport,
    level: PreflightCheck,
    code: PreflightCode,
    message: &str,
) {
    if level == PreflightCheck::Error {
        report.ok = false;
    }
    report.issues.push(PreflightIssue {
        level,
        code,
        message: message.to_string(),
    });
}

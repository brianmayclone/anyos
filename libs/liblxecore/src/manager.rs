use crate::config::LxeConfig;
use crate::daemon;
use crate::package::{
    install_package, install_resolved_plan, package_installed, prefetch_install_plan,
    resolve_install_plan, InstallProgress, InstallProgressObserver,
};
use crate::rootfs::{ensure_dir_recursive, ensure_rootfs_layout, file_size, path_exists};
use alloc::format;
use alloc::string::String;
use anyos_std::{fs, sys};

const FS_TYPE_DIRECTORY: u32 = 1;
const INSTALL_WEIGHT_PERCENT: u32 = 92;

pub struct InstallationInfo {
    pub root: String,
    pub rootfs: String,
    pub files: u32,
    pub dirs: u32,
    pub bytes: u64,
    pub seed_total: u32,
    pub seed_installed: u32,
    pub has_payload: bool,
    pub complete: bool,
}

pub struct ManagerProgress {
    pub percent: u32,
    pub step: u32,
    pub steps: u32,
    pub phase: String,
    pub detail: String,
    pub bytes_done: u32,
    pub bytes_total: u32,
    pub eta_secs: u32,
}

pub trait ManagerProgressSink {
    fn update(&mut self, progress: &ManagerProgress);
}

pub fn inspect(config: &LxeConfig) -> InstallationInfo {
    let mut info = InstallationInfo {
        root: config.root.clone(),
        rootfs: config.rootfs.clone(),
        files: 0,
        dirs: 0,
        bytes: 0,
        seed_total: config.bootstrap_seed.len() as u32,
        seed_installed: 0,
        has_payload: false,
        complete: false,
    };
    scan_tree(
        &config.rootfs,
        &mut info.files,
        &mut info.dirs,
        &mut info.bytes,
        0,
    );
    info.has_payload = info.files > 0;
    for package in &config.bootstrap_seed {
        if package_installed(config, package, &config.rootfs) {
            info.seed_installed += 1;
        }
    }
    info.complete = info.seed_total > 0 && info.seed_installed == info.seed_total;
    info
}

pub fn install_or_repair(
    config: &mut LxeConfig,
    sink: &mut dyn ManagerProgressSink,
) -> Result<(), String> {
    let _lease = daemon::acquire_write(config, "manager-install")?;
    publish(
        sink,
        1,
        0,
        4,
        "Preparing LXE rootfs",
        &config.rootfs,
        0,
        0,
        0,
    );
    ensure_rootfs_layout(config);

    {
        let mut observer = BootstrapObserver::new(sink, config.bootstrap_seed.len() as u32);
        let mut progress = InstallProgress::with_observer(
            false,
            config.bootstrap_seed.len() as u32,
            "packages",
            &mut observer,
        );
        progress.set_overall(0, config.bootstrap_seed.len() as u32);

        let mut done = 0u32;
        let mut plan_installed = false;
        let plan_packages = config.bootstrap_seed.clone();
        if let Some(plan) =
            resolve_install_plan(config, &plan_packages, &config.rootfs, &mut progress)
        {
            let _ = prefetch_install_plan(config, &plan, &mut progress);
            plan_installed = install_resolved_plan(config, &plan, &config.rootfs, &mut progress);
        }

        if plan_installed {
            done = count_installed_bootstrap_packages(config);
            progress.set_overall(done, config.bootstrap_seed.len() as u32);
        }

        if !plan_installed {
            for package in &config.bootstrap_seed {
                if package_installed(config, package, &config.rootfs) {
                    done += 1;
                    progress.set_overall(done, config.bootstrap_seed.len() as u32);
                    continue;
                }
                if !install_package(config, package, &config.rootfs, 0, &mut progress) {
                    return Err(format!("package '{}' could not be installed", package));
                }
                done += 1;
                progress.set_overall(done, config.bootstrap_seed.len() as u32);
            }
        }
        progress.finish();
    }

    publish(
        sink,
        94,
        3,
        4,
        "Repairing runtime links",
        "ld.so, libc and common library aliases",
        0,
        0,
        0,
    );
    crate::rootfs::repair_rootfs_runtime(&config.rootfs);
    publish(
        sink,
        98,
        4,
        4,
        "Synchronizing rootfs",
        "Flushing LXE files to disk",
        0,
        0,
        0,
    );
    daemon::sync_rootfs();
    fs::sync();
    publish(
        sink,
        100,
        4,
        4,
        "LXE is ready",
        "Installation completed successfully.",
        0,
        0,
        0,
    );
    Ok(())
}

pub fn repair_only(
    config: &mut LxeConfig,
    sink: &mut dyn ManagerProgressSink,
) -> Result<(), String> {
    let _lease = daemon::acquire_write(config, "manager-repair")?;
    publish(
        sink,
        15,
        1,
        3,
        "Checking rootfs layout",
        &config.rootfs,
        0,
        0,
        0,
    );
    ensure_rootfs_layout(config);
    publish(
        sink,
        55,
        2,
        3,
        "Repairing runtime links",
        "ld.so, libc and common library aliases",
        0,
        0,
        0,
    );
    crate::rootfs::repair_rootfs_runtime(&config.rootfs);
    publish(
        sink,
        90,
        3,
        3,
        "Synchronizing rootfs",
        "Flushing repaired files to disk",
        0,
        0,
        0,
    );
    daemon::sync_rootfs();
    fs::sync();
    publish(
        sink,
        100,
        3,
        3,
        "Repair complete",
        "The existing LXE rootfs has been repaired.",
        0,
        0,
        0,
    );
    Ok(())
}

fn count_installed_bootstrap_packages(config: &LxeConfig) -> u32 {
    let mut installed = 0u32;
    for package in &config.bootstrap_seed {
        if package_installed(config, package, &config.rootfs) {
            installed += 1;
        }
    }
    installed
}

pub fn delete_rootfs(
    config: &mut LxeConfig,
    sink: &mut dyn ManagerProgressSink,
) -> Result<(), String> {
    if !config.rootfs.starts_with("/System/var/lxe/") {
        return Err(String::from(
            "refusing to delete rootfs outside /System/var/lxe",
        ));
    }
    let _lease = daemon::acquire_write(config, "manager-delete")?;
    publish(sink, 10, 1, 3, "Preparing removal", &config.rootfs, 0, 0, 0);
    delete_recursive(&config.rootfs);
    publish(
        sink,
        80,
        2,
        3,
        "Recreating empty rootfs",
        &config.rootfs,
        0,
        0,
        0,
    );
    ensure_dir_recursive(&config.rootfs);
    publish(
        sink,
        95,
        3,
        3,
        "Synchronizing",
        "Flushing removal to disk",
        0,
        0,
        0,
    );
    daemon::sync_rootfs();
    fs::sync();
    publish(
        sink,
        100,
        3,
        3,
        "Rootfs deleted",
        "The LXE rootfs is empty.",
        0,
        0,
        0,
    );
    Ok(())
}

struct BootstrapObserver<'a> {
    sink: &'a mut dyn ManagerProgressSink,
    started_ms: u32,
    steps: u32,
}

impl<'a> BootstrapObserver<'a> {
    fn new(sink: &'a mut dyn ManagerProgressSink, steps: u32) -> Self {
        Self {
            sink,
            started_ms: sys::uptime_ms(),
            steps,
        }
    }
}

impl InstallProgressObserver for BootstrapObserver<'_> {
    fn on_install_progress(
        &mut self,
        overall_done: u32,
        overall_total: u32,
        overall_unit: &'static str,
        phase: &str,
        detail: &str,
        package_done: u32,
        package_total: u32,
        _files_written: u32,
        download_done: u32,
        download_total: u32,
    ) {
        let total = overall_total.max(1);
        let sub_percent = if download_total > 0 {
            download_done.saturating_mul(100) / download_total
        } else if package_total > 0 {
            package_done.saturating_mul(100) / package_total
        } else {
            0
        };
        let install_units = overall_done
            .saturating_mul(100)
            .saturating_add(sub_percent.min(100));
        let percent = 2 + install_units.saturating_mul(INSTALL_WEIGHT_PERCENT) / total / 100;
        let eta = estimate_eta(
            self.started_ms,
            percent.min(99),
            download_done,
            download_total,
        );
        publish(
            self.sink,
            percent.min(99),
            overall_done.saturating_add(1).min(total),
            self.steps.max(total),
            phase,
            &format!("{} {}", detail, overall_unit),
            download_done,
            download_total,
            eta,
        );
    }
}

fn publish(
    sink: &mut dyn ManagerProgressSink,
    percent: u32,
    step: u32,
    steps: u32,
    phase: &str,
    detail: &str,
    bytes_done: u32,
    bytes_total: u32,
    eta_secs: u32,
) {
    sink.update(&ManagerProgress {
        percent: percent.min(100),
        step,
        steps: steps.max(1),
        phase: String::from(phase),
        detail: String::from(detail),
        bytes_done,
        bytes_total,
        eta_secs,
    });
}

fn estimate_eta(started_ms: u32, percent: u32, bytes_done: u32, bytes_total: u32) -> u32 {
    if bytes_total > 0 && bytes_done > 0 && bytes_done <= bytes_total {
        let elapsed = sys::uptime_ms().wrapping_sub(started_ms) / 1000;
        return elapsed.saturating_mul(bytes_total.saturating_sub(bytes_done)) / bytes_done.max(1);
    }
    if percent == 0 {
        return 0;
    }
    let elapsed = sys::uptime_ms().wrapping_sub(started_ms) / 1000;
    elapsed.saturating_mul(100u32.saturating_sub(percent)) / percent
}

fn scan_tree(path: &str, files: &mut u32, dirs: &mut u32, bytes: &mut u64, depth: u8) {
    if depth > 24 || !path_exists(path) {
        return;
    }
    let mut dir_buf = [0u8; 64 * 96];
    let count = fs::readdir(path, &mut dir_buf);
    if count == u32::MAX {
        *files = files.saturating_add(1);
        *bytes = bytes.saturating_add(file_size(path) as u64);
        return;
    }
    *dirs = dirs.saturating_add(1);
    let max = (count as usize).min(dir_buf.len() / 64);
    for idx in 0..max {
        let off = idx * 64;
        let entry_type = dir_buf[off];
        let name_len = (dir_buf[off + 1] as usize).min(56);
        if name_len == 0 {
            continue;
        }
        let Ok(name) = core::str::from_utf8(&dir_buf[off + 8..off + 8 + name_len]) else {
            continue;
        };
        if name == "." || name == ".." {
            continue;
        }
        let child = format!("{}/{}", path, name);
        if entry_type as u32 == FS_TYPE_DIRECTORY {
            scan_tree(&child, files, dirs, bytes, depth + 1);
        } else {
            *files = files.saturating_add(1);
            *bytes = bytes.saturating_add(file_size(&child) as u64);
        }
    }
}

fn delete_recursive(path: &str) {
    let mut dir_buf = [0u8; 64 * 96];
    let count = fs::readdir(path, &mut dir_buf);
    if count == u32::MAX {
        let _ = fs::unlink(path);
        return;
    }
    let max = (count as usize).min(dir_buf.len() / 64);
    for idx in 0..max {
        let off = idx * 64;
        let entry_type = dir_buf[off];
        let name_len = (dir_buf[off + 1] as usize).min(56);
        if name_len == 0 {
            continue;
        }
        let Ok(name) = core::str::from_utf8(&dir_buf[off + 8..off + 8 + name_len]) else {
            continue;
        };
        if name == "." || name == ".." {
            continue;
        }
        let child = format!("{}/{}", path, name);
        if entry_type as u32 == FS_TYPE_DIRECTORY {
            delete_recursive(&child);
        } else {
            let _ = fs::unlink(&child);
        }
    }
    let _ = fs::unlink(path);
}

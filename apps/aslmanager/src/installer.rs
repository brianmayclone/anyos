use alloc::format;
use alloc::string::String;
use anyos_std::{fs, process, sys};
use aslmanager_core::{
    artifact_size_ok, gzip_header_ok, is_safe_artifact_path as core_is_safe_artifact_path,
    linux_kernel_header_ok, official_http_fallback, parse_u64, should_try_http_fallback,
};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use libanyui_client as anyui;

use crate::asld;
use crate::config::{is_allowed_debian_url, ManagerConfig};
use crate::constants::*;

static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
static WORKER_DONE: AtomicBool = AtomicBool::new(false);
static WORKER_ERROR: AtomicBool = AtomicBool::new(false);
const LOG_SLOT_COUNT: usize = 32;
static LOG_SEQ: AtomicU32 = AtomicU32::new(0);
static LOG_LINE_LEN: [AtomicU32; LOG_SLOT_COUNT] = [const { AtomicU32::new(0) }; LOG_SLOT_COUNT];
static mut LOG_LINE_BUF: [[u8; 512]; LOG_SLOT_COUNT] = [[0u8; 512]; LOG_SLOT_COUNT];
static mut PHASE_LABEL_ID: u32 = 0;
static mut STATUS_LABEL_ID: u32 = 0;
static mut TRANSFER_LABEL_ID: u32 = 0;
static mut PROGRESS_BAR_ID: u32 = 0;
static DOWNLOAD_LAST_SAMPLE_MS: AtomicU32 = AtomicU32::new(0);
static DOWNLOAD_LAST_SAMPLE_BYTES: AtomicU32 = AtomicU32::new(0);
static DOWNLOAD_LAST_UI_MS: AtomicU32 = AtomicU32::new(0);
static DOWNLOAD_LAST_PROGRESS_MS: AtomicU32 = AtomicU32::new(0);
static DOWNLOAD_LAST_PROGRESS_PCT: AtomicU32 = AtomicU32::new(0);
static DOWNLOAD_RATE_BPS: AtomicU32 = AtomicU32::new(0);

pub fn register_controls(
    phase_label_id: u32,
    status_label_id: u32,
    transfer_label_id: u32,
    progress_bar_id: u32,
) {
    unsafe {
        PHASE_LABEL_ID = phase_label_id;
        STATUS_LABEL_ID = status_label_id;
        TRANSFER_LABEL_ID = transfer_label_id;
        PROGRESS_BAR_ID = progress_bar_id;
    }
}

pub fn refresh_status() {
    let installed = artifacts_ready();
    let status = asld::request_ui(&format!("STATUS {}", DISTRO_NAME));
    let state = match status {
        Ok(resp) if resp.ok => {
            String::from(asld::field_value(&resp.lines, "state").unwrap_or("registered"))
        }
        _ if installed => String::from("artifacts ready"),
        _ => String::from("not installed"),
    };
    let text = format!("Debian: {}", state);
    crate::app().status_label.set_text(&text);
    crate::app().terminal_btn.set_enabled(installed);
}

pub fn refresh_runtime_metrics(force: bool) {
    static METRIC_TICKS: AtomicU32 = AtomicU32::new(0);
    let ticks = METRIC_TICKS.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    if !force && ticks % 2 != 0 {
        return;
    }

    let vm_status = asld::request_ui(&format!("VM_STATUS {}", DISTRO_NAME));
    let config = asld::request_ui(&format!("CONFIG_SHOW {}", DISTRO_NAME));

    let mut vcpus = String::from("-");
    if let Ok(resp) = config {
        if resp.ok {
            vcpus =
                String::from(asld::field_value(&resp.lines, "resources.vcpu_count").unwrap_or("-"));
        }
    }

    match vm_status {
        Ok(resp) if resp.ok => {
            let run_state = asld::field_value(&resp.lines, "run_state").unwrap_or("running");
            let backend = asld::field_value(&resp.lines, "backend").unwrap_or("-");
            let memory = asld::field_value(&resp.lines, "guest_memory_mb").unwrap_or("-");
            let total_exits =
                parse_u64(asld::field_value(&resp.lines, "total_exits").unwrap_or("0"));
            let recent_exits = asld::field_value(&resp.lines, "recent_exit_count").unwrap_or("0");
            let boot = asld::field_value(&resp.lines, "boot_summary").unwrap_or("");

            let previous = crate::app().last_total_exits;
            crate::app().last_total_exits = total_exits;
            let delta = total_exits.saturating_sub(previous);
            let activity = (delta as u32).saturating_mul(12).min(100);

            crate::app().run_state_label.set_text(run_state);
            crate::app().backend_label.set_text(backend);
            crate::app()
                .memory_label
                .set_text(&format!("{} MiB", memory));
            crate::app().vcpu_label.set_text(&vcpus);
            crate::app()
                .exit_label
                .set_text(&format!("{} total", total_exits));
            crate::app().activity_bar.set_state(activity);
            if boot.is_empty() {
                crate::app().activity_label.set_text(&format!(
                    "{} exits in the last sample, {} recent",
                    delta, recent_exits
                ));
            } else {
                crate::app().activity_label.set_text(&format!(
                    "{} exits in the last sample, boot: {}",
                    delta, boot
                ));
            }
        }
        Ok(resp) => show_runtime_unavailable(&resp.message, &vcpus),
        Err(err) => show_runtime_unavailable(err, &vcpus),
    }
}

pub fn start_install() {
    if WORKER_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    WORKER_DONE.store(false, Ordering::Release);
    WORKER_ERROR.store(false, Ordering::Release);
    crate::app().install_btn.set_enabled(false);
    crate::app().terminal_btn.set_enabled(false);
    crate::app().skip_verify_btn.set_enabled(false);
    crate::app().skip_verify_btn_enabled = false;
    crate::app().progress_bar.set_state(0);
    crate::app()
        .transfer_label
        .set_text("Download: waiting to start");
    crate::app()
        .phase_label
        .set_text("Starting Debian setup...");
    log_line_ui("Starting Debian one-click setup.");

    match process::Thread::spawn(install_worker, "asl-debian-install") {
        Ok(thread) => {
            crate::app().worker = Some(thread);
        }
        Err(_) => {
            WORKER_ACTIVE.store(false, Ordering::Release);
            WORKER_DONE.store(true, Ordering::Release);
            WORKER_ERROR.store(true, Ordering::Release);
            crate::app().install_btn.set_enabled(true);
            crate::app()
                .status_label
                .set_text("Could not start installer thread.");
            log_line_ui("ERROR: failed to spawn installer thread.");
        }
    }
}

pub fn on_timer() {
    let seq = LOG_SEQ.load(Ordering::Acquire);
    let mut had_logs = false;
    if seq > crate::app().last_log_seq {
        if seq - crate::app().last_log_seq > LOG_SLOT_COUNT as u32 {
            crate::app().last_log_seq = seq - LOG_SLOT_COUNT as u32;
            crate::app()
                .log_text
                .push_str("WARNING: older installer log lines were skipped.\n");
            had_logs = true;
        }
    }
    while crate::app().last_log_seq < seq {
        had_logs = true;
        crate::app().last_log_seq += 1;
        let slot = (crate::app().last_log_seq as usize) % LOG_SLOT_COUNT;
        let len = LOG_LINE_LEN[slot].load(Ordering::Acquire) as usize;
        let line = unsafe { core::str::from_utf8(&LOG_LINE_BUF[slot][..len]).unwrap_or("") };
        crate::app().log_text.push_str(line);
        crate::app().log_text.push('\n');
    }
    if had_logs {
        crate::app().log_area.set_text(&crate::app().log_text);
        crate::app()
            .log_area
            .set_cursor(crate::app().log_text.len() as u32);
    }

    if WORKER_DONE.swap(false, Ordering::AcqRel) {
        if let Some(thread) = crate::app().worker.as_mut() {
            if thread.try_join().is_none() {
                WORKER_DONE.store(true, Ordering::Release);
                return;
            }
        }
        let _ = crate::app().worker.take();
        WORKER_ACTIVE.store(false, Ordering::Release);
        crate::app().install_btn.set_enabled(true);
        crate::app().terminal_btn.set_enabled(artifacts_ready());
        crate::app().skip_verify_btn.set_enabled(false);
        crate::app().skip_verify_btn_enabled = false;
        if WORKER_ERROR.load(Ordering::Acquire) {
            crate::app().status_label.set_text("Debian setup failed.");
        } else {
            crate::app()
                .status_label
                .set_text("Debian is installed and starting.");
            crate::app().progress_bar.set_state(100);
        }
    }
    refresh_asld_logs(false);
}

pub fn skip_verify() {
    log_line("No pinned hash verifier is active for Debian netboot artifacts.");
}

pub fn open_terminal() {
    let tid = process::launch_app("/Applications/ASL Console.app/ASL Console", DISTRO_NAME);
    if tid == u32::MAX {
        crate::app()
            .status_label
            .set_text("Could not open ASL Console. Run aslctl console-canvas debian.");
    } else {
        crate::app()
            .status_label
            .set_text("ASL Console opened for Debian.");
        log_line_ui("ASL Console opened and attached to Debian.");
    }
}

extern "C" fn download_progress_cb(received: u32, total: u32, userdata: u64) {
    let base = (userdata >> 32) as u32;
    let span = userdata as u32;
    let pct = if total == 0 {
        let mib = received / (1024 * 1024);
        base + mib.min(span)
    } else {
        base + (((received as u64 * span as u64) / total as u64) as u32)
    };
    update_progress_status(pct.min(100));
    update_transfer_status(received, total);
}

fn install_worker() {
    let cfg = crate::app().config.clone();

    set_status("Preparing HTTP download engine...");
    set_phase("Loading HTTP library");
    set_progress(1);

    if !libhttp_client::init() {
        finish_error("could not load /Libraries/libhttp.so");
        return;
    }

    set_status("Preparing ASL storage...");
    set_phase("Checking ASL services");
    set_progress(3);

    if !run_asl_self_check() {
        finish_error("ASL service self-check failed");
        return;
    }

    set_phase("Creating ASL distro directories");
    set_progress(6);

    if !ensure_dirs(&cfg) {
        finish_error("could not create ASL directory tree");
        return;
    }

    if !ensure_artifact(
        &cfg.kernel_path,
        &cfg.kernel_tmp,
        &cfg.debian_kernel_url,
        &cfg,
        ArtifactKind::LinuxKernel,
        "Debian netboot kernel",
        10,
        34,
    ) {
        finish_error("Debian netboot kernel download failed");
        return;
    }

    if !ensure_artifact(
        &cfg.initrd_path,
        &cfg.initrd_tmp,
        &cfg.debian_initrd_url,
        &cfg,
        ArtifactKind::InitrdGzip,
        "Debian netboot initrd",
        44,
        34,
    ) {
        finish_error("Debian netboot initrd download failed");
        return;
    }

    set_progress(80);
    set_phase("Registering Debian with asld");
    match asld::request(&format!("STATUS {}", DISTRO_NAME)) {
        Ok(resp) if resp.ok => {
            if !ensure_kernel_profile_config() {
                finish_error("could not update Debian ASL profile");
                return;
            }
        }
        _ => {
            let cmd = format!(
                "CREATE {}\t{}\t{}\t{}",
                DISTRO_NAME, IMAGE_REF, OWNER, KERNEL_PROFILE
            );
            match asld::request(&cmd) {
                Ok(resp) if resp.ok => log_line("ASL distro created."),
                Ok(resp) => {
                    finish_error(&format!("asld create failed: {}", resp.message));
                    return;
                }
                Err(err) => {
                    finish_error(err);
                    return;
                }
            }
        }
    }

    set_progress(90);
    set_phase("Starting Debian VM");
    match asld::request(&format!("START {}", DISTRO_NAME)) {
        Ok(resp) if resp.ok => {
            let health = asld::field_value(&resp.lines, "health").unwrap_or("-");
            log_line(&format!(
                "Debian start request accepted by asld: {}",
                health
            ));
        }
        Ok(resp) if resp.message.contains("already running") => {
            log_line("Debian is already running.")
        }
        Ok(resp) => {
            finish_error(&format!("asld start failed: {}", resp.message));
            return;
        }
        Err(err) => {
            finish_error(err);
            return;
        }
    }

    set_progress(94);
    set_phase("Checking VM runtime health");
    match wait_for_vm_health() {
        VmHealthCheck::Ready(summary) => log_line(&format!("VM health check passed: {}", summary)),
        VmHealthCheck::Degraded(summary) => {
            log_line(&format!("VM started with degraded health: {}", summary))
        }
        VmHealthCheck::Failed(message) => {
            finish_error(&format!("VM health check failed: {}", message));
            return;
        }
    }

    set_progress(100);
    set_phase("Debian installer is starting");
    set_status("Debian netboot is starting. Open Console to attach.");
    log_line("Done. Debian netboot has been installed and started.");
    WORKER_ERROR.store(false, Ordering::Release);
    WORKER_DONE.store(true, Ordering::Release);
}

#[derive(Clone, Copy)]
enum ArtifactKind {
    LinuxKernel,
    InitrdGzip,
}

fn ensure_artifact(
    path: &str,
    tmp: &str,
    url: &str,
    cfg: &ManagerConfig,
    kind: ArtifactKind,
    label: &str,
    base: u32,
    span: u32,
) -> bool {
    if !is_safe_artifact_path(path, cfg) || !is_safe_artifact_path(tmp, cfg) || path == tmp {
        log_line(&format!("Refusing unsafe artifact path for {}.", label));
        return false;
    }
    if !is_allowed_debian_url(url) {
        log_line(&format!("Refusing untrusted download URL for {}.", label));
        return false;
    }

    if verified_artifact(path, cfg, kind) {
        log_line(&format!("{} already present and verified.", label));
        set_transfer_status(&format!("Download: {} already present.", label));
        return true;
    }
    if file_exists(path) {
        log_line(&format!(
            "Existing {} failed validation; replacing it.",
            label
        ));
        let _ = fs::unlink(path);
    }

    log_line(&format!("Download target: {}", tmp));
    if verified_artifact(tmp, cfg, kind) {
        log_line(&format!(
            "Temporary {} download is already complete; verifying it.",
            label
        ));
    } else {
        if !preflight_tmp_path(tmp, label, file_exists(tmp)) {
            return false;
        }
        set_phase(&format!("Downloading {}", label));
        log_line(&format!("Downloading {} from Debian mirror.", label));
        let userdata = ((base as u64) << 32) | span as u64;
        if !download_with_retries(url, tmp, label, userdata) {
            return false;
        }
    }
    if !verified_artifact(tmp, cfg, kind) {
        log_line(&format!("Downloaded {} failed validation.", label));
        let _ = fs::unlink(tmp);
        return false;
    }
    log_line(&format!(
        "{} verified by size + format signature (ADR-0011 stage 1).",
        label
    ));

    let _ = fs::unlink(path);
    if fs::rename(tmp, path) != 0 {
        let _ = fs::unlink(tmp);
        return false;
    }
    log_line(&format!("{} ready.", label));
    true
}

fn preflight_tmp_path(tmp: &str, label: &str, keep_existing: bool) -> bool {
    let flags = if keep_existing {
        fs::O_WRITE | fs::O_APPEND
    } else {
        fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC
    };
    let fd = fs::open(tmp, flags);
    if fd == u32::MAX {
        log_line(&format!(
            "{} download target is not writable: {}.",
            label, tmp
        ));
        return false;
    }
    let _ = fs::close(fd);
    if !keep_existing {
        let _ = fs::unlink(tmp);
    }
    true
}

fn download_with_retries(url: &str, tmp: &str, label: &str, userdata: u64) -> bool {
    let fallback_url = official_http_fallback(url);
    if attempt_download(url, tmp, label, userdata, "HTTPS") {
        return true;
    }

    let status = libhttp_client::last_status();
    let error = libhttp_client::last_error();
    if !should_try_http_fallback(status, error) {
        return false;
    }

    let Some(fallback_url) = fallback_url else {
        return false;
    };

    log_line("Retrying through Debian's HTTP cloud mirror endpoint.");
    if attempt_download(&fallback_url, tmp, label, userdata, "HTTP fallback") {
        return true;
    }

    false
}

fn attempt_download(url: &str, tmp: &str, label: &str, userdata: u64, mode: &str) -> bool {
    reset_transfer_metrics(label, mode);
    let resume_from = partial_file_size(tmp);
    if resume_from > 0 {
        log_line(&format!(
            "Resuming {} download via {} from {}.",
            label,
            mode,
            format_bytes(resume_from as u64)
        ));
    }
    if libhttp_client::download_progress_resume(
        url,
        tmp,
        download_progress_cb,
        userdata,
        resume_from,
    ) {
        set_transfer_status(&format!("Download: {} complete.", label));
        return true;
    }

    let status = libhttp_client::last_status();
    let error = libhttp_client::last_error();
    log_line(&format!(
        "{} download failed via {}: http status {}, error {} ({}).",
        label,
        mode,
        status,
        error,
        http_error_name(error)
    ));
    false
}

fn http_error_name(error: u32) -> &'static str {
    match error {
        0 => "none",
        1 => "invalid URL",
        2 => "DNS failure",
        3 => "TCP connection failure",
        4 => "send failure",
        5 => "no response or timeout",
        6 => "too many redirects",
        7 => "TLS handshake failed",
        8 => "output buffer too small",
        9 => "file write error",
        _ => "unknown",
    }
}

fn reset_transfer_metrics(label: &str, mode: &str) {
    let now = sys::uptime_ms();
    DOWNLOAD_LAST_SAMPLE_MS.store(now, Ordering::Release);
    DOWNLOAD_LAST_SAMPLE_BYTES.store(0, Ordering::Release);
    DOWNLOAD_LAST_UI_MS.store(0, Ordering::Release);
    DOWNLOAD_LAST_PROGRESS_MS.store(0, Ordering::Release);
    DOWNLOAD_LAST_PROGRESS_PCT.store(0, Ordering::Release);
    DOWNLOAD_RATE_BPS.store(0, Ordering::Release);
    set_transfer_status(&format!(
        "Download: connecting for {} via {}...",
        label, mode
    ));
}

fn update_progress_status(pct: u32) {
    let pct = pct.min(100);
    let now = sys::uptime_ms();
    let last_pct = DOWNLOAD_LAST_PROGRESS_PCT.load(Ordering::Acquire);
    let last_ms = DOWNLOAD_LAST_PROGRESS_MS.load(Ordering::Acquire);
    let changed_enough = pct >= last_pct.saturating_add(1);
    let stale = now.wrapping_sub(last_ms) >= 500;
    let done = pct >= 100;
    if !done && !changed_enough && !stale {
        return;
    }
    DOWNLOAD_LAST_PROGRESS_PCT.store(pct, Ordering::Release);
    DOWNLOAD_LAST_PROGRESS_MS.store(now, Ordering::Release);
    set_progress(pct);
}

fn update_transfer_status(received: u32, total: u32) {
    let now = sys::uptime_ms();
    let done = total != 0 && received >= total;
    let last_ui = DOWNLOAD_LAST_UI_MS.load(Ordering::Acquire);
    if !done && now.wrapping_sub(last_ui) < 500 {
        return;
    }
    DOWNLOAD_LAST_UI_MS.store(now, Ordering::Release);

    let last_ms = DOWNLOAD_LAST_SAMPLE_MS.load(Ordering::Acquire);
    let last_bytes = DOWNLOAD_LAST_SAMPLE_BYTES.load(Ordering::Acquire);
    let elapsed_ms = now.wrapping_sub(last_ms).max(1);
    let delta_bytes = received.saturating_sub(last_bytes);
    let measured_bps = ((delta_bytes as u64 * 1000) / elapsed_ms as u64) as u32;
    if measured_bps > 0 {
        DOWNLOAD_RATE_BPS.store(measured_bps, Ordering::Release);
    }
    DOWNLOAD_LAST_SAMPLE_MS.store(now, Ordering::Release);
    DOWNLOAD_LAST_SAMPLE_BYTES.store(received, Ordering::Release);

    let rate_bps = DOWNLOAD_RATE_BPS.load(Ordering::Acquire);
    let rate = if rate_bps == 0 {
        String::from("measuring speed")
    } else {
        format!("{}/s", format_bytes(rate_bps as u64))
    };

    let text = if total == 0 {
        format!(
            "Download: {} received - {} - ETA unknown",
            format_bytes(received as u64),
            rate
        )
    } else {
        let pct = ((received as u64 * 100) / total.max(1) as u64) as u32;
        let eta = if rate_bps == 0 || received >= total {
            String::from("ETA --:--")
        } else {
            let remaining = total.saturating_sub(received) as u64;
            let secs = ((remaining + rate_bps as u64 - 1) / rate_bps as u64) as u32;
            format!("{} left", format_duration(secs))
        };
        format!(
            "Download: {} / {} ({}%) - {} - {}",
            format_bytes(received as u64),
            format_bytes(total as u64),
            pct.min(100),
            rate,
            eta
        )
    };
    set_transfer_status(&text);
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;

    if bytes >= GIB {
        format!("{}.{} GiB", bytes / GIB, ((bytes % GIB) * 10) / GIB)
    } else if bytes >= MIB {
        format!("{}.{} MiB", bytes / MIB, ((bytes % MIB) * 10) / MIB)
    } else if bytes >= KIB {
        format!("{} KiB", bytes / KIB)
    } else {
        format!("{} B", bytes)
    }
}

fn format_duration(seconds: u32) -> String {
    if seconds >= 3600 {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        format!("{}h {:02}m", hours, minutes)
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

fn run_asl_self_check() -> bool {
    match asld::request("SELF_CHECK") {
        Ok(resp) if resp.ok => {
            log_line("ASL service self-check passed.");
            true
        }
        Ok(resp) => {
            log_line(&format!("ASL service self-check failed: {}", resp.message));
            false
        }
        Err(err) => {
            log_line(&format!("ASL service self-check failed: {}", err));
            false
        }
    }
}

fn ensure_dirs(cfg: &ManagerConfig) -> bool {
    log_line(&format!("ASL root: {}", cfg.asl_root));
    log_line(&format!("Distro directory: {}", cfg.distro_root));
    log_line(&format!("Image directory: {}", cfg.images_dir));
    log_line(&format!("Boot directory: {}", cfg.boot_dir));
    for dir in [
        "/System/var",
        cfg.asl_root.as_str(),
        cfg.distros_root.as_str(),
        cfg.distro_root.as_str(),
        cfg.images_dir.as_str(),
        cfg.boot_dir.as_str(),
    ] {
        if fs::mkdir(dir) == u32::MAX && !dir_exists(dir) {
            log_line(&format!("Could not create ASL directory: {}", dir));
            return false;
        }
    }
    true
}

fn artifacts_ready() -> bool {
    let cfg = &crate::app().config;
    verified_artifact(&cfg.kernel_path, cfg, ArtifactKind::LinuxKernel)
        && verified_artifact(&cfg.initrd_path, cfg, ArtifactKind::InitrdGzip)
}

fn verified_artifact(path: &str, cfg: &ManagerConfig, kind: ArtifactKind) -> bool {
    if !is_safe_artifact_path(path, cfg) {
        return false;
    }
    let min_size = match kind {
        ArtifactKind::LinuxKernel => LINUX_KERNEL_MIN_BYTES,
        ArtifactKind::InitrdGzip => INITRD_MIN_BYTES,
    };
    let mut stat_buf = [0u32; 7];
    if fs::stat(path, &mut stat_buf) != 0 {
        return false;
    }
    if !artifact_size_ok(stat_buf[1], min_size) {
        return false;
    }
    let mut header = [0u8; 520];
    let read = read_prefix(path, &mut header);
    match kind {
        ArtifactKind::LinuxKernel => linux_kernel_header_ok(&header, read),
        ArtifactKind::InitrdGzip => gzip_header_ok(&header, read),
    }
}

fn read_prefix(path: &str, buf: &mut [u8]) -> usize {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return 0;
    }
    let n = fs::read(fd, buf);
    let _ = fs::close(fd);
    if n == u32::MAX {
        0
    } else {
        n as usize
    }
}

fn is_safe_artifact_path(path: &str, cfg: &ManagerConfig) -> bool {
    core_is_safe_artifact_path(path, &cfg.distro_root)
}

fn ensure_kernel_profile_config() -> bool {
    let config = asld::request(&format!("CONFIG_SHOW {}", DISTRO_NAME));
    match config {
        Ok(resp) if resp.ok => {
            let profile = asld::field_value(&resp.lines, "kernel_profile").unwrap_or("-");
            if profile == KERNEL_PROFILE {
                log_line("ASL distro already exists with direct Linux profile; reusing it.");
                return true;
            }
            log_line(
                "Existing Debian profile is not the netboot profile; replacing ASL registration.",
            );
            match asld::request(&format!("DELETE {}\t1", DISTRO_NAME)) {
                Ok(resp) if resp.ok => {}
                Ok(resp) => {
                    log_line(&format!("ASL delete failed: {}", resp.message));
                    return false;
                }
                Err(err) => {
                    log_line(err);
                    return false;
                }
            }
            let cmd = format!(
                "CREATE {}\t{}\t{}\t{}",
                DISTRO_NAME, IMAGE_REF, OWNER, KERNEL_PROFILE
            );
            match asld::request(&cmd) {
                Ok(resp) if resp.ok => {
                    log_line("ASL distro recreated with direct Linux netboot profile.");
                    true
                }
                Ok(resp) => {
                    log_line(&format!("ASL create failed: {}", resp.message));
                    false
                }
                Err(err) => {
                    log_line(err);
                    false
                }
            }
        }
        Ok(resp) => {
            log_line(&format!("ASL config check failed: {}", resp.message));
            false
        }
        Err(err) => {
            log_line(err);
            false
        }
    }
}

fn file_exists(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    fs::stat(path, &mut stat_buf) == 0 && stat_buf[1] > 0
}

fn partial_file_size(path: &str) -> u32 {
    let mut stat_buf = [0u32; 7];
    if fs::stat(path, &mut stat_buf) == 0 {
        stat_buf[1]
    } else {
        0
    }
}

fn dir_exists(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    fs::stat(path, &mut stat_buf) == 0
}

fn set_phase(text: &str) {
    unsafe {
        anyui::marshal_set_text(PHASE_LABEL_ID, text);
    }
    log_line(text);
}

fn set_status(text: &str) {
    unsafe {
        anyui::marshal_set_text(STATUS_LABEL_ID, text);
    }
}

fn set_transfer_status(text: &str) {
    unsafe {
        anyui::marshal_set_text(TRANSFER_LABEL_ID, text);
    }
}

fn set_progress(pct: u32) {
    unsafe {
        anyui::marshal_set_state(PROGRESS_BAR_ID, pct.min(100));
    }
}

fn finish_error(message: &str) {
    set_phase("Setup failed");
    set_status(message);
    set_transfer_status("Download: stopped.");
    log_line(&format!("ERROR: {}", message));
    WORKER_ERROR.store(true, Ordering::Release);
    WORKER_DONE.store(true, Ordering::Release);
}

fn log_line(line: &str) {
    let len = line.len().min(511);
    let seq = LOG_SEQ.load(Ordering::Relaxed).wrapping_add(1);
    let slot = (seq as usize) % LOG_SLOT_COUNT;
    unsafe {
        LOG_LINE_BUF[slot][..len].copy_from_slice(&line.as_bytes()[..len]);
    }
    LOG_LINE_LEN[slot].store(len as u32, Ordering::Release);
    LOG_SEQ.store(seq, Ordering::Release);
}

fn log_line_ui(line: &str) {
    append_log_area_line(line);
}

pub fn refresh_asld_logs(force: bool) {
    static ASLD_LOG_TICKS: AtomicU32 = AtomicU32::new(0);
    let ticks = ASLD_LOG_TICKS
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    if !force && ticks % 4 != 0 {
        return;
    }

    let command = format!("LOGS {}\t80", DISTRO_NAME);
    let request = if force {
        asld::request(&command)
    } else {
        asld::request_ui(&command)
    };
    let Ok(resp) = request else {
        return;
    };
    if !resp.ok || resp.lines.is_empty() {
        return;
    }

    let mut start = 0usize;
    if !crate::app().last_asld_log_line.is_empty() {
        let last = crate::app().last_asld_log_line.clone();
        let mut found = false;
        for (index, line) in resp.lines.iter().enumerate() {
            if *line == last {
                start = index + 1;
                found = true;
            }
        }
        if !found {
            append_log_area_line("[asld] log history advanced; showing newest available entries.");
        }
    }

    for line in resp.lines.iter().skip(start) {
        append_log_area_line(line);
    }
    if let Some(last) = resp.lines.last() {
        crate::app().last_asld_log_line = last.clone();
    }
}

fn append_log_area_line(line: &str) {
    crate::app().log_text.push_str(line);
    crate::app().log_text.push('\n');
    crate::app().log_area.set_text(&crate::app().log_text);
    crate::app()
        .log_area
        .set_cursor(crate::app().log_text.len() as u32);
}

fn show_runtime_unavailable(message: &str, vcpus: &str) {
    crate::app().run_state_label.set_text("offline");
    crate::app().backend_label.set_text("-");
    crate::app().memory_label.set_text("-");
    crate::app().vcpu_label.set_text(vcpus);
    crate::app().exit_label.set_text("-");
    crate::app().activity_bar.set_state(0);
    crate::app().activity_label.set_text(message);
}

enum VmHealthCheck {
    Ready(String),
    Degraded(String),
    Failed(String),
}

fn wait_for_vm_health() -> VmHealthCheck {
    let mut last_error = String::from("asld did not report a running Debian VM");
    for _ in 0..12 {
        match asld::request_health(&format!("STATUS {}", DISTRO_NAME)) {
            Ok(resp) if resp.ok => {
                let state = asld::field_value(&resp.lines, "state").unwrap_or("-");
                let health = asld::field_value(&resp.lines, "health").unwrap_or("-");
                let summary = asld::field_value(&resp.lines, "boot_summary")
                    .or_else(|| asld::field_value(&resp.lines, "last_error"))
                    .unwrap_or("no boot summary");
                if health == "ready" || state == "ready" {
                    return VmHealthCheck::Ready(String::from(summary));
                }
                if health == "degraded" || state == "degraded" {
                    return VmHealthCheck::Degraded(String::from(summary));
                }
                if health == "failed" || state == "failed" {
                    return VmHealthCheck::Failed(String::from(summary));
                }
            }
            Ok(resp) => {
                last_error = resp.message;
            }
            Err(err) => {
                last_error = String::from(err);
            }
        }
        process::sleep(250);
    }
    VmHealthCheck::Failed(last_error)
}

use alloc::format;
use alloc::string::String;
use anyos_std::{fs, process};
use aslmanager_core::{
    artifact_size_ok, const_time_eq, hex_encode, is_safe_artifact_path as core_is_safe_artifact_path,
    official_http_fallback, parse_sha512_hex, parse_u64, raw_disk_header_ok, should_try_http_fallback,
    Sha512,
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
static mut PROGRESS_BAR_ID: u32 = 0;

pub fn register_controls(phase_label_id: u32, status_label_id: u32, progress_bar_id: u32) {
    unsafe {
        PHASE_LABEL_ID = phase_label_id;
        STATUS_LABEL_ID = status_label_id;
        PROGRESS_BAR_ID = progress_bar_id;
    }
}

pub fn refresh_status() {
    let installed = artifacts_ready();
    let status = asld::request(&format!("STATUS {}", DISTRO_NAME));
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

    let vm_status = asld::request(&format!("VM_STATUS {}", DISTRO_NAME));
    let config = asld::request(&format!("CONFIG_SHOW {}", DISTRO_NAME));

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
    crate::app().progress_bar.set_state(0);
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
        if let Some(thread) = crate::app().worker.take() {
            let _ = thread.join();
        }
        WORKER_ACTIVE.store(false, Ordering::Release);
        crate::app().install_btn.set_enabled(true);
        crate::app().terminal_btn.set_enabled(artifacts_ready());
        if WORKER_ERROR.load(Ordering::Acquire) {
            crate::app().status_label.set_text("Debian setup failed.");
        } else {
            crate::app()
                .status_label
                .set_text("Debian is installed and starting.");
            crate::app().progress_bar.set_state(100);
        }
    }
}

pub fn open_terminal() {
    let tid = process::launch_app(
        "/Applications/Terminal.app/Terminal",
        "ASL --run aslctl shell debian --fallback-console",
    );
    if tid == u32::MAX {
        crate::app()
            .status_label
            .set_text("Could not open Terminal. Run aslctl shell debian.");
    } else {
        crate::app()
            .status_label
            .set_text("ASL terminal opened for Debian.");
        log_line_ui("Terminal opened and attached to Debian.");
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
    set_progress(pct.min(100));
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
        &cfg.base_image_path,
        &cfg.base_image_tmp,
        &cfg.debian_raw_url,
        &cfg,
        ArtifactKind::RawDisk {
            expected_sha512_hex: Some(DEBIAN_RAW_SHA512_HEX),
        },
        "Debian VM disk",
        10,
        68,
    ) {
        finish_error("Debian VM disk download failed");
        return;
    }

    set_progress(80);
    set_phase("Registering Debian with asld");
    match asld::request(&format!("STATUS {}", DISTRO_NAME)) {
        Ok(resp) if resp.ok => {
            if !ensure_seabios_config() {
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
    set_phase("Debian is starting");
    set_status("Debian is starting. Open Terminal to attach.");
    log_line("Done. Debian has been installed and started.");
    WORKER_ERROR.store(false, Ordering::Release);
    WORKER_DONE.store(true, Ordering::Release);
}

#[derive(Clone, Copy)]
enum ArtifactKind {
    /// Raw bootable disk image. `expected_sha512_hex`, when `Some`, is
    /// the pinned SHA-512 from `constants.rs` (ADR-0011 stage 2). When
    /// `None`, only stage-1 size + MBR validation runs.
    RawDisk {
        expected_sha512_hex: Option<&'static str>,
    },
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
        return true;
    }
    if file_exists(path) {
        log_line(&format!(
            "Existing {} failed validation; replacing it.",
            label
        ));
        let _ = fs::unlink(path);
    }

    let _ = fs::unlink(tmp);
    log_line(&format!("Download target: {}", tmp));
    if !preflight_tmp_path(tmp, label) {
        return false;
    }
    set_phase(&format!("Downloading {}", label));
    log_line(&format!("Downloading {} from Debian mirror.", label));
    let userdata = ((base as u64) << 32) | span as u64;
    if !download_with_retries(url, tmp, label, userdata) {
        return false;
    }
    if !verified_artifact(tmp, cfg, kind) {
        log_line(&format!("Downloaded {} failed validation.", label));
        let _ = fs::unlink(tmp);
        return false;
    }
    // ADR-0011 stage 2: SHA-512 hash pin. If the artifact specifies an
    // expected hash, run a second read-pass and compare. On mismatch,
    // refuse the download and unlink. Mismatch likely means: stale pin
    // after Debian rotated `latest`, mirror corruption, or active
    // tampering — all require operator attention.
    if let Some(verdict) = verify_artifact_hash(tmp, kind, label) {
        if !verdict {
            let _ = fs::unlink(tmp);
            return false;
        }
    } else {
        // No hash pinned for this artifact — fall back to stage-1
        // language so operators don't think it was hash-verified.
        log_line(&format!(
            "WARN: {} verified by size + MBR signature only (ADR-0011 stage 1).",
            label
        ));
    }

    let _ = fs::unlink(path);
    if fs::rename(tmp, path) != 0 {
        let _ = fs::unlink(tmp);
        return false;
    }
    log_line(&format!("{} ready.", label));
    true
}

/// Run ADR-0011 stage-2 hash verification against the pinned hex digest
/// associated with `kind`.
///
/// Returns:
/// - `None` if no hash is pinned for this artifact kind (caller should
///   fall back to stage-1 language).
/// - `Some(true)` on successful match.
/// - `Some(false)` on read failure, parse failure, or hash mismatch.
fn verify_artifact_hash(path: &str, kind: ArtifactKind, label: &str) -> Option<bool> {
    let expected_hex = match kind {
        ArtifactKind::RawDisk { expected_sha512_hex } => expected_sha512_hex?,
    };
    let Some(expected) = parse_sha512_hex(expected_hex) else {
        log_line(&format!(
            "ERROR: pinned SHA-512 for {} is not 128 hex chars; refusing artifact.",
            label
        ));
        return Some(false);
    };

    set_phase(&format!("Verifying {} (SHA-512)", label));
    log_line(&format!(
        "Computing SHA-512 of {} for stage-2 image-trust check.",
        label
    ));
    let actual = match compute_file_sha512(path) {
        Some(digest) => digest,
        None => {
            log_line(&format!(
                "ERROR: could not read back {} for hash verification.",
                label
            ));
            return Some(false);
        }
    };

    if !const_time_eq(&actual, &expected) {
        log_line(&format!(
            "ERROR: {} SHA-512 mismatch (ADR-0011 stage 2).",
            label
        ));
        log_line(&format!("  expected: {}", expected_hex));
        log_line(&format!("  actual:   {}", hex_encode(&actual)));
        log_line(
            "  Pinned hash may be stale after a Debian release rotation. \
             Verify against https://cloud.debian.org/images/cloud/trixie/latest/SHA512SUMS \
             and update DEBIAN_RAW_SHA512_HEX if appropriate.",
        );
        return Some(false);
    }

    log_line(&format!(
        "OK: {} SHA-512 matches pinned digest (ADR-0011 stage 2).",
        label
    ));
    Some(true)
}

/// Stream-read `path` and return its SHA-512 digest. 64 KiB buffer keeps
/// the stack small. Returns `None` if open fails or any read errors.
fn compute_file_sha512(path: &str) -> Option<[u8; 64]> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut hasher = Sha512::new();
    let mut buf = alloc::vec![0u8; 64 * 1024];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == u32::MAX {
            let _ = fs::close(fd);
            return None;
        }
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n as usize]);
    }
    let _ = fs::close(fd);
    Some(hasher.finalize())
}

fn preflight_tmp_path(tmp: &str, label: &str) -> bool {
    let fd = fs::open(tmp, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        log_line(&format!(
            "{} download target is not writable: {}.",
            label, tmp
        ));
        return false;
    }
    let _ = fs::close(fd);
    let _ = fs::unlink(tmp);
    true
}

fn download_with_retries(url: &str, tmp: &str, label: &str, userdata: u64) -> bool {
    if attempt_download(url, tmp, label, userdata, "HTTPS") {
        return true;
    }

    let status = libhttp_client::last_status();
    let error = libhttp_client::last_error();
    if !should_try_http_fallback(status, error) {
        let _ = fs::unlink(tmp);
        return false;
    }

    let Some(fallback_url) = official_http_fallback(url) else {
        let _ = fs::unlink(tmp);
        return false;
    };

    let _ = fs::unlink(tmp);
    log_line("Retrying through Debian's HTTP cloud mirror endpoint.");
    if attempt_download(&fallback_url, tmp, label, userdata, "HTTP fallback") {
        return true;
    }

    let _ = fs::unlink(tmp);
    false
}

fn attempt_download(url: &str, tmp: &str, label: &str, userdata: u64, mode: &str) -> bool {
    if libhttp_client::download_progress(url, tmp, download_progress_cb, userdata) {
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
    for dir in [
        "/System/var",
        cfg.asl_root.as_str(),
        cfg.distros_root.as_str(),
        cfg.distro_root.as_str(),
        cfg.images_dir.as_str(),
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
    verified_artifact(
        &cfg.base_image_path,
        cfg,
        ArtifactKind::RawDisk {
            expected_sha512_hex: Some(DEBIAN_RAW_SHA512_HEX),
        },
    )
}

fn verified_artifact(path: &str, cfg: &ManagerConfig, kind: ArtifactKind) -> bool {
    if !is_safe_artifact_path(path, cfg) {
        return false;
    }
    let min_size = match kind {
        ArtifactKind::RawDisk { .. } => RAW_DISK_MIN_BYTES,
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
        ArtifactKind::RawDisk { .. } => raw_disk_header_ok(&header, read),
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
    core_is_safe_artifact_path(path, &cfg.images_dir)
}

fn ensure_seabios_config() -> bool {
    let config = asld::request(&format!("CONFIG_SHOW {}", DISTRO_NAME));
    match config {
        Ok(resp) if resp.ok => {
            let profile = asld::field_value(&resp.lines, "kernel_profile").unwrap_or("-");
            if profile == KERNEL_PROFILE {
                log_line("ASL distro already exists with SeaBIOS profile; reusing it.");
                return true;
            }
            log_line(
                "Existing Debian profile is not a bootable VM profile; replacing ASL registration.",
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
                    log_line("ASL distro recreated with SeaBIOS VM profile.");
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

fn set_progress(pct: u32) {
    unsafe {
        anyui::marshal_set_state(PROGRESS_BAR_ID, pct.min(100));
    }
}

fn finish_error(message: &str) {
    set_phase("Setup failed");
    set_status(message);
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
    crate::app().log_text.push_str(line);
    crate::app().log_text.push('\n');
    crate::app().log_area.set_text(&crate::app().log_text);
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
    for _ in 0..12 {
        match asld::request(&format!("STATUS {}", DISTRO_NAME)) {
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
            Ok(resp) => return VmHealthCheck::Failed(resp.message),
            Err(err) => return VmHealthCheck::Failed(String::from(err)),
        }
        process::sleep(250);
    }
    VmHealthCheck::Failed(String::from("asld did not report a running Debian VM"))
}

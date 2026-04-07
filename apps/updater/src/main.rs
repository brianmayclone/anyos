// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use anyos_std::{fs, process, crypto};
use anyos_std::json::Value;

use libanyui_client as ui;
use ui::{ColumnDef, Widget, DOCK_TOP, DOCK_BOTTOM, DOCK_FILL, ALIGN_RIGHT};

anyos_std::entry!(main);

// ── Paths ────────────────────────────────────────────────────────────────────

const APKG_DIR: &str = "/System/etc/apkg";
const MIRRORS_PATH: &str = "/System/etc/apkg/mirrors.conf";
const INDEX_PATH: &str = "/System/etc/apkg/index.json";
const INSTALLED_PATH: &str = "/System/etc/apkg/installed.json";
const CACHE_DIR: &str = "/System/etc/apkg/cache";
const BACKUP_DIR: &str = "/System/etc/apkg/backup";

// System manifest paths
const MANIFEST_LOCAL: &str = "/System/etc/apkg/system-manifest.json";
const MANIFEST_REMOTE: &str = "/System/etc/apkg/system-manifest-remote.json";

// ── Worker thread communication ──────────────────────────────────────────────

static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
static WORKER_DONE: AtomicBool = AtomicBool::new(false);
static WORKER_ERROR: AtomicBool = AtomicBool::new(false);
static WORKER_PROGRESS: AtomicU32 = AtomicU32::new(0);
static WORKER_PKG_IDX: AtomicU32 = AtomicU32::new(0);
static WORKER_PKG_TOTAL: AtomicU32 = AtomicU32::new(0);
static WORKER_NEEDS_REBOOT: AtomicBool = AtomicBool::new(false);

static WORKER_NAME_LEN: AtomicU32 = AtomicU32::new(0);
static mut WORKER_NAME_BUF: [u8; 128] = [0u8; 128];

// Phase: 0=idle, 1=unused, 2=downloading index+manifest, 3=upgrading packages,
//        4=upgrading system files, 5=done
static WORKER_PHASE: AtomicU32 = AtomicU32::new(0);

// System file update results (set by worker, read by UI after completion)
static SYS_FILES_UPDATED: AtomicU32 = AtomicU32::new(0);
static SYS_FILES_TOTAL: AtomicU32 = AtomicU32::new(0);

// ── Version parsing ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct Version { major: u32, minor: u32, patch: u32 }

impl Version {
    fn parse(s: &str) -> Option<Version> {
        let mut parts = s.split('.');
        let major = parse_u32(parts.next()?)?;
        let minor = parts.next().map(parse_u32).unwrap_or(Some(0))?;
        let patch = parts.next().map(parse_u32).unwrap_or(Some(0))?;
        if parts.next().is_some() { return None; }
        Some(Version { major, minor, patch })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.major.cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch)))
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() { return None; }
    let mut n: u32 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() { return None; }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}

// ── Update entry (unified: packages + system files) ──────────────────────────

struct AvailUpdate {
    name: String,
    old_ver: String,
    new_ver: String,
    pkg_type: String,     // "bin", "system", "sysfile"
    description: String,
    size: u64,
    md5: String,
    filename: String,     // for packages: archive filename; for sysfiles: file path
    selected: bool,
}

// ── App state ────────────────────────────────────────────────────────────────

const WIN_W: u32 = 740;
const WIN_H: u32 = 540;

struct UpdaterApp {
    win: ui::Window,
    header_label: ui::Label,
    sub_label: ui::Label,
    grid: ui::DataGrid,
    btn_check: ui::Button,
    btn_update: ui::Button,
    btn_select_all: ui::Button,
    btn_reboot: ui::Button,
    progress_view: ui::View,
    progress_bar: ui::ProgressBar,
    progress_label: ui::Label,
    status_label: ui::Label,
    updates: Vec<AvailUpdate>,
    timer_id: u32,
}

static mut APP: Option<UpdaterApp> = None;
fn app() -> &'static mut UpdaterApp { unsafe { APP.as_mut().unwrap() } }

// ── Helpers ──────────────────────────────────────────────────────────────────

fn file_exists(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    fs::stat(path, &mut stat_buf) == 0
}

fn ensure_dirs() {
    fs::mkdir(APKG_DIR);
    fs::mkdir(CACHE_DIR);
    fs::mkdir(BACKUP_DIR);
}

fn read_mirrors() -> Vec<String> {
    let mut mirrors = Vec::new();
    let content = match fs::read_to_string(MIRRORS_PATH) {
        Ok(s) => s, Err(_) => return mirrors,
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        mirrors.push(String::from(line));
    }
    mirrors
}

fn index_url(mirror: &str) -> String {
    let base = mirror.trim_end_matches('/');
    format!("{}/index.json", base)
}

fn manifest_url(mirror: &str) -> String {
    let base = mirror.trim_end_matches('/');
    format!("{}/system-manifest.json", base)
}

fn package_url(mirror: &str, arch_str: &str, filename: &str) -> String {
    let base = mirror.trim_end_matches('/');
    format!("{}/packages/{}/{}", base, arch_str, filename)
}

fn file_url(mirror: &str, arch_str: &str, path: &str) -> String {
    let base = mirror.trim_end_matches('/');
    // path starts with "/" — strip it for URL
    let rel = if path.starts_with('/') { &path[1..] } else { path };
    format!("{}/files/{}/{}", base, arch_str, rel)
}

fn arch() -> &'static str {
    #[cfg(target_arch = "x86_64")]  { "x86_64" }
    #[cfg(target_arch = "aarch64")] { "aarch64" }
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{}.{} MB", bytes / 1_048_576, (bytes % 1_048_576) * 10 / 1_048_576)
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} B", bytes)
    }
}

fn ensure_parent_dirs(path: &str) {
    let bytes = path.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] == b'/' && pos > 0 {
            fs::mkdir(&path[..pos]);
        }
        pos += 1;
    }
}

fn set_worker_name(name: &str) {
    let bytes = name.as_bytes();
    let len = bytes.len().min(127);
    unsafe { WORKER_NAME_BUF[..len].copy_from_slice(&bytes[..len]); }
    WORKER_NAME_LEN.store(len as u32, Ordering::SeqCst);
}

fn get_worker_name() -> String {
    let len = WORKER_NAME_LEN.load(Ordering::SeqCst) as usize;
    if len == 0 { return String::new(); }
    let bytes = unsafe { &WORKER_NAME_BUF[..len] };
    String::from(core::str::from_utf8(bytes).unwrap_or(""))
}

/// Compute MD5 hex of a file on disk. Returns empty string on error.
fn md5_of_file(path: &str) -> String {
    match fs::read_to_vec(path) {
        Ok(data) => {
            let hash = crypto::md5_hex(&data);
            String::from(core::str::from_utf8(&hash).unwrap_or(""))
        }
        Err(_) => String::new(),
    }
}

// ── apkg: installed database ─────────────────────────────────────────────────

struct InstalledPkg {
    name: String,
    version: String,
    files: Vec<String>,
    pkg_type: String,
    auto_dep: bool,
}

fn load_installed() -> Vec<InstalledPkg> {
    let content = match fs::read_to_string(INSTALLED_PATH) {
        Ok(s) => s, Err(_) => return Vec::new(),
    };
    let val = match Value::parse(&content) {
        Ok(v) => v, Err(_) => return Vec::new(),
    };
    let mut pkgs = Vec::new();
    if let Some(obj) = val["packages"].as_object() {
        for (name, pv) in obj.iter() {
            pkgs.push(InstalledPkg {
                name: String::from(name),
                version: String::from(pv["version"].as_str().unwrap_or("0.0.0")),
                files: match pv["files"].as_array() {
                    Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                    None => Vec::new(),
                },
                pkg_type: String::from(pv["type"].as_str().unwrap_or("bin")),
                auto_dep: pv["auto"].as_bool().unwrap_or(false),
            });
        }
    }
    pkgs
}

fn save_installed(pkgs: &[InstalledPkg]) {
    let mut root = Value::new_object();
    let mut packages = Value::new_object();
    for pkg in pkgs {
        let mut obj = Value::new_object();
        obj.set("version", Value::from(pkg.version.as_str()));
        obj.set("type", Value::from(pkg.pkg_type.as_str()));
        obj.set("auto", Value::Bool(pkg.auto_dep));
        let files: Vec<Value> = pkg.files.iter().map(|f| Value::from(f.as_str())).collect();
        obj.set("files", Value::Array(files));
        packages.set(&pkg.name, obj);
    }
    root.set("packages", packages);
    let content = root.to_json_string_pretty();
    if let Ok(mut f) = fs::File::create(INSTALLED_PATH) {
        use anyos_std::fs::Write;
        let _ = f.write_all(content.as_bytes());
    }
}

// ── apkg: remote index ───────────────────────────────────────────────────────

struct RemotePkg {
    name: String,
    version_str: String,
    version: Version,
    description: String,
    pkg_type: String,
    size: u64,
    md5: String,
    filename: String,
}

fn parse_index() -> Option<Vec<RemotePkg>> {
    let content = fs::read_to_string(INDEX_PATH).ok()?;
    let val = Value::parse(&content).ok()?;
    let arr = val["packages"].as_array()?;
    let mut pkgs = Vec::new();
    for p in arr {
        let ver_str = p["version"].as_str().unwrap_or("0.0.0");
        pkgs.push(RemotePkg {
            name: String::from(p["name"].as_str().unwrap_or("")),
            version_str: String::from(ver_str),
            version: Version::parse(ver_str).unwrap_or(Version { major: 0, minor: 0, patch: 0 }),
            description: String::from(p["description"].as_str().unwrap_or("")),
            pkg_type: String::from(p["type"].as_str().unwrap_or("bin")),
            size: p["size"].as_u64().unwrap_or(0),
            md5: String::from(p["md5"].as_str().unwrap_or("")),
            filename: String::from(p["filename"].as_str().unwrap_or("")),
        });
    }
    Some(pkgs)
}

// ── System manifest: parse + diff ────────────────────────────────────────────

struct ManifestFile {
    path: String,
    md5: String,
    size: u64,
}

struct SystemManifest {
    version: String,
    files: Vec<ManifestFile>,
}

fn parse_manifest(json_path: &str) -> Option<SystemManifest> {
    let content = fs::read_to_string(json_path).ok()?;
    let val = Value::parse(&content).ok()?;
    let version = String::from(val["version"].as_str().unwrap_or("0.0.0"));
    let files_obj = val["files"].as_object()?;
    let mut files = Vec::new();
    for (path, info) in files_obj.iter() {
        files.push(ManifestFile {
            path: String::from(path),
            md5: String::from(info["md5"].as_str().unwrap_or("")),
            size: info["size"].as_u64().unwrap_or(0),
        });
    }
    Some(SystemManifest { version, files })
}

/// Compare remote manifest against local files.
/// Returns list of files that are new or changed (MD5 mismatch).
fn diff_manifest(remote: &SystemManifest) -> Vec<&ManifestFile> {
    let mut changed = Vec::new();
    for mf in &remote.files {
        let local_md5 = md5_of_file(&mf.path);
        if local_md5.is_empty() || local_md5 != mf.md5 {
            changed.push(mf);
        }
    }
    changed
}

// ── Check for all updates (packages + system files) ──────────────────────────

fn check_all_updates() -> Vec<AvailUpdate> {
    let mut updates = Vec::new();

    // 1. Package updates (apkg)
    let installed = load_installed();
    if let Some(remote) = parse_index() {
        for inst in &installed {
            let inst_ver = Version::parse(&inst.version)
                .unwrap_or(Version { major: 0, minor: 0, patch: 0 });
            if let Some(rpkg) = remote.iter().find(|r| r.name == inst.name) {
                if rpkg.version > inst_ver {
                    updates.push(AvailUpdate {
                        name: String::from(&inst.name),
                        old_ver: String::from(&inst.version),
                        new_ver: String::from(&rpkg.version_str),
                        pkg_type: String::from(&rpkg.pkg_type),
                        description: String::from(&rpkg.description),
                        size: rpkg.size,
                        md5: String::from(&rpkg.md5),
                        filename: String::from(&rpkg.filename),
                        selected: true,
                    });
                }
            }
        }
    }

    // 2. System file updates (manifest diff)
    if let Some(remote_manifest) = parse_manifest(MANIFEST_REMOTE) {
        let local_ver = current_system_version();
        let remote_ver_str = &remote_manifest.version;

        let changed = diff_manifest(&remote_manifest);
        if !changed.is_empty() {
            // Group by directory for display
            for mf in &changed {
                // Determine category from path
                let category = if mf.path == "/System/krnl64" {
                    "Kernel"
                } else if mf.path.starts_with("/System/bin/") {
                    "System-Programm"
                } else if mf.path.starts_with("/System/sbin/") {
                    "System-Dienst"
                } else if mf.path.starts_with("/System/lib/") || mf.path.ends_with(".so") {
                    "Systembibliothek"
                } else if mf.path.starts_with("/System/Drivers/") {
                    "Treiber"
                } else if mf.path.starts_with("/Applications/") {
                    "Anwendung"
                } else if mf.path.starts_with("/System/fonts/") {
                    "Schriftart"
                } else if mf.path.starts_with("/boot/") {
                    "Bootloader"
                } else {
                    "Systemdatei"
                };

                // Short display name: filename from path
                let display_name = mf.path.rsplit('/').next().unwrap_or(&mf.path);

                updates.push(AvailUpdate {
                    name: String::from(display_name),
                    old_ver: String::from(&local_ver),
                    new_ver: String::from(remote_ver_str),
                    pkg_type: String::from("sysfile"),
                    description: format!("{} — {}", category, mf.path),
                    size: mf.size,
                    md5: String::from(&mf.md5),
                    filename: String::from(&mf.path),  // full path for download
                    selected: true,
                });
            }
        }
    }

    updates
}

fn current_system_version() -> String {
    // Try to read from local manifest
    if let Some(m) = parse_manifest(MANIFEST_LOCAL) {
        return m.version;
    }
    // Fallback: compile-time version
    String::from(env!("ANYOS_VERSION"))
}

// ── Worker: download index + manifest ────────────────────────────────────────

fn worker_check_and_download() {
    WORKER_PHASE.store(2, Ordering::SeqCst);
    WORKER_PROGRESS.store(0, Ordering::SeqCst);

    let mirrors = read_mirrors();
    if mirrors.is_empty() {
        WORKER_ERROR.store(true, Ordering::SeqCst);
        set_worker_name("Keine Mirror konfiguriert");
        WORKER_DONE.store(true, Ordering::SeqCst);
        return;
    }

    // Download package index
    set_worker_name("Paketindex herunterladen...");
    WORKER_PROGRESS.store(20, Ordering::SeqCst);
    let mut idx_ok = false;
    for mirror in &mirrors {
        if libhttp_client::download(&index_url(mirror), INDEX_PATH) {
            idx_ok = true;
            break;
        }
    }

    // Download system manifest (not fatal if it fails — just no system updates)
    set_worker_name("Systemmanifest herunterladen...");
    WORKER_PROGRESS.store(60, Ordering::SeqCst);
    for mirror in &mirrors {
        if libhttp_client::download(&manifest_url(mirror), MANIFEST_REMOTE) {
            break;
        }
    }

    if !idx_ok && !file_exists(MANIFEST_REMOTE) {
        WORKER_ERROR.store(true, Ordering::SeqCst);
        set_worker_name("Download fehlgeschlagen");
    }

    WORKER_PROGRESS.store(100, Ordering::SeqCst);
    WORKER_DONE.store(true, Ordering::SeqCst);
}

// ── Worker: upgrade packages (apkg) ──────────────────────────────────────────

static mut UPGRADE_LIST: Option<Vec<UpgradeItem>> = None;
static mut SYSFILE_LIST: Option<Vec<SysFileItem>> = None;

struct UpgradeItem {
    name: String,
    new_ver: String,
    pkg_type: String,
    md5: String,
    filename: String,
}

struct SysFileItem {
    path: String,
    md5: String,
    size: u64,
}

fn worker_upgrade_all() {
    let pkg_items = unsafe { UPGRADE_LIST.as_ref() };
    let sys_items = unsafe { SYSFILE_LIST.as_ref() };

    let pkg_count = pkg_items.map(|v| v.len()).unwrap_or(0);
    let sys_count = sys_items.map(|v| v.len()).unwrap_or(0);
    let total = (pkg_count + sys_count) as u32;

    if total == 0 {
        WORKER_DONE.store(true, Ordering::SeqCst);
        return;
    }

    WORKER_PKG_TOTAL.store(total, Ordering::SeqCst);

    let mirrors = read_mirrors();
    if mirrors.is_empty() {
        WORKER_ERROR.store(true, Ordering::SeqCst);
        set_worker_name("Keine Mirror konfiguriert");
        WORKER_DONE.store(true, Ordering::SeqCst);
        return;
    }

    let a = arch();
    let mut step: u32 = 0;

    // ── Phase 1: Package upgrades ────────────────────────────────────────
    if let Some(items) = pkg_items {
        WORKER_PHASE.store(3, Ordering::SeqCst);
        let mut installed_db = load_installed();

        for item in items {
            WORKER_PKG_IDX.store(step, Ordering::SeqCst);
            set_worker_name(&item.name);
            let pct = (step * 100) / total;
            WORKER_PROGRESS.store(pct, Ordering::SeqCst);

            // Backup system packages
            if item.pkg_type == "system" {
                if let Some(inst) = installed_db.iter().find(|p| p.name == item.name) {
                    for fp in &inst.files {
                        let fname = fp.rsplit('/').next().unwrap_or("unknown");
                        let bak = format!("{}/{}.{}", BACKUP_DIR, fname, inst.version);
                        if let Ok(data) = fs::read_to_vec(fp) {
                            if let Ok(mut f) = fs::File::create(&bak) {
                                use anyos_std::fs::Write;
                                let _ = f.write_all(&data);
                            }
                        }
                    }
                }
            }

            // Download
            let cache_path = format!("{}/{}", CACHE_DIR, item.filename);
            if !file_exists(&cache_path) {
                let mut ok = false;
                for mirror in &mirrors {
                    if libhttp_client::download(&package_url(mirror, a, &item.filename), &cache_path) {
                        ok = true; break;
                    }
                }
                if !ok { step += 1; continue; }
            }

            // Verify MD5
            if !item.md5.is_empty() {
                if let Ok(data) = fs::read_to_vec(&cache_path) {
                    let hash = crypto::md5_hex(&data);
                    let hs = core::str::from_utf8(&hash).unwrap_or("");
                    if hs != item.md5 {
                        fs::unlink(&cache_path);
                        step += 1; continue;
                    }
                }
            }

            // Extract
            if let Some(reader) = libzip_client::TarReader::open(&cache_path) {
                let count = reader.entry_count();
                let mut prefix: Option<String> = None;
                for i in 0..count {
                    let name = reader.entry_name(i);
                    if name.ends_with("/pkg.json") {
                        if let Some(slash) = name.rfind('/') {
                            prefix = Some(format!("{}/files/", &name[..slash]));
                        }
                        break;
                    }
                }
                if let Some(pfx) = prefix {
                    let mut new_files: Vec<String> = Vec::new();
                    for i in 0..count {
                        let name = reader.entry_name(i);
                        if !name.starts_with(&pfx) { continue; }
                        let rel = &name[pfx.len()..];
                        if rel.is_empty() { continue; }
                        let target = format!("/{}", rel);
                        if reader.entry_is_dir(i) {
                            fs::mkdir(&target);
                        } else {
                            ensure_parent_dirs(&target);
                            if reader.extract_to_file(i, &target) {
                                new_files.push(target);
                            }
                        }
                    }
                    if let Some(inst) = installed_db.iter_mut().find(|p| p.name == item.name) {
                        inst.version = String::from(&item.new_ver);
                        inst.files = new_files;
                    } else {
                        installed_db.push(InstalledPkg {
                            name: String::from(&item.name),
                            version: String::from(&item.new_ver),
                            files: new_files,
                            pkg_type: String::from(&item.pkg_type),
                            auto_dep: false,
                        });
                    }
                    if item.pkg_type == "system" {
                        WORKER_NEEDS_REBOOT.store(true, Ordering::SeqCst);
                    }
                }
            }

            step += 1;
        }

        save_installed(&installed_db);
    }

    // ── Phase 2: System file updates (manifest delta) ────────────────────
    if let Some(items) = sys_items {
        WORKER_PHASE.store(4, Ordering::SeqCst);
        let mut updated_count = 0u32;

        for item in items {
            WORKER_PKG_IDX.store(step, Ordering::SeqCst);
            let short_name = item.path.rsplit('/').next().unwrap_or(&item.path);
            set_worker_name(short_name);
            let pct = (step * 100) / total;
            WORKER_PROGRESS.store(pct, Ordering::SeqCst);

            // Kernel gets a backup before overwrite
            let is_kernel = item.path == "/System/krnl64";
            if is_kernel && file_exists("/System/krnl64") {
                if let Ok(data) = fs::read_to_vec("/System/krnl64") {
                    if let Ok(mut f) = fs::File::create("/System/krnl64.bak") {
                        use anyos_std::fs::Write;
                        let _ = f.write_all(&data);
                    }
                }
            }

            // Download the file to a temp path, then move
            let tmp_path = format!("{}/sysfile_tmp", CACHE_DIR);

            let mut ok = false;
            for mirror in &mirrors {
                let url = file_url(mirror, a, &item.path);
                if libhttp_client::download(&url, &tmp_path) {
                    ok = true; break;
                }
            }

            if ok {
                // Verify MD5
                let mut valid = true;
                if !item.md5.is_empty() {
                    if let Ok(data) = fs::read_to_vec(&tmp_path) {
                        let hash = crypto::md5_hex(&data);
                        let hs = core::str::from_utf8(&hash).unwrap_or("");
                        if hs != item.md5 {
                            valid = false;
                        }
                    }
                }

                if valid {
                    ensure_parent_dirs(&item.path);
                    // Copy temp to final path
                    if let Ok(data) = fs::read_to_vec(&tmp_path) {
                        if let Ok(mut f) = fs::File::create(&item.path) {
                            use anyos_std::fs::Write;
                            let _ = f.write_all(&data);
                            updated_count += 1;
                        }
                    }
                }

                fs::unlink(&tmp_path);
            }

            // Kernel, libs, drivers, compositor all need reboot
            if is_kernel
                || item.path.starts_with("/System/lib/")
                || item.path.starts_with("/System/Drivers/")
                || item.path == "/System/compositor"
                || item.path.starts_with("/boot/")
            {
                WORKER_NEEDS_REBOOT.store(true, Ordering::SeqCst);
            }

            step += 1;
        }

        SYS_FILES_UPDATED.store(updated_count, Ordering::SeqCst);
        SYS_FILES_TOTAL.store(items.len() as u32, Ordering::SeqCst);

        // Update local manifest to reflect new state
        if file_exists(MANIFEST_REMOTE) {
            if let Ok(data) = fs::read_to_vec(MANIFEST_REMOTE) {
                if let Ok(mut f) = fs::File::create(MANIFEST_LOCAL) {
                    use anyos_std::fs::Write;
                    let _ = f.write_all(&data);
                }
            }
        }
    }

    WORKER_PROGRESS.store(100, Ordering::SeqCst);
    WORKER_DONE.store(true, Ordering::SeqCst);
}

// ── UI ───────────────────────────────────────────────────────────────────────

fn build_ui() {
    let win = ui::Window::new("Aktualisierungsverwaltung", -1, -1, WIN_W, WIN_H);

    // Toolbar
    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(DOCK_TOP);
    toolbar.set_size(WIN_W, 40);
    toolbar.set_color(0xFF252526);
    toolbar.set_padding(6, 6, 6, 6);

    let btn_check = toolbar.add_button("Nach Updates suchen");
    btn_check.set_size(170, 28);

    let btn_select_all = toolbar.add_button("Alle auswaehlen");
    btn_select_all.set_size(130, 28);

    let btn_update = toolbar.add_button("Aktualisieren");
    btn_update.set_size(120, 28);
    btn_update.set_enabled(false);

    let btn_reboot = toolbar.add_button("Neu starten");
    btn_reboot.set_size(110, 28);
    btn_reboot.set_visible(false);

    win.add(&toolbar);

    // Header
    let header_view = ui::View::new();
    header_view.set_dock(DOCK_TOP);
    header_view.set_size(WIN_W, 60);
    header_view.set_color(0xFF1E1E1E);
    header_view.set_padding(16, 12, 16, 8);

    let header_label = ui::Label::new("Software-Aktualisierung");
    header_label.set_dock(DOCK_TOP);
    header_label.set_size(WIN_W, 24);
    header_label.set_color(0xFF1E1E1E);
    header_label.set_text_color(0xFFFFFFFF);
    header_label.set_font_size(16);
    header_view.add(&header_label);

    let ver = current_system_version();
    let sub_text = format!("anyOS {} — Druecken Sie 'Nach Updates suchen'.", ver);
    let sub_label = ui::Label::new(&sub_text);
    sub_label.set_dock(DOCK_TOP);
    sub_label.set_size(WIN_W, 20);
    sub_label.set_color(0xFF1E1E1E);
    sub_label.set_text_color(0xFF999999);
    sub_label.set_font_size(12);
    header_view.add(&sub_label);

    win.add(&header_view);

    // Status bar
    let status_view = ui::View::new();
    status_view.set_dock(DOCK_BOTTOM);
    status_view.set_size(WIN_W, 24);
    status_view.set_color(0xFF007ACC);
    status_view.set_padding(8, 4, 8, 4);

    let status_label = ui::Label::new("Bereit");
    status_label.set_dock(DOCK_FILL);
    status_label.set_color(0xFF007ACC);
    status_label.set_text_color(0xFFFFFFFF);
    status_label.set_font_size(11);
    status_view.add(&status_label);

    win.add(&status_view);

    // Progress area (hidden)
    let progress_view = ui::View::new();
    progress_view.set_dock(DOCK_BOTTOM);
    progress_view.set_size(WIN_W, 44);
    progress_view.set_color(0xFF252526);
    progress_view.set_padding(16, 6, 16, 6);
    progress_view.set_visible(false);

    let progress_label = ui::Label::new("");
    progress_label.set_dock(DOCK_TOP);
    progress_label.set_size(WIN_W, 16);
    progress_label.set_color(0xFF252526);
    progress_label.set_text_color(0xFFCCCCCC);
    progress_label.set_font_size(11);
    progress_view.add(&progress_label);

    let progress_bar = ui::ProgressBar::new(0);
    progress_bar.set_dock(DOCK_TOP);
    progress_bar.set_size(WIN_W - 32, 14);
    progress_view.add(&progress_bar);

    win.add(&progress_view);

    // Update list (DOCK_FILL, last)
    let grid = ui::DataGrid::new(WIN_W, 300);
    grid.set_dock(DOCK_FILL);
    grid.set_columns(&[
        ColumnDef::new("").width(30),
        ColumnDef::new("Name").width(140),
        ColumnDef::new("Installiert").width(80),
        ColumnDef::new("Neu").width(80),
        ColumnDef::new("Typ").width(110),
        ColumnDef::new("Groesse").width(70).align(ALIGN_RIGHT),
        ColumnDef::new("Beschreibung").width(250),
    ]);
    grid.set_row_count(0);
    win.add(&grid);

    let timer_id = ui::set_timer(200, || on_timer());

    unsafe {
        APP = Some(UpdaterApp {
            win, header_label, sub_label, grid,
            btn_check, btn_update, btn_select_all, btn_reboot,
            progress_view, progress_bar, progress_label, status_label,
            updates: Vec::new(), timer_id,
        });
    }

    btn_check.on_click(|_| on_check_clicked());
    btn_update.on_click(|_| on_update_clicked());
    btn_select_all.on_click(|_| on_select_all_clicked());
    btn_reboot.on_click(|_| on_reboot_clicked());
    grid.on_selection_changed(|_| on_grid_clicked());
    win.on_close(|_| ui::quit());
}

fn refresh_grid() {
    let a = app();
    let count = a.updates.len();
    a.grid.set_row_count(count as u32);

    if count == 0 {
        a.grid.set_data_raw(&[]);
        return;
    }

    let mut data: Vec<u8> = Vec::new();
    let mut text_colors: Vec<u32> = Vec::new();
    let mut bg_colors: Vec<u32> = Vec::new();

    for (i, upd) in a.updates.iter().enumerate() {
        if i > 0 { data.push(0x1E); }

        // Checkbox
        data.extend_from_slice(if upd.selected { b"[x]" } else { b"[ ]" });
        data.push(0x1F);
        // Name
        data.extend_from_slice(upd.name.as_bytes());
        data.push(0x1F);
        // Old version
        data.extend_from_slice(upd.old_ver.as_bytes());
        data.push(0x1F);
        // New version
        data.extend_from_slice(upd.new_ver.as_bytes());
        data.push(0x1F);
        // Type
        let type_label = match upd.pkg_type.as_str() {
            "system" => "System-Paket",
            "sysfile" => "Systemdatei",
            _ => "Paket",
        };
        data.extend_from_slice(type_label.as_bytes());
        data.push(0x1F);
        // Size
        data.extend_from_slice(format_size(upd.size).as_bytes());
        data.push(0x1F);
        // Description
        data.extend_from_slice(upd.description.as_bytes());

        // Colors (7 columns)
        let is_sys = upd.pkg_type == "system" || upd.pkg_type == "sysfile";
        text_colors.push(if upd.selected { 0xFF4EC9B0 } else { 0xFF666666 });
        text_colors.push(0xFFCCCCCC);
        text_colors.push(0xFF999999);
        text_colors.push(0xFF4EC9B0);
        text_colors.push(if is_sys { 0xFFFF8800 } else { 0xFF999999 });
        text_colors.push(0xFF999999);
        text_colors.push(0xFF808080);

        let bg = if i % 2 == 0 { 0xFF1E1E1E } else { 0xFF252526 };
        for _ in 0..7 { bg_colors.push(bg); }
    }

    a.grid.set_data_raw(&data);
    a.grid.set_cell_colors(&text_colors);
    a.grid.set_cell_bg_colors(&bg_colors);

    let any_selected = a.updates.iter().any(|u| u.selected);
    a.btn_update.set_enabled(any_selected);

    let sel = a.updates.iter().filter(|u| u.selected).count();
    let sz: u64 = a.updates.iter().filter(|u| u.selected).map(|u| u.size).sum();
    let pkg_count = a.updates.iter().filter(|u| u.pkg_type != "sysfile").count();
    let sys_count = a.updates.iter().filter(|u| u.pkg_type == "sysfile").count();

    if count > 0 {
        let mut desc = format!("{} ausgewaehlt ({})", sel, format_size(sz));
        if pkg_count > 0 && sys_count > 0 {
            desc = format!("{} Paket(e), {} Systemdatei(en). {}", pkg_count, sys_count, desc);
        } else if sys_count > 0 {
            desc = format!("{} Systemdatei(en). {}", sys_count, desc);
        } else {
            desc = format!("{} Paket(e). {}", pkg_count, desc);
        }
        a.sub_label.set_text(&desc);
    }
}

// ── Event handlers ───────────────────────────────────────────────────────────

fn on_check_clicked() {
    if WORKER_ACTIVE.load(Ordering::SeqCst) { return; }

    let a = app();
    a.btn_check.set_enabled(false);
    a.btn_update.set_enabled(false);
    a.progress_view.set_visible(true);
    a.progress_bar.set_state(0);
    a.progress_label.set_text("Lade Paketindex und Systemmanifest...");
    a.status_label.set_text("Suche nach Aktualisierungen...");

    WORKER_ACTIVE.store(true, Ordering::SeqCst);
    WORKER_DONE.store(false, Ordering::SeqCst);
    WORKER_ERROR.store(false, Ordering::SeqCst);
    WORKER_PROGRESS.store(0, Ordering::SeqCst);

    let _ = process::Thread::spawn_with_stack(
        || {
            if !libhttp_client::init() {
                WORKER_ERROR.store(true, Ordering::SeqCst);
                set_worker_name("libhttp.so konnte nicht geladen werden");
                WORKER_DONE.store(true, Ordering::SeqCst);
                return;
            }
            worker_check_and_download();
        },
        256 * 1024,
        "updater-check",
    );
}

fn on_update_clicked() {
    if WORKER_ACTIVE.load(Ordering::SeqCst) { return; }

    let a = app();

    // Split selected updates into packages and system files
    let mut pkg_items = Vec::new();
    let mut sys_items = Vec::new();

    for upd in &a.updates {
        if !upd.selected { continue; }
        if upd.pkg_type == "sysfile" {
            sys_items.push(SysFileItem {
                path: String::from(&upd.filename),
                md5: String::from(&upd.md5),
                size: upd.size,
            });
        } else {
            pkg_items.push(UpgradeItem {
                name: String::from(&upd.name),
                new_ver: String::from(&upd.new_ver),
                pkg_type: String::from(&upd.pkg_type),
                md5: String::from(&upd.md5),
                filename: String::from(&upd.filename),
            });
        }
    }

    if pkg_items.is_empty() && sys_items.is_empty() { return; }

    unsafe {
        UPGRADE_LIST = if pkg_items.is_empty() { None } else { Some(pkg_items) };
        SYSFILE_LIST = if sys_items.is_empty() { None } else { Some(sys_items) };
    }

    a.btn_check.set_enabled(false);
    a.btn_update.set_enabled(false);
    a.btn_select_all.set_enabled(false);
    a.progress_view.set_visible(true);
    a.progress_bar.set_state(0);
    a.progress_label.set_text("Aktualisierungen werden installiert...");
    a.status_label.set_text("Installiere Updates...");

    WORKER_ACTIVE.store(true, Ordering::SeqCst);
    WORKER_DONE.store(false, Ordering::SeqCst);
    WORKER_ERROR.store(false, Ordering::SeqCst);
    WORKER_NEEDS_REBOOT.store(false, Ordering::SeqCst);
    WORKER_PROGRESS.store(0, Ordering::SeqCst);
    SYS_FILES_UPDATED.store(0, Ordering::SeqCst);
    SYS_FILES_TOTAL.store(0, Ordering::SeqCst);

    let _ = process::Thread::spawn_with_stack(
        || {
            if !libhttp_client::init() {
                WORKER_ERROR.store(true, Ordering::SeqCst);
                set_worker_name("libhttp.so konnte nicht geladen werden");
                WORKER_DONE.store(true, Ordering::SeqCst);
                return;
            }
            if !libzip_client::init() {
                WORKER_ERROR.store(true, Ordering::SeqCst);
                set_worker_name("libzip.so konnte nicht geladen werden");
                WORKER_DONE.store(true, Ordering::SeqCst);
                return;
            }
            worker_upgrade_all();
        },
        256 * 1024,
        "updater-install",
    );
}

fn on_select_all_clicked() {
    let a = app();
    let all_selected = a.updates.iter().all(|u| u.selected);
    for upd in &mut a.updates {
        upd.selected = !all_selected;
    }
    a.btn_select_all.set_text(if all_selected { "Alle auswaehlen" } else { "Keine auswaehlen" });
    refresh_grid();
}

fn on_grid_clicked() {
    let a = app();
    let row = a.grid.selected_row();
    if row < a.updates.len() as u32 {
        a.updates[row as usize].selected = !a.updates[row as usize].selected;
        refresh_grid();
    }
}

fn on_reboot_clicked() {
    process::reboot();
}

fn on_timer() {
    if !WORKER_ACTIVE.load(Ordering::SeqCst) { return; }

    let a = app();
    let pct = WORKER_PROGRESS.load(Ordering::SeqCst);
    a.progress_bar.set_state(pct);

    let phase = WORKER_PHASE.load(Ordering::SeqCst);
    let name = get_worker_name();

    match phase {
        2 => {
            a.progress_label.set_text(&format!("Lade Updates... {}%", pct));
        }
        3 => {
            let idx = WORKER_PKG_IDX.load(Ordering::SeqCst);
            let total = WORKER_PKG_TOTAL.load(Ordering::SeqCst);
            a.progress_label.set_text(&format!(
                "Paket: {} ({}/{}) — {}%", name, idx + 1, total, pct
            ));
        }
        4 => {
            let idx = WORKER_PKG_IDX.load(Ordering::SeqCst);
            let total = WORKER_PKG_TOTAL.load(Ordering::SeqCst);
            a.progress_label.set_text(&format!(
                "Systemdatei: {} ({}/{}) — {}%", name, idx + 1, total, pct
            ));
        }
        _ => {}
    }

    if WORKER_DONE.load(Ordering::SeqCst) {
        WORKER_ACTIVE.store(false, Ordering::SeqCst);
        let had_error = WORKER_ERROR.load(Ordering::SeqCst);

        if phase == 2 {
            // Check phase complete
            if had_error {
                a.status_label.set_text(&format!("Fehler: {}", get_worker_name()));
                a.progress_view.set_visible(false);
                a.btn_check.set_enabled(true);
            } else {
                a.updates = check_all_updates();
                refresh_grid();

                if a.updates.is_empty() {
                    a.sub_label.set_text("Ihr System ist auf dem neuesten Stand.");
                    a.status_label.set_text("Keine Aktualisierungen verfuegbar.");
                } else {
                    let pkg_n = a.updates.iter().filter(|u| u.pkg_type != "sysfile").count();
                    let sys_n = a.updates.iter().filter(|u| u.pkg_type == "sysfile").count();
                    a.status_label.set_text(&format!(
                        "{} Paket(e), {} Systemdatei(en) verfuegbar.", pkg_n, sys_n
                    ));
                }
                a.progress_view.set_visible(false);
                a.btn_check.set_enabled(true);
            }
        } else {
            // Upgrade phase complete (3 or 4)
            let needs_reboot = WORKER_NEEDS_REBOOT.load(Ordering::SeqCst);
            let sys_updated = SYS_FILES_UPDATED.load(Ordering::SeqCst);
            let sys_total = SYS_FILES_TOTAL.load(Ordering::SeqCst);

            if had_error {
                a.status_label.set_text(&format!("Fehler: {}", get_worker_name()));
            } else if needs_reboot {
                let mut msg = String::from("Aktualisierung abgeschlossen.");
                if sys_total > 0 {
                    msg = format!("{} {}/{} Systemdateien aktualisiert.", msg, sys_updated, sys_total);
                }
                msg.push_str(" Neustart erforderlich.");
                a.status_label.set_text(&msg);
                a.sub_label.set_text("Bitte starten Sie den Computer neu, um alle Aenderungen zu uebernehmen.");
                a.btn_reboot.set_visible(true);
                a.header_label.set_text("Neustart erforderlich");
            } else {
                a.status_label.set_text("Alle Aktualisierungen wurden erfolgreich installiert.");
                a.sub_label.set_text("Ihr System ist jetzt auf dem neuesten Stand.");
                a.header_label.set_text("Aktualisierung abgeschlossen");
            }

            a.updates = check_all_updates();
            refresh_grid();
            a.progress_view.set_visible(false);
            a.btn_check.set_enabled(true);
            a.btn_select_all.set_enabled(true);

            unsafe {
                UPGRADE_LIST = None;
                SYSFILE_LIST = None;
            }
        }
    }
}

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    if !ui::init() { return; }
    ensure_dirs();
    build_ui();
    ui::run();
}

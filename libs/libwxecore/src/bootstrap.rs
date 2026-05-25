use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{fs, print, println, sys};
use libtls::crypto::sha256::Sha256;
use libzip_client::ZipReader;

use crate::config::WxeConfig;

const SYSINTERNALS_URL: &str = "https://download.sysinternals.com/files/SysinternalsSuite.zip";
const EDIT_RELEASE_API: &str = "https://api.github.com/repos/microsoft/edit/releases/latest";
const VCREDIST_X64_URL: &str = "https://aka.ms/vc14/vc_redist.x64.exe";
const MICROSOFT_LEARN_SYSINTERNALS: &str =
    "https://learn.microsoft.com/en-us/sysinternals/downloads/sysinternals-suite";
const MICROSOFT_LEARN_SYSINTERNALS_LICENSE: &str =
    "https://learn.microsoft.com/en-us/sysinternals/license-faq";
const MICROSOFT_LEARN_EDIT: &str = "https://learn.microsoft.com/en-us/windows/edit/";
const MICROSOFT_LEARN_VCREDIST: &str =
    "https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist";

pub fn bootstrap_microsoft(config: &WxeConfig, accept_licenses: bool) -> bool {
    let _ = crate::rootfs::ensure_rootfs_layout(config);
    ensure_dir(&config.cache);
    ensure_dir(&config.db);
    let ms_cache = join(&config.cache, "microsoft");
    ensure_dir(&ms_cache);

    if !accept_licenses {
        print_license_gate();
        return false;
    }

    println!("wxe bootstrap: Microsoft payload bootstrap");
    println!("  cache: {}", ms_cache);
    println!("  license acceptance: explicit command-line flag");

    if !libhttp_client::init() {
        println!("wxe bootstrap: failed to load libhttp");
        return false;
    }
    if !libzip_client::init() {
        println!("wxe bootstrap: failed to load libzip");
        return false;
    }

    let mut manifest = String::new();
    manifest.push_str("wxe-microsoft-bootstrap-v1\n");
    manifest.push_str("license.accepted=true\n");
    manifest.push_str("license.accepted_at=");
    manifest.push_str(&current_time_string());
    manifest.push('\n');
    manifest.push_str("license.scope=user-requested-download\n");
    manifest.push_str("source.sysinternals.learn=");
    manifest.push_str(MICROSOFT_LEARN_SYSINTERNALS);
    manifest.push('\n');
    manifest.push_str("source.sysinternals.license=");
    manifest.push_str(MICROSOFT_LEARN_SYSINTERNALS_LICENSE);
    manifest.push('\n');
    manifest.push_str("source.edit.learn=");
    manifest.push_str(MICROSOFT_LEARN_EDIT);
    manifest.push('\n');
    manifest.push_str("source.vcredist.learn=");
    manifest.push_str(MICROSOFT_LEARN_VCREDIST);
    manifest.push('\n');

    let mut ok = true;

    let sysinternals_zip = join(&ms_cache, "SysinternalsSuite.zip");
    if download_payload("sysinternals-suite", SYSINTERNALS_URL, &sysinternals_zip) {
        manifest_file(
            &mut manifest,
            "payload.sysinternals-suite",
            &sysinternals_zip,
        );
        if !install_sysinternals(config, &sysinternals_zip, &mut manifest) {
            ok = false;
        }
    } else {
        ok = false;
    }

    let vcredist_x64 = join(&ms_cache, "VC_redist.x64.exe");
    if download_payload("vcredist-x64", VCREDIST_X64_URL, &vcredist_x64) {
        manifest_file(&mut manifest, "payload.vcredist-x64", &vcredist_x64);
        if !stage_installer(
            config,
            "vcredist-x64",
            &vcredist_x64,
            "VC_redist.x64.exe",
            &mut manifest,
        ) {
            ok = false;
        }
    } else {
        ok = false;
    }

    let edit_metadata = join(&ms_cache, "microsoft-edit-release.json");
    match libhttp_client::get(EDIT_RELEASE_API) {
        Some(data) => {
            let _ = fs::write_bytes(&edit_metadata, &data);
            manifest_file(
                &mut manifest,
                "payload.microsoft-edit-metadata",
                &edit_metadata,
            );
            if let Some(tag) = extract_json_string(&data, "tag_name") {
                manifest.push_str("payload.microsoft-edit.version=");
                manifest.push_str(&tag);
                manifest.push('\n');
            }
            if let Some(url) = select_edit_windows_asset(&data) {
                let edit_zip = join(&ms_cache, "MicrosoftEdit-Windows.zip");
                if download_payload("microsoft-edit", &url, &edit_zip) {
                    manifest_file(&mut manifest, "payload.microsoft-edit", &edit_zip);
                    manifest.push_str("payload.microsoft-edit.url=");
                    manifest.push_str(&url);
                    manifest.push('\n');
                    if !install_microsoft_edit(config, &ms_cache, &edit_zip, &mut manifest) {
                        ok = false;
                    }
                } else {
                    ok = false;
                }
            } else {
                println!("wxe bootstrap: no Windows zip asset found in Microsoft Edit release");
                manifest.push_str("payload.microsoft-edit=metadata-only\n");
            }
        }
        None => {
            println!("wxe bootstrap: failed to fetch Microsoft Edit release metadata");
            ok = false;
        }
    }

    manifest.push_str("windows-os.cmd.exe=provided-by-wxe-shell-until-native-payload\n");
    manifest.push_str("windows-os.command-builtins=provided-by-wxe-shell\n");
    manifest.push_str("windows-os.copy=cmd-builtin\n");
    manifest.push_str("windows-os.dir=cmd-builtin\n");
    manifest.push_str("windows-os.type=cmd-builtin\n");
    manifest.push_str("windows-os.echo=cmd-builtin\n");
    manifest.push_str("windows-os.cls=cmd-builtin\n");

    let manifest_path = join(&config.db, "microsoft-bootstrap");
    if fs::write_bytes(&manifest_path, manifest.as_bytes()).is_err() {
        println!("wxe bootstrap: failed to write {}", manifest_path);
        ok = false;
    } else {
        println!("wxe bootstrap: wrote {}", manifest_path);
    }

    if ok {
        println!("wxe bootstrap: downloads installed");
    } else {
        println!("wxe bootstrap: completed with missing payloads");
    }
    ok
}

pub fn print_license_gate() {
    println!("wxe bootstrap: Microsoft payload downloads require explicit acceptance.");
    println!("Run:");
    println!("  wxe init --bootstrap-ms --accept-microsoft-licenses");
    println!("or:");
    println!("  wxe bootstrap-ms --accept-microsoft-licenses");
    println!();
    println!("Sources that will be contacted:");
    println!("  {}", SYSINTERNALS_URL);
    println!("  {}", VCREDIST_X64_URL);
    println!("  {}", EDIT_RELEASE_API);
    println!();
    println!("License/reference pages:");
    println!("  {}", MICROSOFT_LEARN_SYSINTERNALS);
    println!("  {}", MICROSOFT_LEARN_SYSINTERNALS_LICENSE);
    println!("  {}", MICROSOFT_LEARN_EDIT);
    println!("  {}", MICROSOFT_LEARN_VCREDIST);
    println!();
    println!("Windows OS files such as cmd.exe are not standalone downloads here.");
    println!("WXE uses its built-in command shell until a native cmd.exe payload exists.");
}

fn install_sysinternals(config: &WxeConfig, zip_path: &str, manifest: &mut String) -> bool {
    let dest = join(&config.drive_c, "Program Files/Sysinternals");
    println!("wxe bootstrap: installing Sysinternals to {}", dest);
    let report = install_zip_payload(zip_path, &dest);
    write_install_report(manifest, "install.sysinternals", &dest, &report);
    print_install_report("sysinternals-suite", &report);
    report.ok
}

fn install_microsoft_edit(
    config: &WxeConfig,
    cache_dir: &str,
    package_path: &str,
    manifest: &mut String,
) -> bool {
    let dest = join(&config.drive_c, "Program Files/Microsoft/Edit");
    println!("wxe bootstrap: installing Microsoft Edit to {}", dest);

    let install_source = match extract_nested_msix(package_path, cache_dir, manifest) {
        Some(path) => path,
        None => String::from(package_path),
    };

    let report = install_zip_payload(&install_source, &dest);
    write_install_report(manifest, "install.microsoft-edit", &dest, &report);
    print_install_report("microsoft-edit", &report);
    report.ok
}

fn stage_installer(
    config: &WxeConfig,
    key: &str,
    src: &str,
    file_name: &str,
    manifest: &mut String,
) -> bool {
    let dest_dir = join(&config.drive_c, "ProgramData/WXE/Installers");
    ensure_dir_recursive(&dest_dir);
    let dest = join(&dest_dir, file_name);
    println!("wxe bootstrap: staging {} installer at {}", key, dest);
    if copy_file(src, &dest) {
        manifest_path(manifest, &alloc::format!("install.{}", key), &dest);
        manifest.push_str("install.");
        manifest.push_str(key);
        manifest.push_str(".mode=staged-installer\n");
        true
    } else {
        println!("wxe bootstrap: failed to stage {}", key);
        false
    }
}

fn extract_nested_msix(
    package_path: &str,
    cache_dir: &str,
    manifest: &mut String,
) -> Option<String> {
    let reader = ZipReader::open(package_path)?;
    let mut selected = None;
    for i in 0..reader.entry_count() {
        let name = reader.entry_name(i);
        let lower = ascii_lower(&name);
        if lower.ends_with(".msix") || lower.ends_with(".appx") {
            if selected.is_none()
                || lower.contains("x64")
                || lower.contains("amd64")
                || lower.contains("x86_64")
            {
                selected = Some((i, name));
                if lower.contains("x64") || lower.contains("amd64") || lower.contains("x86_64") {
                    break;
                }
            }
        }
    }

    let (index, name) = selected?;
    let out_path = join(cache_dir, "MicrosoftEdit-Package.msix");
    println!("wxe bootstrap: extracting nested Edit package {}", name);
    if reader.extract_to_file(index, &out_path) {
        manifest_path(manifest, "payload.microsoft-edit.inner-package", &out_path);
        Some(out_path)
    } else {
        println!("wxe bootstrap: failed to extract nested Edit package");
        None
    }
}

struct InstallReport {
    ok: bool,
    files: u32,
    dirs: u32,
    exes: u32,
    skipped: u32,
}

fn install_zip_payload(zip_path: &str, dest_root: &str) -> InstallReport {
    let mut report = InstallReport {
        ok: true,
        files: 0,
        dirs: 0,
        exes: 0,
        skipped: 0,
    };

    ensure_dir_recursive(dest_root);
    let Some(reader) = ZipReader::open(zip_path) else {
        println!("wxe bootstrap: failed to open archive {}", zip_path);
        report.ok = false;
        return report;
    };

    for i in 0..reader.entry_count() {
        let name = reader.entry_name(i);
        let Some(rel) = safe_archive_path(&name) else {
            report.skipped += 1;
            continue;
        };
        if rel.is_empty() {
            report.skipped += 1;
            continue;
        }

        let dest = join(dest_root, &rel);
        if reader.entry_is_dir(i) || name.ends_with('/') || name.ends_with('\\') {
            ensure_dir_recursive(&dest);
            report.dirs += 1;
            continue;
        }

        if let Some(parent) = parent_path(&dest) {
            ensure_dir_recursive(&parent);
        }
        if reader.extract_to_file(i, &dest) {
            report.files += 1;
            if ascii_lower(&rel).ends_with(".exe") {
                report.exes += 1;
            }
        } else {
            println!("wxe bootstrap: failed to extract {}", name);
            report.ok = false;
        }
    }

    report
}

fn write_install_report(manifest: &mut String, key: &str, dest: &str, report: &InstallReport) {
    manifest_path(manifest, key, dest);
    manifest.push_str(key);
    manifest.push_str(".files=");
    manifest.push_str(&alloc::format!("{}", report.files));
    manifest.push('\n');
    manifest.push_str(key);
    manifest.push_str(".dirs=");
    manifest.push_str(&alloc::format!("{}", report.dirs));
    manifest.push('\n');
    manifest.push_str(key);
    manifest.push_str(".exes=");
    manifest.push_str(&alloc::format!("{}", report.exes));
    manifest.push('\n');
    manifest.push_str(key);
    manifest.push_str(".skipped=");
    manifest.push_str(&alloc::format!("{}", report.skipped));
    manifest.push('\n');
    manifest.push_str(key);
    manifest.push_str(".status=");
    manifest.push_str(if report.ok { "ok" } else { "partial" });
    manifest.push('\n');
}

fn print_install_report(name: &str, report: &InstallReport) {
    println!(
        "wxe bootstrap: installed {}: {} files, {} executables, {} skipped",
        name, report.files, report.exes, report.skipped
    );
}

extern "C" fn progress_callback(received: u32, total: u32, _userdata: u64) {
    print!("\r  ");
    if total == 0 {
        print_bytes(received);
        print!(" received   ");
        return;
    }

    let pct = ((received as u64 * 100) / total as u64).min(100) as u32;
    print!("{}% ", pct);
    print_bytes(received);
    print!("/");
    print_bytes(total);
    print!("   ");
}

fn download_payload(name: &str, url: &str, path: &str) -> bool {
    println!("wxe bootstrap: downloading {}", name);
    println!("  url: {}", url);
    println!("  out: {}", path);
    let ok = libhttp_client::download_progress(url, path, progress_callback, 0);
    println!();
    if !ok {
        let status = libhttp_client::last_status();
        if status != 0 {
            println!("wxe bootstrap: {} failed with HTTP {}", name, status);
        } else {
            println!("wxe bootstrap: {} failed", name);
        }
    }
    ok
}

fn select_edit_windows_asset(data: &[u8]) -> Option<String> {
    let text = core::str::from_utf8(data).ok()?;
    let mut urls = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find("\"browser_download_url\"") {
        rest = &rest[pos + "\"browser_download_url\"".len()..];
        let Some(colon) = rest.find(':') else {
            break;
        };
        rest = &rest[colon + 1..];
        let Some(first_quote) = rest.find('"') else {
            break;
        };
        rest = &rest[first_quote + 1..];
        let Some(second_quote) = rest.find('"') else {
            break;
        };
        urls.push(String::from(&rest[..second_quote]));
        rest = &rest[second_quote + 1..];
    }

    for url in urls.iter() {
        let lower = ascii_lower(url);
        if lower.contains("windows")
            && (lower.ends_with(".zip")
                || lower.ends_with(".msix")
                || lower.ends_with(".msixbundle"))
        {
            return Some(url.clone());
        }
    }
    for url in urls.iter() {
        let lower = ascii_lower(url);
        if (lower.contains("x64") || lower.contains("x86_64") || lower.contains("amd64"))
            && lower.ends_with(".zip")
        {
            return Some(url.clone());
        }
    }
    urls.into_iter()
        .find(|url| ascii_lower(url).ends_with(".zip"))
}

fn ascii_lower(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        out.push(byte.to_ascii_lowercase() as char);
    }
    out
}

fn manifest_path(manifest: &mut String, key: &str, path: &str) {
    manifest.push_str(key);
    manifest.push('=');
    manifest.push_str(path);
    manifest.push('\n');
}

fn manifest_file(manifest: &mut String, key: &str, path: &str) {
    manifest_path(manifest, key, path);
    match file_sha256(path) {
        Some((size, digest)) => {
            manifest.push_str(key);
            manifest.push_str(".size=");
            manifest.push_str(&alloc::format!("{}", size));
            manifest.push('\n');
            manifest.push_str(key);
            manifest.push_str(".sha256=");
            manifest.push_str(&hex_digest(&digest));
            manifest.push('\n');
        }
        None => {
            manifest.push_str(key);
            manifest.push_str(".hash=unavailable\n");
        }
    }
}

fn file_sha256(path: &str) -> Option<(u64, [u8; 32])> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }

    let mut total = 0u64;
    let mut ctx = Sha256::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == u32::MAX {
            let _ = fs::close(fd);
            return None;
        }
        if n == 0 {
            break;
        }
        let n = n as usize;
        total += n as u64;
        ctx.update(&buf[..n]);
    }
    let _ = fs::close(fd);
    Some((total, ctx.finalize()))
}

fn hex_digest(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(hex_nibble(byte >> 4));
        out.push(hex_nibble(byte & 0x0f));
    }
    out
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'a' + (n - 10)) as char,
    }
}

fn extract_json_string(data: &[u8], key: &str) -> Option<String> {
    let text = core::str::from_utf8(data).ok()?;
    let needle = alloc::format!("\"{}\"", key);
    let pos = text.find(&needle)?;
    let rest = &text[pos + needle.len()..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let first_quote = rest.find('"')?;
    let rest = &rest[first_quote + 1..];
    let second_quote = rest.find('"')?;
    Some(String::from(&rest[..second_quote]))
}

fn current_time_string() -> String {
    let mut buf = [0u8; 8];
    if sys::time(&mut buf) == u32::MAX {
        return String::from("unknown");
    }
    let year = u16::from_le_bytes([buf[0], buf[1]]) as u32;
    alloc::format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        year,
        buf[2] as u32,
        buf[3] as u32,
        buf[4] as u32,
        buf[5] as u32,
        buf[6] as u32
    )
}

fn safe_archive_path(name: &str) -> Option<String> {
    if name.is_empty() || name.starts_with('/') || name.starts_with('\\') || name.contains(':') {
        return None;
    }

    let mut out = String::new();
    for part in name.split(|c| c == '/' || c == '\\') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return None;
        }
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(part);
    }
    Some(out)
}

fn ensure_dir_recursive(path: &str) {
    let mut cur = String::new();
    for part in path.split('/') {
        if part.is_empty() {
            if cur.is_empty() {
                cur.push('/');
            }
            continue;
        }
        if cur.len() > 1 && !cur.ends_with('/') {
            cur.push('/');
        }
        cur.push_str(part);
        let _ = fs::mkdir(&cur);
    }
}

fn parent_path(path: &str) -> Option<String> {
    let pos = path.rfind('/')?;
    if pos == 0 {
        Some(String::from("/"))
    } else {
        Some(String::from(&path[..pos]))
    }
}

fn copy_file(src: &str, dest: &str) -> bool {
    let in_fd = fs::open(src, 0);
    if in_fd == u32::MAX {
        return false;
    }

    let out_fd = fs::open(dest, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if out_fd == u32::MAX {
        let _ = fs::close(in_fd);
        return false;
    }

    let mut ok = true;
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(in_fd, &mut buf);
        if n == u32::MAX {
            ok = false;
            break;
        }
        if n == 0 {
            break;
        }
        let mut written = 0usize;
        while written < n as usize {
            let m = fs::write(out_fd, &buf[written..n as usize]);
            if m == u32::MAX || m == 0 {
                ok = false;
                break;
            }
            written += m as usize;
        }
        if !ok {
            break;
        }
    }

    let _ = fs::close(in_fd);
    let _ = fs::close(out_fd);
    ok
}

fn ensure_dir(path: &str) {
    let _ = fs::mkdir(path);
}

fn join(base: &str, leaf: &str) -> String {
    if base.ends_with('/') {
        alloc::format!("{}{}", base, leaf)
    } else {
        alloc::format!("{}/{}", base, leaf)
    }
}

fn print_bytes(bytes: u32) {
    if bytes >= 1_048_576 {
        print!(
            "{}.{} MB",
            bytes / 1_048_576,
            (bytes % 1_048_576) * 10 / 1_048_576
        );
    } else if bytes >= 1024 {
        print!("{} KB", bytes / 1024);
    } else {
        print!("{} B", bytes);
    }
}

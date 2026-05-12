use crate::config::LicoConfig;
use crate::model::PackageInfo;
use crate::rootfs::{
    copy_file, ensure_dir, ensure_dir_recursive, ensure_parent_dirs, file_size, is_elf_file,
    linux_path_in_rootfs, normalize_abs_path, path_exists, path_exists_no_follow, path_is_symlink,
    path_under_rootfs, print_path_probe, replace_with_temp_file, resolve_rootfs_symlink_path,
    symlink_points_to, write_bytes_atomic,
};
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{
    crypto,
    fs::{self, Read},
    process, sys,
};

const FS_TYPE_DIRECTORY: u32 = 1;
const DOWNLOAD_PROGRESS_STEP: u32 = 512 * 1024;

static mut DOWNLOAD_LAST_PRINT: u32 = 0;
static mut APT_INDEX_READY: bool = false;

fn package_temp_path(path: &str) -> String {
    alloc::format!("{}.licof-tmp", path)
}

fn extract_tar_entry_atomic(reader: &libzip_client::TarReader, index: u32, dest: &str) -> bool {
    ensure_parent_dirs(dest);
    let temp = package_temp_path(dest);
    let _ = fs::unlink(&temp);
    if !reader.extract_to_file(index, &temp) {
        let _ = fs::unlink(&temp);
        return false;
    }
    if file_size(&temp) != reader.entry_size(index) {
        let _ = fs::unlink(&temp);
        return false;
    }
    replace_with_temp_file(&temp, dest)
}

struct PackageLink {
    index: u32,
    rel: String,
    dest: String,
    target: String,
    symlink: bool,
}

macro_rules! println {
    () => {
        anyos_std::println!()
    };
    ($($arg:tt)*) => {{
        log_bootstrap_line(&alloc::format!($($arg)*));
    }};
}

fn log_bootstrap_line(message: &str) {
    let action = strip_log_prefix(message);
    let level = classify_log_level(&action);
    anyos_std::println!("[{}]\t{}", level, action);
}

fn strip_log_prefix(message: &str) -> String {
    if let Some(rest) = message.strip_prefix("licof apt: ") {
        return alloc::format!("apt: {}", rest);
    }
    if let Some(rest) = message.strip_prefix("licof pkg: ") {
        return alloc::format!("pkg: {}", rest);
    }
    if let Some(rest) = message.strip_prefix("licof download: ") {
        return alloc::format!("download: {}", rest);
    }
    if let Some(rest) = message.strip_prefix("licof: ") {
        return String::from(rest);
    }
    String::from(message)
}

fn classify_log_level(action: &str) -> &'static str {
    if contains_word(action, "panic") || contains_word(action, "corrupt") {
        return "FATAL";
    }
    if contains_word(action, "retrying")
        || contains_word(action, "falling back")
        || contains_word(action, "fallback")
        || contains_word(action, "refreshing")
        || contains_word(action, "already scheduled")
        || contains_word(action, "looks like HTML")
    {
        return "WARN";
    }
    if contains_word(action, "failed")
        || contains_word(action, "cannot")
        || contains_word(action, "missing")
        || contains_word(action, "mismatch")
        || contains_word(action, "unavailable")
        || contains_word(action, "invalid")
        || contains_word(action, "empty")
        || contains_word(action, "not found")
        || contains_word(action, "not satisfied")
        || contains_word(action, "absent")
        || contains_word(action, "did not create")
        || contains_word(action, "no dependency")
    {
        return "ERROR";
    }
    "OK"
}

fn contains_word(haystack: &str, needle: &str) -> bool {
    haystack.find(needle).is_some()
}

pub(crate) struct InstallProgress {
    verbose: bool,
    overall_done: u32,
    overall_total: u32,
    overall_unit: &'static str,
    package_name: String,
    package_version: String,
    package_done: u32,
    package_total: u32,
    files_written: u32,
}

impl InstallProgress {
    pub(crate) fn new(verbose: bool, overall_total: u32, overall_unit: &'static str) -> Self {
        Self {
            verbose,
            overall_done: 0,
            overall_total,
            overall_unit,
            package_name: String::from("preparing"),
            package_version: String::new(),
            package_done: 0,
            package_total: 0,
            files_written: 0,
        }
    }

    pub(crate) fn verbose(&self) -> bool {
        self.verbose
    }

    pub(crate) fn set_overall(&mut self, done: u32, total: u32) {
        self.overall_done = done;
        self.overall_total = total;
        println!(
            "licof: overall {}/{} {}",
            self.overall_done, self.overall_total, self.overall_unit
        );
    }

    pub(crate) fn phase(&mut self, name: &str, detail: &str) {
        self.package_name.clear();
        self.package_name.push_str(name);
        self.package_version.clear();
        self.package_version.push_str(detail);
        self.package_done = 0;
        self.package_total = 0;
        self.files_written = 0;
        if detail.is_empty() {
            println!("licof: {}", name);
        } else {
            println!("licof: {}: {}", name, detail);
        }
    }

    fn package_start(&mut self, name: &str, version: &str, total: u32) {
        self.package_name.clear();
        self.package_name.push_str(name);
        self.package_version.clear();
        self.package_version.push_str(version);
        self.package_done = 0;
        self.package_total = total;
        self.files_written = 0;
        if version.is_empty() {
            println!("licof: unpacking {} ({} entries)", name, total);
        } else {
            println!("licof: unpacking {} {} ({} entries)", name, version, total);
        }
    }

    fn package_file(&mut self, done: u32, files_written: u32) {
        self.package_done = done;
        self.files_written = files_written;
        if self.verbose {
            println!(
                "licof: package progress {}/{} entries, {} files",
                self.package_done, self.package_total, self.files_written
            );
        }
    }

    fn package_done(&mut self, files_written: u32) {
        self.package_done = self.package_total;
        self.files_written = files_written;
        println!(
            "licof: unpacked {} {} ({} files)",
            self.package_name, self.package_version, self.files_written
        );
    }

    pub(crate) fn finish(&mut self) {}
}

pub(crate) fn install_package(
    config: &LicoConfig,
    pkg: &str,
    rootfs: &str,
    depth: u8,
    progress: &mut InstallProgress,
) -> bool {
    let mut pending = Vec::new();
    install_package_inner(config, pkg, rootfs, depth, &mut pending, progress)
}

pub(crate) fn package_installed(config: &LicoConfig, pkg: &str, rootfs: &str) -> bool {
    is_installed(config, pkg, rootfs)
}

fn install_package_inner(
    config: &LicoConfig,
    pkg: &str,
    rootfs: &str,
    depth: u8,
    pending: &mut Vec<String>,
    progress: &mut InstallProgress,
) -> bool {
    if depth > 32 {
        progress.finish();
        println!("licof apt: dependency recursion too deep at '{}'", pkg);
        return false;
    }
    if is_installed(config, pkg, rootfs) {
        return true;
    }
    progress.phase("apt resolve", pkg);
    if !ensure_apt_index(config, progress) {
        return false;
    }

    let package_index_txt = config.package_index_txt();
    progress.phase("apt parse", pkg);
    let Some(info) = find_package_in_index(config, pkg) else {
        progress.finish();
        println!("licof apt: package '{}' not found", pkg);
        if package_name_present(&package_index_txt, pkg) {
            println!(
                "licof apt: package '{}' exists in raw index but could not be parsed",
                pkg
            );
        } else {
            println!(
                "licof apt: package '{}' is absent from cached index ({} bytes)",
                pkg,
                file_size(&package_index_txt)
            );
        }
        return false;
    };
    if is_installed(config, &info.package, rootfs) {
        return true;
    }
    if dependency_pending(pkg, pending) || dependency_pending(&info.package, pending) {
        if progress.verbose() {
            println!(
                "licof apt: dependency '{}' is already scheduled; continuing",
                pkg
            );
        }
        return true;
    }

    pending.push(info.package.clone());

    progress.phase("apt depends", &info.package);
    for dep_group in parse_depends(&info.pre_depends) {
        if !install_dependency_group(config, &dep_group, rootfs, depth + 1, pending, progress) {
            progress.finish();
            println!("licof apt: dependency for '{}' not satisfied", info.package);
            pending.pop();
            return false;
        }
    }
    for dep_group in parse_depends(&info.depends) {
        if !install_dependency_group(config, &dep_group, rootfs, depth + 1, pending, progress) {
            progress.finish();
            println!("licof apt: dependency for '{}' not satisfied", info.package);
            pending.pop();
            return false;
        }
    }

    let deb_path = alloc::format!(
        "{}/{}",
        config.cache,
        cache_deb_name(&info.package, &info.version, &info.filename)
    );
    if fs::stat(&deb_path, &mut [0u32; 7]) == 0 && !verify_package_file(&info, &deb_path) {
        let _ = fs::unlink(&deb_path);
    }
    if fs::stat(&deb_path, &mut [0u32; 7]) != 0 {
        let url = alloc::format!("{}/{}", config.apt_base, info.filename);
        ensure_dir_recursive(&config.cache);
        if !download_verified_package(config, &info, &url, &deb_path, progress) {
            pending.pop();
            return false;
        }
    } else {
        progress.phase("deb cache", &info.package);
    }

    progress.phase("deb extract", &info.package);
    let ok = install_deb(config, &deb_path, rootfs, Some(&info), progress);
    pending.pop();
    ok
}

fn install_dependency_group(
    config: &LicoConfig,
    alternatives: &[String],
    rootfs: &str,
    depth: u8,
    pending: &mut Vec<String>,
    progress: &mut InstallProgress,
) -> bool {
    for dep in alternatives {
        if install_package_inner(config, dep, rootfs, depth, pending, progress) {
            return true;
        }
    }
    if !alternatives.is_empty() {
        progress.finish();
        println!(
            "licof apt: no dependency alternative worked: {}",
            alternatives[0]
        );
    }
    alternatives.is_empty()
}

fn download_verified_package(
    config: &LicoConfig,
    info: &PackageInfo,
    url: &str,
    dest: &str,
    progress: &mut InstallProgress,
) -> bool {
    for attempt in 1..=config.download_attempts {
        let _ = fs::unlink(dest);
        progress.phase(
            "deb download",
            &alloc::format!(
                "{} {} attempt {}/{}",
                info.package,
                info.version,
                attempt,
                config.download_attempts
            ),
        );
        if progress.verbose() {
            println!(
                "licof apt: downloading {} {} (attempt {}/{})",
                info.package, info.version, attempt, config.download_attempts
            );
        }
        progress.finish();
        if !download_url(config, url, dest) {
            progress.finish();
            println!("licof apt: download failed: {}", url);
            return false;
        }
        progress.phase("deb verify", &info.package);
        if verify_package_file(info, dest) {
            return true;
        }
        let _ = fs::unlink(dest);
        if attempt < config.download_attempts {
            progress.finish();
            println!(
                "licof apt: package verification failed for {}; retrying download",
                info.package
            );
        }
    }
    progress.finish();
    println!(
        "licof apt: package verification failed for {} after {} attempts",
        info.package, config.download_attempts
    );
    false
}

fn dependency_pending(pkg: &str, pending: &[String]) -> bool {
    pending.iter().any(|p| p == pkg)
}

fn apt_index_ready() -> bool {
    unsafe { APT_INDEX_READY }
}

fn set_apt_index_ready() {
    unsafe {
        APT_INDEX_READY = true;
    }
}

fn ensure_apt_index(config: &LicoConfig, progress: &mut InstallProgress) -> bool {
    if apt_index_ready() {
        progress.phase("apt index", "using verified cache");
        return true;
    }

    progress.phase("apt index", "initializing libzip");
    if !libzip_client::init() {
        progress.finish();
        println!("licof apt: libzip unavailable");
        return false;
    }
    ensure_dir_recursive(&config.cache);
    let packages_gz = config.package_index_gz();
    let packages_txt = config.package_index_txt();
    progress.phase("apt index", "checking cache");
    if file_size(&packages_txt) > 0 {
        println!(
            "licof apt: cached package index: {} ({} bytes)",
            packages_txt,
            file_size(&packages_txt)
        );
        println!("licof apt: cached package index: checking header");
        if looks_like_plain_packages_index(&packages_txt) {
            println!(
                "licof apt: cached package index: checking {} required packages",
                config.index_required_packages.len()
            );
        }
        if looks_like_plain_packages_index(&packages_txt)
            && packages_index_has_required_entries(config)
        {
            set_apt_index_ready();
            println!("licof apt: cached package index: ok");
            return true;
        }
        println!("licof apt: cached package index is invalid; refreshing");
        let _ = fs::unlink(&packages_txt);
    }
    let url = config.package_index_url();

    for attempt in 1..=config.download_attempts {
        let _ = fs::unlink(&packages_gz);
        let _ = fs::unlink(&packages_txt);
        progress.phase(
            "apt index download",
            &alloc::format!("attempt {}/{}", attempt, config.download_attempts),
        );
        if progress.verbose() {
            println!(
                "licof apt: fetching package index (attempt {}/{})",
                attempt, config.download_attempts
            );
        }
        progress.finish();
        if !download_url(config, &url, &packages_gz) {
            progress.finish();
            println!("licof apt: failed to download {}", url);
            continue;
        }
        let downloaded = file_size(&packages_gz);
        progress.phase(
            "apt index validate",
            &alloc::format!("{} bytes downloaded", downloaded),
        );
        if downloaded == 0 {
            progress.finish();
            println!("licof apt: downloaded package index is empty");
            continue;
        }
        if looks_like_plain_packages_index(&packages_gz) {
            progress.finish();
            println!(
                "licof apt: package index arrived uncompressed ({} bytes)",
                downloaded
            );
            if !copy_file(&packages_gz, &packages_txt) {
                progress.finish();
                println!("licof apt: cannot store uncompressed package index");
                continue;
            }
        } else {
            if !looks_like_gzip(&packages_gz) {
                progress.finish();
                print_index_download_diagnostic(&packages_gz, downloaded);
                continue;
            }
            progress.phase(
                "apt index decompress",
                &alloc::format!("{} bytes gzip", downloaded),
            );
            let gzip_status =
                libzip_client::gzip_decompress_file_status(&packages_gz, &packages_txt);
            if gzip_status != libzip_client::GZIP_STATUS_OK {
                progress.finish();
                println!(
                    "licof apt: failed to decompress package index: {} (downloaded {} bytes)",
                    gzip_status_text(gzip_status),
                    downloaded
                );
                print_gzip_diagnostic(&packages_gz, downloaded);
                continue;
            }
        }

        let unpacked = file_size(&packages_txt);
        progress.phase(
            "apt index verify",
            &alloc::format!("{} bytes unpacked", unpacked),
        );
        if unpacked == 0 {
            progress.finish();
            println!("licof apt: decompressed package index is empty");
            continue;
        }
        if !looks_like_plain_packages_index(&packages_txt) {
            progress.finish();
            println!("licof apt: decompressed package index is not a Packages file");
            continue;
        }
        if !packages_index_has_required_entries(config) {
            progress.finish();
            println!("licof apt: decompressed package index is missing bootstrap entries");
            continue;
        }
        set_apt_index_ready();
        println!("licof apt: package index ready ({} bytes)", unpacked);
        return true;
    }

    let _ = fs::unlink(&packages_txt);
    false
}

fn find_package_in_index(config: &LicoConfig, wanted: &str) -> Option<PackageInfo> {
    let wanted = preferred_package(wanted).unwrap_or(wanted);
    let packages_txt = config.package_index_txt();
    let mut file = match fs::File::open(&packages_txt) {
        Ok(file) => file,
        Err(_) => {
            println!("licof apt: cannot open package index '{}'", packages_txt);
            return find_package_in_compressed_index(config, wanted);
        }
    };
    let mut chunk = [0u8; 4096];
    let mut para = Vec::with_capacity(1024);
    let mut newline_run = 0usize;

    loop {
        let n = match file.read(&mut chunk) {
            Ok(n) => n,
            Err(_) => {
                println!("licof apt: cannot read package index '{}'", packages_txt);
                return find_package_in_compressed_index(config, wanted);
            }
        };
        if n == 0 {
            break;
        }
        for &b in &chunk[..n] {
            para.push(b);
            if b == b'\n' {
                newline_run += 1;
                if newline_run >= 2 {
                    if let Some(info) = package_info_from_para(config, &para, wanted) {
                        return Some(info);
                    }
                    para.clear();
                    newline_run = 0;
                }
            } else if b != b'\r' {
                newline_run = 0;
            }
        }
    }

    let found = if !para.is_empty() {
        package_info_from_para(config, &para, wanted)
    } else {
        None
    };
    if found.is_some() {
        found
    } else {
        find_package_in_compressed_index(config, wanted)
    }
}

fn find_package_in_compressed_index(config: &LicoConfig, wanted: &str) -> Option<PackageInfo> {
    let Some(index) = read_compressed_package_index(config) else {
        return None;
    };
    let found = find_package_in_bytes(config, &index, wanted);
    if found.is_some() {
        println!(
            "licof apt: resolved '{}' from compressed package index",
            wanted
        );
    }
    found
}

fn find_package_in_bytes(config: &LicoConfig, index: &[u8], wanted: &str) -> Option<PackageInfo> {
    let mut start = 0usize;
    let mut pos = 0usize;
    let mut newline_run = 0usize;

    while pos < index.len() {
        let b = index[pos];
        if b == b'\n' {
            newline_run += 1;
            if newline_run >= 2 {
                if let Some(info) = package_info_from_para(config, &index[start..=pos], wanted) {
                    return Some(info);
                }
                start = pos + 1;
                newline_run = 0;
            }
        } else if b != b'\r' {
            newline_run = 0;
        }
        pos += 1;
    }

    if start < index.len() {
        package_info_from_para(config, &index[start..], wanted)
    } else {
        None
    }
}

fn package_info_from_para(config: &LicoConfig, para: &[u8], wanted: &str) -> Option<PackageInfo> {
    let package = field_value(para, b"Package")?;
    let exact = package == wanted;
    if !exact && !provides_package_bytes(para, wanted) {
        return None;
    }
    let arch = field_value(para, b"Architecture").unwrap_or_default();
    if arch != config.apt_arch && arch != "all" {
        return None;
    }
    Some(PackageInfo {
        package,
        version: field_value(para, b"Version").unwrap_or_else(|| String::from("unknown")),
        filename: field_value(para, b"Filename")?,
        size: field_value(para, b"Size")
            .and_then(|s| parse_decimal(&s))
            .unwrap_or(0),
        md5: field_value(para, b"MD5sum").unwrap_or_default(),
        depends: field_value(para, b"Depends").unwrap_or_default(),
        pre_depends: field_value(para, b"Pre-Depends").unwrap_or_default(),
    })
}

fn preferred_package(wanted: &str) -> Option<&'static str> {
    match wanted {
        "awk" => Some("mawk"),
        _ => None,
    }
}

fn provides_package_bytes(para: &[u8], wanted: &str) -> bool {
    let provides = field_value(para, b"Provides").unwrap_or_default();
    for provided in parse_depends(&provides) {
        if provided.iter().any(|name| name == wanted) {
            return true;
        }
    }
    false
}

fn field_value(para: &[u8], key: &[u8]) -> Option<String> {
    let mut line_start = 0usize;
    let mut collecting = false;
    let mut out = String::new();
    while line_start < para.len() {
        let mut line_end = line_start;
        while line_end < para.len() && para[line_end] != b'\n' {
            line_end += 1;
        }
        let mut line = &para[line_start..line_end];
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        if line.len() > key.len() + 2
            && &line[..key.len()] == key
            && line[key.len()] == b':'
            && line[key.len() + 1] == b' '
        {
            out.clear();
            push_utf8_trimmed(&mut out, &line[key.len() + 2..]);
            collecting = true;
        } else if collecting && (line.first() == Some(&b' ') || line.first() == Some(&b'\t')) {
            if !out.is_empty() {
                out.push(' ');
            }
            push_utf8_trimmed(&mut out, line);
        } else if collecting {
            break;
        }
        line_start = line_end.saturating_add(1);
    }
    if collecting {
        Some(out)
    } else {
        None
    }
}

fn push_utf8_trimmed(out: &mut String, bytes: &[u8]) {
    if let Ok(s) = core::str::from_utf8(bytes) {
        out.push_str(s.trim());
    }
}

fn parse_depends(raw: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let mut group = Vec::new();
        for alt in part.split('|') {
            let Some(name) = dependency_name(alt.trim()) else {
                continue;
            };
            if !group.iter().any(|p| p == &name) {
                group.push(name);
            }
        }
        if !group.is_empty() {
            out.push(group);
        }
    }
    out
}

fn dependency_name(raw: &str) -> Option<String> {
    let mut name = raw.split_ascii_whitespace().next().unwrap_or("").trim();
    if let Some(pos) = name.find(':') {
        name = &name[..pos];
    }
    if name.is_empty() {
        None
    } else {
        Some(String::from(name))
    }
}

pub(crate) fn install_deb(
    config: &LicoConfig,
    path: &str,
    rootfs: &str,
    info: Option<&PackageInfo>,
    progress: &mut InstallProgress,
) -> bool {
    if !libzip_client::init() {
        progress.finish();
        println!("licof pkg: libzip unavailable");
        return false;
    }
    let data = match fs::read_to_vec(path) {
        Ok(d) => d,
        Err(_) => {
            progress.finish();
            println!("licof pkg: cannot read '{}'", path);
            return false;
        }
    };
    let Some(reader) = data_tar_reader(&data, path) else {
        progress.finish();
        println!("licof pkg: cannot open package data archive: {}", path);
        return false;
    };
    if reader.entry_count() == 0 {
        progress.finish();
        println!("licof pkg: staged data archive has no entries: {}", path);
        return false;
    }
    let package_name = info
        .map(|i| i.package.as_str())
        .unwrap_or(deb_basename(path));
    let package_version = info.map(|i| i.version.as_str()).unwrap_or("");
    progress.package_start(package_name, package_version, reader.entry_count());

    let mut files = 0u32;
    let mut complete = true;
    let mut manifest = String::new();
    let mut links = Vec::new();
    for i in 0..reader.entry_count() {
        let name = reader.entry_name(i);
        let Some(rel) = sanitize_tar_path(&name) else {
            continue;
        };
        let dest = alloc::format!("{}/{}", rootfs, rel);
        let typeflag = reader.entry_typeflag(i) as u8;
        if reader.entry_is_dir(i) || typeflag == b'5' {
            ensure_dir_recursive(&dest);
            apply_tar_metadata(&reader, i, &dest);
            progress.package_file(i + 1, files);
        } else if typeflag == b'2' {
            links.push(PackageLink {
                index: i,
                rel,
                dest,
                target: reader.entry_link_name(i),
                symlink: true,
            });
            progress.package_file(i + 1, files);
        } else if typeflag == b'1' {
            links.push(PackageLink {
                index: i,
                rel,
                dest,
                target: reader.entry_link_name(i),
                symlink: false,
            });
            progress.package_file(i + 1, files);
        } else {
            ensure_parent_dirs(&dest);
            if progress.verbose() {
                println!("licof pkg: extracting {}", rel);
            }
            if extract_tar_entry_atomic(&reader, i, &dest) {
                apply_tar_metadata(&reader, i, &dest);
                files += 1;
                append_manifest_path(&mut manifest, &rel);
                progress.package_file(i + 1, files);
            } else {
                progress.finish();
                println!("licof pkg: failed to extract {}", rel);
                complete = false;
            }
        }
    }
    for link in &links {
        if install_package_link(rootfs, &reader, link, progress.verbose()) {
            files += 1;
            append_manifest_path(&mut manifest, &link.rel);
            progress.package_file(reader.entry_count(), files);
        } else if link.symlink {
            progress.finish();
            println!(
                "licof pkg: failed to create symlink {} -> {}",
                link.rel, link.target
            );
            complete = false;
        } else {
            progress.finish();
            println!(
                "licof pkg: failed to materialize hardlink {} -> {}",
                link.rel, link.target
            );
            complete = false;
        }
    }
    if files == 0 {
        progress.finish();
        println!("licof pkg: extracted no files from '{}'", path);
        return false;
    }
    if !complete {
        progress.finish();
        println!("licof pkg: package '{}' extracted incompletely", path);
        return false;
    }

    if let Some(info) = info {
        if !validate_installed_package(rootfs, info) {
            progress.finish();
            return false;
        }
        mark_installed(config, info, rootfs, files, &manifest);
        progress.package_done(files);
        if progress.verbose() {
            println!(
                "licof apt: installed {} {} ({} files)",
                info.package, info.version, files
            );
        }
    } else {
        progress.package_done(files);
        println!("licof pkg: extracted {} files", files);
    }
    true
}

fn apply_tar_metadata(reader: &libzip_client::TarReader, index: u32, path: &str) {
    let mode = reader.entry_mode(index) as u16;
    if mode != 0 {
        let _ = fs::chmod(path, mode);
    }
    let uid = reader.entry_uid(index) as u16;
    let gid = reader.entry_gid(index) as u16;
    if uid != 0 || gid != 0 {
        let _ = fs::chown(path, uid, gid);
    }
}

fn install_package_link(
    rootfs: &str,
    reader: &libzip_client::TarReader,
    link: &PackageLink,
    verbose: bool,
) -> bool {
    if link.target.is_empty() {
        return false;
    }
    ensure_parent_dirs(&link.dest);
    if link.symlink {
        if verbose {
            println!("licof pkg: linking {} -> {}", link.rel, link.target);
        }
        let _ = fs::unlink(&link.dest);
        if fs::symlink(&link.target, &link.dest) == 0 {
            if symlink_points_to(&link.dest, &link.target) {
                return true;
            }
            println!(
                "licof pkg: symlink verification failed {} -> {}",
                link.rel, link.target
            );
            let _ = fs::unlink(&link.dest);
        }
        return false;
    } else {
        if verbose {
            println!("licof pkg: hardlink {} -> {}", link.rel, link.target);
        }
    }
    if materialize_hardlink_target(rootfs, reader, &link.dest, &link.target) {
        apply_tar_metadata(reader, link.index, &link.dest);
        return true;
    }
    false
}

fn materialize_hardlink_target(
    rootfs: &str,
    reader: &libzip_client::TarReader,
    dest: &str,
    target: &str,
) -> bool {
    let Some(src) = resolve_package_link_target(rootfs, dest, target, false) else {
        return false;
    };
    if !path_under_rootfs(rootfs, &src) {
        return false;
    }
    let mut stat_buf = [0u32; 7];
    if fs::stat(&src, &mut stat_buf) != 0 {
        return materialize_hardlink_from_archive(rootfs, reader, dest, target, 0);
    }
    let _ = fs::unlink(dest);
    if stat_buf[0] == FS_TYPE_DIRECTORY {
        ensure_dir_recursive(dest);
        true
    } else {
        copy_file(&src, dest)
    }
}

fn materialize_hardlink_from_archive(
    rootfs: &str,
    reader: &libzip_client::TarReader,
    dest: &str,
    target: &str,
    depth: u8,
) -> bool {
    if depth > 8 || !path_under_rootfs(rootfs, dest) {
        return false;
    }
    let Some(index) = find_tar_entry(reader, target) else {
        return false;
    };
    let typeflag = reader.entry_typeflag(index) as u8;
    let name = reader.entry_name(index);
    let Some(rel) = sanitize_tar_path(&name) else {
        return false;
    };
    if reader.entry_is_dir(index) || typeflag == b'5' {
        let _ = fs::unlink(dest);
        ensure_dir_recursive(dest);
        return true;
    }
    if typeflag == b'1' {
        let next = reader.entry_link_name(index);
        return materialize_hardlink_from_archive(rootfs, reader, dest, &next, depth + 1);
    }
    if typeflag == b'2' {
        let link_target = reader.entry_link_name(index);
        let _ = fs::unlink(dest);
        return fs::symlink(&link_target, dest) == 0 && symlink_points_to(dest, &link_target);
    }

    ensure_parent_dirs(dest);
    if extract_tar_entry_atomic(reader, index, dest) {
        apply_tar_metadata(reader, index, dest);
        return true;
    }
    println!(
        "licof pkg: failed to extract hardlink target {} for {}",
        rel, target
    );
    false
}

fn find_tar_entry(reader: &libzip_client::TarReader, wanted: &str) -> Option<u32> {
    let clean_wanted = sanitize_tar_path(wanted)?;
    for i in 0..reader.entry_count() {
        let name = reader.entry_name(i);
        if sanitize_tar_path(&name).as_deref() == Some(clean_wanted.as_str()) {
            return Some(i);
        }
    }
    None
}

fn validate_installed_package(rootfs: &str, info: &PackageInfo) -> bool {
    match info.package.as_str() {
        "coreutils" => validate_coreutils_runtime(rootfs),
        "debian-archive-keyring" => validate_debian_archive_keyring_runtime(rootfs),
        "libc6" => validate_libc6_runtime(rootfs),
        "libpam0g" => validate_libpam_runtime(rootfs),
        _ => true,
    }
}

fn validate_coreutils_runtime(rootfs: &str) -> bool {
    let tee_ok = validate_runtime_elf(rootfs, "/usr/bin/tee", "coreutils tee");
    let ls_ok = validate_runtime_elf(rootfs, "/bin/ls", "coreutils ls");
    let cat_ok = validate_runtime_elf(rootfs, "/bin/cat", "coreutils cat");
    tee_ok && ls_ok && cat_ok
}

fn validate_libc6_runtime(rootfs: &str) -> bool {
    ensure_runtime_alias(
        rootfs,
        "/lib64/ld-linux-x86-64.so.2",
        "/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
        "dynamic loader",
    );

    let loader_ok = validate_runtime_elf(rootfs, "/lib64/ld-linux-x86-64.so.2", "dynamic loader");
    let loader_alias_ok = validate_runtime_elf(
        rootfs,
        "/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
        "dynamic loader alias",
    );
    let libc_ok = validate_runtime_elf(rootfs, "/lib/x86_64-linux-gnu/libc.so.6", "libc");
    loader_ok && loader_alias_ok && libc_ok
}

fn validate_libpam_runtime(rootfs: &str) -> bool {
    let pam_ok = validate_runtime_elf(rootfs, "/lib/x86_64-linux-gnu/libpam.so.0", "libpam");
    let misc_ok = validate_runtime_elf(
        rootfs,
        "/lib/x86_64-linux-gnu/libpam_misc.so.0",
        "libpam_misc",
    );
    let pamc_ok = validate_runtime_elf(rootfs, "/lib/x86_64-linux-gnu/libpamc.so.0", "libpamc");
    pam_ok && misc_ok && pamc_ok
}

fn validate_debian_archive_keyring_runtime(rootfs: &str) -> bool {
    let required = [
        "/usr/share/keyrings/debian-archive-keyring.gpg",
        "/usr/share/keyrings/debian-archive-bookworm-automatic.gpg",
        "/usr/share/keyrings/debian-archive-bookworm-security-automatic.gpg",
        "/usr/share/keyrings/debian-archive-bookworm-stable.gpg",
        "/etc/apt/trusted.gpg.d/debian-archive-bookworm-automatic.asc",
        "/etc/apt/trusted.gpg.d/debian-archive-bookworm-security-automatic.asc",
        "/etc/apt/trusted.gpg.d/debian-archive-bookworm-stable.asc",
    ];
    let mut ok = true;
    for linux_path in required {
        let path = linux_path_in_rootfs(rootfs, linux_path);
        let size = file_size(&path);
        if size < 1024 {
            println!(
                "licof pkg: debian archive keyring check failed: {} size={}",
                path, size
            );
            print_path_probe("licof pkg", &path);
            ok = false;
        }
    }
    ok
}

fn ensure_runtime_alias(rootfs: &str, dest_linux: &str, target: &str, label: &str) -> bool {
    let dest = linux_path_in_rootfs(rootfs, dest_linux);
    let src = resolve_package_link_target(rootfs, &dest, target, true)
        .unwrap_or_else(|| linux_path_in_rootfs(rootfs, target));
    if !is_elf_file(&src) {
        println!(
            "licof pkg: cannot repair {} {}; source {} is not an ELF",
            label, dest, src
        );
        return false;
    }

    if symlink_points_to(&dest, target) && rootfs_resolved_is_elf(rootfs, &dest) {
        return true;
    }
    if path_exists_no_follow(&dest) && !path_is_symlink(&dest) && is_elf_file(&dest) {
        let dest_size = file_size(&dest);
        let src_size = file_size(&src);
        if dest_size != 0 && dest_size == src_size {
            return true;
        }
        println!(
            "licof pkg: replacing stale {} {} (size {}, expected {})",
            label, dest, dest_size, src_size
        );
    } else if path_exists_no_follow(&dest) && path_is_symlink(&dest) {
        println!("licof pkg: replacing stale {} symlink {}", label, dest);
    } else if path_exists_no_follow(&dest) {
        println!("licof pkg: replacing stale {} {}", label, dest);
    }

    ensure_parent_dirs(&dest);
    let _ = fs::unlink(&dest);
    if fs::symlink(target, &dest) == 0 && rootfs_resolved_is_elf(rootfs, &dest) {
        println!("licof pkg: restored {} {} -> {}", label, dest, target);
        return true;
    }

    println!(
        "licof pkg: failed to restore {} {} -> {}",
        label, dest, target
    );
    false
}

fn rootfs_resolved_is_elf(rootfs: &str, path: &str) -> bool {
    if is_elf_file(path) {
        return true;
    }
    resolve_rootfs_symlink_path(rootfs, path)
        .map(|resolved| is_elf_file(&resolved))
        .unwrap_or(false)
}

fn validate_runtime_elf(rootfs: &str, linux_path: &str, label: &str) -> bool {
    let path = linux_path_in_rootfs(rootfs, linux_path);
    let resolved = resolve_rootfs_symlink_path(rootfs, &path).unwrap_or_else(|| path.clone());
    if is_elf_file(&path) || is_elf_file(&resolved) {
        return true;
    }
    println!(
        "licof pkg: {} runtime check failed: {} -> {} is not an ELF",
        label, path, resolved
    );
    print_path_probe("licof pkg", &path);
    if resolved != path {
        print_path_probe("licof pkg", &resolved);
    }
    false
}

fn resolve_package_link_target(
    rootfs: &str,
    dest: &str,
    target: &str,
    symlink: bool,
) -> Option<String> {
    if target.starts_with('/') {
        return Some(normalize_abs_path(&alloc::format!("{}{}", rootfs, target)));
    }
    if symlink {
        let parent = dest.rfind('/').map(|pos| &dest[..pos]).unwrap_or(rootfs);
        return Some(normalize_abs_path(&alloc::format!("{}/{}", parent, target)));
    }
    let clean = sanitize_tar_path(target)?;
    Some(normalize_abs_path(&alloc::format!("{}/{}", rootfs, clean)))
}

fn data_tar_reader(data: &[u8], path: &str) -> Option<libzip_client::TarReader> {
    if let Some(tar_data) = ar_entry(data, "data.tar.gz") {
        return libzip_client::TarReader::from_bytes(&tar_data);
    }
    if let Some(xz_data) = ar_entry(data, "data.tar.xz") {
        let Some(tar_data) = libzip_client::unxz(&xz_data) else {
            println!("licof pkg: cannot decompress data.tar.xz from '{}'", path);
            return None;
        };
        return libzip_client::TarReader::from_bytes(&tar_data);
    }
    println!("licof pkg: '{}' has no supported data.tar.* member", path);
    None
}

fn ar_entry(data: &[u8], wanted: &str) -> Option<Vec<u8>> {
    if data.len() < 8 || &data[..8] != b"!<arch>\n" {
        return None;
    }
    let mut pos = 8usize;
    while pos + 60 <= data.len() {
        let header = &data[pos..pos + 60];
        let raw_name = core::str::from_utf8(&header[..16]).ok()?.trim();
        let name = raw_name.trim_end_matches('/');
        let size_str = core::str::from_utf8(&header[48..58]).ok()?.trim();
        let size = parse_decimal(size_str)?;
        let start = pos + 60;
        let end = start.checked_add(size)?;
        if end > data.len() {
            return None;
        }
        if name == wanted {
            return Some(data[start..end].to_vec());
        }
        pos = end + (size & 1);
    }
    None
}

fn parse_decimal(s: &str) -> Option<usize> {
    let mut out = 0usize;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(out)
}

fn sanitize_tar_path(name: &str) -> Option<String> {
    let mut rel = name.trim_start_matches("./").trim_start_matches('/');
    while rel.starts_with("./") {
        rel = rel.trim_start_matches("./");
    }
    if rel.is_empty() || rel.contains("..") {
        return None;
    }
    Some(String::from(rel))
}

fn append_manifest_path(manifest: &mut String, rel: &str) {
    manifest.push_str("Path: ");
    manifest.push_str(rel);
    manifest.push('\n');
}

fn mark_installed(
    config: &LicoConfig,
    info: &PackageInfo,
    rootfs: &str,
    files: u32,
    manifest: &str,
) {
    let db_dir = installed_db_dir(config, rootfs);
    ensure_dir(&db_dir);
    let path = installed_package_path(config, &info.package, rootfs);
    let body = alloc::format!(
        "Package: {}\nVersion: {}\nRootFS: {}\nFilename: {}\nFiles: {}\n{}",
        info.package,
        info.version,
        rootfs,
        info.filename,
        files,
        manifest
    );
    let _ = write_bytes_atomic(&path, body.as_bytes());
}

fn is_installed(config: &LicoConfig, pkg: &str, rootfs: &str) -> bool {
    let path = installed_package_path(config, pkg, rootfs);
    let Ok(data) = fs::read_to_vec(&path) else {
        return false;
    };
    if installed_manifest_valid(rootfs, pkg, &data) && installed_payload_sane(rootfs, pkg) {
        return true;
    }
    println!(
        "licof apt: installed marker for '{}' failed validation; reinstalling",
        pkg
    );
    let _ = fs::unlink(&path);
    false
}

fn installed_payload_sane(rootfs: &str, pkg: &str) -> bool {
    match pkg {
        "coreutils" => validate_coreutils_runtime(rootfs),
        "debian-archive-keyring" => validate_debian_archive_keyring_runtime(rootfs),
        _ => true,
    }
}

fn installed_manifest_valid(rootfs: &str, pkg: &str, data: &[u8]) -> bool {
    let Ok(text) = core::str::from_utf8(data) else {
        println!(
            "licof apt: installed marker for '{}' is not valid UTF-8",
            pkg
        );
        return false;
    };
    let mut paths = 0u32;
    for line in text.lines() {
        let Some(rel) = line.strip_prefix("Path: ") else {
            continue;
        };
        if !valid_manifest_relative_path(rel) {
            println!(
                "licof apt: installed marker for '{}' has invalid payload path '{}'",
                pkg, rel
            );
            return false;
        }
        paths += 1;
        if !installed_payload_path_exists(rootfs, rel) {
            println!(
                "licof apt: installed marker for '{}' references missing payload '{}'",
                pkg, rel
            );
            return false;
        }
    }
    if paths == 0 {
        println!(
            "licof apt: installed marker for '{}' has no payload paths",
            pkg
        );
        return false;
    }
    true
}

fn installed_payload_path_exists(rootfs: &str, rel: &str) -> bool {
    let path = normalize_abs_path(&alloc::format!("{}/{}", rootfs, rel));
    path_under_rootfs(rootfs, &path) && path_exists_no_follow(&path)
}

fn valid_manifest_relative_path(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && rel
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn installed_package_path(config: &LicoConfig, pkg: &str, rootfs: &str) -> String {
    let mut package_key = String::new();
    push_cache_safe(&mut package_key, pkg);
    alloc::format!("{}/{}", installed_db_dir(config, rootfs), package_key)
}

fn installed_db_dir(config: &LicoConfig, rootfs: &str) -> String {
    let mut key = String::new();
    push_cache_safe(&mut key, rootfs);
    alloc::format!("{}/{}", config.installed_db, key)
}

fn deb_basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn cache_deb_name(package: &str, version: &str, filename: &str) -> String {
    let mut out = String::new();
    push_cache_safe(&mut out, package);
    out.push('_');
    push_cache_safe(&mut out, version);
    out.push('_');
    push_cache_safe(&mut out, deb_basename(filename));
    out
}

fn push_cache_safe(out: &mut String, raw: &str) {
    for b in raw.bytes() {
        let safe = b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_';
        out.push(if safe { b as char } else { '_' });
    }
}

fn verify_package_file(info: &PackageInfo, path: &str) -> bool {
    let actual_size = file_size(path) as usize;
    if info.size > 0 && actual_size != info.size {
        println!(
            "licof apt: invalid package size for {}: got {}, expected {}",
            info.package, actual_size, info.size
        );
        return false;
    }

    let data = match fs::read_to_vec(path) {
        Ok(data) => data,
        Err(_) => {
            println!("licof apt: cannot read downloaded package '{}'", path);
            return false;
        }
    };
    if !looks_like_deb_bytes(&data) {
        println!(
            "licof apt: downloaded file is not a Debian archive: {}",
            path
        );
        return false;
    }

    if !info.md5.is_empty() {
        let actual = crypto::md5_hex(&data);
        let actual = core::str::from_utf8(&actual).unwrap_or("");
        if actual != info.md5 {
            println!(
                "licof apt: checksum mismatch for {}: got {}, expected {}",
                info.package, actual, info.md5
            );
            return false;
        }
    }
    true
}

fn looks_like_gzip(path: &str) -> bool {
    let prefix = read_prefix(path);
    prefix[0] == 0x1f && prefix[1] == 0x8b && prefix[2] == 0x08
}

fn looks_like_plain_packages_index(path: &str) -> bool {
    let prefix = read_prefix(path);
    prefix.starts_with(b"Package:")
}

fn looks_like_deb_bytes(data: &[u8]) -> bool {
    data.starts_with(b"!<arch>\n")
}

fn read_prefix(path: &str) -> [u8; 16] {
    let mut prefix = [0u8; 16];
    if let Ok(mut file) = fs::File::open(path) {
        let _ = file.read(&mut prefix);
    }
    prefix
}

fn print_index_download_diagnostic(path: &str, size: u32) {
    let prefix = read_prefix(path);
    println!(
        "licof apt: downloaded package index is not gzip ({} bytes, first bytes {:02x} {:02x} {:02x} {:02x})",
        size, prefix[0], prefix[1], prefix[2], prefix[3]
    );
    if prefix[0] == b'<' {
        println!("licof apt: response looks like HTML; archive server returned an error page");
    }
}

fn print_gzip_diagnostic(path: &str, size: u32) {
    let prefix = read_prefix(path);
    println!(
        "licof apt: gzip header: {:02x} {:02x} method={:02x} flags={:02x}",
        prefix[0], prefix[1], prefix[2], prefix[3]
    );
    match compressed_file_crc32(path) {
        Some(crc) => println!("licof apt: gzip file crc32=0x{:08x}", crc),
        None => println!("licof apt: cannot compute gzip file crc32"),
    }
    match read_gzip_trailer(path, size) {
        Some((crc, isize)) => println!(
            "licof apt: gzip trailer: crc32=0x{:08x} isize={} bytes",
            crc, isize
        ),
        None => println!("licof apt: cannot read gzip trailer"),
    }
}

fn read_gzip_trailer(path: &str, size: u32) -> Option<(u32, u32)> {
    if size < 8 || size > i32::MAX as u32 {
        return None;
    }
    let mut file = fs::File::open(path).ok()?;
    if fs::lseek(file.fd(), (size - 8) as i32, fs::SEEK_SET) == u32::MAX {
        return None;
    }
    let mut trailer = [0u8; 8];
    if file.read(&mut trailer).ok()? != trailer.len() {
        return None;
    }
    let crc = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
    let isize = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
    Some((crc, isize))
}

fn compressed_file_crc32(path: &str) -> Option<u32> {
    let mut file = fs::File::open(path).ok()?;
    let mut crc = 0xffff_ffffu32;
    let mut buf = [0u8; 4096];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        for &byte in &buf[..n] {
            crc ^= byte as u32;
            for _ in 0..8 {
                let mask = 0u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
    }
    Some(!crc)
}

fn gzip_status_text(status: u32) -> &'static str {
    match status {
        libzip_client::GZIP_STATUS_OK => "ok",
        libzip_client::GZIP_ERR_TOO_SHORT => "input too short",
        libzip_client::GZIP_ERR_BAD_MAGIC => "bad gzip magic",
        libzip_client::GZIP_ERR_BAD_METHOD => "unsupported gzip method",
        libzip_client::GZIP_ERR_BAD_FLAGS => "invalid gzip flags",
        libzip_client::GZIP_ERR_BAD_HEADER => "invalid gzip header",
        libzip_client::GZIP_ERR_TOO_LARGE => "uncompressed output exceeds limit",
        libzip_client::GZIP_ERR_INFLATE => "deflate stream failed",
        libzip_client::GZIP_ERR_BAD_CRC => "crc mismatch",
        libzip_client::GZIP_ERR_BAD_SIZE => "uncompressed size mismatch",
        libzip_client::GZIP_ERR_READ_FILE => "cannot read gzip file",
        libzip_client::GZIP_ERR_WRITE_FILE => "cannot write decompressed file",
        _ => "unknown gzip error",
    }
}

fn download_url(config: &LicoConfig, url: &str, dest: &str) -> bool {
    if download_url_with_libhttp(config, url, dest) {
        return true;
    }

    if !path_exists(&config.wget) {
        println!(
            "licof download: wget not found at {}; no fallback available",
            config.wget
        );
        return false;
    }

    println!("licof download: falling back to wget");
    download_url_with_wget(config, url, dest)
}

fn download_url_with_libhttp(config: &LicoConfig, url: &str, dest: &str) -> bool {
    if !libhttp_client::init() {
        println!("licof download: libhttp unavailable");
        return false;
    }

    let mut last_error = String::new();
    for attempt in 1..=config.download_attempts {
        let _ = fs::unlink(dest);
        reset_download_progress();
        let started = sys::uptime_ms();
        println!(
            "licof download: libhttp GET attempt {}/{} -> {}",
            attempt, config.download_attempts, dest
        );
        println!("licof download: url {}", url);
        if !libhttp_client::download_progress(url, dest, download_progress, 0) {
            let status = libhttp_client::last_status();
            last_error = alloc::format!(
                "libhttp failed with status {} error {}",
                status,
                libhttp_client::last_error()
            );
            if is_permanent_http_status(status) {
                break;
            }
            continue;
        }
        if file_size(dest) == 0 {
            last_error = alloc::format!("libhttp produced an empty file: {}", dest);
            continue;
        }
        let ms = sys::uptime_ms().wrapping_sub(started);
        println!(
            "licof download: received {} bytes in {} ms",
            file_size(dest),
            ms
        );
        return true;
    }
    if last_error.is_empty() {
        last_error.push_str("libhttp download failed");
    }
    println!(
        "licof download: failed after {} attempts: {}",
        config.download_attempts, last_error
    );
    false
}

fn download_url_with_wget(config: &LicoConfig, url: &str, dest: &str) -> bool {
    let mut last_error = String::new();
    let mut permanent_http_error = false;
    for attempt in 1..=config.download_attempts {
        let _ = fs::unlink(dest);
        let args = alloc::format!("wget -q -O {} {}", dest, url);
        let started = sys::uptime_ms();
        println!(
            "licof download: wget attempt {}/{} -> {}",
            attempt, config.download_attempts, dest
        );
        println!("licof download: url {}", url);
        let tid = process::spawn(&config.wget, &args);
        if tid == u32::MAX {
            last_error = String::from("failed to start wget");
            break;
        }

        let code = process::waitpid(tid);
        if code == process::STILL_RUNNING {
            last_error = String::from("wget is still running");
            continue;
        }
        if code == u32::MAX {
            last_error = String::from("wait failed for wget");
            continue;
        }
        if code != 0 {
            last_error = alloc::format!("wget exited with status {}", code);
            if code == 8 {
                permanent_http_error = true;
                break;
            }
            continue;
        }
        if !path_exists(dest) {
            last_error = alloc::format!("wget did not create file: {}", dest);
            continue;
        }
        if file_size(dest) == 0 {
            last_error = alloc::format!("wget produced an empty file: {}", dest);
            continue;
        }
        let ms = sys::uptime_ms().wrapping_sub(started);
        println!(
            "licof download: received {} bytes in {} ms",
            file_size(dest),
            ms
        );
        return true;
    }
    if last_error.is_empty() {
        last_error.push_str("wget download failed");
    }
    println!(
        "licof download: failed after {} attempts: {}",
        config.download_attempts, last_error
    );
    if permanent_http_error {
        return false;
    }
    println!("licof download: falling back to libhttp");
    download_url_with_libhttp(config, url, dest)
}

fn is_permanent_http_status(status: u32) -> bool {
    (400..500).contains(&status) && status != 408 && status != 429
}

extern "C" fn download_progress(received: u32, total: u32, _userdata: u64) {
    let should_print = unsafe {
        if received == total && total > 0 {
            DOWNLOAD_LAST_PRINT = received;
            true
        } else if received.saturating_sub(DOWNLOAD_LAST_PRINT) >= DOWNLOAD_PROGRESS_STEP {
            DOWNLOAD_LAST_PRINT = received;
            true
        } else {
            false
        }
    };
    if should_print {
        if total > 0 {
            println!("licof download: {} / {} bytes", received, total);
        } else {
            println!("licof download: {} bytes", received);
        }
    }
}

fn reset_download_progress() {
    unsafe {
        DOWNLOAD_LAST_PRINT = 0;
    }
}

fn packages_index_has_required_entries(config: &LicoConfig) -> bool {
    let packages_txt = config.package_index_txt();
    let mut missing =
        missing_required_package_names(&packages_txt, &config.index_required_packages);
    if missing.is_empty() {
        return true;
    }

    let compressed = match read_compressed_package_index(config) {
        Some(index) => index,
        None => return false,
    };
    for pkg in missing {
        if find_package_in_bytes(config, &compressed, &pkg)
            .map(|info| info.package == pkg)
            .unwrap_or(false)
        {
            continue;
        }
        println!("licof apt: package index missing '{}'", pkg);
        return false;
    }
    true
}

fn missing_required_package_names(path: &str, required: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for _ in required {
        found.push(false);
    }

    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return clone_package_names(required),
    };
    let mut chunk = [0u8; 4096];
    let mut para = Vec::with_capacity(1024);
    let mut newline_run = 0usize;

    loop {
        let n = match file.read(&mut chunk) {
            Ok(n) => n,
            Err(_) => return clone_package_names(required),
        };
        if n == 0 {
            break;
        }
        for &b in &chunk[..n] {
            para.push(b);
            if b == b'\n' {
                newline_run += 1;
                if newline_run >= 2 {
                    mark_required_package(&para, required, &mut found);
                    para.clear();
                    newline_run = 0;
                }
            } else if b != b'\r' {
                newline_run = 0;
            }
        }
    }
    if !para.is_empty() {
        mark_required_package(&para, required, &mut found);
    }

    let mut missing = Vec::new();
    for (idx, pkg) in required.iter().enumerate() {
        if !found.get(idx).copied().unwrap_or(false) {
            missing.push(pkg.clone());
        }
    }
    missing
}

fn clone_package_names(packages: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for pkg in packages {
        out.push(pkg.clone());
    }
    out
}

fn mark_required_package(para: &[u8], required: &[String], found: &mut [bool]) {
    let Some(package) = field_value(para, b"Package") else {
        return;
    };
    for (idx, wanted) in required.iter().enumerate() {
        if !found.get(idx).copied().unwrap_or(false) && &package == wanted {
            if let Some(slot) = found.get_mut(idx) {
                *slot = true;
            }
            return;
        }
    }
}

fn read_compressed_package_index(config: &LicoConfig) -> Option<Vec<u8>> {
    let packages_gz = config.package_index_gz();
    let gz = match fs::read_to_vec(&packages_gz) {
        Ok(data) => data,
        Err(_) => return None,
    };
    libzip_client::gunzip(&gz)
}

fn package_name_present(path: &str, package: &str) -> bool {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut pattern = Vec::new();
    pattern.extend_from_slice(b"\nPackage: ");
    pattern.extend_from_slice(package.as_bytes());
    pattern.push(b'\n');
    let mut at_start = Vec::new();
    at_start.extend_from_slice(b"Package: ");
    at_start.extend_from_slice(package.as_bytes());
    at_start.push(b'\n');

    let mut buf = [0u8; 4096];
    let mut window = Vec::new();
    let keep = pattern.len().max(at_start.len()).saturating_sub(1);
    let mut first = true;
    loop {
        let n = match file.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if n == 0 {
            return false;
        }
        window.extend_from_slice(&buf[..n]);
        if first && window.starts_with(&at_start) {
            return true;
        }
        first = false;
        if contains_bytes(&window, &pattern) {
            return true;
        }
        if window.len() > keep {
            let drop = window.len() - keep;
            let mut next = Vec::with_capacity(keep);
            next.extend_from_slice(&window[drop..]);
            window = next;
        }
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    let end = haystack.len() - needle.len();
    for idx in 0..=end {
        if &haystack[idx..idx + needle.len()] == needle {
            return true;
        }
    }
    false
}

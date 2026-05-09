//! licof - Linux Compatibility Framework command line tool.

#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{
    crypto,
    fs::{self, Read, Write},
    println, process,
};

mod config;
mod model;

use config::LicoConfig;
use model::{Elf64Diag, PackageInfo};

const EI_CLASS: usize = 4;
const ELFCLASS64: u8 = 2;
const EI_DATA: usize = 5;
const ELFDATA2LSB: u8 = 1;
const ET_DYN: u16 = 3;
const PT_INTERP: u32 = 3;
const ELF64_E_TYPE: usize = 16;
const ELF64_E_ENTRY: usize = 24;
const ELF64_E_PHOFF: usize = 32;
const ELF64_E_PHENTSIZE: usize = 54;
const ELF64_E_PHNUM: usize = 56;
const ELF64_PH_TYPE: usize = 0;
const ELF64_PH_OFFSET: usize = 8;
const ELF64_PH_FILESZ: usize = 32;
const ELF64_PH_SIZE: usize = 56;
const FS_TYPE_REGULAR: u32 = 0;
const FS_TYPE_DIRECTORY: u32 = 1;

struct PackageLink {
    index: u32,
    rel: String,
    dest: String,
    target: String,
    symlink: bool,
}

anyos_std::entry!(main);

fn main() {
    let config = LicoConfig::load();
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let argv: Vec<&str> = raw.split_ascii_whitespace().collect();

    match argv.first().copied() {
        None | Some("help") | Some("--help") | Some("-h") => usage(),
        Some("status") => status(&config),
        Some("init") => init(&config, true),
        Some("repair") => repair(&config),
        Some("run") => run(&config, &argv[1..]),
        Some("pkg") => pkg(&config, &argv[1..]),
        Some("apt") => apt(&config, &argv[1..]),
        Some(cmd) => {
            println!("licof: unknown command '{}'", cmd);
            usage();
        }
    }
}

fn usage() {
    println!("licof - Linux Compatibility Framework");
    println!();
    println!("Usage:");
    println!("  licof status");
    println!("  licof init");
    println!("  licof repair");
    println!("  licof run <linux-elf64> [args...]");
    println!("  licof pkg install <file.deb>");
    println!("  licof apt install <package> [package...]");
}

fn status(config: &LicoConfig) {
    println!("licof status");
    println!("  abi: linux-x86_64 tier-0");
    println!("  root: {}", config.root);
    println!("  linux-base: {}", config.rootfs);
    println!(
        "  apt-source: {}/dists/{}/{}/binary-{}/Packages.gz",
        config.apt_base, config.apt_dist, config.apt_component, config.apt_arch
    );
    println!("  config: confd system/services/licof");
    println!("  supported-package-data: data.tar.gz, data.tar.xz");
}

fn run(config: &LicoConfig, args: &[&str]) {
    if args.is_empty() {
        println!("licof run: missing Linux ELF64 path");
        return;
    }

    let path = resolve_run_path(&config.rootfs, args[0]);
    let child_args = join_args(&args[1..]);
    run_linux_process(config, "licof run", &path, &child_args);
}

fn init(config: &LicoConfig, configure_password: bool) {
    ensure_rootfs_layout(config);

    println!("licof: Linux base ready at {}", config.rootfs);
    println!("licof: bootstrapping minimal Debian userland with apt");
    let bootstrapped = bootstrap_rootfs(config, &config.rootfs);
    fs::sync();
    if bootstrapped && configure_password {
        configure_root_password(config, &config.rootfs);
    } else if configure_password {
        println!("licof init: bootstrap incomplete; skipping root password setup");
    }
}

fn repair(config: &LicoConfig) {
    ensure_rootfs_layout(config);
    repair_rootfs_runtime(&config.rootfs);
    fs::sync();
    println!("licof repair: Linux base repaired at {}", config.rootfs);
}

fn ensure_rootfs_layout(config: &LicoConfig) {
    ensure_dir(&config.root);
    ensure_dir(&config.cache);
    ensure_dir(&config.db);
    ensure_dir(&config.installed_db);

    let rootfs = &config.rootfs;
    ensure_dir(rootfs);
    ensure_dir(&alloc::format!("{}/bin", rootfs));
    ensure_dir(&alloc::format!("{}/lib", rootfs));
    ensure_dir(&alloc::format!("{}/lib64", rootfs));
    ensure_dir(&alloc::format!("{}/usr", rootfs));
    ensure_dir(&alloc::format!("{}/usr/bin", rootfs));
    ensure_dir(&alloc::format!("{}/etc", rootfs));
    ensure_dir(&alloc::format!("{}/etc/apt", rootfs));
    ensure_dir(&alloc::format!("{}/etc/apt/apt.conf.d", rootfs));
    let _ = fs::write_bytes(
        &alloc::format!("{}/etc/apt/sources.list", rootfs),
        alloc::format!(
            "deb {} {} {}\n",
            config.apt_base,
            config.apt_dist,
            config.apt_component
        )
        .as_bytes(),
    );
    let _ = fs::write_bytes(
        &alloc::format!("{}/etc/apt/apt.conf.d/99licof", rootfs),
        b"Acquire::Check-Valid-Until \"false\";\n",
    );
}

fn resolve_run_path(rootfs: &str, path: &str) -> String {
    if path.starts_with("/System/")
        || path.starts_with("/Applications/")
        || path.starts_with("/Users/")
    {
        String::from(path)
    } else if path.starts_with('/') {
        linux_path_in_rootfs(rootfs, path)
    } else {
        alloc::format!("{}/{}", rootfs, path)
    }
}

fn pkg(config: &LicoConfig, args: &[&str]) {
    match args.first().copied() {
        Some("install") => {
            if let Some(path) = args.get(1) {
                ensure_rootfs_layout(config);
                if install_deb(config, path, &config.rootfs, None) {
                    println!("licof pkg: installed '{}'", path);
                }
            } else {
                println!("licof pkg install: missing .deb path");
            }
        }
        _ => println!("licof pkg: expected install <file.deb>"),
    }
}

fn apt(config: &LicoConfig, args: &[&str]) {
    match args.first().copied() {
        Some("install") => {
            let packages = &args[1..];
            if packages.is_empty() {
                println!("licof apt install: missing package name");
                return;
            }
            ensure_rootfs_layout(config);
            for pkg in packages {
                install_package(config, pkg, &config.rootfs, 0);
            }
        }
        _ => println!("licof apt: expected install <package>"),
    }
}

fn bootstrap_rootfs(config: &LicoConfig, rootfs: &str) -> bool {
    let mut ok = true;
    for pkg in &config.bootstrap_seed {
        if !install_package(config, pkg, rootfs, 0) {
            println!("licof init: bootstrap seed '{}' failed", pkg);
            ok = false;
        }
    }
    if ok {
        println!("licof init: bootstrap complete");
    } else {
        println!("licof init: bootstrap incomplete; see failed seed package above");
    }
    ok
}

fn configure_root_password(config: &LicoConfig, rootfs: &str) {
    let passwd = linux_path_in_rootfs(rootfs, "/usr/bin/passwd");
    let passwd = if path_exists(&passwd) {
        passwd
    } else {
        let fallback = linux_path_in_rootfs(rootfs, "/bin/passwd");
        if path_exists(&fallback) {
            fallback
        } else {
            println!("licof init: passwd binary not found; root password not configured");
            println!("licof init: try later after 'licof apt install passwd'");
            return;
        }
    };

    if fs::isatty(0) != 1 || fs::isatty(1) != 1 {
        println!("licof init: root password setup needs an interactive terminal");
        println!("licof init: run later: licof run {} root", passwd);
        return;
    }

    println!("licof init: starting passwd for root");
    let code = run_linux_process(config, "licof passwd", &passwd, "root");
    if code == Some(0) {
        println!("licof init: root password configured");
    }
}

fn run_linux_process(config: &LicoConfig, label: &str, path: &str, args: &str) -> Option<u32> {
    diagnose_linux_binary(config, label, path);
    let tid = process::licof_spawn(path, args);
    if tid == u32::MAX {
        println!("{}: failed to start '{}'", label, path);
        println!("{}: check the diagnostics above and missing Linux syscalls in kernel/src/syscall/linux.rs", label);
        return None;
    }

    let code = process::waitpid(tid);
    if code == process::STILL_RUNNING {
        println!("{}: process {} is still running after waitpid", label, tid);
        None
    } else if code == u32::MAX {
        println!("{}: wait failed for process {}", label, tid);
        None
    } else {
        if code != 0 {
            println!("{}: '{}' exited with status {}", label, path, code);
        }
        Some(code)
    }
}

fn install_package(config: &LicoConfig, pkg: &str, rootfs: &str, depth: u8) -> bool {
    let mut pending = Vec::new();
    install_package_inner(config, pkg, rootfs, depth, &mut pending)
}

fn install_package_inner(
    config: &LicoConfig,
    pkg: &str,
    rootfs: &str,
    depth: u8,
    pending: &mut Vec<String>,
) -> bool {
    if depth > 32 {
        println!("licof apt: dependency recursion too deep at '{}'", pkg);
        return false;
    }
    if is_installed(config, pkg, rootfs) {
        return true;
    }
    if !ensure_apt_index(config) {
        return false;
    }

    let package_index_txt = config.package_index_txt();
    let Some(info) = find_package_in_index(config, pkg) else {
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
        println!(
            "licof apt: dependency '{}' is already scheduled; continuing",
            pkg
        );
        return true;
    }

    pending.push(info.package.clone());

    for dep_group in parse_depends(&info.pre_depends) {
        if !install_dependency_group(config, &dep_group, rootfs, depth + 1, pending) {
            println!("licof apt: dependency for '{}' not satisfied", info.package);
            pending.pop();
            return false;
        }
    }
    for dep_group in parse_depends(&info.depends) {
        if !install_dependency_group(config, &dep_group, rootfs, depth + 1, pending) {
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
        println!("licof apt: downloading {} {}", info.package, info.version);
        if !download_url(config, &url, &deb_path) {
            println!("licof apt: download failed: {}", url);
            pending.pop();
            return false;
        }
        if !verify_package_file(&info, &deb_path) {
            let _ = fs::unlink(&deb_path);
            pending.pop();
            return false;
        }
    }

    let ok = install_deb(config, &deb_path, rootfs, Some(&info));
    pending.pop();
    ok
}

fn install_dependency_group(
    config: &LicoConfig,
    alternatives: &[String],
    rootfs: &str,
    depth: u8,
    pending: &mut Vec<String>,
) -> bool {
    for dep in alternatives {
        if install_package_inner(config, dep, rootfs, depth, pending) {
            return true;
        }
    }
    if !alternatives.is_empty() {
        println!(
            "licof apt: no dependency alternative worked: {}",
            alternatives[0]
        );
    }
    alternatives.is_empty()
}

fn dependency_pending(pkg: &str, pending: &[String]) -> bool {
    pending.iter().any(|p| p == pkg)
}

fn ensure_apt_index(config: &LicoConfig) -> bool {
    if !libzip_client::init() {
        println!("licof apt: libzip unavailable");
        return false;
    }
    ensure_dir(&config.cache);
    let packages_gz = config.package_index_gz();
    let packages_txt = config.package_index_txt();
    if file_size(&packages_txt) > 0 {
        if looks_like_plain_packages_index(&packages_txt)
            && packages_index_has_required_entries(config)
        {
            return true;
        }
        println!("licof apt: cached package index is invalid; refreshing");
        let _ = fs::unlink(&packages_txt);
    }
    let url = config.package_index_url();

    for attempt in 1..=config.download_attempts {
        let _ = fs::unlink(&packages_gz);
        let _ = fs::unlink(&packages_txt);
        println!(
            "licof apt: fetching package index (attempt {}/{})",
            attempt, config.download_attempts
        );
        if !download_url(config, &url, &packages_gz) {
            println!("licof apt: failed to download {}", url);
            continue;
        }
        let downloaded = file_size(&packages_gz);
        if downloaded == 0 {
            println!("licof apt: downloaded package index is empty");
            continue;
        }
        if looks_like_plain_packages_index(&packages_gz) {
            println!(
                "licof apt: package index arrived uncompressed ({} bytes)",
                downloaded
            );
            if !copy_file(&packages_gz, &packages_txt) {
                println!("licof apt: cannot store uncompressed package index");
                continue;
            }
        } else {
            if !looks_like_gzip(&packages_gz) {
                print_index_download_diagnostic(&packages_gz, downloaded);
                continue;
            }
            let gzip_status =
                libzip_client::gzip_decompress_file_status(&packages_gz, &packages_txt);
            if gzip_status != libzip_client::GZIP_STATUS_OK {
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
        if unpacked == 0 {
            println!("licof apt: decompressed package index is empty");
            continue;
        }
        if !looks_like_plain_packages_index(&packages_txt) {
            println!("licof apt: decompressed package index is not a Packages file");
            continue;
        }
        if !packages_index_has_required_entries(config) {
            println!("licof apt: decompressed package index is missing bootstrap entries");
            continue;
        }
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

fn diagnose_linux_binary(config: &LicoConfig, label: &str, path: &str) {
    let read_path =
        resolve_rootfs_symlink_path(&config.rootfs, path).unwrap_or_else(|| String::from(path));
    let data = match fs::read_to_vec(&read_path) {
        Ok(data) => data,
        Err(_) => {
            println!("{}: cannot read '{}'", label, path);
            return;
        }
    };
    let Some(info) = inspect_elf64(&data) else {
        println!(
            "{}: '{}' is not a supported little-endian ELF64 binary",
            label, path
        );
        return;
    };
    if info.is_dyn {
        println!(
            "{}: ELF64 ET_DYN binary; licof will use a compatibility load bias",
            label
        );
    }
    if info.entry == 0 {
        println!(
            "{}: ELF64 entry point is 0; loader may reject this binary",
            label
        );
    }
    if let Some(interp) = info.interp_path {
        let rootfs = rootfs_for_path(config, path);
        let resolved = linux_path_in_rootfs(&rootfs, &interp);
        let final_path =
            resolve_rootfs_symlink_path(&rootfs, &resolved).unwrap_or_else(|| resolved.clone());
        if path_exists_no_follow(&resolved) || path_exists(&final_path) {
            println!("{}: PT_INTERP {} -> {}", label, interp, final_path);
        } else {
            println!("{}: missing PT_INTERP {}", label, interp);
            println!("{}: expected interpreter at {}", label, resolved);
        }
    } else {
        println!("{}: static/no-PT_INTERP Linux ELF64", label);
    }
    if fs::isatty(0) != 1 {
        println!(
            "{}: stdin is not a tty; interactive Linux tools may fail",
            label
        );
    }
    if fs::isatty(1) != 1 {
        println!(
            "{}: stdout is not a tty; terminal-oriented Linux tools may fail",
            label
        );
    }
}

fn inspect_elf64(data: &[u8]) -> Option<Elf64Diag> {
    if data.len() < 64 || &data[..4] != b"\x7FELF" {
        return None;
    }
    if data[EI_CLASS] != ELFCLASS64 || data[EI_DATA] != ELFDATA2LSB {
        return None;
    }
    let phoff = read_u64(data, ELF64_E_PHOFF)? as usize;
    let phentsize = read_u16(data, ELF64_E_PHENTSIZE)? as usize;
    let phnum = read_u16(data, ELF64_E_PHNUM)? as usize;
    if phentsize < ELF64_PH_SIZE {
        return None;
    }

    let mut interp_path = None;
    for idx in 0..phnum {
        let off = phoff.checked_add(idx.checked_mul(phentsize)?)?;
        if off.checked_add(ELF64_PH_SIZE)? > data.len() {
            return None;
        }
        if read_u32(data, off + ELF64_PH_TYPE)? != PT_INTERP {
            continue;
        }
        let interp_off = read_u64(data, off + ELF64_PH_OFFSET)? as usize;
        let interp_size = read_u64(data, off + ELF64_PH_FILESZ)? as usize;
        if interp_size == 0 || interp_size > 512 {
            continue;
        }
        let end = interp_off.checked_add(interp_size)?;
        if end > data.len() {
            continue;
        }
        let raw = &data[interp_off..end];
        let nul = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
        if let Ok(path) = core::str::from_utf8(&raw[..nul]) {
            if !path.is_empty() {
                interp_path = Some(String::from(path));
            }
        }
    }

    Some(Elf64Diag {
        entry: read_u64(data, ELF64_E_ENTRY)?,
        is_dyn: read_u16(data, ELF64_E_TYPE)? == ET_DYN,
        interp_path,
    })
}

fn read_u16(data: &[u8], off: usize) -> Option<u16> {
    if off + 2 > data.len() {
        return None;
    }
    Some(u16::from_le_bytes([data[off], data[off + 1]]))
}

fn read_u32(data: &[u8], off: usize) -> Option<u32> {
    if off + 4 > data.len() {
        return None;
    }
    Some(u32::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
    ]))
}

fn read_u64(data: &[u8], off: usize) -> Option<u64> {
    if off + 8 > data.len() {
        return None;
    }
    Some(u64::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
        data[off + 4],
        data[off + 5],
        data[off + 6],
        data[off + 7],
    ]))
}

fn install_deb(config: &LicoConfig, path: &str, rootfs: &str, info: Option<&PackageInfo>) -> bool {
    if !libzip_client::init() {
        println!("licof pkg: libzip unavailable");
        return false;
    }
    let data = match fs::read_to_vec(path) {
        Ok(d) => d,
        Err(_) => {
            println!("licof pkg: cannot read '{}'", path);
            return false;
        }
    };
    let Some(reader) = data_tar_reader(&data, path) else {
        println!("licof pkg: cannot open package data archive: {}", path);
        return false;
    };
    if reader.entry_count() == 0 {
        println!("licof pkg: staged data archive has no entries: {}", path);
        return false;
    }

    let mut files = 0u32;
    let mut complete = true;
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
        } else if typeflag == b'2' {
            links.push(PackageLink {
                index: i,
                rel,
                dest,
                target: reader.entry_link_name(i),
                symlink: true,
            });
        } else if typeflag == b'1' {
            links.push(PackageLink {
                index: i,
                rel,
                dest,
                target: reader.entry_link_name(i),
                symlink: false,
            });
        } else {
            ensure_parent_dirs(&dest);
            println!("licof pkg: extracting {}", rel);
            if reader.extract_to_file(i, &dest) {
                apply_tar_metadata(&reader, i, &dest);
                files += 1;
            } else {
                println!("licof pkg: failed to extract {}", rel);
                complete = false;
            }
        }
    }
    for link in &links {
        if install_package_link(rootfs, &reader, link) {
            files += 1;
        } else if link.symlink {
            println!(
                "licof pkg: failed to create symlink {} -> {}",
                link.rel, link.target
            );
            complete = false;
        } else {
            println!(
                "licof pkg: failed to materialize hardlink {} -> {}",
                link.rel, link.target
            );
            complete = false;
        }
    }
    if files == 0 {
        println!("licof pkg: extracted no files from '{}'", path);
        return false;
    }
    if !complete {
        println!("licof pkg: package '{}' extracted incompletely", path);
        return false;
    }

    if let Some(info) = info {
        mark_installed(config, info, rootfs, files);
        println!(
            "licof apt: installed {} {} ({} files)",
            info.package, info.version, files
        );
    } else {
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
) -> bool {
    if link.target.is_empty() {
        return false;
    }
    ensure_parent_dirs(&link.dest);
    if link.symlink {
        println!("licof pkg: linking {} -> {}", link.rel, link.target);
        let _ = fs::unlink(&link.dest);
        if fs::symlink(&link.target, &link.dest) == 0 {
            return true;
        }
    } else {
        println!("licof pkg: hardlink {} -> {}", link.rel, link.target);
    }
    if materialize_link_target(rootfs, &link.dest, &link.target, link.symlink) {
        apply_tar_metadata(reader, link.index, &link.dest);
        return true;
    }
    false
}

fn materialize_link_target(rootfs: &str, dest: &str, target: &str, symlink: bool) -> bool {
    let Some(src) = resolve_package_link_target(rootfs, dest, target, symlink) else {
        return false;
    };
    if !path_under_rootfs(rootfs, &src) {
        return false;
    }
    let mut stat_buf = [0u32; 7];
    if fs::stat(&src, &mut stat_buf) != 0 {
        return false;
    }
    let _ = fs::unlink(dest);
    if stat_buf[0] == FS_TYPE_DIRECTORY {
        ensure_dir_recursive(dest);
        true
    } else {
        copy_file(&src, dest)
    }
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

fn mark_installed(config: &LicoConfig, info: &PackageInfo, rootfs: &str, files: u32) {
    let db_dir = installed_db_dir(config, rootfs);
    ensure_dir(&db_dir);
    let path = installed_package_path(config, &info.package, rootfs);
    let body = alloc::format!(
        "Package: {}\nVersion: {}\nRootFS: {}\nFilename: {}\nFiles: {}\n",
        info.package,
        info.version,
        rootfs,
        info.filename,
        files
    );
    let _ = fs::write_bytes(&path, body.as_bytes());
}

fn is_installed(config: &LicoConfig, pkg: &str, rootfs: &str) -> bool {
    let path = installed_package_path(config, pkg, rootfs);
    fs::stat(&path, &mut [0u32; 7]) == 0
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

fn linux_path_in_rootfs(rootfs: &str, linux_path: &str) -> String {
    let rel = linux_path.trim_start_matches('/');
    alloc::format!("{}/{}", rootfs, rel)
}

fn rootfs_for_path(config: &LicoConfig, path: &str) -> String {
    let rootfs = config.rootfs.trim_end_matches('/');
    if path == rootfs
        || (path.starts_with(rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/'))
    {
        return config.rootfs.clone();
    }
    config.rootfs.clone()
}

fn path_exists(path: &str) -> bool {
    fs::stat(path, &mut [0u32; 7]) == 0
}

fn regular_file_exists(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    fs::stat(path, &mut stat_buf) == 0 && stat_buf[0] == FS_TYPE_REGULAR
}

fn path_exists_no_follow(path: &str) -> bool {
    fs::lstat(path, &mut [0u32; 7]) == 0
}

fn path_is_symlink(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    fs::lstat(path, &mut stat_buf) == 0 && (stat_buf[2] & 1) != 0
}

fn symlink_points_to(path: &str, target: &str) -> bool {
    readlink_string(path).as_deref() == Some(target)
}

fn readlink_string(path: &str) -> Option<String> {
    let mut buf = [0u8; 512];
    let len = fs::readlink(path, &mut buf);
    if len == u32::MAX {
        return None;
    }
    core::str::from_utf8(&buf[..len as usize])
        .ok()
        .map(String::from)
}

fn resolve_rootfs_symlink_path(rootfs: &str, path: &str) -> Option<String> {
    if !path_under_rootfs(rootfs, path) {
        return Some(normalize_abs_path(path));
    }
    resolve_rootfs_symlink_path_inner(rootfs, path, 0)
}

fn resolve_rootfs_symlink_path_inner(rootfs: &str, path: &str, depth: u32) -> Option<String> {
    if depth > 16 {
        return None;
    }
    let normalized = normalize_abs_path(path);
    let rel = rootfs_relative_path(rootfs, &normalized);
    let components: Vec<&str> = rel
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    let mut current = String::from(rootfs);
    if components.is_empty() {
        return Some(current);
    }

    for (idx, component) in components.iter().enumerate() {
        let candidate = join_path_component(&current, component);
        if path_is_symlink(&candidate) {
            let target = readlink_string(&candidate)?;
            let parent = parent_path(&candidate);
            let next = resolve_link_target(rootfs, &parent, &target, &components[idx + 1..]);
            return resolve_rootfs_symlink_path_inner(rootfs, &next, depth + 1);
        }
        current = candidate;
    }
    Some(current)
}

fn rootfs_relative_path<'a>(rootfs: &str, path: &'a str) -> &'a str {
    if path == rootfs {
        ""
    } else if path.starts_with(rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/') {
        &path[rootfs.len() + 1..]
    } else {
        path.trim_start_matches('/')
    }
}

fn join_path_component(base: &str, component: &str) -> String {
    if base == "/" {
        alloc::format!("/{}", component)
    } else if base.ends_with('/') {
        alloc::format!("{}{}", base, component)
    } else {
        alloc::format!("{}/{}", base, component)
    }
}

fn parent_path(path: &str) -> String {
    let normalized = normalize_abs_path(path);
    match normalized.rfind('/') {
        Some(0) | None => String::from("/"),
        Some(idx) => String::from(&normalized[..idx]),
    }
}

fn resolve_link_target(rootfs: &str, parent: &str, target: &str, remaining: &[&str]) -> String {
    let mut path = if target.starts_with('/') {
        if target == "/" {
            String::from(rootfs)
        } else {
            alloc::format!("{}{}", rootfs, target)
        }
    } else {
        join_path_component(parent, target)
    };
    for component in remaining {
        path = join_path_component(&path, component);
    }
    normalize_abs_path(&path)
}

fn repair_rootfs_runtime(rootfs: &str) {
    ensure_dir_recursive(&linux_path_in_rootfs(rootfs, "/lib64"));
    repair_dynamic_loader(rootfs);
    repair_common_library_links(rootfs, "/lib/x86_64-linux-gnu");
    repair_common_library_links(rootfs, "/usr/lib/x86_64-linux-gnu");
}

fn repair_dynamic_loader(rootfs: &str) {
    let interp = linux_path_in_rootfs(rootfs, "/lib64/ld-linux-x86-64.so.2");
    if is_elf_file(&interp) || path_is_symlink(&interp) {
        return;
    }
    let candidates = [
        "/lib/x86_64-linux-gnu/ld-2.13.so",
        "/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
    ];
    for candidate in &candidates {
        let src = linux_path_in_rootfs(rootfs, candidate);
        if is_elf_file(&src) {
            repair_symlink(&interp, candidate, "dynamic loader");
            return;
        }
    }
}

fn repair_common_library_links(rootfs: &str, linux_dir: &str) {
    let dir = linux_path_in_rootfs(rootfs, linux_dir);
    let entries = read_dir_entries(&dir);
    for name in &entries {
        if name.starts_with("ld-") && name.ends_with(".so") {
            let src = alloc::format!("{}/{}", dir, name);
            repair_known_library_symlink(
                &src,
                &alloc::format!("{}/ld-linux-x86-64.so.2", dir),
                name,
            );
        } else if name.starts_with("libc-") && name.ends_with(".so") {
            let src = alloc::format!("{}/{}", dir, name);
            repair_known_library_symlink(&src, &alloc::format!("{}/libc.so.6", dir), name);
        } else if let Some(pos) = name.find(".so.") {
            let src = alloc::format!("{}/{}", dir, name);
            let version = &name[pos + 4..];
            if let Some(first_dot) = version.find('.') {
                let soname = alloc::format!("{}{}", &name[..pos + 4], &version[..first_dot]);
                repair_known_library_symlink(&src, &alloc::format!("{}/{}", dir, soname), name);
                if let Some(second_dot) = version[first_dot + 1..].find('.') {
                    let end = first_dot + 1 + second_dot;
                    let soname = alloc::format!("{}{}", &name[..pos + 4], &version[..end]);
                    repair_known_library_symlink(&src, &alloc::format!("{}/{}", dir, soname), name);
                }
            }
        }
    }
}

fn repair_known_library_symlink(src: &str, dest: &str, target: &str) {
    if !is_elf_file(src) || src == dest || path_exists_no_follow(dest) {
        return;
    }
    repair_symlink(dest, target, "library alias");
}

fn repair_symlink(dest: &str, target: &str, label: &str) {
    if symlink_points_to(dest, target) {
        return;
    }
    let _ = fs::unlink(dest);
    if fs::symlink(target, dest) == 0 {
        println!("licof repair: repaired {} {} -> {}", label, dest, target);
    }
}

fn read_dir_entries(path: &str) -> Vec<String> {
    let mut buf = [0u8; 8192];
    let count = fs::readdir(path, &mut buf);
    if count == u32::MAX {
        return Vec::new();
    }
    let mut out = Vec::new();
    let max_entries = (buf.len() / 64).min(count as usize);
    for idx in 0..max_entries {
        let off = idx * 64;
        let name_len = buf[off + 1] as usize;
        if name_len == 0 || name_len > 55 {
            continue;
        }
        if let Ok(name) = core::str::from_utf8(&buf[off + 8..off + 8 + name_len]) {
            out.push(String::from(name));
        }
    }
    out
}

fn is_elf_file(path: &str) -> bool {
    if !regular_file_exists(path) {
        return false;
    }
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut magic = [0u8; 4];
    match file.read(&mut magic) {
        Ok(4) => magic == [0x7f, b'E', b'L', b'F'],
        _ => false,
    }
}

fn file_size(path: &str) -> u32 {
    let mut stat_buf = [0u32; 7];
    if fs::stat(path, &mut stat_buf) == 0 {
        stat_buf[1]
    } else {
        0
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
    if libhttp_client::init() {
        let mut last_error = String::new();
        for _attempt in 1..=config.download_attempts {
            let _ = fs::unlink(dest);
            if !libhttp_client::download(url, dest) {
                last_error = alloc::format!(
                    "libhttp failed with status {} error {}",
                    libhttp_client::last_status(),
                    libhttp_client::last_error()
                );
                continue;
            }
            if file_size(dest) == 0 {
                last_error = alloc::format!("libhttp produced an empty file: {}", dest);
                continue;
            }
            return true;
        }
        if last_error.is_empty() {
            last_error.push_str("libhttp download failed");
        }
        println!(
            "licof download: failed after {} attempts: {}",
            config.download_attempts, last_error
        );
        return false;
    }

    println!("licof download: libhttp unavailable; falling back to wget");
    if !path_exists(&config.wget) {
        println!("licof download: wget not found at {}", config.wget);
        return false;
    }
    let mut last_error = String::new();
    for _attempt in 1..=config.download_attempts {
        let _ = fs::unlink(dest);
        let args = alloc::format!("-q -O {} {}", dest, url);
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
            continue;
        }
        if file_size(dest) == 0 {
            last_error = alloc::format!("wget produced an empty file: {}", dest);
            continue;
        }
        return true;
    }
    if last_error.is_empty() {
        last_error.push_str("wget download failed");
    }
    println!(
        "licof download: failed after {} attempts: {}",
        config.download_attempts, last_error
    );
    false
}

fn packages_index_has_required_entries(config: &LicoConfig) -> bool {
    let packages_txt = config.package_index_txt();
    let mut missing = Vec::new();
    for pkg in &config.index_required_packages {
        if !package_name_present(&packages_txt, pkg) {
            missing.push(pkg.clone());
        }
    }
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

fn read_compressed_package_index(config: &LicoConfig) -> Option<Vec<u8>> {
    let packages_gz = config.package_index_gz();
    let gz = match fs::read_to_vec(&packages_gz) {
        Ok(data) => data,
        Err(_) => return None,
    };
    libzip_client::gunzip(&gz)
}

fn copy_file(src: &str, dst: &str) -> bool {
    let mut input = match fs::File::open(src) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut output = match fs::File::create(dst) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut buf = [0u8; 4096];
    loop {
        let n = match input.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if n == 0 {
            let _ = fs::fsync(output.fd() as i32);
            return true;
        }
        if output.write_all(&buf[..n]).is_err() {
            return false;
        }
    }
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

fn ensure_dir(path: &str) {
    let mut stat_buf = [0u32; 7];
    if fs::stat(path, &mut stat_buf) == 0 {
        return;
    }
    let _ = fs::mkdir(path);
}

fn ensure_dir_recursive(path: &str) {
    let bytes = path.as_bytes();
    let mut pos = 1usize;
    while pos <= bytes.len() {
        if pos == bytes.len() || bytes[pos] == b'/' {
            let dir = &path[..pos];
            ensure_dir(dir);
        }
        pos += 1;
    }
}

fn ensure_parent_dirs(path: &str) {
    if let Some(pos) = path.rfind('/') {
        if pos > 0 {
            ensure_dir_recursive(&path[..pos]);
        }
    }
}

fn normalize_abs_path(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let mut out = String::from("/");
    for (idx, part) in parts.iter().enumerate() {
        if idx > 0 {
            out.push('/');
        }
        out.push_str(part);
    }
    out
}

fn path_under_rootfs(rootfs: &str, path: &str) -> bool {
    path == rootfs || (path.starts_with(rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/'))
}

fn join_args(args: &[&str]) -> String {
    let mut out = String::new();
    for (idx, arg) in args.iter().enumerate() {
        if idx != 0 {
            out.push(' ');
        }
        out.push_str(arg);
    }
    out
}

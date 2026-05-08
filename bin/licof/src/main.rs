//! licof - Linux Compatibility Framework command line tool.

#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{fs, println, process};

const ROOT: &str = "/System/var/licof";
const ROOTFS_DEFAULT: &str = "/System/var/licof/rootfs/default";
const CACHE: &str = "/System/var/licof/cache";
const DB: &str = "/System/var/licof/db";
const INSTALLED_DB: &str = "/System/var/licof/db/installed";
const APT_BASE: &str = "http://archive.debian.org/debian";
const APT_DIST: &str = "wheezy";
const APT_ARCH: &str = "amd64";
const PACKAGES_GZ: &str = "/System/var/licof/cache/debian-wheezy-amd64-Packages.gz";
const PACKAGES_TXT: &str = "/System/var/licof/cache/debian-wheezy-amd64-Packages";
const BOOTSTRAP_SEED: &[&str] = &[
    "base-files",
    "base-passwd",
    "libc6",
    "libgcc1",
    "libstdc++6",
    "zlib1g",
    "libapt-pkg4.12",
    "apt",
    "passwd",
];

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

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let argv: Vec<&str> = raw.split_ascii_whitespace().collect();

    match argv.first().copied() {
        None | Some("help") | Some("--help") | Some("-h") => usage(),
        Some("status") => status(),
        Some("run") => run(&argv[1..]),
        Some("rootfs") => rootfs(&argv[1..]),
        Some("pkg") => pkg(&argv[1..]),
        Some("apt") => apt(&argv[1..]),
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
    println!("  licof run <linux-elf64> [args...]");
    println!("  licof rootfs create [name]");
    println!("  licof rootfs list");
    println!("  licof pkg install <file.deb>");
    println!("  licof apt install <package> [package...]");
}

fn status() {
    println!("licof status");
    println!("  abi: linux-x86_64 tier-0");
    println!("  root: {}", ROOT);
    println!("  default-rootfs: {}", ROOTFS_DEFAULT);
    println!(
        "  apt-source: {}/dists/{}/main/binary-{}/Packages.gz",
        APT_BASE, APT_DIST, APT_ARCH
    );
    println!("  supported-package-data: data.tar.gz, data.tar.xz");
}

fn run(args: &[&str]) {
    if args.is_empty() {
        println!("licof run: missing Linux ELF64 path");
        return;
    }

    let path = args[0];
    let child_args = join_args(&args[1..]);
    run_linux_process("licof run", path, &child_args);
}

fn rootfs(args: &[&str]) {
    match args.first().copied() {
        Some("create") => {
            let name = args.get(1).copied().unwrap_or("default");
            create_rootfs(name, true);
        }
        Some("list") => {
            println!("default  {}", ROOTFS_DEFAULT);
        }
        _ => {
            println!("licof rootfs: expected create or list");
        }
    }
}

fn create_rootfs(name: &str, configure_password: bool) {
    ensure_dir(ROOT);
    ensure_dir("/System/var/licof/rootfs");
    ensure_dir(CACHE);
    ensure_dir(DB);
    ensure_dir(INSTALLED_DB);

    let rootfs = if name == "default" {
        String::from(ROOTFS_DEFAULT)
    } else {
        alloc::format!("/System/var/licof/rootfs/{}", name)
    };
    ensure_dir(&rootfs);
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
        alloc::format!("deb {} {} main\n", APT_BASE, APT_DIST).as_bytes(),
    );
    let _ = fs::write_bytes(
        &alloc::format!("{}/etc/apt/apt.conf.d/99licof", rootfs),
        b"Acquire::Check-Valid-Until \"false\";\n",
    );

    println!("licof: rootfs '{}' ready at {}", name, rootfs);
    println!("licof: bootstrapping minimal Debian userland with apt");
    if bootstrap_rootfs(&rootfs) && configure_password {
        configure_root_password(&rootfs);
    } else if configure_password {
        println!("licof rootfs: bootstrap incomplete; skipping root password setup");
    }
}

fn pkg(args: &[&str]) {
    match args.first().copied() {
        Some("install") => {
            if let Some(path) = args.get(1) {
                create_rootfs("default", false);
                if install_deb(path, ROOTFS_DEFAULT, None) {
                    println!("licof pkg: installed '{}'", path);
                }
            } else {
                println!("licof pkg install: missing .deb path");
            }
        }
        _ => println!("licof pkg: expected install <file.deb>"),
    }
}

fn apt(args: &[&str]) {
    match args.first().copied() {
        Some("install") => {
            if args.len() < 2 {
                println!("licof apt install: missing package name");
                return;
            }
            create_rootfs("default", false);
            for pkg in &args[1..] {
                install_package(pkg, ROOTFS_DEFAULT, 0);
            }
        }
        _ => println!("licof apt: expected install <package>"),
    }
}

fn bootstrap_rootfs(rootfs: &str) -> bool {
    let mut ok = true;
    for pkg in BOOTSTRAP_SEED {
        if !install_package(pkg, rootfs, 0) {
            ok = false;
        }
    }
    ok
}

fn configure_root_password(rootfs: &str) {
    let passwd = linux_path_in_rootfs(rootfs, "/usr/bin/passwd");
    let passwd = if path_exists(&passwd) {
        passwd
    } else {
        let fallback = linux_path_in_rootfs(rootfs, "/bin/passwd");
        if path_exists(&fallback) {
            fallback
        } else {
            println!("licof rootfs: passwd binary not found; root password not configured");
            println!("licof rootfs: try later after 'licof apt install passwd'");
            return;
        }
    };

    if fs::isatty(0) != 1 || fs::isatty(1) != 1 {
        println!("licof rootfs: root password setup needs an interactive terminal");
        println!("licof rootfs: run later: licof run {} root", passwd);
        return;
    }

    println!("licof rootfs: starting passwd for root");
    let code = run_linux_process("licof passwd", &passwd, "root");
    if code == Some(0) {
        println!("licof rootfs: root password configured");
    }
}

fn run_linux_process(label: &str, path: &str, args: &str) -> Option<u32> {
    diagnose_linux_binary(label, path);
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

#[derive(Clone)]
struct PackageInfo {
    package: String,
    version: String,
    filename: String,
    depends: String,
    pre_depends: String,
}

fn install_package(pkg: &str, rootfs: &str, depth: u8) -> bool {
    if depth > 8 {
        println!("licof apt: dependency recursion too deep at '{}'", pkg);
        return false;
    }
    if is_installed(pkg) {
        return true;
    }
    if !ensure_apt_index() {
        return false;
    }

    let index = match fs::read_to_string(PACKAGES_TXT) {
        Ok(s) => s,
        Err(_) => {
            println!("licof apt: cannot read package index");
            return false;
        }
    };
    let Some(info) = find_package(&index, pkg) else {
        println!("licof apt: package '{}' not found", pkg);
        return false;
    };

    for dep_group in parse_depends(&info.pre_depends) {
        if !install_dependency_group(&dep_group, rootfs, depth + 1) {
            println!("licof apt: dependency for '{}' not satisfied", info.package);
            return false;
        }
    }
    for dep_group in parse_depends(&info.depends) {
        if !install_dependency_group(&dep_group, rootfs, depth + 1) {
            println!("licof apt: dependency for '{}' not satisfied", info.package);
            return false;
        }
    }

    let deb_path = alloc::format!(
        "{}/{}",
        CACHE,
        cache_deb_name(&info.package, &info.version, &info.filename)
    );
    if fs::stat(&deb_path, &mut [0u32; 7]) != 0 {
        let url = alloc::format!("{}/{}", APT_BASE, info.filename);
        println!("licof apt: downloading {} {}", info.package, info.version);
        if !libhttp_client::download(&url, &deb_path) {
            println!("licof apt: download failed: {}", url);
            return false;
        }
    }

    install_deb(&deb_path, rootfs, Some(&info))
}

fn install_dependency_group(alternatives: &[String], rootfs: &str, depth: u8) -> bool {
    for dep in alternatives {
        if install_package(dep, rootfs, depth) {
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

fn ensure_apt_index() -> bool {
    if !libhttp_client::init() {
        println!("licof apt: libhttp unavailable");
        return false;
    }
    if !libzip_client::init() {
        println!("licof apt: libzip unavailable");
        return false;
    }
    ensure_dir(CACHE);
    if fs::stat(PACKAGES_TXT, &mut [0u32; 7]) == 0 {
        return true;
    }
    let url = alloc::format!(
        "{}/dists/{}/main/binary-{}/Packages.gz",
        APT_BASE,
        APT_DIST,
        APT_ARCH
    );
    println!("licof apt: fetching package index");
    if !libhttp_client::download(&url, PACKAGES_GZ) {
        println!("licof apt: failed to download {}", url);
        return false;
    }
    if !libzip_client::gzip_decompress_file(PACKAGES_GZ, PACKAGES_TXT) {
        println!("licof apt: failed to decompress package index");
        return false;
    }
    true
}

fn find_package(index: &str, wanted: &str) -> Option<PackageInfo> {
    for para in index.split("\n\n") {
        let Some(package) = field(para, "Package") else {
            continue;
        };
        if package != wanted {
            continue;
        }
        let arch = field(para, "Architecture").unwrap_or("");
        if arch != APT_ARCH && arch != "all" {
            continue;
        }
        return Some(PackageInfo {
            package: String::from(wanted),
            version: String::from(field(para, "Version").unwrap_or("unknown")),
            filename: String::from(field(para, "Filename")?),
            depends: String::from(field(para, "Depends").unwrap_or("")),
            pre_depends: String::from(field(para, "Pre-Depends").unwrap_or("")),
        });
    }
    None
}

fn field<'a>(para: &'a str, key: &str) -> Option<&'a str> {
    let prefix = alloc::format!("{}: ", key);
    for line in para.lines() {
        if line.starts_with(&prefix) {
            return Some(line[prefix.len()..].trim());
        }
    }
    None
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
    let name = raw.split_ascii_whitespace().next().unwrap_or("").trim();
    if name.is_empty() {
        None
    } else {
        Some(String::from(name))
    }
}

fn diagnose_linux_binary(label: &str, path: &str) {
    let data = match fs::read_to_vec(path) {
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
        let resolved = linux_path_in_rootfs(ROOTFS_DEFAULT, &interp);
        if path_exists(&resolved) {
            println!("{}: PT_INTERP {} -> {}", label, interp, resolved);
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

struct Elf64Diag {
    entry: u64,
    is_dyn: bool,
    interp_path: Option<String>,
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

fn install_deb(path: &str, rootfs: &str, info: Option<&PackageInfo>) -> bool {
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
    let tar_path = alloc::format!("{}/licof-data.tar", CACHE);
    if let Some(tar_data) = ar_entry(&data, "data.tar.gz") {
        if fs::write_bytes(&tar_path, &tar_data).is_err() {
            println!("licof pkg: cannot stage data archive");
            return false;
        }
    } else if let Some(xz_data) = ar_entry(&data, "data.tar.xz") {
        let xz_path = alloc::format!("{}/licof-data.tar.xz", CACHE);
        if fs::write_bytes(&xz_path, &xz_data).is_err() {
            println!("licof pkg: cannot stage XZ data archive");
            return false;
        }
        if !libzip_client::xz_decompress_file(&xz_path, &tar_path) {
            println!("licof pkg: cannot decompress data.tar.xz from '{}'", path);
            return false;
        }
    } else {
        println!("licof pkg: '{}' has no supported data.tar.* member", path);
        return false;
    }
    let Some(reader) = libzip_client::TarReader::open(&tar_path) else {
        println!("licof pkg: cannot open staged data archive");
        return false;
    };

    let mut files = 0u32;
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
            ensure_parent_dirs(&dest);
            let link_name = reader.entry_link_name(i);
            if link_name.is_empty() || fs::symlink(&link_name, &dest) != 0 {
                println!(
                    "licof pkg: failed to create symlink {} -> {}",
                    rel, link_name
                );
            } else {
                files += 1;
            }
        } else if typeflag == b'1' {
            println!(
                "licof pkg: skipping hardlink {} -> {}",
                rel,
                reader.entry_link_name(i)
            );
        } else {
            ensure_parent_dirs(&dest);
            if reader.extract_to_file(i, &dest) {
                apply_tar_metadata(&reader, i, &dest);
                files += 1;
            } else {
                println!("licof pkg: failed to extract {}", rel);
            }
        }
    }

    if let Some(info) = info {
        mark_installed(info, files);
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

fn mark_installed(info: &PackageInfo, files: u32) {
    ensure_dir(INSTALLED_DB);
    let path = alloc::format!("{}/{}", INSTALLED_DB, info.package);
    let body = alloc::format!(
        "Package: {}\nVersion: {}\nFilename: {}\nFiles: {}\n",
        info.package,
        info.version,
        info.filename,
        files
    );
    let _ = fs::write_bytes(&path, body.as_bytes());
}

fn is_installed(pkg: &str) -> bool {
    let path = alloc::format!("{}/{}", INSTALLED_DB, pkg);
    fs::stat(&path, &mut [0u32; 7]) == 0
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

fn path_exists(path: &str) -> bool {
    fs::stat(path, &mut [0u32; 7]) == 0
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

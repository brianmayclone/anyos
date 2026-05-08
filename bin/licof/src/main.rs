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
];

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
    println!("  supported-package-data: data.tar.gz");
}

fn run(args: &[&str]) {
    if args.is_empty() {
        println!("licof run: missing Linux ELF64 path");
        return;
    }

    let path = args[0];
    let child_args = join_args(&args[1..]);
    let tid = process::licof_spawn(path, &child_args);
    if tid == u32::MAX {
        println!("licof run: failed to start '{}'", path);
        return;
    }

    let code = process::waitpid(tid);
    if code == process::STILL_RUNNING {
        println!("licof run: process {} is still running", tid);
    } else if code == u32::MAX {
        println!("licof run: wait failed for process {}", tid);
    }
}

fn rootfs(args: &[&str]) {
    match args.first().copied() {
        Some("create") => {
            let name = args.get(1).copied().unwrap_or("default");
            create_rootfs(name);
        }
        Some("list") => {
            println!("default  {}", ROOTFS_DEFAULT);
        }
        _ => {
            println!("licof rootfs: expected create or list");
        }
    }
}

fn create_rootfs(name: &str) {
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
    bootstrap_rootfs(&rootfs);
}

fn pkg(args: &[&str]) {
    match args.first().copied() {
        Some("install") => {
            if let Some(path) = args.get(1) {
                create_rootfs("default");
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
            create_rootfs("default");
            for pkg in &args[1..] {
                install_package(pkg, ROOTFS_DEFAULT, 0);
            }
        }
        _ => println!("licof apt: expected install <package>"),
    }
}

fn bootstrap_rootfs(rootfs: &str) {
    for pkg in BOOTSTRAP_SEED {
        install_package(pkg, rootfs, 0);
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

    for dep in parse_depends(&info.pre_depends) {
        install_package(&dep, rootfs, depth + 1);
    }
    for dep in parse_depends(&info.depends) {
        install_package(&dep, rootfs, depth + 1);
    }

    let deb_path = alloc::format!("{}/{}", CACHE, deb_basename(&info.filename));
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

fn parse_depends(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let first_alt = part.split('|').next().unwrap_or("").trim();
        if first_alt.is_empty() {
            continue;
        }
        let name = first_alt
            .split_ascii_whitespace()
            .next()
            .unwrap_or("")
            .trim();
        if !name.is_empty() && !out.iter().any(|p| p == name) {
            out.push(String::from(name));
        }
    }
    out
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
    if ar_entry(&data, "data.tar.xz").is_some() {
        println!(
            "licof pkg: '{}' uses data.tar.xz; XZ support is the next licof blocker",
            path
        );
        return false;
    }
    let Some(tar_data) = ar_entry(&data, "data.tar.gz") else {
        println!("licof pkg: '{}' has no supported data.tar.gz member", path);
        return false;
    };

    let tar_path = alloc::format!("{}/licof-data.tar.gz", CACHE);
    if fs::write_bytes(&tar_path, &tar_data).is_err() {
        println!("licof pkg: cannot stage data archive");
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
        if reader.entry_is_dir(i) {
            ensure_dir_recursive(&dest);
        } else {
            ensure_parent_dirs(&dest);
            if reader.extract_to_file(i, &dest) {
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

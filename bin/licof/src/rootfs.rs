use crate::config::LicoConfig;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{
    fs::{self, Read, Write},
    println,
};

const FS_TYPE_REGULAR: u32 = 0;
const FS_TYPE_DIRECTORY: u32 = 1;

pub(crate) fn ensure_rootfs_layout(config: &LicoConfig) {
    ensure_dir_recursive(&config.root);
    ensure_dir_recursive(&config.cache);
    ensure_dir_recursive(&config.db);
    ensure_dir_recursive(&config.installed_db);

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
    ensure_dir(&alloc::format!("{}/etc/pam.d", rootfs));
    ensure_dir(&alloc::format!("{}/root", rootfs));
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
    ensure_linux_account_files(rootfs);
}

fn ensure_linux_account_files(rootfs: &str) {
    ensure_rootfs_file(
        rootfs,
        "/etc/passwd",
        b"root:x:0:0:root:/root:/bin/sh\n",
        0o644,
    );
    ensure_rootfs_file(rootfs, "/etc/group", b"root:x:0:\n", 0o644);
    ensure_rootfs_file(rootfs, "/etc/shadow", b"root:*:19700:0:99999:7:::\n", 0o640);
    ensure_rootfs_file(rootfs, "/etc/gshadow", b"root:*::\n", 0o640);
    ensure_rootfs_file(
        rootfs,
        "/etc/nsswitch.conf",
        b"passwd: files\ngroup: files\nshadow: files\nhosts: files dns\nnetworks: files\nprotocols: files\nservices: files\nethers: files\nrpc: files\n",
        0o644,
    );
    ensure_linux_pam_files(rootfs);
}

fn ensure_linux_pam_files(rootfs: &str) {
    ensure_rootfs_file(
        rootfs,
        "/etc/pam.d/common-auth",
        b"auth [success=1 default=ignore] pam_unix.so nullok_secure\nauth requisite pam_deny.so\nauth required pam_permit.so\n",
        0o644,
    );
    ensure_rootfs_file(
        rootfs,
        "/etc/pam.d/common-account",
        b"account [success=1 new_authtok_reqd=done default=ignore] pam_unix.so\naccount requisite pam_deny.so\naccount required pam_permit.so\n",
        0o644,
    );
    ensure_rootfs_file(
        rootfs,
        "/etc/pam.d/common-password",
        b"password [success=1 default=ignore] pam_unix.so obscure sha512\npassword requisite pam_deny.so\npassword required pam_permit.so\n",
        0o644,
    );
    ensure_rootfs_file(
        rootfs,
        "/etc/pam.d/common-session",
        b"session required pam_unix.so\n",
        0o644,
    );
    ensure_rootfs_file(
        rootfs,
        "/etc/pam.d/other",
        b"@include common-auth\n@include common-account\n@include common-password\n@include common-session\n",
        0o644,
    );
}

fn ensure_rootfs_file(rootfs: &str, linux_path: &str, data: &[u8], mode: u16) {
    let path = linux_path_in_rootfs(rootfs, linux_path);
    if path_exists(&path) {
        return;
    }
    ensure_parent_dirs(&path);
    if fs::write_bytes(&path, data).is_ok() {
        let _ = fs::chmod(&path, mode);
    }
}

pub(crate) fn linux_path_in_rootfs(rootfs: &str, linux_path: &str) -> String {
    let rel = linux_path.trim_start_matches('/');
    alloc::format!("{}/{}", rootfs, rel)
}

pub(crate) fn rootfs_for_path(config: &LicoConfig, path: &str) -> String {
    let rootfs = config.rootfs.trim_end_matches('/');
    if path == rootfs
        || (path.starts_with(rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/'))
    {
        return config.rootfs.clone();
    }
    config.rootfs.clone()
}

pub(crate) fn find_linux_shell(rootfs: &str) -> Option<String> {
    for linux_path in ["/bin/bash", "/usr/bin/bash", "/bin/dash", "/bin/sh"] {
        let path = linux_path_in_rootfs(rootfs, linux_path);
        if regular_file_exists(&path) || path_exists(&path) {
            return Some(path);
        }
    }
    None
}

pub(crate) fn path_exists(path: &str) -> bool {
    fs::stat(path, &mut [0u32; 7]) == 0
}

fn regular_file_exists(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    fs::stat(path, &mut stat_buf) == 0 && stat_buf[0] == FS_TYPE_REGULAR
}

pub(crate) fn path_exists_no_follow(path: &str) -> bool {
    fs::lstat(path, &mut [0u32; 7]) == 0
}

pub(crate) fn path_is_symlink(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    fs::lstat(path, &mut stat_buf) == 0 && (stat_buf[2] & 1) != 0
}

pub(crate) fn print_path_probe(prefix: &str, path: &str) {
    let mut stat_buf = [0u32; 7];
    if fs::lstat(path, &mut stat_buf) == 0 {
        let kind = match stat_buf[0] {
            FS_TYPE_DIRECTORY => "dir",
            FS_TYPE_REGULAR => "file",
            _ => "other",
        };
        let link = if (stat_buf[2] & 1) != 0 {
            readlink_string(path).unwrap_or_else(|| String::from("<unreadable-link>"))
        } else {
            String::from("")
        };
        if link.is_empty() {
            println!(
                "[OK]\t{}: path {} exists kind={} size={}",
                prefix, path, kind, stat_buf[1]
            );
        } else {
            println!(
                "[OK]\t{}: path {} exists kind={} symlink->{}",
                prefix, path, kind, link
            );
        }
    } else {
        println!("[ERROR]\t{}: path {} missing", prefix, path);
    }
}

pub(crate) fn symlink_points_to(path: &str, target: &str) -> bool {
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

pub(crate) fn resolve_rootfs_symlink_path(rootfs: &str, path: &str) -> Option<String> {
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

pub(crate) fn repair_rootfs_runtime(rootfs: &str) {
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
    let candidates = ["/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2"];
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
        println!(
            "[OK]\tlicof repair: repaired {} {} -> {}",
            label, dest, target
        );
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

pub(crate) fn is_elf_file(path: &str) -> bool {
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

pub(crate) fn file_size(path: &str) -> u32 {
    let mut stat_buf = [0u32; 7];
    if fs::stat(path, &mut stat_buf) == 0 {
        stat_buf[1]
    } else {
        0
    }
}

pub(crate) fn copy_file(src: &str, dst: &str) -> bool {
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

pub(crate) fn ensure_dir(path: &str) {
    let mut stat_buf = [0u32; 7];
    if fs::stat(path, &mut stat_buf) == 0 {
        return;
    }
    let _ = fs::mkdir(path);
}

pub(crate) fn ensure_dir_recursive(path: &str) {
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

pub(crate) fn ensure_parent_dirs(path: &str) {
    if let Some(pos) = path.rfind('/') {
        if pos > 0 {
            ensure_dir_recursive(&path[..pos]);
        }
    }
}

pub(crate) fn normalize_abs_path(path: &str) -> String {
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

pub(crate) fn path_under_rootfs(rootfs: &str, path: &str) -> bool {
    path == rootfs || (path.starts_with(rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/'))
}

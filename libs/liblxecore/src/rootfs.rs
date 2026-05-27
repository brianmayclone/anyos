use crate::config::LxeConfig;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{
    fs::{self, Read, Write},
    println,
};

const FS_TYPE_REGULAR: u32 = 0;
const FS_TYPE_DIRECTORY: u32 = 1;
const COPY_CHUNK: usize = 128 * 1024;
const LXE_APT_UID: u16 = 100;
const LXE_ROOT_GID: u16 = 0;
const UTC_TZIF: &[u8] = b"TZif\0\
\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\
\0\0\0\0\0\0\0\0\0\0\0\0\
\0\0\0\0\0\0\0\x01\0\0\0\x04\
\0\0\0\0\0\0UTC\0";

pub(crate) fn ensure_rootfs_layout(config: &LxeConfig) {
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
    ensure_dir(&alloc::format!("{}/usr/sbin", rootfs));
    ensure_dir(&alloc::format!("{}/usr/local", rootfs));
    ensure_dir(&alloc::format!("{}/usr/local/bin", rootfs));
    ensure_dir(&alloc::format!("{}/usr/local/sbin", rootfs));
    ensure_dir(&alloc::format!("{}/usr/local/lib", rootfs));
    ensure_dir(&alloc::format!("{}/usr/local/share", rootfs));
    ensure_dir(&alloc::format!("{}/usr/local/etc", rootfs));
    ensure_dir(&alloc::format!("{}/etc", rootfs));
    ensure_dir(&alloc::format!("{}/etc/apt", rootfs));
    ensure_dir(&alloc::format!("{}/etc/apt/apt.conf.d", rootfs));
    ensure_dir(&alloc::format!("{}/etc/pam.d", rootfs));
    ensure_dir(&alloc::format!("{}/root", rootfs));
    ensure_dir(&alloc::format!("{}/sbin", rootfs));
    ensure_linux_apt_dirs(rootfs);
    cleanup_dpkg_temp_state(rootfs);
    let apt_keyring = "/usr/share/keyrings/debian-archive-keyring.gpg";
    let apt_security_base = debian_security_base(&config.apt_base);
    let _ = write_bytes_atomic(
        &alloc::format!("{}/etc/apt/sources.list", rootfs),
        alloc::format!(
            "deb [signed-by={}] {} {} {}\n\
deb [signed-by={}] {} {}-updates {}\n\
deb [signed-by={}] {} {}-security {}\n",
            apt_keyring,
            config.apt_base,
            config.apt_dist,
            config.apt_component,
            apt_keyring,
            config.apt_base,
            config.apt_dist,
            config.apt_component,
            apt_keyring,
            apt_security_base,
            config.apt_dist,
            config.apt_component
        )
        .as_bytes(),
    );
    let _ = write_bytes_atomic(
        &alloc::format!("{}/etc/apt/apt.conf.d/99lxe", rootfs),
        b"APT::Cache-Start \"134217728\";\nAPT::Cache-Grow \"16777216\";\nAPT::Cache-Limit \"0\";\nDir::Log::Planner \"/dev/null\";\nDpkg::Use-Pty \"false\";\nAcquire::Check-Valid-Until \"false\";\nAcquire::Languages \"none\";\nAcquire::PDiffs \"false\";\nAcquire::Queue-Mode \"access\";\nAcquire::http::Pipeline-Depth \"0\";\nAcquire::http::No-Cache \"true\";\nAcquire::http::No-Store \"true\";\n",
    );
    reset_apt_binary_cache_once(rootfs);
    ensure_linux_account_files(rootfs);
    ensure_linux_network_files(rootfs);
    ensure_linux_timezone_files(rootfs);
    ensure_linux_terminal_files(rootfs);
    ensure_linux_dpkg_helper_modes(rootfs);
}

fn reset_apt_binary_cache_once(rootfs: &str) {
    let marker = alloc::format!("{}/var/cache/apt/.lxe-cache-reset-v7", rootfs);
    if path_exists(&marker) {
        return;
    }
    let _ = fs::unlink(&alloc::format!("{}/var/cache/apt/pkgcache.bin", rootfs));
    let _ = fs::unlink(&alloc::format!("{}/var/cache/apt/srcpkgcache.bin", rootfs));
    clear_apt_list_files(&alloc::format!("{}/var/lib/apt/lists", rootfs));
    clear_apt_list_files(&alloc::format!("{}/var/lib/apt/lists/partial", rootfs));
    let _ = write_bytes_atomic(&marker, b"ok\n");
}

fn clear_apt_list_files(path: &str) {
    let mut buf = [0u8; fs::READDIR_LONG_ENTRY_SIZE * 96];
    loop {
        let count = fs::readdir_long(path, &mut buf);
        if count == u32::MAX {
            return;
        }

        let mut removed = 0u32;
        let max_entries = (count as usize).min(buf.len() / fs::READDIR_LONG_ENTRY_SIZE);
        for idx in 0..max_entries {
            let off = idx * fs::READDIR_LONG_ENTRY_SIZE;
            let entry_type = buf[off];
            let name_len = u16::from_le_bytes([buf[off + 2], buf[off + 3]]) as usize;
            if name_len == 0 || name_len > 256 {
                continue;
            }
            let name_start = off + 8;
            let name_end = name_start + name_len;
            let Ok(name) = core::str::from_utf8(&buf[name_start..name_end]) else {
                continue;
            };
            if name == "." || name == ".." || entry_type as u32 == FS_TYPE_DIRECTORY {
                continue;
            }
            if fs::unlink(&alloc::format!("{}/{}", path, name)) == 0 {
                removed += 1;
            }
        }

        if removed == 0 || count as usize <= max_entries {
            break;
        }
    }
}

fn debian_security_base(apt_base: &str) -> String {
    let base = apt_base.trim_end_matches('/');
    if let Some(prefix) = base.strip_suffix("/debian") {
        alloc::format!("{}/debian-security", prefix)
    } else {
        alloc::format!("{}/debian-security", base)
    }
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
    ensure_rootfs_line(
        rootfs,
        "/etc/passwd",
        "_apt:",
        "_apt:x:100:65534::/nonexistent:/usr/sbin/nologin\n",
        0o644,
    );
    ensure_rootfs_line(
        rootfs,
        "/etc/passwd",
        "nobody:",
        "nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n",
        0o644,
    );
    ensure_rootfs_line(
        rootfs,
        "/etc/group",
        "nogroup:",
        "nogroup:x:65534:\n",
        0o644,
    );
    ensure_rootfs_line(
        rootfs,
        "/etc/shadow",
        "_apt:",
        "_apt:*:19700:0:99999:7:::\n",
        0o640,
    );
    ensure_rootfs_line(
        rootfs,
        "/etc/shadow",
        "nobody:",
        "nobody:*:19700:0:99999:7:::\n",
        0o640,
    );
    ensure_rootfs_line(rootfs, "/etc/gshadow", "nogroup:", "nogroup:*::\n", 0o640);
    ensure_rootfs_file(
        rootfs,
        "/etc/nsswitch.conf",
        b"passwd: files\ngroup: files\nshadow: files\nhosts: files dns\nnetworks: files\nprotocols: files\nservices: files\nethers: files\nrpc: files\n",
        0o644,
    );
    ensure_linux_pam_files(rootfs);
}

fn ensure_linux_apt_dirs(rootfs: &str) {
    for dir in [
        "/var",
        "/var/cache",
        "/var/cache/apt",
        "/var/cache/apt/archives",
        "/var/lib",
        "/var/lib/apt",
        "/var/lib/apt/lists",
        "/var/lib/dpkg",
        "/var/lib/dpkg/alternatives",
        "/var/lib/dpkg/info",
        "/var/lib/dpkg/updates",
        "/var/log",
        "/var/log/apt",
        "/tmp",
        "/nonexistent",
    ] {
        ensure_dir_recursive(&linux_path_in_rootfs(rootfs, dir));
    }

    for dir in [
        "/var/cache/apt/archives/partial",
        "/var/lib/apt/lists/partial",
    ] {
        let path = linux_path_in_rootfs(rootfs, dir);
        ensure_dir_recursive(&path);
        let _ = fs::chown(&path, LXE_APT_UID, LXE_ROOT_GID);
        // AnyOS mode bits are RMDC nibbles; keep these writable after apt drops to _apt.
        let _ = fs::chmod(&path, 0xFFF);
    }
    ensure_rootfs_file(rootfs, "/var/lib/dpkg/status", b"", 0o644);
    ensure_rootfs_file(rootfs, "/var/lib/dpkg/available", b"", 0o644);
}

fn cleanup_dpkg_temp_state(rootfs: &str) {
    for linux_path in ["/var/lib/dpkg/tmp.ci", "/var/lib/dpkg/reassemble.deb"] {
        remove_tree(&linux_path_in_rootfs(rootfs, linux_path), 0);
    }
}

fn remove_tree(path: &str, depth: u32) {
    if depth > 32 {
        return;
    }
    let mut stat_buf = [0u32; 7];
    if fs::lstat(path, &mut stat_buf) != 0 {
        return;
    }
    let is_dir = stat_buf[0] == FS_TYPE_DIRECTORY && (stat_buf[2] & 1) == 0;
    if is_dir {
        let mut buf = [0u8; fs::READDIR_LONG_ENTRY_SIZE * 96];
        loop {
            let count = fs::readdir_long(path, &mut buf);
            if count == u32::MAX {
                break;
            }

            let mut removed = 0u32;
            let max_entries = (count as usize).min(buf.len() / fs::READDIR_LONG_ENTRY_SIZE);
            for idx in 0..max_entries {
                let off = idx * fs::READDIR_LONG_ENTRY_SIZE;
                let name_len = u16::from_le_bytes([buf[off + 2], buf[off + 3]]) as usize;
                if name_len == 0 || name_len > 256 {
                    continue;
                }
                let name_start = off + 8;
                let name_end = name_start + name_len;
                let Ok(name) = core::str::from_utf8(&buf[name_start..name_end]) else {
                    continue;
                };
                if name == "." || name == ".." {
                    continue;
                }
                let child = alloc::format!("{}/{}", path, name);
                remove_tree(&child, depth + 1);
                if fs::lstat(&child, &mut [0u32; 7]) != 0 {
                    removed += 1;
                }
            }

            if removed == 0 || count as usize <= max_entries {
                break;
            }
        }
    }
    let _ = fs::unlink(path);
}

fn ensure_linux_network_files(rootfs: &str) {
    ensure_rootfs_file(
        rootfs,
        "/etc/mtab",
        b"rootfs / rootfs rw 0 0\nproc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n",
        0o644,
    );
    ensure_rootfs_file(
        rootfs,
        "/etc/hosts",
        b"127.0.0.1\tlocalhost localhost.localdomain\n::1\tlocalhost ip6-localhost ip6-loopback\n",
        0o644,
    );
    ensure_rootfs_file(
        rootfs,
        "/etc/resolv.conf",
        b"nameserver 1.1.1.1\nnameserver 8.8.8.8\noptions timeout:2 attempts:2\n",
        0o644,
    );
    ensure_rootfs_file(rootfs, "/etc/host.conf", b"multi on\n", 0o644);
    ensure_rootfs_file(rootfs, "/etc/hostname", b"anyos\n", 0o644);
    ensure_rootfs_file(
        rootfs,
        "/etc/gai.conf",
        b"precedence ::ffff:0:0/96  100\n",
        0o644,
    );
    ensure_rootfs_file(
        rootfs,
        "/etc/networks",
        b"default\t0.0.0.0\nloopback\t127.0.0.0\nlink-local\t169.254.0.0\n",
        0o644,
    );
    ensure_rootfs_file(
        rootfs,
        "/etc/protocols",
        b"ip\t0\tIP\nicmp\t1\tICMP\ntcp\t6\tTCP\nudp\t17\tUDP\n",
        0o644,
    );
    ensure_rootfs_file(
        rootfs,
        "/etc/services",
        b"tcpmux\t\t1/tcp\n\
echo\t\t7/tcp\n\
echo\t\t7/udp\n\
discard\t\t9/tcp\n\
discard\t\t9/udp\n\
daytime\t\t13/tcp\n\
daytime\t\t13/udp\n\
ftp-data\t20/tcp\n\
ftp\t\t21/tcp\n\
ssh\t\t22/tcp\n\
telnet\t\t23/tcp\n\
smtp\t\t25/tcp\n\
domain\t\t53/tcp\n\
domain\t\t53/udp\n\
http\t\t80/tcp\twww www-http\n\
pop3\t\t110/tcp\n\
ntp\t\t123/udp\n\
imap2\t\t143/tcp\timap\n\
https\t\t443/tcp\n\
submission\t587/tcp\n\
imaps\t\t993/tcp\n\
pop3s\t\t995/tcp\n\
http-alt\t8080/tcp\n",
        0o644,
    );
}

fn ensure_linux_timezone_files(rootfs: &str) {
    ensure_dir_recursive(&linux_path_in_rootfs(rootfs, "/usr/share/zoneinfo/Etc"));
    ensure_rootfs_file(rootfs, "/etc/timezone", b"Etc/UTC\n", 0o644);
    ensure_rootfs_file(rootfs, "/usr/share/zoneinfo/Etc/UTC", UTC_TZIF, 0o644);

    let localtime = linux_path_in_rootfs(rootfs, "/etc/localtime");
    if path_exists_no_follow(&localtime) {
        return;
    }
    let _ = fs::symlink("/usr/share/zoneinfo/Etc/UTC", &localtime);
}

fn ensure_linux_terminal_files(rootfs: &str) {
    ensure_dir_recursive(&linux_path_in_rootfs(rootfs, "/etc/profile.d"));
    ensure_rootfs_file(
        rootfs,
        "/etc/profile.d/anyos-path.sh",
        b"case \":$PATH:\" in *:/usr/local/sbin:*) ;; *) PATH=\"/usr/local/sbin:$PATH\" ;; esac\ncase \":$PATH:\" in *:/usr/sbin:*) ;; *) PATH=\"$PATH:/usr/sbin\" ;; esac\ncase \":$PATH:\" in *:/sbin:*) ;; *) PATH=\"$PATH:/sbin\" ;; esac\nexport PATH\n",
        0o644,
    );
    ensure_rootfs_file(
        rootfs,
        "/etc/profile.d/anyos-term.sh",
        b"case \"$TERM\" in\n  \"\"|anyos|unknown|xterm-256-color) export TERM=xterm-256color ;;\nesac\n",
        0o644,
    );
    ensure_rootfs_line(
        rootfs,
        "/etc/bash.bashrc",
        "# anyOS LXE PATH",
        "# anyOS LXE PATH\ncase \":$PATH:\" in *:/usr/local/sbin:*) ;; *) PATH=\"/usr/local/sbin:$PATH\" ;; esac\ncase \":$PATH:\" in *:/usr/sbin:*) ;; *) PATH=\"$PATH:/usr/sbin\" ;; esac\ncase \":$PATH:\" in *:/sbin:*) ;; *) PATH=\"$PATH:/sbin\" ;; esac\nexport PATH\n",
        0o644,
    );
    ensure_rootfs_line(
        rootfs,
        "/etc/bash.bashrc",
        "# anyOS LXE terminal",
        "# anyOS LXE terminal\ncase \"$TERM\" in\n  \"\"|anyos|unknown|xterm-256-color) export TERM=xterm-256color ;;\nesac\n",
        0o644,
    );

    let canonical = linux_path_in_rootfs(rootfs, "/usr/share/terminfo/x/xterm-256color");
    let dashed = linux_path_in_rootfs(rootfs, "/usr/share/terminfo/x/xterm-256-color");
    if path_exists_no_follow(&canonical) && !path_exists_no_follow(&dashed) {
        ensure_parent_dirs(&dashed);
        let _ = fs::symlink("xterm-256color", &dashed);
    }
}

fn ensure_linux_dpkg_helper_modes(rootfs: &str) {
    for linux_path in [
        "/usr/bin/dpkg",
        "/usr/bin/dpkg-deb",
        "/usr/bin/dpkg-divert",
        "/usr/bin/dpkg-fsys-usrunmess",
        "/usr/bin/dpkg-maintscript-helper",
        "/usr/bin/dpkg-query",
        "/usr/bin/dpkg-realpath",
        "/usr/bin/dpkg-split",
        "/usr/bin/dpkg-statoverride",
        "/usr/bin/dpkg-trigger",
        "/usr/sbin/dpkg-preconfigure",
        "/usr/sbin/dpkg-reconfigure",
        "/usr/sbin/start-stop-daemon",
        "/sbin/start-stop-daemon",
    ] {
        let path = linux_path_in_rootfs(rootfs, linux_path);
        if path_exists_no_follow(&path) {
            let _ = fs::chmod(&path, 0o755);
        }
    }
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
    if write_bytes_atomic(&path, data) {
        let _ = fs::chmod(&path, mode);
    }
}

fn ensure_rootfs_line(rootfs: &str, linux_path: &str, key: &str, line: &str, mode: u16) {
    let path = linux_path_in_rootfs(rootfs, linux_path);
    let mut body = read_text_file(&path).unwrap_or_default();
    if body.lines().any(|existing| existing.starts_with(key)) {
        return;
    }
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(line);
    if write_bytes_atomic(&path, body.as_bytes()) {
        let _ = fs::chmod(&path, mode);
    }
}

fn read_text_file(path: &str) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut data = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
    }
    String::from_utf8(data).ok()
}

fn temp_file_path(path: &str) -> String {
    alloc::format!("{}.lxe-tmp", path)
}

pub(crate) fn replace_with_temp_file(temp: &str, dest: &str) -> bool {
    if path_exists_no_follow(dest) || path_exists(dest) {
        if fs::unlink(dest) != 0 {
            return false;
        }
    }
    fs::rename(temp, dest) == 0
}

pub(crate) fn write_bytes_atomic(dest: &str, data: &[u8]) -> bool {
    ensure_parent_dirs(dest);
    let temp = temp_file_path(dest);
    let _ = fs::unlink(&temp);
    let wrote = {
        let mut file = match fs::File::create(&temp) {
            Ok(file) => file,
            Err(_) => return false,
        };
        if file.write_all(data).is_err() {
            false
        } else {
            let _ = fs::fsync(file.fd() as i32);
            true
        }
    };
    if !wrote {
        let _ = fs::unlink(&temp);
        return false;
    }
    if file_size(&temp) != data.len().min(u32::MAX as usize) as u32 {
        let _ = fs::unlink(&temp);
        return false;
    }
    replace_with_temp_file(&temp, dest)
}

pub(crate) fn linux_path_in_rootfs(rootfs: &str, linux_path: &str) -> String {
    let rel = linux_path.trim_start_matches('/');
    alloc::format!("{}/{}", rootfs, rel)
}

pub(crate) fn rootfs_for_path(config: &LxeConfig, path: &str) -> String {
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

pub(crate) fn find_linux_bash(rootfs: &str) -> Option<String> {
    for linux_path in ["/bin/bash", "/usr/bin/bash"] {
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
    ensure_linux_account_files(rootfs);
    ensure_linux_network_files(rootfs);
    ensure_linux_timezone_files(rootfs);
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
            "[OK]\tlxe repair: repaired {} {} -> {}",
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
    ensure_parent_dirs(dst);
    let temp = temp_file_path(dst);
    let _ = fs::unlink(&temp);
    let mut buf = [0u8; COPY_CHUNK];
    let mut copied = 0u32;
    let copied_all = {
        let mut output = match fs::File::create(&temp) {
            Ok(file) => file,
            Err(_) => return false,
        };
        loop {
            let n = match input.read(&mut buf) {
                Ok(n) => n,
                Err(_) => break false,
            };
            if n == 0 {
                let _ = fs::fsync(output.fd() as i32);
                break true;
            }
            if output.write_all(&buf[..n]).is_err() {
                break false;
            }
            copied = copied.saturating_add(n as u32);
        }
    };
    if !copied_all {
        let _ = fs::unlink(&temp);
        return false;
    }
    if file_size(&temp) != copied {
        let _ = fs::unlink(&temp);
        return false;
    }
    replace_with_temp_file(&temp, dst)
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

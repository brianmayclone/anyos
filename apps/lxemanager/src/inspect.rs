use anyos_std::{format, String};
use liblxecore::config::LxeConfig;
use liblxecore::{daemon, manager};

pub struct Snapshot {
    pub title: String,
    pub subtitle: String,
    pub root_line: String,
    pub file_line: String,
    pub package_line: String,
    pub daemon_line: String,
    pub has_payload: bool,
    pub complete: bool,
}

pub fn load_snapshot() -> Snapshot {
    let config = LxeConfig::load();
    let info = manager::inspect(&config);
    let daemon_line = match daemon::status() {
        Ok(status) => format!(
            "lxed ready - {} active run(s), {} process(es), {} sync(s)",
            status.active_runs, status.active_processes, status.syncs
        ),
        Err(err) => format!("lxed unavailable - {}", err),
    };

    let title = if !info.has_payload {
        String::from("LXE is not installed")
    } else if info.complete {
        String::from("LXE is installed")
    } else {
        String::from("Partial LXE installation detected")
    };
    let subtitle = if !info.has_payload {
        String::from("Install a Debian-compatible Linux rootfs for LXE.")
    } else if info.complete {
        String::from("Manage, repair or remove the existing Linux Experience Extension rootfs.")
    } else {
        String::from("Some rootfs files exist, but the bootstrap package set is incomplete.")
    };

    Snapshot {
        title,
        subtitle,
        root_line: format!("Rootfs: {}", info.rootfs),
        file_line: format!(
            "{} files, {} directories, {}",
            info.files,
            info.dirs,
            format_bytes(info.bytes)
        ),
        package_line: format!(
            "{}/{} bootstrap packages present",
            info.seed_installed, info.seed_total
        ),
        daemon_line,
        has_payload: info.has_payload,
        complete: info.complete,
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{} GiB", bytes / (1024 * 1024 * 1024))
    } else if bytes >= 1024 * 1024 {
        format!("{} MiB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{} B", bytes)
    }
}

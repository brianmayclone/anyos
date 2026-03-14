use std::path::PathBuf;

/// Returns the directory where VM configs are stored.
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".config/corevm/vms")
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| "C:\\CoreVM".into());
        PathBuf::from(appdata).join("CoreVM\\vms")
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        PathBuf::from("./vms")
    }
}

/// Returns the directory where layout.conf is stored.
pub fn layout_dir() -> PathBuf {
    config_dir().parent().unwrap_or(&config_dir()).to_path_buf()
}

/// Search paths for BIOS files.
pub fn bios_search_paths() -> Vec<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    let mut paths = Vec::new();
    if let Some(d) = &exe_dir {
        paths.push(d.join("bios"));
        paths.push(d.to_path_buf());
    }

    // Bundled BIOS in assets/bios/ next to the vmmanager crate
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    paths.push(manifest_dir.join("assets/bios"));

    // Project source tree: libs/libcorevm/bios/ relative to the vmmanager crate
    // vmmanager is at corevm/vmmanager, project root is ../../
    if let Some(project_root) = manifest_dir.parent().and_then(|p| p.parent()) {
        paths.push(project_root.join("libs/libcorevm/bios"));
        paths.push(project_root.join("build"));
    }

    #[cfg(target_os = "linux")]
    {
        paths.push(PathBuf::from("/usr/share/corevm/bios"));
        paths.push(PathBuf::from("/usr/local/share/corevm/bios"));
    }

    // SeaBIOS from qemu (WSL/Linux)
    paths.push(PathBuf::from("/mnt/c/Program Files/qemu/share"));
    paths.push(PathBuf::from("/usr/share/seabios"));
    paths.push(PathBuf::from("/usr/share/qemu"));

    paths
}

/// Find a BIOS file by name in search paths.
pub fn find_bios(name: &str) -> Option<PathBuf> {
    for dir in bios_search_paths() {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Returns the directory where disk images are pooled.
pub fn disk_pool_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".config/corevm/disks")
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| "C:\\CoreVM".into());
        PathBuf::from(appdata).join("CoreVM\\disks")
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        PathBuf::from("./disks")
    }
}

/// Host system info for display in settings dialogs.
pub struct HostInfo {
    pub cpu_cores: u32,
    pub ram_total_mb: u64,
}

/// Query host CPU core count and total RAM.
pub fn host_info() -> HostInfo {
    HostInfo {
        cpu_cores: host_cpu_cores(),
        ram_total_mb: host_ram_mb(),
    }
}

fn host_cpu_cores() -> u32 {
    #[cfg(target_os = "linux")]
    {
        // Count "processor" lines in /proc/cpuinfo
        if let Ok(s) = std::fs::read_to_string("/proc/cpuinfo") {
            let n = s.lines().filter(|l| l.starts_with("processor")).count();
            if n > 0 { return n as u32; }
        }
        1
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("NUMBER_OF_PROCESSORS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    { 1 }
}

fn host_ram_mb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/meminfo") {
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    let kb_str = rest.trim().trim_end_matches("kB").trim();
                    if let Ok(kb) = kb_str.parse::<u64>() {
                        return kb / 1024;
                    }
                }
            }
        }
        4096
    }
    #[cfg(target_os = "windows")]
    {
        // GlobalMemoryStatusEx via extern
        4096 // fallback; proper impl would use winapi
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    { 4096 }
}

/// Ensure the config and disk pool directories exist.
pub fn ensure_dirs() {
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::create_dir_all(disk_pool_dir());
}

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

/// Ensure the config directory exists.
pub fn ensure_dirs() {
    let _ = std::fs::create_dir_all(config_dir());
}

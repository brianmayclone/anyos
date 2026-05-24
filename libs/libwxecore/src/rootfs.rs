use alloc::string::String;
use anyos_std::fs;

use crate::config::WxeConfig;

const DLL_MANIFEST: &str = "\
ntdll.dll
kernelbase.dll
kernel32.dll
msvcrt.dll
ucrtbase.dll
vcruntime140.dll
api-ms-win-core-file-l1-1-0.dll
api-ms-win-core-processthreads-l1-1-0.dll
api-ms-win-core-memory-l1-1-0.dll
api-ms-win-core-synch-l1-1-0.dll
api-ms-win-core-console-l1-1-0.dll
";

pub fn ensure_rootfs_layout(config: &WxeConfig) -> bool {
    let mut ok = true;
    for dir in [
        config.root.as_str(),
        config.drive_c.as_str(),
        config.cache.as_str(),
        config.db.as_str(),
    ] {
        if !ensure_dir(dir) {
            ok = false;
        }
    }
    for dir in [
        join(&config.drive_c, "Windows"),
        join(&config.drive_c, "Windows/System32"),
        join(&config.drive_c, "Windows/Temp"),
        join(&config.drive_c, "Users"),
        join(&config.drive_c, "Users/Default"),
        join(&config.drive_c, "Program Files"),
        join(&config.drive_c, "ProgramData"),
    ] {
        if !ensure_dir(&dir) {
            ok = false;
        }
    }

    let manifest = join(&config.db, "dll-manifest");
    if fs::write_bytes(&manifest, DLL_MANIFEST.as_bytes()).is_err() {
        ok = false;
    }

    let drive_map = alloc::format!(
        "C={}\nZ={}\n",
        config.drive_c,
        if config.drive_z.is_empty() {
            "<disabled>"
        } else {
            config.drive_z.as_str()
        }
    );
    if fs::write_bytes(&join(&config.db, "drive-map"), drive_map.as_bytes()).is_err() {
        ok = false;
    }

    ok
}

pub fn path_exists(path: &str) -> bool {
    fs::stat(path, &mut [0u32; 7]) == 0
}

pub fn installed_dll_path(config: &WxeConfig, name: &str) -> String {
    join(&config.system32(), name)
}

pub fn expected_dlls() -> &'static [&'static str] {
    &[
        "ntdll.dll",
        "kernelbase.dll",
        "kernel32.dll",
        "msvcrt.dll",
        "ucrtbase.dll",
        "vcruntime140.dll",
        "api-ms-win-core-file-l1-1-0.dll",
        "api-ms-win-core-processthreads-l1-1-0.dll",
        "api-ms-win-core-memory-l1-1-0.dll",
        "api-ms-win-core-synch-l1-1-0.dll",
        "api-ms-win-core-console-l1-1-0.dll",
    ]
}

fn ensure_dir(path: &str) -> bool {
    path_exists(path) || fs::mkdir(path) == 0 || path_exists(path)
}

fn join(base: &str, rel: &str) -> String {
    if base.ends_with('/') {
        alloc::format!("{}{}", base, rel)
    } else {
        alloc::format!("{}/{}", base, rel)
    }
}

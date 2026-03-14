//! File I/O for boot.cfg — read, write, and restore defaults.

use crate::config::Config;

pub const BOOT_CFG: &str = "/boot/boot.cfg";

/// Default boot.cfg content, restored by `bcedit init`.
pub const DEFAULT_CFG: &str = "\
# anyOS Boot Configuration
timeout=5
default=0

[anyOS]
kernel=0
description=anyOS with default settings

[anyOS (Verbose)]
kernel=0
params=verbose
description=anyOS with verbose kernel logging

[anyOS (Textmode)]
kernel=0
params=nogui
description=anyOS without compositor (text console login)

[anyOS (Custom)]
kernel=0
params=custom
description=anyOS with custom boot parameters
";

// ─── read ─────────────────────────────────────────────────────────────────────

pub fn read_file(cfg: &mut Config) -> bool {
    let fd = anyos_std::fs::open(BOOT_CFG, 0);
    if fd == u32::MAX {
        anyos_std::println!("bcedit: cannot open {}", BOOT_CFG);
        return false;
    }
    let mut raw = [0u8; 8192];
    let n = anyos_std::fs::read(fd, &mut raw);
    anyos_std::fs::close(fd);
    let n = if n == u32::MAX { 0 } else { n as usize };

    let text = core::str::from_utf8(&raw[..n]).unwrap_or("");
    for line in text.split('\n') {
        let line = if line.ends_with('\r') { &line[..line.len()-1] } else { line };
        cfg.push(line);
    }
    // Remove spurious trailing empty line that split() adds when file ends with \n
    while cfg.count > 0 && cfg.lines[cfg.count - 1].as_str().is_empty() {
        cfg.count -= 1;
    }
    true
}

// ─── write ────────────────────────────────────────────────────────────────────

pub fn write_file(cfg: &Config) -> bool {
    use anyos_std::fs::{O_WRITE, O_CREATE, O_TRUNC};
    let fd = anyos_std::fs::open(BOOT_CFG, O_WRITE | O_CREATE | O_TRUNC);
    if fd == u32::MAX {
        anyos_std::println!("bcedit: cannot write {} (permission denied?)", BOOT_CFG);
        return false;
    }
    for i in 0..cfg.count {
        anyos_std::fs::write(fd, cfg.lines[i].as_str().as_bytes());
        anyos_std::fs::write(fd, b"\n");
    }
    anyos_std::fs::close(fd);
    true
}

// ─── init (restore defaults) ──────────────────────────────────────────────────

pub fn write_default() {
    use anyos_std::fs::{O_WRITE, O_CREATE, O_TRUNC};
    let fd = anyos_std::fs::open(BOOT_CFG, O_WRITE | O_CREATE | O_TRUNC);
    if fd == u32::MAX {
        anyos_std::println!("bcedit: cannot write {} (permission denied?)", BOOT_CFG);
        return;
    }
    anyos_std::fs::write(fd, DEFAULT_CFG.as_bytes());
    anyos_std::fs::close(fd);
    anyos_std::println!("Restored default boot configuration to {}.", BOOT_CFG);
    anyos_std::println!("Entries: anyOS | anyOS (Verbose) | anyOS (Textmode) | anyOS (Custom)");
    anyos_std::println!("Default: entry 0 (anyOS), timeout: 5 seconds");
}

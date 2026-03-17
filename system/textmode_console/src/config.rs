//! System configuration applied at startup.
//!
//! Reads two sources:
//!
//! - **`/System/etc/inputmon.conf`** — keyboard layout (section `[keyboard]`,
//!   key `layout=<id>`).
//! - **`/System/env`** — console size via `CONSOLE_MODE=<1|2|3>`.
//!
//! | Mode | Columns | Rows |
//! |------|---------|------|
//! | 1    | 80      | 25   |
//! | 2    | 120     | 37   |
//! | 3    | 160     | 50   |
//!
//! These modes match the values accepted by the `bin/mode` utility.

use anyos_std::{fs, kbd, sys};

/// Predefined console dimensions indexed by `CONSOLE_MODE` (1-based).
const MODES: [(u32, u32); 3] = [(80, 25), (120, 37), (160, 50)];

/// Apply keyboard layout and optional console dimensions from the system
/// configuration files.  Called once at program start before printing the
/// banner.
pub fn apply_system_config() {
    apply_keyboard_layout();
    apply_console_mode();
}

// ─── Keyboard layout ─────────────────────────────────────────────────────────

fn apply_keyboard_layout() {
    let conf = match fs::read_to_string("/System/etc/inputmon.conf") {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut in_keyboard = false;
    for line in conf.split('\n') {
        let line = line.trim();
        if line == "[keyboard]" { in_keyboard = true; continue; }
        if line.starts_with('[') { in_keyboard = false; continue; }
        if in_keyboard {
            if let Some(val) = line.strip_prefix("layout=") {
                if let Ok(id) = val.trim().parse::<u32>() {
                    kbd::set_layout(id);
                }
            }
        }
    }
}

// ─── Console mode ─────────────────────────────────────────────────────────────

fn apply_console_mode() {
    let conf = match fs::read_to_string("/System/env") {
        Ok(c) => c,
        Err(_) => return,
    };

    for line in conf.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some(val) = line.strip_prefix("CONSOLE_MODE=") {
            let val = val.trim().trim_matches('"').trim_matches('\'');
            if let Ok(mode) = val.parse::<usize>() {
                if mode >= 1 && mode <= MODES.len() {
                    let (cols, rows) = MODES[mode - 1];
                    sys::con_resize(cols, rows);
                }
            }
        }
    }
}

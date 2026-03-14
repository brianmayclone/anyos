//! mode — anyOS text console resolution switcher
//!
//! Usage:  mode [1|2|3]
//!
//!   mode 1  →   80 × 25  (classic VGA)
//!   mode 2  →  120 × 37  (medium)
//!   mode 3  →  160 × 50  (large / high-DPI)
//!
//! With no argument, prints the current console size and available modes.
//! The chosen mode can be persisted via CONSOLE_MODE in ~/.env or /System/env.

#![no_std]
#![no_main]

anyos_std::entry!(main);

const MODES: [(u32, u32); 3] = [
    ( 80, 25),
    (120, 37),
    (160, 50),
];

fn main() {
    let mut buf = [0u8; 256];
    let arg = anyos_std::process::args(&mut buf).trim();

    if arg.is_empty() {
        let (cols, rows) = anyos_std::sys::con_get_size();
        anyos_std::println!("Current console size: {}x{}", cols, rows);
        anyos_std::println!("Available modes:");
        for (i, &(c, r)) in MODES.iter().enumerate() {
            anyos_std::println!("  {}  {}x{}", i + 1, c, r);
        }
        anyos_std::println!("Usage: mode <1|2|3>");
        return;
    }

    let mode_num: usize = arg.parse().unwrap_or(0);
    if mode_num < 1 || mode_num > MODES.len() {
        anyos_std::println!("mode: invalid mode '{}' (valid: 1-{})", arg, MODES.len());
        return;
    }

    let (cols, rows) = MODES[mode_num - 1];
    let (new_cols, new_rows) = anyos_std::sys::con_resize(cols, rows);
    if new_cols == 0 {
        anyos_std::println!("mode: failed to resize console (not in nogui mode?)");
    } else {
        anyos_std::println!("Console resized to {}x{}", new_cols, new_rows);
    }
}

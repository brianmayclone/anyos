//! Userspace Compositor for anyOS (WP19)
//!
//! Multi-threaded compositor with:
//!   - Management thread: IPC, window lifecycle, input, menus
//!   - Render thread: layer compositing, framebuffer flush, GPU commands
//!   - Layer-based compositing with damage tracking
//!   - macOS dark theme desktop (menubar, wallpaper, window chrome)
//!   - Window management (create, destroy, focus, drag, resize)
//!   - HW cursor support with SW fallback
//!   - GPU acceleration commands (UPDATE, FLIP, CURSOR)
//!   - Event channel IPC for app communication

#![no_std]
#![no_main]

use anyos_std::ipc;
use anyos_std::println;

mod app;
mod compositor;
mod config;
mod desktop;
mod ipc_protocol;
mod keys;
mod menu;
mod render;

use render::{acquire_lock, desktop_ref, release_lock, signal_render};

anyos_std::entry!(main);

// ── Main (Management Thread) ────────────────────────────────────────────────

fn main() {
    app::run();
}

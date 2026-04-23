//! Host-native test harness for the libanyui drag & drop framework.
//!
//! Re-includes `libs/libanyui/src/dnd.rs` via `#[path]` so the pure logic
//! module can be exercised on any host (Linux/macOS/Windows) with
//! `cargo test`, without pulling in anyos_std, the kernel, or the
//! compositor.
//!
//! Runtime behaviour that depends on the event loop, global state, or FFI
//! (actual session dispatch, cursor changes, visual feedback) is verified
//! end-to-end by the `demo_anyui` DnD tab.

#[path = "../../libanyui/src/dnd.rs"]
pub mod dnd;

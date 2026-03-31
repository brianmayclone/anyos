//! libanyui_client — UI widget library.
//!
//! On anyOS: loads libanyui.so via dl_open and provides typed control wrappers.
//! On host: provides stub implementations for surf-host testing.

#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

// ── Host mode: stub implementations ─────────────────────────────────────────
#[cfg(feature = "host")]
mod host;
#[cfg(feature = "host")]
pub use host::*;

// ── anyOS mode: full DLL-backed implementation ──────────────────────────────
#[cfg(not(feature = "host"))]
mod events;
#[cfg(not(feature = "host"))]
pub use events::*;

#[cfg(not(feature = "host"))]
pub mod icon;
#[cfg(not(feature = "host"))]
pub use icon::{Icon, IconType};

#[cfg(not(feature = "host"))]
pub mod theme;

#[cfg(not(feature = "host"))]
use dynlink::{DlHandle, dl_open, dl_sym};

// The remaining ~1200 lines of anyOS implementation (constants, DLL binding,
// Control/Container structs, all widget types, etc.) are in anyos_rest.rs.
// include!() pastes them here verbatim, so all paths (mod controls, etc.) work.
#[cfg(not(feature = "host"))]
include!("anyos_rest.rs");

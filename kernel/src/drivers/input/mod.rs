//! Input device drivers for keyboard and mouse.

pub mod keyboard;
pub mod layout;
#[cfg(target_arch = "x86_64")]
pub mod i2c_hid;
#[cfg(target_arch = "x86_64")]
pub mod mouse;
#[cfg(target_arch = "x86_64")]
pub mod vmmouse;

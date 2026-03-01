//! Architecture-specific modules.
//!
//! Platform-agnostic code should use `arch::hal::*` instead of
//! directly referencing `arch::x86::*` or `arch::arm64::*`.

#[cfg(target_arch = "x86_64")]
pub mod x86;

#[cfg(target_arch = "aarch64")]
pub mod arm64;

pub mod hal;

#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

pub mod config;
pub mod model;

#[cfg(not(feature = "host"))]
pub mod cli;

#[cfg(not(feature = "host"))]
mod elf;
#[cfg(not(feature = "host"))]
mod package;
#[cfg(not(feature = "host"))]
mod rootfs;

#[cfg(not(feature = "host"))]
pub use cli::run_cli;

#[cfg(feature = "host")]
pub mod hosttest;

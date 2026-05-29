#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

pub mod config;

#[cfg(not(feature = "host"))]
pub mod bootstrap;
#[cfg(not(feature = "host"))]
pub mod cli;
#[cfg(not(feature = "host"))]
pub mod cmdroutes;
#[cfg(not(feature = "host"))]
pub mod pe;
#[cfg(not(feature = "host"))]
pub mod rootfs;
#[cfg(not(feature = "host"))]
pub mod wxedll;

#[cfg(not(feature = "host"))]
pub use cli::run_cli;

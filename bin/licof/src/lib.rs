#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

pub mod config;
pub mod model;

#[cfg(feature = "host")]
pub mod hosttest;

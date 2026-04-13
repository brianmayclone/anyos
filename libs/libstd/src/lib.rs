//! std-compatible shim for anyOS.
//!
//! Provides `std::io`, `std::fs`, `std::path`, `std::sync`, `std::net`,
//! `std::thread`, `std::time`, `std::env`, `std::fmt`, `std::collections`,
//! and `std::ffi` modules that delegate to `anyos_std` and `core`/`alloc`.
//!
//! This allows crates written against Rust's standard library (e.g. gitoxide)
//! to compile on anyOS by replacing `use std::` with `use libstd::`.

#![no_std]

extern crate alloc;

pub mod io;
pub mod path;
pub mod sync;
pub mod fs;
pub mod net;
pub mod thread;
pub mod time;
pub mod env;
pub mod ffi;
pub mod process;

// Re-export core/alloc modules that std normally provides
pub mod fmt {
    pub use core::fmt::*;
}

pub mod collections {
    pub use alloc::collections::*;
    pub use anyos_std::HashMap;
    pub use anyos_std::collections::HashSet;
}

pub mod string {
    pub use alloc::string::*;
}

pub mod vec {
    pub use alloc::vec::*;
}

pub mod boxed {
    pub use alloc::boxed::*;
}

pub mod borrow {
    pub use alloc::borrow::*;
}

pub mod rc {
    pub use alloc::rc::*;
}

pub mod str {
    pub use core::str::*;
}

pub mod char {
    pub use core::char::*;
}

pub mod slice {
    pub use core::slice::*;
}

pub mod f32 {
    pub use core::f32::*;
}

pub mod f64 {
    pub use core::f64::*;
}

pub mod mem {
    pub use core::mem::*;
}

pub mod ops {
    pub use core::ops::*;
}

pub mod cmp {
    pub use core::cmp::*;
}

pub mod hash {
    pub use core::hash::*;
}

pub mod iter {
    pub use core::iter::*;
}

pub mod marker {
    pub use core::marker::*;
}

pub mod convert {
    pub use core::convert::*;
}

pub mod num {
    pub use core::num::*;
}

pub mod cell {
    pub use core::cell::*;
}

pub mod any {
    pub use core::any::*;
}

// Re-export common prelude items
pub mod prelude {
    pub mod v1 {
        pub use alloc::boxed::Box;
        pub use alloc::string::{String, ToString};
        pub use alloc::vec::Vec;
        pub use alloc::format;
        pub use alloc::vec;
        pub use core::prelude::v1::*;
    }
}

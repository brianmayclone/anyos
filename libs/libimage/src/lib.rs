// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

pub mod types;
pub mod bmp;
pub mod png;
pub mod deflate;
pub mod jpeg;
pub mod jpeg_tables;
pub mod gif;
pub mod ico;
pub mod webp;
pub mod lzw;
pub mod video;
pub mod scale;
#[cfg(not(feature = "host"))]
mod syscall;
#[cfg(not(feature = "host"))]
pub mod iconpack;
pub mod svg_raster;

// Stub module for host mode (iconpack uses anyOS syscalls)
#[cfg(feature = "host")]
pub mod iconpack {}

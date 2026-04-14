// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! Shared helper library for the CoreFS userspace CLI tools
//! (`mkfs.corefs`, `fsck.corefs`, `corefs-dump`, `corefs-scrub`,
//! `corefs-defrag`, `corefs-snapshot`, `corefs-resize`, `corefs-tier`).
//!
//! The library is `no_std + alloc` and re-exports the essential building
//! blocks every CoreFS tool needs:
//!
//! - [`block_device`] — [`AnyOsBlockDevice`] adapter that implements the
//!   plattformneutralen [`corefs_core::storage::block_device::BlockDevice`]
//!   trait on top of the anyOS `disk_read`/`disk_write` syscalls.
//! - [`args`] — long-option aware argument parser (`--device 0`,
//!   `--capacity 16777216`, `--json`, `--repair`, …) complementing the
//!   short-flag parser in `anyos_std::args`.
//! - [`report`] — structured reporting helpers with plain-text and JSON
//!   back-ends (hand-rolled, no serde_json dependency).
//! - [`error`] — mapping [`CoreFsError`](corefs_core::error::CoreFsError)
//!   to stable anyOS exit codes.
//!
//! All tool binaries link against this crate exactly once; the only
//! other anyOS-side dependency is `anyos_std`.

#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

pub mod args;
pub mod block_device;
pub mod error;
pub mod report;

pub use block_device::AnyOsBlockDevice;
pub use error::{exit_code_for, ExitCode};
pub use report::Report;

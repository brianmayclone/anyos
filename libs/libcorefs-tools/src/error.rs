// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! Mapping of [`CoreFsError`] variants to stable anyOS exit codes.
//!
//! The codes follow the Unix convention of `0` = success and keep
//! compatibility with the values already in use across the AnyOS
//! CLI ecosystem (`1` = generic error, `2` = usage/unsupported, …).

use corefs_core::error::CoreFsError;

/// Stable exit codes used by every CoreFS CLI tool.
#[repr(u32)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ExitCode {
    Success = 0,
    Generic = 1,
    Unsupported = 2,
    IoError = 5,
    InvalidArgument = 22,
    NotFound = 44,
    Corruption = 74,
}

impl ExitCode {
    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

/// Translate a [`CoreFsError`] into the corresponding exit code.
///
/// The mapping distinguishes between:
///
/// * usage/validation problems → [`ExitCode::InvalidArgument`],
/// * missing objects → [`ExitCode::NotFound`],
/// * on-disk corruption / policy failures → [`ExitCode::Corruption`] /
///   [`ExitCode::InvalidArgument`] respectively,
/// * everything else (I/O syscall error, generic driver failure) →
///   [`ExitCode::IoError`].
pub fn exit_code_for(err: &CoreFsError) -> ExitCode {
    match err {
        CoreFsError::NotFound(_) => ExitCode::NotFound,
        CoreFsError::AlreadyExists(_) => ExitCode::Generic,
        CoreFsError::PolicyViolation(_) => ExitCode::InvalidArgument,
        CoreFsError::InvalidInput(_) | CoreFsError::InvalidCommand(_) => {
            ExitCode::InvalidArgument
        }
        CoreFsError::State(msg) => {
            if msg.contains("checksum")
                || msg.contains("corrupt")
                || msg.contains("magic")
                || msg.contains("CRC")
            {
                ExitCode::Corruption
            } else {
                ExitCode::IoError
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_variants() {
        assert_eq!(
            exit_code_for(&CoreFsError::NotFound("x".into())),
            ExitCode::NotFound
        );
        assert_eq!(
            exit_code_for(&CoreFsError::InvalidInput("x".into())),
            ExitCode::InvalidArgument
        );
        assert_eq!(
            exit_code_for(&CoreFsError::InvalidCommand("x".into())),
            ExitCode::InvalidArgument
        );
        assert_eq!(
            exit_code_for(&CoreFsError::State("io failure".into())),
            ExitCode::IoError
        );
        assert_eq!(
            exit_code_for(&CoreFsError::State("checksum mismatch".into())),
            ExitCode::Corruption
        );
    }
}

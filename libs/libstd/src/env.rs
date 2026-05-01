//! std::env compatible environment functions.

use crate::io;
use crate::path::PathBuf;
use alloc::string::String;

/// Get an environment variable.
pub fn var(key: &str) -> Result<String, VarError> {
    let mut buf = [0u8; 256];
    let len = anyos_std::env::get(key, &mut buf);
    if len == 0 || len == u32::MAX {
        Err(VarError::NotPresent)
    } else {
        let s = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        Ok(String::from(s))
    }
}

/// Get an environment variable as OsString.
pub fn var_os(key: &str) -> Option<crate::path::OsString> {
    let mut buf = [0u8; 256];
    let len = anyos_std::env::get(key, &mut buf);
    if len == 0 || len == u32::MAX {
        None
    } else {
        let s = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        Some(crate::path::OsString::from(s))
    }
}

/// Set an environment variable.
pub fn set_var(key: &str, value: &str) {
    anyos_std::env::set(key, value);
}

/// Remove an environment variable.
pub fn remove_var(key: &str) {
    anyos_std::env::set(key, "");
}

/// Get the current working directory.
pub fn current_dir() -> io::Result<PathBuf> {
    let mut buf = [0u8; 256];
    let len = anyos_std::fs::getcwd(&mut buf);
    if len == u32::MAX || len == 0 {
        Err(io::Error::from_kind(io::ErrorKind::Other))
    } else {
        let s = core::str::from_utf8(&buf[..len as usize])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid CWD"))?;
        Ok(PathBuf::from(s))
    }
}

/// Set the current working directory.
pub fn set_current_dir<P: AsRef<crate::path::Path>>(path: P) -> io::Result<()> {
    let ret = anyos_std::fs::chdir(path.as_ref().as_str());
    if ret == u32::MAX {
        Err(io::Error::from_kind(io::ErrorKind::NotFound))
    } else {
        Ok(())
    }
}

/// Get the path to a temporary directory.
pub fn temp_dir() -> PathBuf {
    PathBuf::from("/tmp")
}

/// Get the home directory of the current user.
pub fn home_dir() -> Option<PathBuf> {
    let mut buf = [0u8; 256];
    let len = anyos_std::env::get("HOME", &mut buf);
    if len == 0 || len == u32::MAX {
        Some(PathBuf::from("/"))
    } else {
        let s = core::str::from_utf8(&buf[..len as usize]).unwrap_or("/");
        Some(PathBuf::from(s))
    }
}

/// Error for environment variable lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarError {
    NotPresent,
    NotUnicode(crate::path::OsString),
}

impl core::fmt::Display for VarError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            VarError::NotPresent => f.write_str("environment variable not found"),
            VarError::NotUnicode(_) => f.write_str("environment variable was not valid unicode"),
        }
    }
}

/// Command-line arguments iterator.
pub fn args() -> Args {
    Args { index: 0 }
}

/// Iterator over command-line arguments.
pub struct Args {
    index: usize,
}

impl Iterator for Args {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        if self.index == 0 {
            self.index = 1;
            Some(String::from("program"))
        } else {
            None
        }
    }
}

impl ExactSizeIterator for Args {
    fn len(&self) -> usize {
        if self.index == 0 {
            1
        } else {
            0
        }
    }
}

/// OS-string version of args.
pub fn args_os() -> ArgsOs {
    ArgsOs { inner: args() }
}

pub struct ArgsOs {
    inner: Args,
}

impl Iterator for ArgsOs {
    type Item = crate::path::OsString;
    fn next(&mut self) -> Option<crate::path::OsString> {
        self.inner.next().map(crate::path::OsString::from)
    }
}

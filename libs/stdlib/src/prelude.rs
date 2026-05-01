//! The anyos_std prelude — convenient re-exports for common types.
//!
//! Usage: `use anyos_std::prelude::*;`

pub use crate::error::{Error, Result};
pub use crate::fs::{
    read_dir, read_to_string, read_to_vec, write_bytes, DirEntry, File, Read, ReadDir, Write,
};
pub use crate::io::{stdout, Stdout};
pub use crate::process::{Child, Thread};
pub use crate::{print, println};
pub use alloc::string::String;
pub use alloc::vec::Vec;
pub use alloc::{format, vec};

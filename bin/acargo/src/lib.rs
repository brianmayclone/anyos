#![no_std]

extern crate alloc;

pub mod prelude {
    pub use alloc::boxed::Box;
    pub use alloc::format;
    pub use alloc::string::{String, ToString};
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}

pub mod build;
pub mod build_script;
pub mod fingerprint;
pub mod fs;
pub mod jobs;
pub mod lockfile;
pub mod manifest;
pub mod registry;
pub mod resolve;
pub mod scaffold;
pub mod semver;
pub mod toml;
pub mod workspace;

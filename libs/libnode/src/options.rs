use alloc::string::String;
use alloc::vec::Vec;

use crate::DEFAULT_NPM_REGISTRY;

#[derive(Clone, Debug)]
pub struct NodeOptions {
    pub argv: Vec<String>,
    pub cwd: String,
    pub registry_url: String,
    pub allow_native_ffi: bool,
}

impl Default for NodeOptions {
    fn default() -> Self {
        Self {
            argv: Vec::new(),
            cwd: String::from("."),
            registry_url: String::from(DEFAULT_NPM_REGISTRY),
            allow_native_ffi: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct NativeModulePolicy {
    pub allow_ffi: bool,
    pub allowed_libraries: Vec<String>,
}

impl NativeModulePolicy {
    pub fn deny_all() -> Self {
        Self {
            allow_ffi: false,
            allowed_libraries: Vec::new(),
        }
    }

    pub fn from_options(options: &NodeOptions) -> Self {
        Self {
            allow_ffi: options.allow_native_ffi,
            allowed_libraries: Vec::new(),
        }
    }
}

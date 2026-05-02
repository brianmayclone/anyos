//! libnode — Node.js-compatible host layer for anyOS JavaScript apps.
//!
//! `libjs` remains the ECMAScript core. `libnode` owns Node-like host APIs,
//! module policy, libuv integration and npm registry/manifest behavior.

#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

pub mod modules;
pub mod npm;
pub mod options;
pub mod resolver;
pub mod runtime;

pub use options::{NativeModulePolicy, NodeOptions};
pub use runtime::NodeRuntime;

pub const VERSION: &str = "0.1.0";
pub const DEFAULT_NPM_REGISTRY: &str = "https://registry.npmjs.org/";

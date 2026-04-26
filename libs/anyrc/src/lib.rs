#![no_std]

extern crate alloc;
#[cfg(feature = "host")]
extern crate std;

/// Prelude for no_std: common types available in every module.
pub(crate) mod prelude {
    pub use alloc::boxed::Box;
    pub use alloc::format;
    pub use alloc::string::{String, ToString};
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}

pub mod ast;
pub mod borrowck;
pub mod cfg;
pub mod codegen;
pub mod coerce;
pub mod diagnostics;
pub mod driver;
pub mod hir;
pub mod hir_lower;
pub mod intern;
pub mod lang_items;
pub mod lexer;
pub mod linker;
pub mod loader;
pub mod macros;
pub mod mir;
pub mod mir_build;
pub mod mir_opt;
pub mod mono;
pub mod parser;
pub mod resolve;
pub mod runtime;
pub mod typeck;

#![no_std]

extern crate alloc;

/// Prelude for no_std: common types available in every module.
pub(crate) mod prelude {
    pub use alloc::boxed::Box;
    pub use alloc::string::{String, ToString};
    pub use alloc::vec;
    pub use alloc::vec::Vec;
    pub use alloc::format;
}

pub mod intern;
pub mod diagnostics;
pub mod lexer;
pub mod ast;
pub mod parser;
pub mod macros;
pub mod cfg;
pub mod lang_items;
pub mod coerce;
pub mod hir;
pub mod hir_lower;
pub mod resolve;
pub mod typeck;
pub mod mir;
pub mod mir_build;
pub mod borrowck;
pub mod mir_opt;
pub mod mono;
pub mod codegen;
pub mod linker;
pub mod loader;
pub mod runtime;
pub mod driver;

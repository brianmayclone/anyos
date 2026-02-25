//! libjs — JavaScript engine for anyOS.
//!
//! A complete ECMAScript-compatible JavaScript engine with:
//! - Lexer/tokenizer
//! - Recursive descent parser (full ES2020+ syntax)
//! - AST (Abstract Syntax Tree) representation
//! - Bytecode compiler (AST → opcodes)
//! - Stack-based virtual machine with prototype chains
//! - Built-in objects: Object, Array, String, Number, Math, JSON, console
//! - Async/await and Promise support
//!
//! # Usage
//! ```rust
//! use libjs::JsEngine;
//!
//! let mut engine = JsEngine::new();
//! let result = engine.eval("1 + 2");
//! assert_eq!(result.to_number(), 3.0);
//!
//! engine.eval("var x = 'hello'; console.log(x + ' world');");
//! for msg in &engine.console_output() {
//!     // prints: "hello world"
//! }
//! ```

#![no_std]

extern crate alloc;

pub mod token;
pub mod lexer;
pub mod ast;
pub mod parser;
pub mod bytecode;
pub mod compiler;
pub mod vm;
pub mod value;

use alloc::string::String;
use alloc::vec::Vec;

pub use value::JsValue;
pub use vm::Vm;
pub use bytecode::Chunk;

/// High-level JavaScript engine interface.
pub struct JsEngine {
    vm: Vm,
}

impl JsEngine {
    /// Create a new JavaScript engine instance.
    pub fn new() -> Self {
        JsEngine { vm: Vm::new() }
    }

    /// Evaluate JavaScript source code and return the result.
    pub fn eval(&mut self, source: &str) -> JsValue {
        // Tokenize
        let tokens = lexer::Lexer::tokenize(source);

        // Parse
        let mut parser = parser::Parser::new(tokens);
        let program = parser.parse_program();

        // Compile
        let mut compiler = compiler::Compiler::new();
        let chunk = compiler.compile(&program);

        // Execute
        self.vm.execute(chunk)
    }

    /// Set a global variable in the engine.
    pub fn set_global(&mut self, name: &str, value: JsValue) {
        self.vm.set_global(name, value);
    }

    /// Get a global variable from the engine.
    pub fn get_global(&mut self, name: &str) -> JsValue {
        self.vm.get_global(name)
    }

    /// Register a native function as a global.
    pub fn register_native(&mut self, name: &str, func: fn(&mut Vm, &[JsValue]) -> JsValue) {
        self.vm.register_native(name, func);
    }

    /// Get console output messages.
    pub fn console_output(&self) -> &[String] {
        &self.vm.console_output
    }

    /// Clear console output.
    pub fn clear_console(&mut self) {
        self.vm.console_output.clear();
    }

    /// Set maximum execution steps (prevents infinite loops).
    pub fn set_step_limit(&mut self, limit: u64) {
        self.vm.set_step_limit(limit);
    }

    /// Access the underlying VM directly.
    pub fn vm(&mut self) -> &mut Vm {
        &mut self.vm
    }

    /// Compile JavaScript source without executing it.
    pub fn compile(&self, source: &str) -> Chunk {
        let tokens = lexer::Lexer::tokenize(source);
        let mut parser = parser::Parser::new(tokens);
        let program = parser.parse_program();
        let mut compiler = compiler::Compiler::new();
        compiler.compile(&program)
    }
}

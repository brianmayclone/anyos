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

#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

pub mod ast;
pub mod bytecode;
pub mod compiler;
pub mod lexer;
pub mod parser;
pub mod regexp;
pub mod token;
pub mod value;
pub mod vm;

use alloc::string::String;
use alloc::vec::Vec;

pub use bytecode::Chunk;
pub use value::JsValue;
pub use vm::Vm;

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

        // If there were parse errors, store a SyntaxError as last_exception
        if !parser.errors.is_empty() {
            let err = self.vm.make_syntax_error(&parser.errors[0]);
            self.vm.last_exception = Some(err);
            return JsValue::Undefined;
        }

        // Compile (using eval mode to return the last expression's value)
        let mut compiler = compiler::Compiler::new();
        let chunk = compiler.compile_eval(&program);

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

    /// Returns the last unhandled exception, if any (set during the most recent eval()).
    pub fn last_exception(&self) -> Option<&JsValue> {
        self.vm.last_exception.as_ref()
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

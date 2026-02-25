//! Host-native libjs engine harness.
//!
//! Re-includes all libjs source modules via `#[path]` so the engine can be
//! compiled and tested on macOS without the anyOS kernel or `anyos_std`.
//! The standard library provides the global allocator; `extern crate alloc`
//! makes the `alloc::` module paths used throughout libjs available.

// In a std crate, `alloc` is a dependency of std. The `extern crate` declaration
// makes it accessible under the `alloc::` prefix, exactly as libjs expects.
extern crate alloc;

// ── JsValue convenience extension ────────────────────────────────────────────

/// Extends `JsValue` with index-based array access for test ergonomics.
/// `JsValue::get_property` already exists for named keys; this wrapper converts
/// a numeric index to the string key that the array storage expects.
pub trait JsValueExt {
    /// Access array element by numeric index (delegates to `get_property`).
    fn get_index(&self, i: usize) -> crate::value::JsValue;
}

impl JsValueExt for crate::value::JsValue {
    fn get_index(&self, i: usize) -> crate::value::JsValue {
        self.get_property(&i.to_string())
    }
}

// ── Re-include libjs source modules ──────────────────────────────────────────
// Module subpaths are resolved relative to each included file's directory, so
// the vm/ sub-modules are found correctly in libs/libjs/src/vm/.

#[path = "../../libjs/src/token.rs"]
pub mod token;

#[path = "../../libjs/src/lexer.rs"]
pub mod lexer;

#[path = "../../libjs/src/ast.rs"]
pub mod ast;

#[path = "../../libjs/src/parser.rs"]
pub mod parser;

#[path = "../../libjs/src/bytecode.rs"]
pub mod bytecode;

#[path = "../../libjs/src/compiler.rs"]
pub mod compiler;

#[path = "../../libjs/src/value.rs"]
pub mod value;

#[path = "../../libjs/src/vm/mod.rs"]
pub mod vm;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use value::JsValue;
pub use vm::Vm;
pub use bytecode::Chunk;

// ── eval completion-value fix ─────────────────────────────────────────────────

use bytecode::Op;

/// The compiler always appends `Pop; LoadUndefined; Return` after the last
/// top-level `Stmt::Expr` (the `Pop` discards the expression result and the
/// implicit tail returns `undefined`).
///
/// For REPL / `eval()` semantics we want the *completion value*: the value of
/// the last expression.  This function detects the trailing three-opcode
/// pattern `[…, Pop, LoadUndefined, Return]` and collapses it to just
/// `[…, Return]`, so the expression result stays on the stack and is returned
/// to the caller.
fn patch_chunk_for_eval(chunk: &mut Chunk) {
    let code = &mut chunk.code;
    let n = code.len();
    // Need at least 3 ops: Pop, LoadUndefined, Return
    if n < 3 {
        return;
    }
    let matches = matches!(code[n - 3], Op::Pop)
        && matches!(code[n - 2], Op::LoadUndefined)
        && matches!(code[n - 1], Op::Return);
    if matches {
        // Remove the Pop and LoadUndefined; keep Return
        code.remove(n - 2); // remove LoadUndefined (now at n-2)
        code.remove(n - 3); // remove Pop (now at n-3)
    }
}

// ── JsEngine — mirrors libs/libjs/src/lib.rs ─────────────────────────────────

use alloc::string::String;

/// High-level JavaScript engine interface (host build).
pub struct JsEngine {
    vm: Vm,
}

impl JsEngine {
    /// Create a new JavaScript engine instance.
    pub fn new() -> Self {
        JsEngine { vm: Vm::new() }
    }

    /// Evaluate JavaScript source code and return the result.
    ///
    /// The compiler emits `Pop; LoadUndefined; Return` at the end of every
    /// top-level expression statement (the `Pop` discards the value and the
    /// implicit `Return` sends `undefined` back).  For `eval()`-style usage
    /// we want the *completion value* — the last expression's result — instead
    /// of `undefined`.  We fix this by stripping the trailing
    /// `[Pop, LoadUndefined, Return]` triple and replacing it with a bare
    /// `Return`, so the expression result stays on the stack and is returned.
    pub fn eval(&mut self, source: &str) -> JsValue {
        let tokens = lexer::Lexer::tokenize(source);
        let mut parser = parser::Parser::new(tokens);
        let program = parser.parse_program();
        let mut compiler_inst = compiler::Compiler::new();
        let mut chunk = compiler_inst.compile(&program);
        patch_chunk_for_eval(&mut chunk);
        self.vm.execute(chunk)
    }

    /// Set a global variable.
    pub fn set_global(&mut self, name: &str, value: JsValue) {
        self.vm.set_global(name, value);
    }

    /// Get a global variable.
    pub fn get_global(&mut self, name: &str) -> JsValue {
        self.vm.get_global(name)
    }

    /// Register a native Rust function as a JavaScript global.
    pub fn register_native(&mut self, name: &str, func: fn(&mut Vm, &[JsValue]) -> JsValue) {
        self.vm.register_native(name, func);
    }

    /// Access captured `console.log` / `console.error` output.
    pub fn console_output(&self) -> &[String] {
        &self.vm.console_output
    }

    /// Clear previously captured console output.
    pub fn clear_console(&mut self) {
        self.vm.console_output.clear();
    }

    /// Limit execution steps to prevent runaway scripts (default: 10_000_000).
    pub fn set_step_limit(&mut self, limit: u64) {
        self.vm.set_step_limit(limit);
    }

    /// Direct access to the underlying VM.
    pub fn vm(&mut self) -> &mut Vm {
        &mut self.vm
    }

    /// Compile without executing (useful for syntax-checking benchmarks).
    pub fn compile(&self, source: &str) -> Chunk {
        let tokens = lexer::Lexer::tokenize(source);
        let mut parser = parser::Parser::new(tokens);
        let program = parser.parse_program();
        let mut compiler = compiler::Compiler::new();
        compiler.compile(&program)
    }
}

impl Default for JsEngine {
    fn default() -> Self {
        Self::new()
    }
}

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
        #[cfg(feature = "host")]
        if std::env::var_os("LIBJS_DEBUG_PARSE").is_some() {
            if let Some(last) = tokens.last() {
                std::eprintln!(
                    "[libjs-parse] tokens={} last_span={}..{} line={}",
                    tokens.len(),
                    last.span.start,
                    last.span.end,
                    last.span.line
                );
            }
        }

        // Parse
        let mut parser = parser::Parser::new(tokens);
        let program = parser.parse_program();
        #[cfg(feature = "host")]
        if std::env::var_os("LIBJS_DEBUG_PARSE").is_some() {
            std::eprintln!(
                "[libjs-parse] source_bytes={} stmts={} remaining_tokens={} errors={}",
                source.len(),
                program.body.len(),
                parser.remaining_tokens(),
                parser.errors.len()
            );
            if let Some(err) = parser.errors.first() {
                std::eprintln!("[libjs-parse] first_error={}", err);
            }
            if std::env::var_os("LIBJS_DEBUG_AST").is_some() {
                for (idx, stmt) in program.body.iter().take(8).enumerate() {
                    let detail = match stmt {
                        ast::Stmt::Expr(expr) => ast::expr_summary(expr, 0),
                        other => ast::stmt_variant_name(other).into(),
                    };
                    std::eprintln!("[libjs-ast] stmt#{} {}", idx, detail);
                }
            }
        }

        // If there were parse errors, store a SyntaxError as last_exception
        if !parser.errors.is_empty() {
            let err = self.vm.make_syntax_error(&parser.errors[0]);
            self.vm.last_exception = Some(err);
            return JsValue::Undefined;
        }

        // Compile (using eval mode to return the last expression's value)
        let mut compiler = compiler::Compiler::new();
        let chunk = compiler.compile_eval(&program);
        #[cfg(feature = "host")]
        if std::env::var_os("LIBJS_DEBUG_PARSE").is_some() {
            std::eprintln!(
                "[libjs-parse] chunk_ops={} constants={}",
                chunk.code.len(),
                chunk.constants.len()
            );
            debug_chunk_functions(&chunk, 0, 6);
            if std::env::var_os("LIBJS_DEBUG_BYTECODE").is_some() {
                debug_chunk_ops(&chunk, "top", 96);
            }
        }

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

    /// Register a module by its source code.
    ///
    /// When JS code executes `import { x } from 'specifier'`, the engine
    /// will compile and execute this source, cache the resulting exports,
    /// and return the namespace object.
    pub fn register_module_source(&mut self, specifier: &str, source: &str) {
        self.vm
            .module_sources
            .insert(String::from(specifier), String::from(source));
    }

    /// Register a pre-built module namespace object.
    ///
    /// Useful for native/host modules (e.g. `fs`, `path`, `http`).
    pub fn register_module_object(&mut self, specifier: &str, namespace: JsValue) {
        self.vm
            .module_registry
            .insert(String::from(specifier), namespace);
    }
}

#[cfg(feature = "host")]
fn debug_chunk_functions(chunk: &bytecode::Chunk, depth: usize, remaining: usize) {
    if remaining == 0 {
        return;
    }
    for (idx, constant) in chunk.constants.iter().enumerate() {
        if let bytecode::Constant::Function(func) = constant {
            let indent = "  ".repeat(depth);
            std::eprintln!(
                "[libjs-parse] {}fn_const#{} name={} ops={} constants={} locals={} params={} strict={} generator={} async={}",
                indent,
                idx,
                func.name.as_deref().unwrap_or("<anon>"),
                func.code.len(),
                func.constants.len(),
                func.local_names.len(),
                func.param_count,
                func.strict,
                func.is_generator,
                func.is_async
            );
            if std::env::var_os("LIBJS_DEBUG_BYTECODE").is_some() && func.code.len() > 1000 {
                let mut label = String::from("fn:");
                label.push_str(func.name.as_deref().unwrap_or("<anon>"));
                debug_chunk_ops(func, &label, 96);
            }
            debug_chunk_functions(func, depth + 1, remaining - 1);
        }
    }
}

#[cfg(feature = "host")]
fn debug_chunk_ops(chunk: &bytecode::Chunk, label: &str, limit: usize) {
    for (idx, op) in chunk.code.iter().take(limit).enumerate() {
        std::eprintln!("[libjs-bytecode] {} {:04}: {:?}", label, idx, op);
    }
}

#[cfg(test)]
mod tests {
    use super::JsEngine;

    #[test]
    fn array_iterators_expose_a_real_prototype_with_next() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var it = [1].values(); \
             Object.getPrototypeOf(it) !== null && \
             typeof Object.getPrototypeOf(it).next === 'function'",
        );
        assert!(result.to_boolean());
    }

    #[test]
    fn map_iterators_expose_a_real_prototype_with_next() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var it = new Map([[1, 2]]).entries(); \
             Object.getPrototypeOf(it) !== null && \
             typeof Object.getPrototypeOf(it).next === 'function'",
        );
        assert!(result.to_boolean());
    }

    #[test]
    fn named_function_expression_does_not_create_a_global_binding() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "'use strict'; \
             var outer = typeof e; \
             var f = function e(n) { return n <= 1 ? 1 : n * e(n - 1); }; \
             [outer, typeof e, f(4)].join(',')",
        );
        assert_eq!(result.to_js_string(), "undefined,undefined,24");
    }

    #[test]
    fn assignment_branches_inside_conditional_expression_execute() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var a = null, b = 0; \
             a === null || a === void 0 ? b = 1 : b = 2; \
             b",
        );
        assert_eq!(result.to_number(), 1.0);
    }

    #[test]
    fn google_style_void_conditional_assignment_execute() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var a=null,b; \
             a===null||a===void 0?b=null:b=7; \
             b===null",
        );
        assert!(result.to_boolean());
    }

    #[test]
    fn google_style_optional_chain_lowering_assignment_rhs_executes() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var document = { querySelector: function() { return null; } }; \
             var _ = {}; \
             _.Ye=function(a,b){ \
               b=b===void 0?document:b; \
               var c,d; \
               b=(d=(c=b).querySelector)==null?void 0:d.call(c,a+'[nonce]'); \
               return b==null?'':b.nonce||b.getAttribute('nonce')||''; \
             }; \
             _.Ye('script') === ''",
        );
        assert!(result.to_boolean());
    }

    #[test]
    fn contextual_of_can_be_assigned_as_identifier() {
        let mut engine = JsEngine::new();
        let result = engine.eval("of=function(a){return a+1}; of(4)");
        assert_eq!(result.to_number(), 5.0);
    }

    #[test]
    fn contextual_as_from_let_can_be_assigned_as_identifiers() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var as, from; \
             var io = 0; \
             as = io = !1; \
             from = as === false ? 4 : 1; \
             [as, io, from].join(',')",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "false,false,4");
    }

    #[test]
    fn google_namespace_export_updates_iife_argument_object() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "this.gbar_=this.gbar_||{}; \
             (function(_){ \
               _.u=this||self; \
               _.Md=function(a,b){ \
                 a=a.split('.'); \
                 for(var c=_.u,d; a.length && (d=a.shift());) \
                   a.length || b===void 0 ? c[d] && c[d]!==Object.prototype[d] ? c=c[d] : c=c[d]={} : c[d]=b; \
               }; \
               _.Md('gbar_._DumpException', function(a){ return a; }); \
             }).call(this, this.gbar_); \
             typeof this.gbar_._DumpException",
        );
        assert_eq!(result.to_js_string(), "function");
    }

    #[test]
    fn google_int32_helper_accepts_numeric_split_fields() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var _ = {}; \
             _.gb = Number.isFinite; \
             _.Ca = function(a){ return Error(a); }; \
             _.lb = function(a){ if(typeof a !== 'number') throw _.Ca('int32'); if (!(0, _.gb)(a)) throw _.Ca('int32'); return a|0; }; \
             _.pc = function(a,b,c,d){ a.push(c(d)); return a; }; \
             var out = []; \
             var items = '3700942,3701384,102772546,116119825,116249040,116249043'.split(','); \
             for (var c = 0; c < items.length; c++) { let d = Number(items[c]); isNaN(d) || d == 0 || _.pc(out, 3, _.lb, d); } \
             out.join(',')",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(
            result.to_js_string(),
            "3700942,3701384,102772546,116119825,116249040,116249043"
        );
    }

    #[test]
    fn number_function_inside_constructor_returns_primitive() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "class Holder { \
               constructor() { \
                 this.value = Number('3701384'); \
                 this.type = typeof this.value; \
               } \
             } \
             var holder = new Holder(); \
             [holder.type, holder.value === 3701384, typeof new Number('7')].join(',')",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "number,true,object");
    }

    #[test]
    fn computed_compound_assignment_uses_property_value_not_key() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var key = Symbol.for('jas'); \
             var obj = []; \
             obj[key] = 7; \
             function mark(a, b) { a[key] |= b; return a[key]; } \
             [mark(obj, 34), obj[key]].join(',')",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "39,39");
    }

    #[test]
    fn computed_compound_assignment_evaluates_receiver_once() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var count = 0; \
             var obj = { value: 1 }; \
             function receiver() { count++; return obj; } \
             receiver()['value'] |= 2; \
             [count, obj.value].join(',')",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "1,3");
    }

    #[test]
    fn catch_binding_is_visible_in_completion_position() {
        let mut engine = JsEngine::new();
        let result = engine.eval("try { throw 7; } catch(e) { e }");
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_number(), 7.0);
    }

    #[test]
    fn object_pattern_supports_computed_property_names() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var key = 'answer'; \
             var {[key]: value, plain} = { answer: 42, plain: 8 }; \
             [value, plain].join(',')",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "42,8");
    }

    #[test]
    fn object_pattern_supports_computed_symbol_property_names() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var key = Symbol('answer'); \
             var obj = {}; \
             obj[key] = 42; \
             var {[key]: value} = obj; \
             value",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_number(), 42.0);
    }

    #[test]
    fn computed_object_pattern_does_not_break_enclosing_iife() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "this._hd=this._hd||{}; \
             (function(_){try{var key='x'; var {[key]: value}= {x: 5}; _.value=value;}catch(e){_.err=e}})(this._hd); \
             this._hd.value",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_number(), 5.0);
    }

    #[test]
    fn google_hash_style_array_argument_survives_nested_call() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var eDa,fDa; \
             eDa=function(a){var b=[];for(let c=0;c<a.length;c++)b.push(a.charCodeAt(c));return b}; \
             fDa=function(a,b){return a[b]+(a[b+1]<<8)+(a[b+2]<<16)+(a[b+3]<<24)}; \
             var a=eDa('abcdef'); \
             [a.length, fDa(a,0), fDa(a,1)].join(',')",
        );
        assert_eq!(result.to_js_string(), "6,1684234849,1701077858");
    }

    #[test]
    fn member_expression_argument_is_not_called_as_method() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var seen = ''; \
             console.assert = function(value) { seen = typeof value + ':' + value; }; \
             var obj = { _isRunning: true }; \
             console.assert(obj._isRunning); \
             seen",
        );
        assert_eq!(result.to_js_string(), "boolean:true");
    }

    #[test]
    fn console_assert_exists_and_does_not_throw_when_true() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var obj = { _isRunning: true }; \
             console.assert(obj._isRunning); \
             'ok'",
        );
        assert_eq!(result.to_js_string(), "ok");
    }

    #[test]
    fn numeric_in_operator_uses_js_number_to_property_key() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var o = { 0.975: true }; \
             var probability = 1 - (1 - 0.95) / 2; \
             [String(probability), probability in o].join(',')",
        );
        assert_eq!(result.to_js_string(), "0.975,true");
    }

    #[test]
    fn speedometer_score_formatting_handles_zero_delta() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "function sigFigFromPercentDelta(percentDelta) { \
                 return Math.ceil(-Math.log(percentDelta) / Math.log(10)) + 3; \
             } \
             function toSigFigPrecision(number, sigFig) { \
                 const nonDecimalDigitCount = number < 1 ? 0 : Math.floor(Math.log(number) / Math.log(10)) + 1; \
                 return number.toPrecision(Math.max(nonDecimalDigitCount, Math.min(6, sigFig))); \
             } \
             var meanSigFig = sigFigFromPercentDelta(0); \
             [String(meanSigFig), toSigFigPrecision(0, 2), toSigFigPrecision(123.456, Math.max(meanSigFig, 3))].join(',')",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "Infinity,0.0,123.456");
    }

    #[test]
    fn async_method_optional_chain_reaches_prototype_method() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "class Client { didRunSuites(v) { this.values.push(v); } } \
             class Runner { \
                 constructor(client) { this._client = client; } \
                 async finalize() { if (this._client?.didRunSuites) await this._client.didRunSuites(7); } \
             } \
             var c = new Client(); \
             c.values = []; \
             var r = new Runner(c); \
             r.finalize(); \
             c.values.length + ':' + c.values[0]",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "1:7");
    }

    #[test]
    fn speedometer_finalize_calls_client_with_measured_values() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "class Client { \
                 constructor() { this.values = []; } \
                 didRunSuites(v) { this.values.push(v); } \
             } \
             class Runner { \
                 constructor(client) { \
                     this._client = client; \
                     this._measuredValues = { tests: { A: { total: 10 }, B: { total: 40 } } }; \
                 } \
                 async finalize() { \
                     if (this._client?.didRunSuites) { \
                         let product = 1; \
                         const values = []; \
                         for (const suiteName in this._measuredValues.tests) { \
                             const suiteTotal = this._measuredValues.tests[suiteName].total; \
                             product *= suiteTotal; \
                             values.push(suiteTotal); \
                         } \
                         values.sort((a, b) => a - b); \
                         const total = values.reduce((a, b) => a + b); \
                         const geomean = Math.pow(product, 1 / values.length); \
                         this._measuredValues.total = total; \
                         this._measuredValues.mean = total / values.length; \
                         this._measuredValues.geomean = geomean; \
                         this._measuredValues.score = 1000 / geomean; \
                         await this._client.didRunSuites(this._measuredValues); \
                     } \
                 } \
             } \
             var c = new Client(); \
             var r = new Runner(c); \
             r.finalize(); \
             [c.values.length, c.values[0] && c.values[0].total, c.values[0] && c.values[0].score].join(',')",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "1,50,50");
    }
}

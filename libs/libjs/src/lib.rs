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
        self.eval_named(source, None)
    }

    /// Evaluate JavaScript source code with a source name used in stack traces.
    pub fn eval_named(&mut self, source: &str, name: Option<&str>) -> JsValue {
        self.execute_source(source, name, true)
    }

    /// Execute JavaScript as a classic script with a source name used in stack traces.
    ///
    /// Unlike `eval_named`, this uses normal script compilation instead of eval
    /// completion semantics. Entrypoints and CommonJS modules should use this path.
    pub fn run_named(&mut self, source: &str, name: Option<&str>) -> JsValue {
        self.execute_source(source, name, false)
    }

    fn execute_source(&mut self, source: &str, name: Option<&str>, is_eval: bool) -> JsValue {
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

        // Compile
        let mut compiler = compiler::Compiler::new();
        let mut chunk = if is_eval {
            compiler.compile_eval(&program)
        } else {
            compiler.compile(&program)
        };
        if let Some(name) = name {
            chunk.name = Some(alloc::string::String::from(name));
        }
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

    /// Clear the last unhandled exception.
    pub fn clear_last_exception(&mut self) {
        self.vm.last_exception = None;
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
                "[libjs-parse] {}fn_const#{} name={} ops={} constants={} locals={} params={} strict={} generator={} async={} upvalues={:?} local_names={:?}",
                indent,
                idx,
                func.name.as_deref().unwrap_or("<anon>"),
                func.code.len(),
                func.constants.len(),
                func.local_names.len(),
                func.param_count,
                func.strict,
                func.is_generator,
                func.is_async,
                func.upvalue_names,
                func.local_names
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
    use super::{JsEngine, JsValue};
    use alloc::string::String;

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
    fn function_call_bind_uncurries_native_methods() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var replace = Function.prototype.call.bind(String.prototype.replace); \
             var replace2 = Function.call.bind(String.prototype.replace); \
             replace('a.b', /\\./g, '#') + ':' + replace2('c.d', /\\./g, '#')",
        );
        assert_eq!(result.to_js_string(), "a#b:c#d");
    }

    #[test]
    fn function_call_reference_survives_closure_uncurry() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var uncurry = (function() { \
                 var call = Function.prototype.call; \
                 return function(fn, thisArg, a, b) { return call.call(fn, thisArg, a, b); }; \
             })(); \
             uncurry(String.prototype.replace, 'a.b', /\\./g, '#')",
        );
        assert_eq!(result.to_js_string(), "a#b");
    }

    #[test]
    fn es_module_named_reexport_copies_exports() {
        let mut engine = JsEngine::new();
        engine.register_module_source("./dep.js", "const value = 42; export { value as _ };");
        engine.register_module_source("./bar.js", "export { _ as alias } from './dep.js';");
        let result = engine.eval("var m = __import__('./bar.js'); m.alias");
        assert_eq!(result.to_number(), 42.0);
    }

    #[test]
    fn es_module_top_level_names_do_not_pollute_script_globals() {
        let mut engine = JsEngine::new();
        engine.register_module_source(
            "./chunk.js",
            "function $(value) { return value + 1; } const P = { ok: 7 }; export { P as configValuesSerialized };",
        );
        let result = engine.eval(
            "function $(e) { return Object.entries(e).length; } \
             var ns = __import__('./chunk.js'); \
             $(ns.configValuesSerialized)",
        );
        assert_eq!(result.to_number(), 1.0);
    }

    #[test]
    fn dynamic_import_keeps_entry_bundle_helper_binding() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        engine.register_module_source(
            "./route.js",
            "function $(value) { return value; } const P = { ok: 7 }; export { P as configValuesSerialized };",
        );
        let result = engine.eval(
            "function $(e) { return Object.entries(e).length; } \
             async function run() { \
                 const ns = await import('./route.js'); \
                 return $(ns.configValuesSerialized); \
             } \
             var seen = 'pending'; \
             run().then(v => { seen = v; }); \
             seen",
        );
        assert_eq!(result.to_js_string(), "function");
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "1");
    }

    #[test]
    fn object_entries_sees_frozen_null_proto_vike_export_values() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "function render() {} \
             const exports = Object.freeze(Object.defineProperty( \
               { __proto__: null, onRenderClient: render }, \
               Symbol.toStringTag, \
               { value: 'Module' } \
             )); \
             Object.entries(exports).map(function(entry) { return entry[0]; }).join(',') + \
               ':' + typeof exports.onRenderClient",
        );
        assert_eq!(result.to_js_string(), "onRenderClient:function");
    }

    #[test]
    fn module_import_preserves_frozen_vike_export_values_entries() {
        let mut engine = JsEngine::new();
        engine.register_module_source(
            "./route.js",
            "function o() {} \
             const i = Object.freeze(Object.defineProperty( \
               { __proto__: null, onRenderClient: o }, \
               Symbol.toStringTag, \
               { value: 'Module' } \
             )); \
             const P = { onRenderClient: { valueSerialized: { exportValues: i } } }; \
             export { P as configValuesSerialized };",
        );
        let result = engine.eval(
            "const ns = __import__('./route.js'); \
             const exports = ns.configValuesSerialized.onRenderClient.valueSerialized.exportValues; \
             Object.entries(exports).map(function(entry) { return entry[0]; }).join(',') + \
               ':' + typeof exports.onRenderClient",
        );
        assert_eq!(result.to_js_string(), "onRenderClient:function");
    }

    #[test]
    fn optional_call_short_circuits_following_member_chain() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var env; \
             var value = env?.split(',').map(function(v) { return v.trim(); }); \
             value === undefined",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert!(result.to_boolean());
    }

    #[test]
    fn optional_call_continuation_still_runs_for_present_base() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var env = ' a, b '; \
             env?.split(',').map(function(v) { return v.trim(); }).join('|')",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "a|b");
    }

    #[test]
    fn module_static_imports_all_named_aliases_before_execution() {
        let mut engine = JsEngine::new();
        engine.register_module_source(
            "./helper.js",
            "const exports = Object.freeze(Object.defineProperty( \
               { __proto__: null, Header: 1 }, \
               Symbol.toStringTag, \
               { value: 'Module' } \
             )); \
             export { exports as t };",
        );
        engine.register_module_source(
            "./entry.js",
            "import { t as m } from './helper.js'; \
             export { m as value };",
        );
        let result = engine.eval("var ns = __import__('./entry.js'); ns.value.Header");
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_number(), 1.0);
    }

    #[test]
    fn module_static_imports_are_instantiated_before_top_level_code() {
        let mut engine = JsEngine::new();
        engine.register_module_source("./dep.js", "const value = 42; export { value };");
        engine.register_module_source(
            "./entry.js",
            "const seen = value; import { value } from './dep.js'; export { seen };",
        );
        let result = engine.eval("var ns = __import__('./entry.js'); ns.seen");
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_number(), 42.0);
    }

    #[test]
    fn class_method_body_is_not_executed_during_module_instantiation() {
        let mut engine = JsEngine::new();
        engine.register_module_source(
            "./entry.js",
            "class Player { \
                 getMediaKeysPromise() { const e = undefined; e.mediaKeys; } \
             } \
             const ok = 1; \
             export { ok };",
        );
        let result = engine.eval("var ns = __import__('./entry.js'); ns.ok");
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_number(), 1.0);
    }

    #[test]
    fn named_class_expression_name_is_visible_inside_methods() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var C = class Inner { \
                 static getName() { return Inner.name; } \
                 getSelf() { return Inner; } \
             }; \
             C.getName() + ':' + (new C()).getSelf().name + ':' + (typeof Inner)",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "Inner:Inner:undefined");
    }

    #[test]
    fn focus_like_named_class_expression_static_singleton() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var m6 = class ve { \
                 static _instance = null; \
                 static getInstance() { \
                     return ve._instance || (ve._instance = new ve); \
                 } \
             }; \
             m6.getInstance() instanceof m6",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "true");
    }

    #[test]
    fn focus_like_named_class_expression_static_singleton_in_module() {
        let mut engine = JsEngine::new();
        engine.register_module_source(
            "./entry.js",
            "var h6 = class {}; \
             var m6 = class ve { \
                 static _instance = null; \
                 _initialized = false; \
                 constructor() {} \
                 static getInstance() { \
                     return typeof window > 'u' || typeof document > 'u' || typeof IntersectionObserver > 'u' \
                         ? new h6 \
                         : (ve._instance || (ve._instance = new ve), ve._instance); \
                 } \
             }; \
             globalThis.window = {}; \
             globalThis.document = {}; \
             globalThis.IntersectionObserver = function() {}; \
             var g6 = m6.getInstance(); \
             export { g6, m6 };",
        );
        let result = engine.eval("var ns = __import__('./entry.js'); ns.g6 instanceof ns.m6");
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "true");
    }

    #[test]
    fn focus_like_var_declarator_order_with_named_class_expression() {
        let mut engine = JsEngine::new();
        engine.register_module_source(
            "./entry.js",
            "globalThis.window = {}; \
             globalThis.document = {}; \
             globalThis.IntersectionObserver = function() {}; \
             var d4 = 1, \
                 m6 = class ve { \
                     static _instance = null; \
                     _initialized = false; \
                     constructor() {} \
                     static getInstance() { \
                         return typeof window > 'u' || typeof document > 'u' || typeof IntersectionObserver > 'u' \
                             ? new h6 \
                             : (ve._instance || (ve._instance = new ve), ve._instance); \
                     } \
                 }, \
                 h6 = class {}, \
                 g6 = m6.getInstance(); \
             export { g6, m6 };",
        );
        let result = engine.eval("var ns = __import__('./entry.js'); ns.g6 instanceof ns.m6");
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "true");
    }

    #[test]
    fn named_class_expression_self_binding_in_assignment_expression() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var m6, g6; \
             m6 = class ve { \
                 static _instance = null; \
                 static getInstance() { return ve._instance || (ve._instance = new ve); } \
             }; \
             g6 = m6.getInstance(); \
             g6 instanceof m6",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "true");
    }

    #[test]
    fn hls_like_class_method_body_is_not_executed_during_module_instantiation() {
        let mut engine = JsEngine::new();
        engine.register_module_source(
            "./entry.js",
            "class Base { constructor(prefix, logger) { this.prefix = prefix; this.logger = logger; } } \
             class Eme extends Base { \
                 constructor(hls) { \
                     super('eme', hls.logger); \
                     this.hls = hls; \
                     this.config = hls.config; \
                     this.keySystemAccessPromises = {}; \
                     this.onMediaEncrypted = event => { \
                         const { initDataType, initData } = event; \
                         this.keyFormatPromise.then(format => { \
                             const id = initDataType + format; \
                             this.getKeySystemSelectionPromise([id]).then(({ keySystem, mediaKeys }) => ({ keySystem, mediaKeys })); \
                         }); \
                     }; \
                 } \
                 requestMediaKeySystemAccess(keySystem, config) { \
                     const { requestMediaKeySystemAccessFunc } = this.config; \
                     if (typeof requestMediaKeySystemAccessFunc !== 'function') { \
                         return Promise.reject(new Error('no eme')); \
                     } \
                     return requestMediaKeySystemAccessFunc(keySystem, config); \
                 } \
                 getMediaKeysPromise(keySystem, audio, video) { \
                     const record = this.keySystemAccessPromises[keySystem]; \
                     let access = record == null ? void 0 : record.keySystemAccess; \
                     if (!access) { \
                         access = this.requestMediaKeySystemAccess(keySystem, { audio, video }); \
                         const entry = this.keySystemAccessPromises[keySystem] = { keySystemAccess: access }; \
                         return access.catch(error => {}).then(session => { \
                             entry.mediaKeys = session.createMediaKeys().then(keys => keys); \
                             entry.mediaKeys.catch(error => {}); \
                             return entry.mediaKeys; \
                         }); \
                     } \
                     return access.then(() => record.mediaKeys); \
                 } \
             } \
             const ns = Object.freeze(Object.defineProperty({ __proto__: null, Eme }, Symbol.toStringTag, { value: 'Module' })); \
             export { ns as a };",
        );
        let result = engine.eval("var ns = __import__('./entry.js'); typeof ns.a.Eme");
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "function");
    }

    #[test]
    fn module_aliases_share_one_evaluated_namespace() {
        let mut engine = JsEngine::new();
        let source = "globalThis.count = (globalThis.count || 0) + 1; const value = count; export { value };";
        engine.register_module_source("./entry.js", source);
        engine.register_module_source("https://example.test/entry.js", source);
        let result = engine.eval(
            "var a = __import__('./entry.js'); \
             var b = __import__('https://example.test/entry.js'); \
             a.value + ':' + b.value + ':' + globalThis.count",
        );
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_js_string(), "1:1:1");
    }

    #[test]
    fn module_loader_intrinsics_ignore_window_shadow_properties() {
        let mut engine = JsEngine::new();
        let window = JsValue::new_object();
        window.set_property(String::from("__import__"), JsValue::Undefined);
        engine.set_global("window", window);
        engine.register_module_source("./dep.js", "const value = 7; export { value };");
        engine.register_module_source(
            "./entry.js",
            "import { value } from './dep.js'; export { value };",
        );
        let result = engine.eval("var ns = __import__('./entry.js'); ns.value");
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_number(), 7.0);
    }

    #[test]
    fn module_static_imports_survive_vite_object_spread_bundle_shape() {
        let mut engine = JsEngine::new();
        engine.register_module_source("./loader.js", "function y() {} export { y as _ };");
        engine.register_module_source(
            "./define.js",
            "var o = Object.defineProperty, _ = (t, e) => { \
                 for (var r in e) o(t, r, { get: e[r], enumerable: true }); \
             }; \
             export { _ };",
        );
        engine.register_module_source(
            "./exports.js",
            "function y(islands, o) { return o.Header(); } export { y as h };",
        );
        engine.register_module_source(
            "./widgets.js",
            "var r = function() { return 9; }; \
             const a = Object.freeze(Object.defineProperty( \
               { __proto__: null, Header: r }, \
               Symbol.toStringTag, \
               { value: 'Module' } \
             )); \
             export { a as t };",
        );
        engine.register_module_source(
            "./entry.js",
            "const __vite__mapDeps=(i,m=__vite__mapDeps,d=(m.f||(m.f=['x'])))=>i.map(i=>d[i]);\
             import{_ as t}from'./loader.js';\
             import{_ as s}from'./define.js';\
             import{h as a}from'./exports.js';\
             import{t as m}from'./widgets.js';\
             var r = {}; s(r, { HeadlineOnlyBlock: () => 1 }); \
             function z(o) { return a(o.islands, { ...r, ...m }); } \
             export { z as r };",
        );
        let result = engine.eval("var ns = __import__('./entry.js'); ns.r({ islands: {} });");
        assert!(
            engine.last_exception().is_none(),
            "unexpected exception: {:?}",
            engine.last_exception()
        );
        assert_eq!(result.to_number(), 9.0);
    }

    #[test]
    fn dynamic_import_through_vite_preload_wrapper_resolves_namespace() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        engine.register_module_source(
            "./route.js",
            "const P = { ok: 7 }; export { P as configValuesSerialized };",
        );
        let result = engine.eval(
            "var seen = 'pending'; \
             const preload = (loader) => Promise.resolve().then(() => loader().catch((err) => { throw err; })); \
             async function run() { \
                 const ns = await preload(() => import('./route.js')); \
                 seen = ns.configValuesSerialized.ok; \
             } \
             run(); \
             seen",
        );
        assert_eq!(result.to_js_string(), "7");
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "7");
    }

    #[test]
    fn dynamic_import_through_vite_preload_with_deps_resolves_namespace() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        engine.register_module_source(
            "./route.js",
            "const P = { ok: 11 }; export { P as configValuesSerialized };",
        );
        let result = engine.eval(
            "var seen = 'pending'; \
             var document = { \
                 getElementsByTagName() { return []; }, \
                 querySelector() { return null; }, \
                 createElement() { return { addEventListener(){}, setAttribute(){} }; }, \
                 head: { appendChild(){} } \
             }; \
             var window = { dispatchEvent() { return true; } }; \
             function Event() { this.defaultPrevented = false; } \
             function preload(loader, deps) { \
                 let i = Promise.resolve(); \
                 if (deps && deps.length > 0) { \
                     let settleAll = function(items) { \
                         return Promise.all(items.map(item => Promise.resolve(item).then( \
                             value => ({ status: 'fulfilled', value }), \
                             reason => ({ status: 'rejected', reason }) \
                         ))); \
                     }; \
                     document.getElementsByTagName('link'); \
                     const nonce = document.querySelector('meta[property=csp-nonce]'); \
                     i = settleAll(deps.map(dep => { \
                         const link = document.createElement('link'); \
                         link.rel = 'modulepreload'; \
                         link.as = 'script'; \
                         link.href = dep; \
                         document.head.appendChild(link); \
                         return undefined; \
                     })); \
                 } \
                 function onError(err) { throw err; } \
                 return i.then(results => { \
                     for (const result of results || []) { \
                         if (result.status === 'rejected') onError(result.reason); \
                     } \
                     return loader().catch(onError); \
                 }); \
             } \
             async function run() { \
                 const ns = await preload(() => import('./route.js'), ['dep-a.js', 'dep-b.js']); \
                 seen = ns.configValuesSerialized.ok; \
             } \
             run(); \
             seen",
        );
        assert_eq!(result.to_js_string(), "11");
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "11");
    }

    #[test]
    fn await_assimilates_plain_thenable() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        let result = engine.eval(
            "var seen = 'pending'; \
             const thenable = { then(resolve) { resolve({ ok: 13 }); } }; \
             async function run() { const value = await thenable; seen = value.ok; } \
             run(); \
             seen",
        );
        assert_eq!(result.to_js_string(), "13");
    }

    #[test]
    fn async_pending_await_returns_chainable_promise_to_caller() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        let result = engine.eval(
            "var seen = 'pending'; \
             var savedResolve; \
             async function load() { \
                 await new Promise(resolve => { savedResolve = resolve; }); \
                 return 23; \
             } \
             load().then(value => { seen = value; }).catch(() => { seen = 'rejected'; }); \
             seen",
        );
        assert_eq!(result.to_js_string(), "pending");
        let result = engine.eval("savedResolve(); seen");
        assert_eq!(result.to_js_string(), "pending");
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "23");
    }

    #[test]
    fn object_literal_arrow_return_can_build_vike_page_entry() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        let result = engine.eval(
            "var page = { \
                 loadVirtualFilePageEntry: () => ({ \
                     moduleId: 'virtual:test', \
                     moduleExportsPromise: Promise.resolve({ configValuesSerialized: { ok: 9 } }) \
                 }) \
             }; \
             async function run(e) { \
                 const { moduleId, moduleExportsPromise } = e.loadVirtualFilePageEntry(); \
                 const ns = await moduleExportsPromise; \
                 return ns.configValuesSerialized.ok; \
             } \
             var seen = 'pending'; \
             run(page).then(v => { seen = v; }); \
             seen",
        );
        assert_eq!(result.to_js_string(), "pending");
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "9");
    }

    #[test]
    fn async_function_return_value_settles_then_chain() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var seen = 'pending'; \
             async function run() { return 9; } \
             run().then(v => { seen = v; }); \
             seen",
        );
        assert_eq!(result.to_js_string(), "pending");
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "9");
    }

    #[test]
    fn async_function_returned_promise_is_fulfilled() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "async function run() { return 9; } \
             var p = run(); \
             typeof p.then + ':' + p.__state + ':' + p.__value",
        );
        assert_eq!(result.to_js_string(), "function:fulfilled:9");
    }

    #[test]
    fn object_destructuring_reads_arrow_return_object() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var page = { load: () => ({ moduleId: 'x', moduleExportsPromise: 9 }) }; \
             const { moduleId, moduleExportsPromise } = page.load(); \
             moduleId + ':' + moduleExportsPromise",
        );
        assert_eq!(result.to_js_string(), "x:9");
    }

    #[test]
    fn object_destructuring_alias_reads_arrow_return_object() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "var page = { load: () => ({ moduleId: 'x', moduleExportsPromise: 9 }) }; \
             const { moduleId: a, moduleExportsPromise: r } = page.load(); \
             a + ':' + r",
        );
        assert_eq!(result.to_js_string(), "x:9");
    }

    #[test]
    fn vike_page_entry_survives_promise_all_hydration_merge() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        engine.register_module_source(
            "./route.js",
            "function renderClient() {} \
             const exports = Object.freeze(Object.defineProperty( \
               { __proto__: null, onRenderClient: renderClient }, \
               Symbol.toStringTag, \
               { value: 'Module' } \
             )); \
             const configValuesSerialized = { \
               onRenderClient: { \
                 type: 'standard', \
                 definedAtData: { filePathToShowToUser: '+onRenderClient.tsx' }, \
                 valueSerialized: { type: 'plus-file', exportValues: exports } \
               } \
             }; \
             export { configValuesSerialized };",
        );
        let result = engine.eval(
            "function deserializeConfig(serialized) { \
                 const out = {}; \
                 Object.entries(serialized).forEach(([name, entry]) => { \
                     const values = entry.valueSerialized.exportValues; \
                     let value; \
                     Object.entries(values).forEach(([exportName, exportValue]) => { \
                         if (exportName === 'default' || exportName === name) value = exportValue; \
                     }); \
                     out[name] = { value }; \
                 }); \
                 return out; \
             } \
             var page = { \
                 configValues: {}, \
                 loadVirtualFilePageEntry: () => ({ \
                     moduleId: 'virtual:vike:test', \
                     moduleExportsPromise: Promise.resolve().then(() => import('./route.js')) \
                 }) \
             }; \
             async function loadPageEntry(p) { \
                 const { moduleExportsPromise } = p.loadVirtualFilePageEntry(); \
                 const ns = await moduleExportsPromise; \
                 Object.assign(p.configValues, deserializeConfig(ns.configValuesSerialized)); \
                 return p; \
             } \
             async function merge() { \
                 const loaded = (await Promise.all([loadPageEntry(page), undefined]))[0]; \
                 return typeof loaded.configValues.onRenderClient.value; \
             } \
             var seen = 'pending'; \
             merge().then(v => { seen = v; }); \
             seen",
        );
        assert_eq!(result.to_js_string(), "pending");
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "function");
    }

    #[test]
    fn vike_plus_file_deserialization_populates_exports() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        let result = engine.eval(
            "function renderClient() {} \
             const exportValues = Object.freeze(Object.defineProperty( \
               { __proto__: null, onRenderClient: renderClient }, \
               Symbol.toStringTag, \
               { value: 'Module' } \
             )); \
             const serialized = { \
               onRenderClient: { \
                 type: 'standard', \
                 definedAtData: { filePathToShowToUser: '+onRenderClient.tsx' }, \
                 valueSerialized: { type: 'plus-file', exportValues } \
               } \
             }; \
             function unpackValue(entry, name, definedAt) { \
                 if (entry.type === 'plus-file') { \
                     const values = entry.exportValues; \
                     let value; \
                     const sideExports = []; \
                     Object.entries(values).forEach(([exportName, exportValue]) => { \
                         if (exportName !== 'default' && exportName !== name) { \
                             sideExports.push({ configName: exportName, configValue: { type: 'standard', value: exportValue, definedAtData: definedAt } }); \
                         } else { \
                             value = exportValue; \
                         } \
                     }); \
                     return { value, sideExports }; \
                 } \
                 return { value: entry.value, sideExports: [] }; \
             } \
             function deserializeConfig(input) { \
                 const out = {}; \
                 Object.entries(input).forEach(([name, cfg]) => { \
                     const unpacked = unpackValue(cfg.valueSerialized, name, cfg.definedAtData); \
                     out[name] = { value: unpacked.value, definedAtData: cfg.definedAtData, type: cfg.type }; \
                 }); \
                 return out; \
             } \
             function buildRuntimeConfig(configValues) { \
                 const config = {}; \
                 const exportsAll = {}; \
                 Object.entries(configValues).forEach(([name, cfg]) => { \
                     const value = cfg.value; \
                     config[name] = config[name] ?? value; \
                     exportsAll[name] = exportsAll[name] ?? []; \
                     exportsAll[name].push({ exportValue: value, filePath: cfg.definedAtData.filePathToShowToUser, _fileType: null, _isFromDefaultExport: null }); \
                 }); \
                 return { config, exportsAll }; \
             } \
             function buildPageContext(pageConfig) { \
                 const runtime = buildRuntimeConfig(pageConfig.configValues); \
                 const exports = {}; \
                 Object.entries(runtime.exportsAll).forEach(([name, entries]) => { \
                     entries.forEach(({ exportValue }) => { exports[name] = exports[name] ?? exportValue; }); \
                 }); \
                 return { config: runtime.config, exports, exportsAll: runtime.exportsAll }; \
             } \
             function copyDescriptors(target, source) { \
                 Object.defineProperties(target, Object.getOwnPropertyDescriptors(source)); \
             } \
             const pageConfig = { configValues: deserializeConfig(serialized) }; \
             const pageContext = {}; \
             copyDescriptors(pageContext, buildPageContext(pageConfig)); \
             ('onRenderClient' in pageContext.exports) + ':' + typeof pageContext.exports.onRenderClient",
        );
        assert_eq!(result.to_js_string(), "true:function");
    }

    #[test]
    fn vike_server_routing_hydration_keeps_on_render_client() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        engine.register_module_source(
            "./route.js",
            "function onRenderClient() {} \
             const exportValues = Object.freeze(Object.defineProperty( \
               { __proto__: null, onRenderClient }, \
               Symbol.toStringTag, \
               { value: 'Module' } \
             )); \
             const configValuesSerialized = { \
               onRenderClient: { \
                 type: 'standard', \
                 definedAtData: { filePathToShowToUser: '+onRenderClient.tsx' }, \
                 valueSerialized: { type: 'plus-file', exportValues } \
               } \
             }; \
             export { configValuesSerialized };",
        );
        let result = engine.eval(
            "function copyDescriptors(target, source) { \
                 if (source) Object.defineProperties(target, Object.getOwnPropertyDescriptors(source)); \
             } \
             function deserialize(input) { \
                 const out = {}; \
                 Object.entries(input).forEach(([name, cfg]) => { \
                     let value; \
                     Object.entries(cfg.valueSerialized.exportValues).forEach(([exportName, exportValue]) => { \
                         if (exportName === 'default' || exportName === name) value = exportValue; \
                     }); \
                     out[name] = { value, type: cfg.type, definedAtData: cfg.definedAtData }; \
                 }); \
                 return out; \
             } \
             function runtime(configValues) { \
                 const config = {}; \
                 const exportsAll = {}; \
                 Object.entries(configValues).forEach(([name, cfg]) => { \
                     const value = cfg.value; \
                     config[name] = config[name] ?? value; \
                     exportsAll[name] = exportsAll[name] ?? []; \
                     exportsAll[name].push({ exportValue: value, filePath: cfg.definedAtData.filePathToShowToUser, _fileType: null, _isFromDefaultExport: null }); \
                 }); \
                 return { config, exportsAll }; \
             } \
             function buildPageContext(pageFiles, pageConfig, globalConfig) { \
                 const exportsAll = {}; \
                 pageFiles.forEach(file => { \
                     Object.entries(file.fileExports || {}).forEach(([exportName, exportValue]) => { \
                         exportsAll[exportName] = exportsAll[exportName] ?? []; \
                         exportsAll[exportName].push({ exportValue, filePath: file.filePath, _fileType: file.fileType, _isFromDefaultExport: false }); \
                     }); \
                 }); \
                 const rt = runtime({ ...globalConfig.configValues, ...pageConfig.configValues }); \
                 Object.assign(exportsAll, rt.exportsAll); \
                 const exports = {}; \
                 const pageExports = {}; \
                 Object.entries(exportsAll).forEach(([name, entries]) => { \
                     entries.forEach(({ exportValue, _fileType, _isFromDefaultExport }) => { \
                         exports[name] = exports[name] ?? exportValue; \
                         if (_fileType === '.page' && !_isFromDefaultExport) pageExports[name] = pageExports[name] ?? exportValue; \
                     }); \
                 }); \
                 return { config: rt.config, exports, exportsAll, pageExports }; \
             } \
             async function va(page) { \
                 if ('isPageEntryLoaded' in page) return page; \
                 const { moduleExportsPromise } = page.loadVirtualFilePageEntry(); \
                 const ns = await moduleExportsPromise; \
                 Object.assign(page.configValues, deserialize(ns.configValuesSerialized)); \
                 copyDescriptors(page, { isPageEntryLoaded: true }); \
                 return page; \
             } \
             function relevant(files, pageId) { return files.filter(f => f.pageId === pageId || f.isDefaultPageFile); } \
             function selected(configs, pageId) { const matches = configs.filter(p => p.pageId === pageId); return matches[0] ?? null; } \
             async function Ea(pageId, files, configs, globalConfig) { \
                 const n = relevant(files, pageId); \
                 const i = selected(configs, pageId); \
                 let loaded; \
                 loaded = (await Promise.all([i && va(i, false), ...n.map(s => s.loadFile?.())]))[0]; \
                 const out = {}; \
                 copyDescriptors(out, buildPageContext(n, loaded, globalConfig)); \
                 copyDescriptors(out, { _pageFilesLoaded: n }); \
                 return out; \
             } \
             function de(ctx, name) { return (name in ctx.exports) ? ctx.exports[name] : null; } \
             const D = {}; \
             function makeGlobal(entry) { \
                 const pageConfigs = entry.pageConfigsSerialized.map(p => ({ ...p, configValues: deserialize(p.configValuesSerialized) })); \
                 const pageConfigGlobal = { configValues: deserialize(entry.pageConfigGlobalSerialized.configValuesSerialized) }; \
                 return { _pageFilesAll: entry.pageFilesList, _pageConfigs: pageConfigs, _pageConfigGlobal: pageConfigGlobal, _globalConfigPublic: { config: {}, exports: {}, exportsAll: {} } }; \
             } \
             async function createGlobal(entry) { D.globalContext = makeGlobal(entry); return D.globalContext; } \
             async function qa(entry) { delete D.globalContextPromise; D.entry = entry; await (D.globalContextPromise = createGlobal(entry)); } \
             async function Xa() { return await D.globalContextPromise; } \
             async function nr(ctx) { const loaded = await Ea(ctx.pageId, ctx._pageFilesAll, ctx._globalContext._pageConfigs, ctx._globalContext._pageConfigGlobal); copyDescriptors(ctx, loaded); return ctx; } \
             async function ar() { const global = await Xa(); const ctx = { pageId: '/page', _globalContext: global, _pageFilesAll: global._pageFilesAll }; await nr(ctx); return ctx; } \
             async function Sr() { const ctx = await ar(); seen = typeof de(ctx, 'onRenderClient'); } \
             const entry = { \
                 pageConfigsSerialized: [{ \
                     pageId: '/page', \
                     loadVirtualFilePageEntry: () => ({ moduleExportsPromise: Promise.resolve().then(() => import('./route.js')) }), \
                     configValuesSerialized: {} \
                 }], \
                 pageConfigGlobalSerialized: { configValuesSerialized: {} }, \
                 pageFilesList: [{ pageId: '/page', filePath: '/src/+Page.client.tsx', fileType: '.page.client', isDefaultPageFile: false }] \
             }; \
             var seen = 'pending'; \
             qa(entry); \
             Sr(); \
             seen",
        );
        assert_eq!(result.to_js_string(), "function");
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "function");
    }

    #[test]
    fn vike_on_render_client_hook_runner_accepts_undefined_return() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        let result = engine.eval(
            "var seen = 'pending'; \
             function assert(cond, msg) { if (!cond) throw new Error(msg); } \
             async function He(hooks, ctx, makeCtx) { \
                 if (!hooks.length) return []; \
                 const runtimeCtx = makeCtx(ctx); \
                 return await Promise.all(hooks.map(async hook => { \
                     const hookReturn = await Be(() => hook.hookFn(runtimeCtx), hook, runtimeCtx); \
                     return { ...hook, hookReturn }; \
                 })); \
             } \
             async function Ga(hook, ctx, makeCtx) { \
                 const results = await He([hook], ctx, makeCtx); \
                 const { hookReturn } = results[0]; \
                 assert(hookReturn === undefined, 'hook returned a value'); \
                 seen = 'ok'; \
             } \
             function Be(run, hook, ctx) { \
                 let resolve, reject; \
                 const promise = new Promise((res, rej) => { resolve = res; reject = rej; }); \
                 (async () => { \
                     try { resolve(await run()); } catch (err) { reject(err); } \
                 })(); \
                 return promise; \
             } \
             const hook = { \
                 hookName: 'onRenderClient', \
                 hookFilePath: '+onRenderClient.tsx', \
                 hookTimeout: { error: null, warning: null }, \
                 hookFn(ctx) { seen = ctx.marker; } \
             }; \
             Ga(hook, { marker: 'called' }, ctx => ctx).catch(err => { seen = err.message; }); \
             seen",
        );
        assert_eq!(result.to_js_string(), "ok");
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "ok");
    }

    #[test]
    fn legacy_iterator_alias_keeps_corejs_iterate_compatible() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        let result = engine.eval(
            "function collect(iterable) { \
                 const method = iterable[Symbol.iterator] || iterable['@@iterator']; \
                 if (typeof method !== 'function') throw new TypeError('Target is not iterable'); \
                 const iterator = method.call(iterable); \
                 const out = []; \
                 for (;;) { \
                     const step = iterator.next(); \
                     if (step.done) break; \
                     out.push(step.value); \
                 } \
                 return out; \
             } \
             Symbol.iterator = '__core_js_private_iterator__'; \
             const arr = collect([1, 2]).join(','); \
             const str = collect('ab').join(','); \
             const map = new Map(); \
             map.set('k', 'v'); \
             const entry = collect(map)[0]; \
             const set = new Set(); \
             set.add(7); \
             arr + '|' + str + '|' + entry[0] + ':' + entry[1] + '|' + collect(set)[0]",
        );
        assert_eq!(result.to_js_string(), "1,2|a,b|k:v|7");
    }

    #[test]
    fn promise_all_accepts_result_from_polyfilled_array_map() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        let result = engine.eval(
            "var seen = 'pending'; \
             var bind3 = function(fn, that) { return function(a, b, c) { return fn.call(that, a, b, c); }; }; \
             var toObject = function(value) { return Object(value); }; \
             var indexedObject = function(value) { return Object(value); }; \
             var toLength = function(value) { return value >>> 0; }; \
             var arraySpeciesCreate = function(original, length) { return new Array(length); }; \
             var push = [].push; \
             var createMethod = function(TYPE) { \
                 var IS_MAP = TYPE == 1, IS_FILTER = TYPE == 2, IS_SOME = TYPE == 3; \
                 var IS_EVERY = TYPE == 4, IS_FIND_INDEX = TYPE == 6, IS_FILTER_OUT = TYPE == 7; \
                 var NO_HOLES = TYPE == 5 || IS_FIND_INDEX; \
                 return function(self, callbackfn, that, specificCreate) { \
                     for (var value, result, O = toObject(self), selfIndexed = indexedObject(O), bound = bind3(callbackfn, that), length = toLength(selfIndexed.length), index = 0, create = specificCreate || arraySpeciesCreate, target = IS_MAP ? create(self, length) : IS_FILTER || IS_FILTER_OUT ? create(self, 0) : undefined; length > index; index++) { \
                         if ((NO_HOLES || index in selfIndexed) && (result = bound(value = selfIndexed[index], index, O), TYPE)) { \
                             if (IS_MAP) target[index] = result; \
                             else if (result) switch (TYPE) { case 3: return true; case 5: return value; case 6: return index; case 2: push.call(target, value); } \
                             else switch (TYPE) { case 4: return false; case 7: push.call(target, value); } \
                         } \
                     } \
                     return IS_FIND_INDEX ? -1 : IS_SOME || IS_EVERY ? IS_EVERY : target; \
                 }; \
             }; \
             Array.prototype.map = function(callbackfn) { \
                 return createMethod(1)(this, callbackfn, arguments.length > 1 ? arguments[1] : undefined); \
             }; \
             Promise.all = function(iterable) { \
                 return new Promise(function(resolve, reject) { \
                     var out = []; \
                     var pending = 1; \
                     var index = 0; \
                     for (var i = 0; i < iterable.length; i++) { \
                         var slot = index++; \
                         pending++; \
                         out.push(undefined); \
                         Promise.resolve(iterable[i]).then(function(value) { \
                             out[slot] = value; \
                             if (--pending === 0) resolve(out); \
                         }, reject); \
                     } \
                     if (--pending === 0) resolve(out); \
                 }); \
             }; \
             async function run() { \
                 var hooks = [{ hookFn: function(ctx) { seen = ctx.marker; } }]; \
                 var result = await Promise.all(hooks.map(async function(hook) { \
                     var hookReturn = await hook.hookFn({ marker: 'called' }); \
                     return { hookReturn: hookReturn }; \
                 })); \
                 seen = Array.isArray(result) + '|' + result.length + '|' + result[0].hookReturn; \
             } \
             run().catch(function(err) { seen = err.message; }); \
             seen",
        );
        assert_eq!(result.to_js_string(), "true|1|undefined");
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "true|1|undefined");
    }

    #[test]
    fn async_map_callback_rejection_does_not_escape_synchronously() {
        let mut engine = JsEngine::new();
        engine.set_step_limit(1_000_000);
        let result = engine.eval(
            "var seen = 'pending'; \
             var mapped = [1].map(async function() { \
                 await Promise.reject('boom'); \
             }); \
             seen = Array.isArray(mapped) + '|' + mapped.length + '|' + (typeof mapped[0].then); \
             Promise.all(mapped).then(function() { seen = 'fulfilled'; }, function(err) { seen = err; }); \
             seen",
        );
        assert_eq!(result.to_js_string(), "true|1|function");
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "boom");
    }

    #[test]
    fn regexp_symbol_replace_is_callable() {
        let mut engine = JsEngine::new();
        let result = engine.eval("/\\./[Symbol.replace]('a.b', '#')");
        assert_eq!(result.to_js_string(), "a#b");
    }

    #[test]
    fn regexp_symbol_match_is_callable() {
        let mut engine = JsEngine::new();
        let result = engine.eval("/o+/[Symbol.match]('fooo')[0]");
        assert_eq!(result.to_js_string(), "ooo");
    }

    #[test]
    fn symbol_prototype_description_is_available() {
        let mut engine = JsEngine::new();
        let result = engine.eval(
            "'description' in Symbol.prototype && \
             Symbol('surf').description === 'surf' && \
             Symbol().description === undefined",
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
    fn promise_executor_resolve_adopts_nested_promise() {
        let mut engine = JsEngine::new();
        engine.eval(
            "var seen = 'pending'; \
             new Promise(function(resolve) { \
               resolve(Promise.resolve('response')); \
             }).then(function(value) { \
               seen = value; \
             });",
        );
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "response");
    }

    #[test]
    fn promise_chain_resumes_transpiled_generator_with_resolved_value() {
        let mut engine = JsEngine::new();
        engine.eval(
            "var seen = 'pending'; \
             function run(iterator) { \
               function resume(value) { return iterator.next(value); } \
               function fail(err) { return iterator.throw(err); } \
               return new Promise(function(resolve, reject) { \
                 function step(result) { \
                   result.done ? resolve(result.value) : \
                     Promise.resolve(result.value).then(resume, fail).then(step, reject); \
                 } \
                 step(iterator.next()); \
               }); \
             } \
             var state = 0; \
             var iterator = { \
               next: function(value) { \
                 if (state++ === 0) { \
                   return { done: false, value: Promise.resolve({ headers: { get: function(){ return 'ok'; }, has: function(){ return false; } }, url: 'u' }) }; \
                 } \
                 return { done: true, value: value.headers.get('X-Test') }; \
               }, \
               throw: function(err) { throw err; } \
             }; \
             run(iterator).then(function(value) { seen = value; });",
        );
        let result = engine.eval("seen");
        assert_eq!(result.to_js_string(), "ok");
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

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec;
use libjs::value::JsValue;
use libjs::vm::native_fn;

use crate::modules;
use crate::options::{NativeModulePolicy, NodeOptions};
use crate::resolver::{self, ModuleKind, ModuleResolver, ResolvedModule};

pub struct NodeRuntime {
    engine: libjs::JsEngine,
    options: NodeOptions,
    event_loop: libuv::EventLoop,
    policy: NativeModulePolicy,
    resolver: ModuleResolver,
    loaded_modules: BTreeSet<String>,
}

impl NodeRuntime {
    pub fn new(options: NodeOptions) -> Self {
        let resolver = ModuleResolver::new(&options.cwd);
        let mut runtime = Self {
            engine: libjs::JsEngine::new(),
            event_loop: libuv::EventLoop::new(),
            policy: NativeModulePolicy::from_options(&options),
            resolver,
            loaded_modules: BTreeSet::new(),
            options,
        };
        runtime.install_builtins();
        runtime
    }

    pub fn engine(&mut self) -> &mut libjs::JsEngine {
        &mut self.engine
    }

    pub fn eval(&mut self, source: &str) -> JsValue {
        self.engine.eval(source)
    }

    pub fn run_script(&mut self, path: &str, source: &str) -> JsValue {
        self.options.argv = vec![String::from("node"), String::from(path)];
        self.install_process_object();
        let dirname = resolver::dirname(path);
        self.preload_requires(source, &dirname, 0);
        self.install_commonjs_globals(path, &dirname);
        self.engine.eval(source)
    }

    pub fn run_file(&mut self, path: &str) -> Result<JsValue, &'static str> {
        let source = anyos_std::fs::read_to_string(path).map_err(|_| "Could not read script")?;
        Ok(self.run_script(path, &source))
    }

    pub fn set_policy(&mut self, policy: NativeModulePolicy) {
        self.policy = policy;
        self.install_native_modules();
    }

    pub fn uv_loop(&mut self) -> &mut libuv::UvLoop {
        self.event_loop.uv_loop_mut()
    }

    pub fn event_loop(&mut self) -> &mut libuv::EventLoop {
        &mut self.event_loop
    }

    pub fn run_event_loop_once(&mut self) -> usize {
        self.engine.vm().drain_microtasks();
        let handled_io = modules::net::poll_servers(self.engine.vm())
            + modules::http::poll_servers(self.engine.vm());
        if handled_io > 0 {
            return handled_io;
        }
        let Some(wait_ms) = self.next_js_timer_delay_ms() else {
            return 0;
        };
        self.event_loop.schedule_timer(wait_ms as u64, 0);
        let fired_uv = self.event_loop.run(libuv::UvRunMode::Once);
        if fired_uv.is_empty() && wait_ms > 0 {
            return 0;
        }
        self.engine.vm().tick(wait_ms.max(1))
    }

    pub fn run_event_loop(&mut self) -> usize {
        let mut fired = 0usize;
        let mut rounds = 0u32;
        loop {
            self.engine.vm().drain_microtasks();
            if !self.has_pending_js_tasks() || rounds >= 10_000 {
                break;
            }
            fired += self.run_event_loop_once();
            rounds += 1;
        }
        self.engine.vm().drain_microtasks();
        fired
    }

    fn install_builtins(&mut self) {
        self.install_process_object();
        self.install_require();
        self.install_native_modules();
    }

    fn install_process_object(&mut self) {
        self.engine
            .register_module_object("node:process", modules::process_module(&self.options));
        let process = self.module("node:process");
        self.engine.set_global("process", process);
    }

    fn install_require(&mut self) {
        self.engine
            .register_native("require", modules::node_require);
        let require = self.engine.get_global("require");
        require.set_property(
            String::from("resolve"),
            native_fn("resolve", modules::node_require_resolve),
        );
        let cache = JsValue::new_object();
        require.set_property(String::from("cache"), cache.clone());
        self.engine.set_global("__node_require_cache__", cache);
        self.engine
            .set_global("__node_resolved__", JsValue::new_object());
    }

    fn install_native_modules(&mut self) {
        let fs = modules::fs_module();
        self.engine.register_module_object("fs", fs.clone());
        self.engine.register_module_object("node:fs", fs);
        let path = modules::path_module();
        self.engine.register_module_object("path", path.clone());
        self.engine.register_module_object("node:path", path);
        let url = modules::url_module();
        self.engine.register_module_object("url", url.clone());
        self.engine.register_module_object("node:url", url);
        let querystring = modules::querystring_module();
        self.engine
            .register_module_object("querystring", querystring.clone());
        self.engine
            .register_module_object("node:querystring", querystring);
        let os = modules::os_module();
        self.engine.register_module_object("os", os.clone());
        self.engine.register_module_object("node:os", os);
        let assert = modules::assert_module();
        self.engine.register_module_object("assert", assert.clone());
        self.engine.register_module_object("node:assert", assert);
        let assert_strict = modules::assert_strict_module();
        self.engine
            .register_module_object("node:assert/strict", assert_strict.clone());
        self.engine
            .register_module_object("assert/strict", assert_strict);
        let buffer = modules::buffer_module();
        self.engine.register_module_object("buffer", buffer.clone());
        self.engine.register_module_object("node:buffer", buffer);
        self.engine
            .set_global("Buffer", modules::buffer::buffer_global());
        let crypto = modules::crypto_module();
        self.engine.register_module_object("crypto", crypto.clone());
        self.engine.register_module_object("node:crypto", crypto);
        let dns = modules::dns_module();
        self.engine.register_module_object("dns", dns.clone());
        self.engine.register_module_object("node:dns", dns);
        let events = modules::events_module();
        self.engine.register_module_object("events", events.clone());
        self.engine.register_module_object("node:events", events);
        let util = modules::util_module();
        self.engine.register_module_object("util", util.clone());
        self.engine.register_module_object("node:util", util);
        let stream = modules::stream_module();
        self.engine.register_module_object("stream", stream.clone());
        self.engine.register_module_object("node:stream", stream);
        let zlib = modules::zlib_module();
        self.engine.register_module_object("zlib", zlib.clone());
        self.engine.register_module_object("node:zlib", zlib);
        let timers = modules::timers_module();
        self.engine.register_module_object("timers", timers.clone());
        self.engine.register_module_object("node:timers", timers);
        let net = modules::net_module();
        self.engine.register_module_object("net", net.clone());
        self.engine.register_module_object("node:net", net);
        let http = modules::http_module();
        self.engine.register_module_object("http", http.clone());
        self.engine
            .register_module_object("node:http", http.clone());
        self.engine.register_module_object("https", http.clone());
        self.engine.register_module_object("node:https", http);
        let ffi = modules::ffi_module(&self.policy);
        self.engine
            .register_module_object("@anyos/ffi", ffi.clone());
        self.engine.register_module_object("node:ffi", ffi);
        self.engine
            .register_module_object("@anyos/anyui", modules::anyui_module(&self.policy));
        self.engine
            .register_module_object("@anyos/image", modules::image_module(&self.policy));
        self.engine
            .register_module_object("node:uv", modules::uv_module(self.event_loop.uv_loop()));
    }

    fn install_commonjs_globals(&mut self, filename: &str, dirname: &str) {
        let module = modules::commonjs_module(filename, dirname);
        let exports = module.get_property("exports");
        self.engine.set_global("module", module);
        self.engine.set_global("exports", exports);
        self.engine
            .set_global("__filename", JsValue::String(String::from(filename)));
        self.engine
            .set_global("__dirname", JsValue::String(String::from(dirname)));
    }

    fn preload_requires(&mut self, source: &str, from_dir: &str, depth: usize) {
        if depth > 32 {
            return;
        }
        for specifier in resolver::find_require_specifiers(source) {
            if resolver::is_core_module(&specifier)
                || !matches!(self.module(&specifier), JsValue::Undefined)
            {
                continue;
            }
            if let Some(module) = self.resolver.resolve(&specifier, from_dir) {
                self.load_commonjs_module(&specifier, module, depth + 1);
            }
        }
    }

    fn load_commonjs_module(
        &mut self,
        requested_specifier: &str,
        module: ResolvedModule,
        depth: usize,
    ) {
        if self.loaded_modules.contains(&module.filename) {
            if let Some(cached) = self
                .engine
                .vm()
                .module_registry
                .get(&module.filename)
                .cloned()
            {
                self.engine
                    .register_module_object(requested_specifier, cached.clone());
                self.engine.register_module_object(&module.id, cached);
            }
            return;
        }

        self.loaded_modules.insert(module.filename.clone());
        let exports = match module.kind {
            ModuleKind::Json => self.load_json_module(&module),
            ModuleKind::JavaScript => {
                self.preload_requires(&module.source, &module.dirname, depth);
                self.install_commonjs_globals(&module.filename, &module.dirname);
                self.engine.eval(&module.source);
                let module_global = self.engine.get_global("module");
                module_global.set_property(String::from("loaded"), JsValue::Bool(true));
                self.cache_module_object(&module.filename, module_global.clone());
                module_global.get_property("exports")
            }
        };
        self.engine
            .register_module_object(requested_specifier, exports.clone());
        self.engine
            .register_module_object(&module.id, exports.clone());
        self.engine
            .register_module_object(&module.filename, exports);
        self.record_resolved_module(requested_specifier, &module.filename);
        self.record_resolved_module(&module.id, &module.filename);
    }

    fn load_json_module(&mut self, module: &ResolvedModule) -> JsValue {
        let source = js_string_literal(&module.source);
        let exports = self.engine.eval(&alloc::format!("JSON.parse({})", source));
        let module_object = modules::commonjs_module(&module.filename, &module.dirname);
        module_object.set_property(String::from("exports"), exports.clone());
        module_object.set_property(String::from("loaded"), JsValue::Bool(true));
        self.cache_module_object(&module.filename, module_object);
        exports
    }

    fn cache_module_object(&mut self, filename: &str, module: JsValue) {
        let cache = self.engine.get_global("__node_require_cache__");
        cache.set_property(String::from(filename), module);
    }

    fn record_resolved_module(&mut self, specifier: &str, filename: &str) {
        let resolved = self.engine.get_global("__node_resolved__");
        resolved.set_property(
            String::from(specifier),
            JsValue::String(String::from(filename)),
        );
    }

    fn module(&mut self, name: &str) -> JsValue {
        self.engine
            .vm()
            .module_registry
            .get(name)
            .cloned()
            .unwrap_or(JsValue::Undefined)
    }

    fn has_pending_js_tasks(&mut self) -> bool {
        let vm = self.engine.vm();
        vm.event_loop.has_microtasks()
            || vm.event_loop.has_pending_timers()
            || modules::net::has_active_servers(vm)
            || modules::http::has_active_servers(vm)
    }

    fn next_js_timer_delay_ms(&mut self) -> Option<u32> {
        let timers = &self.engine.vm().event_loop.timers;
        timers
            .iter()
            .filter(|timer| !timer.cleared)
            .map(|timer| timer.delay_ms.saturating_sub(timer.elapsed_ms))
            .min()
            .or(Some(0))
    }
}

fn js_string_literal(source: &str) -> String {
    let mut out = String::from("\"");
    for ch in source.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push(' '),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn process_global_is_installed() {
        let mut runtime = NodeRuntime::new(NodeOptions::default());
        let value = runtime.eval("process.platform");
        assert_eq!(value.to_js_string(), "anyos");
    }

    #[test]
    fn require_resolves_builtin_modules() {
        let mut runtime = NodeRuntime::new(NodeOptions::default());
        let value = runtime.eval("require('node:process').versions.node");
        assert_eq!(value.to_js_string(), crate::VERSION);
    }

    #[test]
    fn require_reports_missing_modules() {
        let mut runtime = NodeRuntime::new(NodeOptions::default());
        runtime.eval("require('missing-package')");
        assert!(runtime.engine().last_exception().is_some());
    }

    #[test]
    fn commonjs_globals_are_installed_for_scripts() {
        let mut runtime = NodeRuntime::new(NodeOptions::default());
        let value = runtime.run_script("/tmp/app/main.js", "__dirname + ':' + __filename");
        assert_eq!(value.to_js_string(), "/tmp/app:/tmp/app/main.js");
    }

    #[test]
    fn require_loads_relative_commonjs_file() {
        let dir = format!("/tmp/libnode-test-{}", libuv::UvLoop::new().now_ms);
        let _ = anyos_std::fs::mkdir(&dir);
        let dep = format!("{}/dep.js", dir);
        let main = format!("{}/main.js", dir);
        anyos_std::fs::write_bytes(&dep, b"module.exports.answer = 42;").unwrap();

        let mut runtime = NodeRuntime::new(NodeOptions::default());
        let value = runtime.run_script(&main, "require('./dep').answer");
        assert_eq!(value.to_js_string(), "42");

        let _ = anyos_std::fs::unlink(&dep);
        let _ = anyos_std::fs::unlink(&dir);
    }

    #[test]
    fn require_loads_json_file_as_exports() {
        let dir = format!("/tmp/libnode-json-test-{}", libuv::UvLoop::new().now_ms);
        let _ = anyos_std::fs::mkdir(&dir);
        let dep = format!("{}/data.json", dir);
        let main = format!("{}/main.js", dir);
        anyos_std::fs::write_bytes(&dep, br#"{"name":"anyOS","answer":42}"#).unwrap();

        let mut runtime = NodeRuntime::new(NodeOptions::default());
        let value = runtime.run_script(&main, "require('./data.json').name");
        assert_eq!(value.to_js_string(), "anyOS");

        let _ = anyos_std::fs::unlink(&dep);
        let _ = anyos_std::fs::unlink(&dir);
    }

    #[test]
    fn require_resolve_and_cache_are_exposed() {
        let dir = format!("/tmp/libnode-resolve-test-{}", libuv::UvLoop::new().now_ms);
        let _ = anyos_std::fs::mkdir(&dir);
        let dep = format!("{}/dep.js", dir);
        let main = format!("{}/main.js", dir);
        anyos_std::fs::write_bytes(&dep, b"module.exports.answer = 42;").unwrap();

        let mut runtime = NodeRuntime::new(NodeOptions::default());
        let resolved = runtime.run_script(&main, "require.resolve('./dep')");
        let filename = resolved.to_js_string();
        assert!(filename.ends_with("/dep.js"));
        let cache = runtime.engine().get_global("__node_require_cache__");
        let cached = cache.get_property(&filename);
        assert_eq!(cached.get_property("loaded").to_js_string(), "true");

        let _ = anyos_std::fs::unlink(&dep);
        let _ = anyos_std::fs::unlink(&dir);
    }

    #[test]
    fn node_event_loop_runs_timeout_callbacks() {
        let mut runtime = NodeRuntime::new(NodeOptions::default());
        runtime.eval("let done = 0; setTimeout(function(){ done = 7; }, 1);");
        assert_eq!(runtime.run_event_loop(), 1);
        let value = runtime.eval("done");
        assert_eq!(value.to_js_string(), "7");
    }

    #[test]
    fn buffer_module_exposes_global_buffer() {
        let mut runtime = NodeRuntime::new(NodeOptions::default());
        let value = runtime.eval("Buffer.from('abc').toString()");
        assert_eq!(value.to_js_string(), "abc");

        let value = runtime.eval("require('node:buffer').Buffer.isBuffer(Buffer.alloc(2))");
        assert_eq!(value.to_js_string(), "true");
    }

    #[test]
    fn events_module_dispatches_listeners() {
        let mut runtime = NodeRuntime::new(NodeOptions::default());
        let value = runtime.eval(
            "let E = require('events').EventEmitter; \
             let e = new E(); let total = 0; \
             e.on('add', function(v){ total = total + v; }); \
             e.once('add', function(v){ total = total + (v * 10); }); \
             e.emit('add', 2); e.emit('add', 3); total",
        );
        assert_eq!(value.to_js_string(), "25");
    }

    #[test]
    fn timers_module_forwards_to_runtime_timers() {
        let mut runtime = NodeRuntime::new(NodeOptions::default());
        runtime.eval(
            "let timers = require('node:timers'); \
             let done = 0; timers.setTimeout(function(){ done = 9; }, 1);",
        );
        assert_eq!(runtime.run_event_loop(), 1);
        let value = runtime.eval("done");
        assert_eq!(value.to_js_string(), "9");
    }

    #[test]
    fn os_path_and_assert_basics_are_available() {
        let mut runtime = NodeRuntime::new(NodeOptions::default());
        let value = runtime.eval(
            "let os = require('node:os'); \
             let path = require('node:path'); \
             let assert = require('node:assert/strict'); \
             assert.strictEqual(os.platform(), 'anyos'); \
             path.relative('/a/b', '/a/c/d') + ':' + path.parse('/a/b.txt').name",
        );
        assert_eq!(value.to_js_string(), "../c/d:b");
    }
}

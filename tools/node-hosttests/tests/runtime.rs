use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use libnode::{NodeOptions, NodeRuntime};

fn temp_project(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX_EPOCH")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "node-hosttest-{}-{}-{}",
        name,
        std::process::id(),
        unique
    ));
    fs::create_dir_all(&path).expect("failed to create temp project");
    path
}

fn runtime_for(root: &std::path::Path) -> NodeRuntime {
    let mut options = NodeOptions::default();
    options.cwd = root.to_string_lossy().into_owned();
    NodeRuntime::new(options)
}

#[test]
fn commonjs_loads_relative_js_json_and_cache_entries() {
    let root = temp_project("commonjs");
    fs::write(root.join("dep.js"), b"module.exports.answer = 41 + 1;")
        .expect("failed to write dep.js");
    fs::write(
        root.join("config.json"),
        br#"{"name":"anyOS","enabled":true}"#,
    )
    .expect("failed to write config.json");

    let main = root.join("main.js");
    let mut runtime = runtime_for(&root);
    let source = "\
        let depPath = require.resolve('./dep'); \
        let dep = require('./dep'); \
        let config = require('./config.json'); \
        dep.answer + ':' + config.name + ':' + require.cache[depPath].loaded";
    let value = runtime.run_script(&main.to_string_lossy(), source);

    assert_eq!(value.to_js_string(), "42:anyOS:true");
}

#[test]
fn node_modules_package_main_resolution_matches_common_npm_layout() {
    let root = temp_project("node-modules");
    let package_dir = root.join("node_modules").join("demo-pkg");
    fs::create_dir_all(package_dir.join("lib")).expect("failed to create package dirs");
    fs::write(
        package_dir.join("package.json"),
        br#"{"name":"demo-pkg","main":"lib/index.js"}"#,
    )
    .expect("failed to write package.json");
    fs::write(
        package_dir.join("lib").join("index.js"),
        b"module.exports.value = 'from package main';",
    )
    .expect("failed to write package index");

    let main = root.join("src").join("main.js");
    fs::create_dir_all(main.parent().unwrap()).expect("failed to create src");
    let mut runtime = runtime_for(&root);
    let value = runtime.run_script(&main.to_string_lossy(), "require('demo-pkg').value");

    assert_eq!(value.to_js_string(), "from package main");
}

#[test]
fn timers_buffers_events_and_assert_work_together() {
    let root = temp_project("builtins");
    let main = root.join("main.js");
    let mut runtime = runtime_for(&root);
    runtime.run_script(
        &main.to_string_lossy(),
        "\
        let assert = require('node:assert/strict'); \
        let EventEmitter = require('events').EventEmitter; \
        let emitter = new EventEmitter(); \
        let out = ''; \
        emitter.on('done', function(value) { out = Buffer.from(value).toString(); }); \
        setTimeout(function() { emitter.emit('done', 'ok'); }, 1); \
        assert.strictEqual(Buffer.alloc(2).length, 2);",
    );

    assert_eq!(runtime.run_event_loop(), 1);
    let value = runtime.eval("out");
    assert_eq!(value.to_js_string(), "ok");
}

#[test]
fn fs_module_exposes_common_sync_project_scanners() {
    let root = temp_project("fs");
    fs::write(root.join("alpha.txt"), b"alpha").expect("failed to write alpha.txt");
    fs::create_dir(root.join("nested")).expect("failed to create nested directory");

    let main = root.join("main.js");
    let mut runtime = runtime_for(&root);
    let value = runtime.run_script(
        &main.to_string_lossy(),
        "\
        let fs = require('node:fs'); \
        let names = fs.readdirSync(__dirname).sort().join(','); \
        let file = fs.statSync(__dirname + '/alpha.txt'); \
        let dir = fs.statSync(__dirname + '/nested'); \
        names + ':' + file.isFile() + ':' + file.size + ':' + dir.isDirectory()",
    );

    assert_eq!(value.to_js_string(), "alpha.txt,nested:true:5:true");
}

#[test]
fn fs_module_supports_recursive_directory_tooling_patterns() {
    let root = temp_project("fs-recursive");
    let main = root.join("main.js");
    let mut runtime = runtime_for(&root);
    let value = runtime.run_script(
        &main.to_string_lossy(),
        "\
        let fs = require('node:fs'); \
        let base = __dirname + '/generated/cache'; \
        fs.mkdirSync(base, { recursive: true }); \
        fs.writeFileSync(base + '/state.json', '{\"ok\":true}'); \
        let entries = fs.readdirSync(__dirname + '/generated', { withFileTypes: true }); \
        let seen = ''; \
        for (let i = 0; i < entries.length; i++) { \
            seen = seen + entries[i].name + ':' + entries[i].isDirectory(); \
        } \
        fs.rmSync(__dirname + '/generated', { recursive: true }); \
        seen + ':' + fs.existsSync(base)",
    );

    assert_eq!(value.to_js_string(), "cache:true:false");
}

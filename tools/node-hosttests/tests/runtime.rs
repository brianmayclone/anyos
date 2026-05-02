use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

fn fixture_project(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("node-hosttests should live under tools/node-hosttests")
        .to_path_buf()
}

fn node_manifest() -> PathBuf {
    repo_root().join("bin/node/Cargo.toml")
}

fn npm_manifest() -> PathBuf {
    repo_root().join("bin/npm/Cargo.toml")
}

fn run_node(args: &[&str], cwd: &std::path::Path) -> std::process::Output {
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("-q")
        .arg("--manifest-path")
        .arg(node_manifest())
        .arg("--features")
        .arg("host")
        .arg("--")
        .args(args)
        .current_dir(cwd);
    command.output().expect("failed to run anyOS node")
}

fn run_node_with_stdin(
    args: &[&str],
    stdin: &str,
    cwd: &std::path::Path,
) -> std::process::Output {
    let mut child = Command::new("cargo")
        .arg("run")
        .arg("-q")
        .arg("--manifest-path")
        .arg(node_manifest())
        .arg("--features")
        .arg("host")
        .arg("--")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn anyOS node");
    let mut child_stdin = child.stdin.take().expect("stdin should be piped");
    child_stdin
        .write_all(stdin.as_bytes())
        .expect("failed to write node stdin");
    drop(child_stdin);
    child.wait_with_output().expect("failed to wait for node")
}

fn run_npm(args: &[&str], cwd: &std::path::Path) -> std::process::Output {
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("-q")
        .arg("--manifest-path")
        .arg(npm_manifest())
        .arg("--features")
        .arg("host")
        .arg("--")
        .args(args)
        .current_dir(cwd);
    command.output().expect("failed to run anyOS npm")
}

fn copy_dir_recursive(from: &std::path::Path, to: &std::path::Path) {
    fs::create_dir_all(to).expect("failed to create destination directory");
    for entry in fs::read_dir(from).expect("failed to read source directory") {
        let entry = entry.expect("failed to read source entry");
        let source = entry.path();
        let dest = to.join(entry.file_name());
        if source.is_dir() {
            copy_dir_recursive(&source, &dest);
        } else {
            fs::copy(&source, &dest).expect("failed to copy fixture file");
        }
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("failed to bind temp listener")
        .local_addr()
        .expect("failed to read temp listener addr")
        .port()
}

fn wait_for_runtime_io(runtime: &mut NodeRuntime) -> usize {
    for _ in 0..100 {
        let handled = runtime.run_event_loop_once();
        if handled > 0 {
            return handled;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    0
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

#[test]
fn process_next_tick_runs_before_later_event_loop_work() {
    let root = temp_project("next-tick");
    let main = root.join("main.js");
    let mut runtime = runtime_for(&root);
    runtime.run_script(
        &main.to_string_lossy(),
        "\
        var out = ''; \
        process.nextTick(function(value) { out = out + value; }, 'tick'); \
        out",
    );
    runtime.run_event_loop();
    let value = runtime.eval("out");

    assert_eq!(value.to_js_string(), "tick");
}

#[test]
fn node_cli_eval_print_script_args_and_repl_match_core_shapes() {
    let root = temp_project("node-cli");
    let eval = run_node(
        &["-e", "console.log(process.argv.join('|'))", "alpha beta", "gamma"],
        &root,
    );
    assert!(eval.status.success());
    let eval_stdout = String::from_utf8_lossy(&eval.stdout);
    assert!(
        eval_stdout.contains("node|alpha beta|gamma"),
        "stdout: {eval_stdout}"
    );
    assert!(
        !eval_stdout.lines().any(|line| line == "undefined"),
        "node -e should not print eval result: {eval_stdout}"
    );

    let print = run_node(&["-p", "1 + 2"], &root);
    assert!(print.status.success());
    assert!(String::from_utf8_lossy(&print.stdout).contains("3"));

    let script = root.join("main.js");
    fs::write(
        &script,
        b"console.log(process.argv[1].endsWith('main.js') + ':' + process.argv[2]);",
    )
    .expect("failed to write node cli script");
    let script_out = run_node(&[script.to_str().unwrap(), "quoted value"], &root);
    assert!(script_out.status.success());
    assert!(
        String::from_utf8_lossy(&script_out.stdout).contains("true:quoted value"),
        "stdout: {}",
        String::from_utf8_lossy(&script_out.stdout)
    );

    let repl = run_node_with_stdin(&[], "1 + 2\n.exit\n", &root);
    assert!(repl.status.success());
    let repl_stdout = String::from_utf8_lossy(&repl.stdout);
    assert!(repl_stdout.contains("Welcome to anyOS Node.js"));
    assert!(repl_stdout.lines().any(|line| line == "3"));
}

#[test]
fn commonjs_entry_module_is_exposed_as_require_main() {
    let root = temp_project("require-main");
    let helper = root.join("helper.js");
    fs::write(
        &helper,
        b"module.exports = require.main === module;",
    )
    .expect("failed to write helper.js");

    let main = root.join("main.js");
    fs::write(
        &main,
        b"console.log((require.main === module) + ':' + require('./helper'));",
    )
    .expect("failed to write main.js");

    let output = run_node(&[main.to_str().unwrap()], &root);
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("true:false"),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
#[ignore = "downloads packages from the npm registry"]
fn npm_cli_accepts_common_flags_list_and_uninstall() {
    let root = temp_project("npm-cli");
    let init = run_npm(&["init", "-y"], &root);
    assert!(init.status.success());
    assert!(root.join("package.json").exists());

    let install = run_npm(
        &[
            "install",
            "left-pad@1.3.0",
            "--save-exact",
            "--no-audit",
            "--registry",
            libnode::DEFAULT_NPM_REGISTRY,
        ],
        &root,
    );
    assert!(
        install.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&install.stdout),
        String::from_utf8_lossy(&install.stderr)
    );

    let list = run_npm(&["list"], &root);
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("left-pad@1.3.0"));

    let uninstall = run_npm(&["uninstall", "left-pad"], &root);
    assert!(uninstall.status.success());
    let manifest = fs::read_to_string(root.join("package.json")).expect("package.json missing");
    assert!(!manifest.contains("\"left-pad\""), "{manifest}");
}

#[test]
#[ignore = "downloads multiple packages and transitive dependencies from the npm registry"]
fn npm_installs_multi_package_project_and_node_runs_it() {
    let root = temp_project("official-multi-package");
    copy_dir_recursive(&fixture_project("multi-package-app"), &root);

    let install = run_npm(
        &[
            "install",
            "--no-audit",
            "--registry",
            libnode::DEFAULT_NPM_REGISTRY,
        ],
        &root,
    );
    assert!(
        install.status.success(),
        "npm process failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&install.stdout),
        String::from_utf8_lossy(&install.stderr)
    );
    let npm_stdout = String::from_utf8_lossy(&install.stdout);
    assert!(
        npm_stdout.contains("installed packages:"),
        "unexpected npm output: {npm_stdout}"
    );
    assert!(
        root.join("node_modules").join("left-pad").join("package.json").exists(),
        "left-pad package was not installed"
    );
    assert!(
        root.join("node_modules").join("is-odd").join("package.json").exists(),
        "is-odd package was not installed"
    );
    assert!(
        root.join("node_modules")
            .join("is-number")
            .join("package.json")
            .exists(),
        "is-odd transitive dependency is-number was not installed"
    );

    let list = run_npm(&["list"], &root);
    assert!(
        list.status.success(),
        "npm list failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&list.stdout),
        String::from_utf8_lossy(&list.stderr)
    );
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    assert!(list_stdout.contains("left-pad@1.3.0"), "{list_stdout}");
    assert!(list_stdout.contains("is-odd@3.0.1"), "{list_stdout}");

    let script = root.join("src").join("app.js");
    let run = run_node(&[script.to_str().unwrap(), "7"], &root);
    assert!(
        run.status.success(),
        "node process failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("007:true"),
        "stdout:\n{}",
        String::from_utf8_lossy(&run.stdout)
    );
}

#[test]
fn zlib_roundtrips_sync_and_callback_helpers() {
    let root = temp_project("zlib");
    let main = root.join("main.js");
    let mut runtime = runtime_for(&root);
    let value = runtime.run_script(
        &main.to_string_lossy(),
        "\
        let zlib = require('node:zlib'); \
        let gzip = zlib.gzipSync(Buffer.from('hello gzip')); \
        let gunzip = zlib.gunzipSync(gzip).toString(); \
        let deflated = zlib.deflateSync('hello zlib'); \
        let inflated = zlib.inflateSync(deflated).toString(); \
        let raw = zlib.deflateRawSync('hello raw'); \
        let rawInflated = zlib.inflateRawSync(raw).toString(); \
        let asyncResult = ''; \
        zlib.gzip('callback', function(err, data) { \
            asyncResult = err ? err.code : zlib.gunzipSync(data).toString(); \
        }); \
        gunzip + ':' + inflated + ':' + rawInflated + ':' + asyncResult",
    );

    assert_eq!(
        value.to_js_string(),
        "hello gzip:hello zlib:hello raw:callback"
    );
}

#[test]
fn zlib_transform_streams_emit_compressed_data_on_end() {
    let root = temp_project("zlib-stream");
    let main = root.join("main.js");
    let mut runtime = runtime_for(&root);
    let value = runtime.run_script(
        &main.to_string_lossy(),
        "\
        let zlib = require('node:zlib'); \
        let gzip = zlib.createGzip(); \
        let compressed = null; \
        gzip.on('data', function(chunk) { compressed = chunk; }); \
        gzip.end('stream body'); \
        let gunzip = zlib.createGunzip(); \
        let decoded = ''; \
        gunzip.on('data', function(chunk) { decoded = chunk.toString(); }); \
        gunzip.end(compressed); \
        decoded",
    );

    assert_eq!(value.to_js_string(), "stream body");
}

#[test]
fn zlib_inflates_external_dynamic_huffman_streams() {
    let root = temp_project("zlib-dynamic");
    let main = root.join("main.js");
    let mut runtime = runtime_for(&root);
    let value = runtime.run_script(
        &main.to_string_lossy(),
        "\
        let zlib = require('node:zlib'); \
        let chunk = 'abcde'; \
        let expected = ''; \
        for (let i = 0; i < 1000; i++) { expected = expected + chunk; } \
        let zlibBytes = [120,156,237,205,187,1,67,0,20,0,192,222,20,111,53,4,241,75,16,223,76,175,74,151,70,237,110,129,75,211,171,34,251,39,242,159,120,68,17,101,84,207,186,105,187,254,245,30,198,233,51,47,235,182,31,223,228,114,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,187,217,118,2,8,42,110,97]; \
        let rawBytes = zlibBytes.slice(2, zlibBytes.length - 4); \
        let gzipBytes = [31,139,8,0,0,0,0,0,0,3,237,205,187,1,67,0,20,0,192,222,20,111,53,4,241,75,16,223,76,175,74,151,70,237,110,129,75,211,171,34,251,39,242,159,120,68,17,101,84,207,186,105,187,254,245,30,198,233,51,47,235,182,31,223,228,114,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,187,217,118,2,242,109,141,160,224,171,0,0]; \
        rawBytes = [237,205,187,1,67,0,20,0,192,222,20,111,53,4,241,75,16,223,76,175,74,151,70,237,110,129,75,211,171,34,251,39,242,159,120,68,17,101,84,207,186,105,187,254,245,30,198,233,51,47,235,182,31,223,228,114,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,187,217,118,2]; \
        let zlibBytes2 = [120,156]; \
        for (let i = 0; i < rawBytes.length; i++) { zlibBytes2.push(rawBytes[i]); } \
        zlibBytes2.push(8); zlibBytes2.push(42); zlibBytes2.push(110); zlibBytes2.push(97); \
        let gzipBytes2 = [31,139,8,0,0,0,0,0,0,3]; \
        for (let i = 0; i < rawBytes.length; i++) { gzipBytes2.push(rawBytes[i]); } \
        gzipBytes2.push(242); gzipBytes2.push(109); gzipBytes2.push(141); gzipBytes2.push(160); gzipBytes2.push(224); gzipBytes2.push(171); gzipBytes2.push(0); gzipBytes2.push(0); \
        let zlibBytes3 = [120,156,237,205,187,1,67,0,20,0,192,222,20,111,53,4,241,75,16,223,76,175,74,151,70,237,110,129,75,211,171,34,251,39,242,159,120,68,17,101,84,207,186,105,187,254,245,30,198,233,51,47,235,182,31,223,228,114,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,179,217,108,54,155,205,102,187,217,118,2,8,42,110,97]; \
        let rawBytes3 = zlibBytes3.slice(2, zlibBytes3.length - 4); \
        let gzipBytes3 = [31,139,8,0,0,0,0,0,0,3]; \
        for (let i = 0; i < rawBytes3.length; i++) { gzipBytes3.push(rawBytes3[i]); } \
        gzipBytes3.push(242); gzipBytes3.push(109); gzipBytes3.push(141); gzipBytes3.push(160); gzipBytes3.push(224); gzipBytes3.push(171); gzipBytes3.push(0); gzipBytes3.push(0); \
        let zlibBytes4 = [120,218,237,196,57,1,0,0,8,4,160,172,231,211,191,130,41,220,96,32,213,179,145,36,73,146,36,73,146,244,217,1,198,122,142,2]; \
        let rawBytes4 = zlibBytes4.slice(2, zlibBytes4.length - 4); \
        let gzipBytes4 = [31,139,8,0,0,0,0,0,0,3]; \
        for (let i = 0; i < rawBytes4.length; i++) { gzipBytes4.push(rawBytes4[i]); } \
        gzipBytes4.push(198); gzipBytes4.push(17); gzipBytes4.push(4); gzipBytes4.push(75); gzipBytes4.push(136); gzipBytes4.push(19); gzipBytes4.push(0); gzipBytes4.push(0); \
        let inflated = zlib.inflateSync(Buffer.from(zlibBytes4)).toString(); \
        let rawInflated = zlib.inflateRawSync(Buffer.from(rawBytes4)).toString(); \
        let gunzipped = zlib.gunzipSync(Buffer.from(gzipBytes4)).toString(); \
        (inflated === expected) + ':' + (rawInflated === expected) + ':' + (gunzipped === expected) + ':' + inflated.length",
    );

    assert_eq!(value.to_js_string(), "true:true:true:5000");
}

#[test]
fn http_server_handles_real_localhost_request() {
    let root = temp_project("http-server");
    let main = root.join("main.js");
    let port = free_port();
    let mut runtime = runtime_for(&root);
    runtime.run_script(
        &main.to_string_lossy(),
        &format!(
            "\
            let http = require('node:http'); \
            let server = http.createServer(function(req, res) {{ \
                res.setHeader('content-type', 'text/plain'); \
                res.end(req.method + ':' + req.url); \
            }}); \
            server.listen({});",
            port
        ),
    );

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("failed to connect server");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("failed to set read timeout");
    stream
        .write_all(b"GET /hello HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("failed to write request");
    assert!(wait_for_runtime_io(&mut runtime) > 0, "server did not accept request");

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("failed to read response");
    assert!(response.contains("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with("GET:/hello"), "{response}");
}

#[test]
#[ignore = "downloads Express and transitive packages from the npm registry"]
fn npm_installs_official_express_and_node_loads_package() {
    let root = temp_project("official-express");
    copy_dir_recursive(&fixture_project("express-app"), &root);

    let npm_manifest = repo_root().join("bin/npm/Cargo.toml");
    let output = Command::new("cargo")
        .arg("run")
        .arg("-q")
        .arg("--manifest-path")
        .arg(npm_manifest)
        .arg("--features")
        .arg("host")
        .arg("--")
        .arg("install")
        .current_dir(&root)
        .output()
        .expect("failed to run anyOS npm");
    assert!(
        output.status.success(),
        "npm process failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let npm_stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        npm_stdout.contains("installed packages:"),
        "unexpected npm output: {npm_stdout}"
    );
    assert!(
        root.join("node_modules").join("express").join("package.json").exists(),
        "official express package was not installed"
    );

    let main = root.join("src").join("server.js");
    let mut runtime = runtime_for(&root);
    let value = runtime.run_script(
        &main.to_string_lossy(),
        "\
        let express = require('express'); \
        let packageInfo = require('express/package.json'); \
        typeof express + ':' + packageInfo.name + ':' + packageInfo.version",
    );
    let exception = runtime
        .engine()
        .last_exception()
        .map(|value| value.to_js_string())
        .unwrap_or_else(|| String::from("<none>"));
    assert_eq!(
        value.to_js_string(),
        "function:express:4.18.2",
        "exception: {exception}"
    );
}

#[test]
#[ignore = "downloads Express and transitive packages from the npm registry"]
fn npm_installs_official_express_and_serves_http_route() {
    let root = temp_project("official-express-server");
    copy_dir_recursive(&fixture_project("express-app"), &root);

    let npm_manifest = repo_root().join("bin/npm/Cargo.toml");
    let output = Command::new("cargo")
        .arg("run")
        .arg("-q")
        .arg("--manifest-path")
        .arg(npm_manifest)
        .arg("--features")
        .arg("host")
        .arg("--")
        .arg("install")
        .current_dir(&root)
        .output()
        .expect("failed to run anyOS npm");
    assert!(
        output.status.success(),
        "npm process failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let port = free_port();
    let main = root.join("src").join("server.js");
    let mut runtime = runtime_for(&root);
    runtime.run_script(
        &main.to_string_lossy(),
        &format!(
            "\
            const server = require('./server'); \
            const app = server.createApp(); \
            const httpServer = app.listen({});",
            port
        ),
    );
    let exception = runtime
        .engine()
        .last_exception()
        .map(|value| value.to_js_string())
        .unwrap_or_else(|| String::from("<none>"));
    assert_eq!(exception, "<none>");

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("failed to connect server");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("failed to set read timeout");
    stream
        .write_all(b"GET /hello HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("failed to write request");
    assert!(wait_for_runtime_io(&mut runtime) > 0, "server did not accept request");

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("failed to read response");
    assert!(response.contains("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with("hello express:GET:/hello"), "{response}");
}

#[test]
fn net_create_connection_can_write_to_host_listener() {
    let root = temp_project("net-client");
    let main = root.join("main.js");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to bind host listener");
    let port = listener.local_addr().unwrap().port();
    let mut runtime = runtime_for(&root);
    let value = runtime.run_script(
        &main.to_string_lossy(),
        &format!(
            "\
            let net = require('node:net'); \
            let socket = net.createConnection({}); \
            socket.write('ping'); \
            socket.end(); \
            'sent';",
            port
        ),
    );

    assert_eq!(value.to_js_string(), "sent");
    let (mut stream, _) = listener.accept().expect("failed to accept net client");
    let mut received = String::new();
    stream
        .read_to_string(&mut received)
        .expect("failed to read client payload");
    assert_eq!(received, "ping");
}

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use libnode::npm::{PackageInstaller, PackageSpec, RegistryConfig};
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

fn system_node_modules_dir() -> PathBuf {
    repo_root()
        .join("sysroot")
        .join("System")
        .join("Library")
        .join("node_modules")
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

fn run_node_with_stdin(args: &[&str], stdin: &str, cwd: &std::path::Path) -> std::process::Output {
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
fn anyos_anyui_module_creates_native_control_bridge_objects() {
    let root = temp_project("anyui-bridge");
    let main = root.join("main.js");
    let mut runtime = runtime_for(&root);
    runtime.run_script(
        &main.to_string_lossy(),
        "\
        const ui = require('@anyos/anyui'); \
        const win = new ui.Window('Designer Preview', -1, -1, 320, 200); \
        const button = new ui.Button('OK'); \
        const label = new ui.Label('Ready'); \
        win.add(button).add(label); \
        button.setPosition(12, 16).setSize(80, 24).setColor('#FF112233').setText('Run'); \
        const eventChain = button.onClick((event) => { globalThis.lastEventId = event.id; }); \
        label.setDock(ui.DOCK_TOP).setMargin(1, 2, 3, 4).setPadding(4, 3, 2, 1); \
        const input = new ui.TextField(); \
        input.setPlaceholder('Name').setPasswordMode(false).setReadOnly(false).setMaxLength(40); \
        input.setCursor(0).setSelection(0, 0).selectAll(); \
        const combo = new ui.ComboBox(); \
        combo.setItems('Debug|Release').setPlaceholder('Profile').setSelectedIndex(null).setEditable(true); \
        const scroll = new ui.ScrollView(); \
        scroll.setScrollOffsets(4, 8).setDropTarget(true).setDraggable(false); \
        globalThis.out = [
            typeof ui.run,
            typeof button.setDock,
            eventChain === button,
            typeof button.onChanged,
            typeof input.setPlaceholder,
            typeof combo.setSelectedIndex,
            typeof scroll.onDrop,
            button.__anyuiKind,
            button.__anyuiId > 0,
            label.__anyuiKind,
            label.__anyuiId > 0,
        ].join(':');",
    );

    runtime.run_event_loop();
    let value = runtime.eval("out");
    assert_eq!(
        value.to_js_string(),
        "function:function:true:function:function:function:function:Button:true:Label:true"
    );
}

#[test]
fn anyos_system_packages_are_visible_to_node_resolution() {
    let root = temp_project("anyos-system-packages");
    std::env::set_var("ANYOS_NODE_SYSTEM_PACKAGES", system_node_modules_dir());
    let main = root.join("main.js");
    let mut runtime = runtime_for(&root);
    let value = runtime.run_script(
        &main.to_string_lossy(),
        "\
        const pkg = require('@anyos/anyui/package.json'); \
        const ui = require('@anyos/anyui'); \
        const viaPackage = require('@anyos/anyui'); \
        [pkg.name, pkg.version, typeof ui.Window, typeof viaPackage.Button].join(':');",
    );

    assert_eq!(value.to_js_string(), "@anyos/anyui:0.1.0:function:function");
}

#[test]
fn npm_installs_anyos_system_packages_without_registry_fetch() {
    let root = temp_project("anyos-system-npm");
    std::env::set_var("ANYOS_NODE_SYSTEM_PACKAGES", system_node_modules_dir());
    let installer = PackageInstaller::new(RegistryConfig::default());
    let report = installer
        .install_package_result(
            root.to_str().unwrap(),
            &PackageSpec::parse("@anyos/anyui@0.1.0"),
        )
        .expect("system package install should succeed");

    assert_eq!(report.installed.len(), 1);
    assert_eq!(report.installed[0].name, "@anyos/anyui");
    assert!(root
        .join("node_modules")
        .join("@anyos")
        .join("anyui")
        .join("index.d.ts")
        .exists());

    let main = root.join("main.js");
    let mut runtime = runtime_for(&root);
    let value = runtime.run_script(
        &main.to_string_lossy(),
        "require('@anyos/anyui/package.json').name + ':' + typeof require('@anyos/anyui').Window",
    );
    assert_eq!(value.to_js_string(), "@anyos/anyui:function");
}

#[test]
fn anycode_generated_standard_node_ui_app_runs_from_disk() {
    let root = temp_project("anycode-node-ui-app");
    let ui_dir = root.join("src").join("ui");
    let form_dir = ui_dir.join("main_form");
    fs::create_dir_all(root.join("src").join("types")).expect("failed to create src/types");
    fs::create_dir_all(&form_dir).expect("failed to create form dir");

    fs::write(
        root.join("package.json"),
        r#"{
  "name": "anycode-standard-ui-app",
  "version": "0.1.0",
  "type": "commonjs",
  "private": true,
  "main": "src/main.js",
  "scripts": {
    "start": "node src/main.js",
    "lint": "eslint src",
    "test": "node src/main.js --self-test"
  },
  "dependencies": {},
  "devDependencies": {
    "eslint": "^8.57.1"
  }
}
"#,
    )
    .expect("failed to write package.json");
    fs::write(
        root.join("jsconfig.json"),
        r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "CommonJS",
    "checkJs": true,
    "allowSyntheticDefaultImports": true,
    "baseUrl": "."
  },
  "include": [
    "src/**/*.js",
    "src/types/**/*.d.ts"
  ]
}
"#,
    )
    .expect("failed to write jsconfig.json");
    fs::write(
        root.join(".eslintrc.json"),
        r#"{
  "env": {
    "es2022": true,
    "node": true
  },
  "extends": "eslint:recommended",
  "parserOptions": {
    "ecmaVersion": "latest",
    "sourceType": "script"
  },
  "rules": {
    "no-unused-vars": ["warn", { "argsIgnorePattern": "^_" }],
    "no-undef": "error"
  }
}
"#,
    )
    .expect("failed to write eslint config");
    fs::write(
        root.join("src/types/anyos-anyui.d.ts"),
        r#"declare module "@anyos/anyui" {
  export const DOCK_FILL: number;
  export function run(): void;
  export class Control {
    add(child: Control): this;
    setPosition(x: number, y: number): this;
    setSize(width: number, height: number): this;
    setColor(color: number | string): this;
    setText(text: string): this;
    setDock(dock: number): this;
  }
  export class View extends Control {}
  export class Button extends Control { constructor(text?: string); }
  export class Window extends Control { constructor(title: string, x: number, y: number, width: number, height: number); }
  export const theme: { colors(): { editorBg: number } };
}
"#,
    )
    .expect("failed to write type declarations");

    let designer_path = form_dir.join("MainForm.Designer");
    fs::write(
        &designer_path,
        "anycode-designer-v1\nform name=\"MainForm\" title=\"MainForm\" width=640 height=420\ncontrol kind=\"Label\" name=\"title\" text=\"Title\" x=24 y=24 width=220 height=24\ncontrol kind=\"Button\" name=\"ok\" text=\"OK\" x=24 y=64 width=96 height=28 event=\"OnClick\"\n",
    )
    .expect("failed to write designer metadata");
    fs::write(
        &form_dir.join("designer.js"),
        r#"// Generated by anyCode Designer. Do not edit by hand.
const ui = require('@anyos/anyui');

class MainFormUi {
  static build() {
    const root = new ui.View();
    root.setSize(640, 420);
    root.setColor(ui.theme.colors().editorBg);
    const title = new ui.Label('Title');
    title.setPosition(24, 24);
    title.setSize(220, 24);
    const ok = new ui.Button('OK');
    ok.setPosition(24, 64);
    ok.setSize(96, 28);
    root.add(title);
    root.add(ok);
    return {
      root: root,
      title: title,
      ok: ok,
    };
  }
}

module.exports = { MainFormUi: MainFormUi };
"#,
    )
    .expect("failed to write designer.js");
    fs::write(
        &form_dir.join("events.js"),
        r#"function ok_onClick() {
  // TODO: handle event
}

module.exports = {
  ok_onClick: ok_onClick
};
"#,
    )
    .expect("failed to write events.js");
    fs::write(
        &form_dir.join("view.js"),
        r#"const designer = require('./designer');
const events = require('./events');

class MainForm {
  constructor() {
    this.ui = designer.MainFormUi.build();
    this.ui.ok.onClick(() => events.ok_onClick());
  }

  root() {
    return this.ui.root;
  }
}

module.exports = MainForm;
module.exports.MainForm = MainForm;
"#,
    )
    .expect("failed to write view.js");
    fs::write(
        &form_dir.join("index.js"),
        "module.exports = require('./view');\n",
    )
    .expect("failed to write index.js");

    let storyboard_path = ui_dir.join("Main.Storyboard");
    let designer_path_text = designer_path.to_string_lossy();
    fs::write(
        &storyboard_path,
        format!(
            "anycode-storyboard-v1\nscene form=\"MainForm\" designer=\"{}\" x=48 y=48\n",
            designer_path_text
        ),
    )
    .expect("failed to write storyboard metadata");
    fs::write(
        ui_dir.join("Main.Storyboard.Designer.js"),
        format!(
            r#"// Generated by anyCode Storyboard Designer. Do not edit by hand.

const STORYBOARD_NAME = "Main";

const SCENES = [
  {{ formName: "MainForm", designerPath: "{}", x: 48, y: 48 }},
];

const SEGUES = [
];

module.exports = {{ STORYBOARD_NAME: STORYBOARD_NAME, SCENES: SCENES, SEGUES: SEGUES }};
"#,
            designer_path_text
        ),
    )
    .expect("failed to write storyboard designer js");
    fs::write(
        ui_dir.join("Main.Storyboard.codebehind.js"),
        r#"// Generated by anyCode Storyboard Designer. Keep custom logic in hooks.

function storyboardTarget(segueId) {
  switch (segueId) {
    default: return null;
  }
}

function storyboardCanNavigate(segueId) {
  switch (segueId) {
    default: return false;
  }
}

function storyboardNavigate(formName) {
  void formName;
  // TODO: connect this to the application navigation host.
}

module.exports = { storyboardTarget: storyboardTarget, storyboardCanNavigate: storyboardCanNavigate, storyboardNavigate: storyboardNavigate };
"#,
    )
    .expect("failed to write storyboard codebehind js");
    fs::write(
        root.join("src/main.js"),
        format!(
            r#"const ui = require('@anyos/anyui');
const MainFormModule = require('./ui/main_form');
const MainForm = resolveFormConstructor(MainFormModule, "MainForm");

const STARTUP_STORYBOARD = "{}";

function resolveFormConstructor(module, formName) {{
  let candidate = module && (module[formName] || module.default || module);
  for (let i = 0; i < 4 && candidate && typeof candidate !== 'function'; i++) {{
    candidate = candidate[formName] || candidate.default || candidate;
  }}
  if (typeof candidate !== 'function') {{
    throw new TypeError("Generated form '" + formName + "' did not export a constructor");
  }}
  return candidate;
}}

function main() {{
  const form = new MainForm();
  const root = form.root();
  if (!root) {{
    throw new Error("Generated form 'MainForm' did not return a root view");
  }}
  root.setDock(ui.DOCK_FILL);

  const win = new ui.Window("MainForm", -1, -1, 640, 420);
  win.add(root);

  void STARTUP_STORYBOARD;
  ui.run();
}}

main();
"#,
            storyboard_path.to_string_lossy()
        ),
    )
    .expect("failed to write main.js");

    let main = root.join("src/main.js");
    let output = run_node(&[main.to_str().unwrap()], &root);
    assert!(
        output.status.success(),
        "generated anyCode Node UI app failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn anycode_startup_accepts_nested_commonjs_form_export_wrappers() {
    let root = temp_project("anycode-nested-form-export");
    fs::create_dir_all(root.join("src").join("ui").join("main_form"))
        .expect("failed to create form module");
    fs::write(
        root.join("src")
            .join("ui")
            .join("main_form")
            .join("index.js"),
        r#"class MainForm {
  constructor() {
    this.created = true;
  }
  root() {
    return {
      setDock: function(_) {}
    };
  }
}

module.exports = { MainForm: { MainForm: MainForm } };
"#,
    )
    .expect("failed to write nested wrapper form");
    fs::write(
        root.join("src").join("main.js"),
        r#"const ui = require('@anyos/anyui');
const MainFormModule = require('./ui/main_form');
const MainForm = resolveFormConstructor(MainFormModule, "MainForm");

function resolveFormConstructor(module, formName) {
  let candidate = module && (module[formName] || module.default || module);
  for (let i = 0; i < 4 && candidate && typeof candidate !== 'function'; i++) {
    candidate = candidate[formName] || candidate.default || candidate;
  }
  if (typeof candidate !== 'function') {
    throw new TypeError("Generated form '" + formName + "' did not export a constructor");
  }
  return candidate;
}

const form = new MainForm();
const root = form.root();
root.setDock(ui.DOCK_FILL);
"#,
    )
    .expect("failed to write main.js");

    let output = run_node(&[root.join("src/main.js").to_str().unwrap()], &root);
    assert!(
        output.status.success(),
        "nested CommonJS form wrapper should be accepted\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn node_web_compat_globals_cover_modern_sdk_primitives() {
    let root = temp_project("web-compat");
    let main = root.join("main.js");
    let mut runtime = runtime_for(&root);
    runtime.run_script(
        &main.to_string_lossy(),
        "\
        let headers = new Headers({ 'x-sdk': 'yes' }); \
        headers.append('x-sdk', 'again'); \
        let req = new Request('https://example.invalid/v1', { method: 'post', headers }); \
        let controller = new AbortController(); \
        controller.abort('stop'); \
        let blob = new Blob(['open', 'ai'], { type: 'text/plain' }); \
        let form = new FormData(); \
        form.set('file', blob); \
        var out = ''; \
        new Response('{\"ok\":true}', { status: 201, headers: { 'content-type': 'application/json' } }) \
            .text() \
            .then(function(body) { \
                out = typeof fetch + ':' + (global === globalThis) + ':' + \
                    req.method + ':' + req.headers.get('x-sdk') + ':' + \
                    controller.signal.aborted + ':' + blob.size + ':' + \
                    form.has('file') + ':' + body; \
            });",
    );

    runtime.run_event_loop();
    let value = runtime.eval("out");
    assert_eq!(
        value.to_js_string(),
        "function:true:POST:yes, again:true:6:true:{\"ok\":true}"
    );
}

#[test]
fn fetch_performs_real_http_request_with_method_headers_and_body() {
    let root = temp_project("fetch-real-http");
    let main = root.join("main.js");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to bind host listener");
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("failed to accept fetch client");
        let mut request = String::new();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("failed to set read timeout");
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).expect("failed to read request");
        request.push_str(&String::from_utf8_lossy(&buf[..n]));
        let lower = request.to_ascii_lowercase();
        assert!(request.starts_with("POST /ai HTTP/1.1"), "{request}");
        assert!(lower.contains("authorization: bearer test"), "{request}");
        assert!(
            lower.contains("content-type: application/json"),
            "{request}"
        );
        assert!(request.ends_with(r#"{"q":"codex"}"#), "{request}");
        stream
            .write_all(
                b"HTTP/1.1 202 Accepted\r\ncontent-type: application/json\r\nx-request-id: req_123\r\ncontent-length: 11\r\nconnection: close\r\n\r\n{\"ok\":true}",
            )
            .expect("failed to write response");
    });

    let mut runtime = runtime_for(&root);
    runtime.run_script(
        &main.to_string_lossy(),
        &format!(
            "\
            var out = ''; \
            fetch('http://127.0.0.1:{}/ai', {{ \
                method: 'POST', \
                headers: {{ 'content-type': 'application/json', authorization: 'Bearer test' }}, \
                body: '{{\"q\":\"codex\"}}' \
            }}).then(function(res) {{ \
                return res.text().then(function(body) {{ \
                    out = res.status + ':' + res.ok + ':' + res.headers.get('x-request-id') + ':' + body; \
                }}); \
            }});",
            port
        ),
    );

    runtime.run_event_loop();
    server.join().expect("fetch server thread failed");
    let value = runtime.eval("out");
    assert_eq!(value.to_js_string(), r#"202:true:req_123:{"ok":true}"#);
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
        &[
            "-e",
            "console.log(process.argv.join('|'))",
            "alpha beta",
            "gamma",
        ],
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
    fs::write(&helper, b"module.exports = require.main === module;")
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
fn node_stack_trace_includes_entry_script_name() {
    let root = temp_project("node-stack");
    let script = root.join("main.js");
    let source =
        "function first(){ second(); }\nfunction second(){ throw new Error('boom'); }\nfirst();";
    let mut runtime = runtime_for(&root);
    runtime.run_script(script.to_str().unwrap(), source);
    let exception = runtime
        .engine()
        .last_exception()
        .expect("expected thrown exception")
        .clone();
    let stack = exception.get_property("stack").to_js_string();
    assert!(stack.contains("Error: boom"), "{stack}");
    assert!(stack.contains("at second"), "{stack}");
    assert!(stack.contains("at first"), "{stack}");
    assert!(
        stack.contains(script.to_str().unwrap()),
        "stack should include source script path:\n{stack}"
    );
}

#[test]
fn npm_manifest_reads_dev_dependencies_by_default() {
    let manifest = libnode::npm::PackageManifest::parse_or_new(Some(String::from(
        r#"{
  "dependencies": {
    "left-pad": "1.3.0"
  },
  "devDependencies": {
    "eslint": "^9.0.0"
  }
}"#,
    )));
    let prod = manifest.manifest_dependencies(false);
    assert_eq!(prod.len(), 1);
    assert_eq!(prod[0].name, "left-pad");

    let all = manifest.manifest_dependencies(true);
    assert_eq!(all.len(), 2);
    assert!(all.iter().any(|dep| dep.name == "left-pad"));
    assert!(all.iter().any(|dep| dep.name == "eslint"));
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
        root.join("node_modules")
            .join("left-pad")
            .join("package.json")
            .exists(),
        "left-pad package was not installed"
    );
    assert!(
        root.join("node_modules")
            .join("is-odd")
            .join("package.json")
            .exists(),
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
#[ignore = "downloads larger CommonJS packages from the npm registry"]
fn npm_installs_larger_packages_and_node_uses_their_apis() {
    let root = temp_project("official-big-package");
    copy_dir_recursive(&fixture_project("big-package-app"), &root);

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
    assert!(
        root.join("node_modules")
            .join("lodash")
            .join("package.json")
            .exists(),
        "lodash package was not installed"
    );
    assert!(
        root.join("node_modules")
            .join("moment")
            .join("package.json")
            .exists(),
        "moment package was not installed"
    );

    let script = root.join("src").join("app.js");
    let run = run_node(&[script.to_str().unwrap()], &root);
    assert!(
        run.status.success(),
        "node process failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("7:anyOsNodeRuntime:2026-05-02"),
        "stdout:\n{}",
        String::from_utf8_lossy(&run.stdout)
    );
}

#[test]
#[ignore = "downloads nodemon, ESLint and transitive packages from the npm registry"]
fn npm_installs_node_tooling_packages_and_node_uses_them() {
    let root = temp_project("official-tooling-packages");
    copy_dir_recursive(&fixture_project("tooling-app"), &root);

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
        root.join("node_modules")
            .join("eslint")
            .join("package.json")
            .exists(),
        "eslint package was not installed"
    );
    assert!(
        root.join("node_modules")
            .join("nodemon")
            .join("package.json")
            .exists(),
        "nodemon package was not installed"
    );
    assert!(
        root.join("node_modules")
            .join(".bin")
            .join("eslint")
            .exists(),
        "eslint .bin shim was not generated"
    );
    assert!(
        root.join("node_modules")
            .join(".bin")
            .join("nodemon")
            .exists(),
        "nodemon .bin shim was not generated"
    );

    let eslint_script = root.join("src").join("eslint-check.js");
    let eslint = run_node(&[eslint_script.to_str().unwrap()], &root);
    assert!(
        eslint.status.success(),
        "eslint check failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&eslint.stdout),
        String::from_utf8_lossy(&eslint.stderr)
    );
    assert!(
        String::from_utf8_lossy(&eslint.stdout).contains("eslint:8.57.1:all,recommended"),
        "stdout:\n{}",
        String::from_utf8_lossy(&eslint.stdout)
    );

    let nodemon_script = root.join("src").join("nodemon-check.js");
    let nodemon = run_node(&[nodemon_script.to_str().unwrap()], &root);
    assert!(
        nodemon.status.success(),
        "nodemon check failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&nodemon.stdout),
        String::from_utf8_lossy(&nodemon.stderr)
    );
    assert!(
        String::from_utf8_lossy(&nodemon.stdout).contains("rs:app.js:src:js,json"),
        "stdout:\n{}",
        String::from_utf8_lossy(&nodemon.stdout)
    );
}

#[test]
#[ignore = "downloads a package with bins from the npm registry"]
fn npm_global_install_creates_prefix_bin_link() {
    let root = temp_project("npm-global-cwd");
    let prefix = temp_project("npm-global-prefix");

    let install = run_npm(
        &[
            "install",
            "-g",
            "nodemon@3.1.10",
            "--no-audit",
            "--prefix",
            prefix.to_str().unwrap(),
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
    assert!(
        prefix
            .join("Library")
            .join("node_modules")
            .join("nodemon")
            .join("package.json")
            .exists(),
        "global nodemon package was not installed"
    );
    assert!(
        prefix.join("bin").join("nodemon").exists(),
        "global nodemon link was not created"
    );
}

#[test]
#[ignore = "downloads the OpenAI Codex CLI and platform packages from the npm registry"]
fn npm_global_install_openai_codex_and_node_runs_cli_help() {
    let root = temp_project("npm-codex-cwd");
    let prefix = temp_project("npm-codex-prefix");

    let install = run_npm(
        &[
            "install",
            "-g",
            "@openai/codex@0.128.0",
            "--no-audit",
            "--prefix",
            prefix.to_str().unwrap(),
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

    let codex_js = prefix
        .join("Library")
        .join("node_modules")
        .join("@openai")
        .join("codex")
        .join("bin")
        .join("codex.js");
    assert!(codex_js.exists(), "codex.js was not installed");
    assert!(
        prefix
            .join("Library")
            .join("node_modules")
            .join("@openai")
            .join("codex-linux-x64")
            .join("package.json")
            .exists(),
        "linux-x64 optional package was not installed"
    );

    let run = run_node(&[codex_js.to_str().unwrap(), "--help"], &root);
    assert!(
        run.status.success(),
        "codex --help failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("Codex CLI"),
        "stdout:\n{}",
        String::from_utf8_lossy(&run.stdout)
    );
}

#[test]
#[ignore = "downloads OpenAI and Anthropic SDK packages from the npm registry"]
fn npm_installs_ai_sdk_packages_and_node_uses_them() {
    let root = temp_project("official-ai-sdk-packages");
    copy_dir_recursive(&fixture_project("ai-sdk-app"), &root);

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
    assert!(
        root.join("node_modules")
            .join("openai")
            .join("package.json")
            .exists(),
        "openai package was not installed"
    );
    assert!(
        root.join("node_modules")
            .join("@anthropic-ai")
            .join("sdk")
            .join("package.json")
            .exists(),
        "@anthropic-ai/sdk package was not installed"
    );
    assert!(
        root.join("node_modules")
            .join(".bin")
            .join("openai")
            .exists(),
        "openai .bin shim was not generated"
    );

    let script = root.join("src").join("app.js");
    let run = run_node(&[script.to_str().unwrap()], &root);
    assert!(
        run.status.success(),
        "node process failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains(
            "openai:4.104.0:function:function:function:object:anthropic:0.39.0:function:function:function:object:function:function:function:function:function"
        ),
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
    assert!(
        wait_for_runtime_io(&mut runtime) > 0,
        "server did not accept request"
    );

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
        root.join("node_modules")
            .join("express")
            .join("package.json")
            .exists(),
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
    assert!(
        wait_for_runtime_io(&mut runtime) > 0,
        "server did not accept request"
    );

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

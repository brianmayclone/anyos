//! tc39_runner — runs test262-compatible .js files through the libjs engine.
//!
//! Usage:
//!   tc39_runner <dir-or-file> [--verbose] [--bail]
//!
//! The runner injects a minimal test262 harness before each test:
//!   - assert(condition, message)
//!   - assert.sameValue(actual, expected, message)
//!   - assert.notSameValue(actual, unexpected, message)
//!   - assert.throws(ErrorType, fn, message)
//!   - print(msg)   (alias for console.log)
//!
//! Tests are considered PASSED when they run without throwing.
//! A test is FAILED when an uncaught exception is produced.
//!
//! Metadata comments (/*--- ... ---*/) are parsed for:
//!   - description
//!   - flags: [onlyStrict, module, raw, ...]  (unsupported flags skip the test)
//!   - negative: { type: ... }                (expects a specific error)

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::time::Instant;

// Pull in the engine from our lib crate.
use libjs_tests::{JsEngine, JsValue};

// ── harness source injected before every test ─────────────────────────────────

const HARNESS_JS: &str = r#"
var Test262Error = (function() {
    function Test262Error(msg) {
        this.message = msg || '';
        this.name    = 'Test262Error';
    }
    Test262Error.prototype = Object.create(Error.prototype);
    Test262Error.prototype.constructor = Test262Error;
    return Test262Error;
})();

function assert(value, message) {
    if (!value) {
        throw new Test262Error(message || 'assertion failed: ' + String(value));
    }
}

assert.sameValue = function(actual, expected, message) {
    var pass;
    if (actual !== actual && expected !== expected) {
        pass = true; // both NaN
    } else {
        pass = (actual === expected);
    }
    if (!pass) {
        throw new Test262Error(
            (message ? message + ' — ' : '') +
            'Expected ' + String(expected) + ' but got ' + String(actual)
        );
    }
};

assert.notSameValue = function(actual, unexpected, message) {
    var same;
    if (actual !== actual && unexpected !== unexpected) {
        same = true;
    } else {
        same = (actual === unexpected);
    }
    if (same) {
        throw new Test262Error(
            (message ? message + ' — ' : '') +
            'Expected a value !== ' + String(unexpected)
        );
    }
};

assert.throws = function(expectedError, fn, message) {
    var thrown = false;
    var actual;
    try {
        fn();
    } catch(e) {
        thrown = true;
        actual = e;
    }
    if (!thrown) {
        throw new Test262Error(
            (message ? message + ' — ' : '') +
            'Expected a ' + String(expectedError) + ' to be thrown but nothing was thrown'
        );
    }
};

function print(msg) { console.log(msg); }
var $262 = { createRealm: function(){}, evalScript: function(s){ return undefined; } };
"#;

// ── metadata parsing ──────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct TestMeta {
    description: String,
    flags: Vec<String>,
    negative: Option<String>,
    includes: Vec<String>,
    es_id: String,
}

fn parse_meta(source: &str) -> TestMeta {
    let mut meta = TestMeta::default();
    let start = match source.find("/*---") {
        Some(i) => i + 5,
        None => return meta,
    };
    let end = match source[start..].find("---*/") {
        Some(i) => start + i,
        None => return meta,
    };
    let block = &source[start..end];

    for line in block.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("description:") {
            meta.description = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("esid:") {
            meta.es_id = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("flags:") {
            // flags: [onlyStrict, module, raw]
            let inner = rest.trim().trim_start_matches('[').trim_end_matches(']');
            for f in inner.split(',') {
                let f = f.trim();
                if !f.is_empty() {
                    meta.flags.push(f.to_string());
                }
            }
        } else if let Some(rest) = line.strip_prefix("includes:") {
            let inner = rest.trim().trim_start_matches('[').trim_end_matches(']');
            for inc in inner.split(',') {
                let inc = inc.trim().trim_matches('"').trim_matches('\'');
                if !inc.is_empty() {
                    meta.includes.push(inc.to_string());
                }
            }
        } else if line.starts_with("negative:") || line.starts_with("  type:") {
            if let Some(rest) = line.strip_prefix("  type:") {
                meta.negative = Some(rest.trim().to_string());
            }
        }
    }
    meta
}

/// Flags that require engine features we don't support yet → skip.
const SKIP_FLAGS: &[&str] = &[
    "module",
    "async",
    "generated",
];

// ── test execution ────────────────────────────────────────────────────────────

#[derive(Debug)]
enum Outcome {
    Pass,
    Fail(String),
    Skip(String),
}

fn run_test(path: &Path, verbose: bool) -> Outcome {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return Outcome::Fail(format!("read error: {e}")),
    };

    let meta = parse_meta(&source);

    // Skip tests that require unsupported flags
    for flag in &meta.flags {
        if SKIP_FLAGS.contains(&flag.as_str()) {
            return Outcome::Skip(format!("flag: {flag}"));
        }
    }

    // Skip tests that include harness files we don't support yet
    for inc in &meta.includes {
        if inc != "assert.js" && inc != "sta.js" && !inc.is_empty() {
            return Outcome::Skip(format!("include: {inc}"));
        }
    }

    let mut engine = JsEngine::new();
    engine.set_step_limit(2_000_000);

    // 1) Inject harness
    engine.eval(HARNESS_JS);

    // 2) Run the test source
    let combined = format!("{HARNESS_JS}\n{source}");
    let _ = engine; // drop old engine, create fresh one with combined source
    let mut engine = JsEngine::new();
    engine.set_step_limit(2_000_000);

    // We need to detect thrown errors. Run the test and check console_error or
    // use a wrapper that catches at top level.
    let wrapped = format!(
        r#"
var __test_passed = false;
var __test_error  = null;
try {{
{harness}
{source}
__test_passed = true;
}} catch(__e) {{
    __test_error = __e && __e.message ? __e.message : String(__e);
}}
"#,
        harness = HARNESS_JS,
        source = source
    );

    engine.eval(&wrapped);

    let passed = engine.get_global("__test_passed").to_boolean();
    let error_val = engine.get_global("__test_error");

    if verbose {
        for line in engine.console_output() {
            println!("    [console] {line}");
        }
    }

    if meta.negative.is_some() {
        // Test expects an error → pass if we got one, fail if we didn't
        if passed {
            Outcome::Fail("expected an error but test passed".to_string())
        } else {
            Outcome::Pass
        }
    } else {
        if passed {
            Outcome::Pass
        } else {
            let msg = match error_val {
                JsValue::String(s) => s,
                JsValue::Undefined | JsValue::Null => "unknown error".to_string(),
                other => other.to_js_string(),
            };
            Outcome::Fail(msg)
        }
    }
}

// ── file discovery ────────────────────────────────────────────────────────────

fn collect_js_files(path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if path.is_file() {
        if path.extension().map(|e| e == "js").unwrap_or(false) {
            files.push(path.to_path_buf());
        }
    } else if path.is_dir() {
        collect_js_files_recursive(path, &mut files);
    }
    files.sort();
    files
}

fn collect_js_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_js_files_recursive(&p, out);
        } else if p.extension().map(|e| e == "js").unwrap_or(false) {
            out.push(p);
        }
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: tc39_runner <dir-or-file> [--verbose] [--bail]");
        eprintln!("       tc39_runner --help");
        process::exit(1);
    }

    if args[1] == "--help" || args[1] == "-h" {
        println!("tc39_runner — libjs test262 runner");
        println!();
        println!("Usage: tc39_runner <dir-or-file> [--verbose] [--bail]");
        println!();
        println!("  --verbose   Print console.log output and per-test details");
        println!("  --bail      Stop on first failure");
        println!();
        println!("The test262 suite can be downloaded with:");
        println!("  ./scripts/test.sh --tc39-download");
        return;
    }

    let target   = Path::new(&args[1]);
    let verbose  = args.contains(&"--verbose".to_string());
    let bail     = args.contains(&"--bail".to_string());

    let files = collect_js_files(target);
    if files.is_empty() {
        eprintln!("No .js files found at: {}", target.display());
        process::exit(1);
    }

    println!("Running {} test(s) from {}", files.len(), target.display());
    println!();

    let start = Instant::now();
    let mut pass   = 0usize;
    let mut fail   = 0usize;
    let mut skip   = 0usize;
    let mut failures: Vec<(PathBuf, String)> = Vec::new();

    for path in &files {
        let rel = path.strip_prefix(target).unwrap_or(path);
        // Print name to stdout BEFORE running — flush immediately so a hang is visible.
        print!("  RUN   {} ...", rel.display());
        let _ = std::io::stdout().flush();
        let outcome = run_test(path, verbose);

        match &outcome {
            Outcome::Pass => {
                pass += 1;
                println!(" PASS");
                if verbose {
                    // extra detail already shown above
                }
            }
            Outcome::Skip(reason) => {
                skip += 1;
                println!(" SKIP ({})", reason);
            }
            Outcome::Fail(reason) => {
                fail += 1;
                println!(" FAIL");
                println!("        {}", reason);
                failures.push((path.clone(), reason.clone()));
                if bail {
                    break;
                }
            }
        }
    }

    let elapsed = start.elapsed();

    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!(
        "  Results: {} passed  {} failed  {} skipped  ({:.2}s)",
        pass, fail, skip,
        elapsed.as_secs_f64()
    );
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    if fail > 0 {
        process::exit(1);
    }
}

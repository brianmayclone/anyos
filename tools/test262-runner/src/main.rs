//! Test262 runner for libjs.
//!
//! Usage:
//!   test262-runner <test262-dir> [filter] [--limit N] [--verbose]
//!
//! Examples:
//!   test262-runner /tmp/test262-main                              # run all
//!   test262-runner /tmp/test262-main language/expressions         # run subset
//!   test262-runner /tmp/test262-main built-ins/Array/prototype    # specific category
//!   test262-runner /tmp/test262-main language/expressions -v      # all failure details

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use libjs::JsEngine;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: test262-runner <test262-dir> [filter] [--limit N] [--start N] [--timeout SECS] [--verbose]");
        std::process::exit(1);
    }

    let test262_dir = PathBuf::from(&args[1]);
    let verbose = args.iter().any(|a| a == "--verbose" || a == "-v");
    let limit: usize = args.iter()
        .position(|a| a == "--limit")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);
    let start: usize = args.iter()
        .position(|a| a == "--start")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(1); // 1-based
    let timeout_secs: u64 = args.iter()
        .position(|a| a == "--timeout")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let filter = args.get(2)
        .filter(|s| !s.starts_with("--") && *s != "-v")
        .map(|s| s.as_str())
        .unwrap_or("");

    let harness_dir = test262_dir.join("harness");
    // Support both layouts: test262/test/language/... and test262/language/...
    let test_dir = if test262_dir.join("test").exists() {
        test262_dir.join("test")
    } else {
        test262_dir.clone()
    };

    // Pre-load harness files
    let sta_js = fs::read_to_string(harness_dir.join("sta.js")).unwrap_or_default();
    let assert_js = fs::read_to_string(harness_dir.join("assert.js")).unwrap_or_default();

    // Cache for harness includes
    let mut harness_cache: HashMap<String, String> = HashMap::new();
    harness_cache.insert("sta.js".to_string(), sta_js.clone());
    harness_cache.insert("assert.js".to_string(), assert_js.clone());

    // Collect test files
    let search_dir = if filter.is_empty() {
        test_dir.clone()
    } else {
        test_dir.join(filter)
    };

    if !search_dir.exists() {
        eprintln!("Directory not found: {}", search_dir.display());
        std::process::exit(1);
    }

    let mut test_files = Vec::new();
    collect_js_files(&search_dir, &mut test_files);
    test_files.sort();

    let start_idx = (start.max(1) - 1).min(test_files.len()); // convert 1-based to 0-based
    let end_idx = test_files.len().min(start_idx.saturating_add(limit));
    let total = end_idx - start_idx;
    eprintln!("[test262] Found {} test files, running {} (tests {}-{}) ...",
        test_files.len(), total, start_idx + 1, end_idx);

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let mut timed_out = 0usize;
    let mut errors: Vec<(String, String)> = Vec::new();

    let timer_start = Instant::now();
    let timeout_dur = Duration::from_secs(timeout_secs);

    for (i, path) in test_files[start_idx..end_idx].iter().enumerate() {
        let test_num = start_idx + i + 1; // 1-based test number
        let rel_path = path.strip_prefix(&test_dir).unwrap_or(path);
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => { skipped += 1; continue; }
        };

        // Parse frontmatter
        let meta = parse_frontmatter(&source);

        // Skip tests with unsupported features
        if should_skip(&meta) {
            skipped += 1;
            continue;
        }

        // Determine flags early
        let is_only_strict = meta.get("flags").map(|f| f.contains("onlyStrict")).unwrap_or(false);
        let is_module = meta.get("flags").map(|f| f.contains("module")).unwrap_or(false);
        let is_async = meta.get("flags").map(|f| f.contains("async")).unwrap_or(false);

        // Skip module tests (async tests are now supported via synchronous await)
        if is_module {
            skipped += 1;
            continue;
        }

        // Build test source: harness + includes + test
        let mut full_source = String::new();
        if is_only_strict {
            full_source.push_str("\"use strict\";\n");
        }
        full_source.push_str(&sta_js);
        full_source.push('\n');
        full_source.push_str(&assert_js);
        full_source.push('\n');
        // $DONOTEVALUATE for negative parse tests
        full_source.push_str("function $DONOTEVALUATE() { throw new Test262Error('$DONOTEVALUATE: test should not have been evaluated'); }\n");
        // Async test support: inject $DONE callback
        if is_async {
            full_source.push_str("var __async_failed = false;\nfunction $DONE(err) { if (err) { __async_failed = true; throw (err instanceof Error ? err : new Test262Error(String(err))); } }\n");
        }

        // Add required includes
        if let Some(includes) = meta.get("includes") {
            for inc in parse_yaml_list(includes) {
                let inc_src = harness_cache.entry(inc.clone()).or_insert_with(|| {
                    fs::read_to_string(harness_dir.join(&inc)).unwrap_or_default()
                });
                full_source.push_str(inc_src);
                full_source.push('\n');
            }
        }

        full_source.push_str(&source);

        // Determine expected outcome
        let expect_error = meta.get("negative").is_some();

        // Run test in a separate thread with bounded stack (16MB) and timeout
        let full_source_clone = full_source.clone();
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let mut engine = JsEngine::new();
                engine.set_step_limit(5_000_000);
                engine.eval(&full_source_clone);
                let exc_msg = engine.last_exception().map(|exc| {
                    match exc {
                        libjs::JsValue::Object(o) => {
                            let o = o.borrow();
                            let msg = o.get("message");
                            let name = o.get("name");
                            if matches!(name, libjs::JsValue::String(_)) {
                                format!("{}: {}", name.to_js_string(), msg.to_js_string())
                            } else {
                                msg.to_js_string()
                            }
                        }
                        libjs::JsValue::String(s) => s.clone(),
                        other => other.to_js_string(),
                    }
                });
                let console = engine.console_output().to_vec();
                let _ = tx.send((exc_msg.is_none(), console, exc_msg));
            });

        let (succeeded, console_out, exc_msg) = match handle {
            Ok(_h) => match rx.recv_timeout(timeout_dur) {
                Ok(result) => result,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    timed_out += 1;
                    eprintln!("  TIMEOUT  #{} {} (>{}s)", test_num, rel_path.display(), timeout_secs);
                    (false, Vec::new(), Some(format!("TIMEOUT after {}s", timeout_secs)))
                }
                Err(_) => (false, Vec::new(), Some("PANIC: channel disconnected".to_string())),
            },
            Err(_) => (false, Vec::new(), Some("PANIC: failed to spawn thread".to_string())),
        };

        let test_passed = if expect_error {
            !succeeded // Expected error: pass if execution failed
        } else if is_async {
            // Async tests: pass if no exception and $DONE was not called with error
            succeeded && !console_out.iter().any(|s| s.contains("Test262Error"))
        } else if succeeded {
            !console_out.iter().any(|s| s.contains("Test262Error"))
        } else {
            false
        };

        if test_passed {
            passed += 1;
        } else {
            failed += 1;
            let short_path = rel_path.display().to_string();
            let reason = if let Some(ref exc) = exc_msg {
                exc.clone()
            } else if let Some(err_line) = console_out.iter().find(|s| s.contains("Test262Error")) {
                err_line.clone()
            } else {
                "unknown failure".to_string()
            };
            errors.push((short_path, reason));
        }

        // Progress every 500 tests
        if (i + 1) % 500 == 0 {
            let elapsed = timer_start.elapsed().as_secs_f64();
            let rate = (i + 1) as f64 / elapsed;
            eprintln!(
                "[test262] {}/{} done ({} pass, {} fail, {} skip, {} timeout) [{:.0} tests/s]",
                i + 1, total, passed, failed, skipped, timed_out, rate
            );
        }
    }

    let elapsed = timer_start.elapsed().as_secs_f64();
    let total_run = passed + failed;
    let pass_rate = if total_run > 0 { passed as f64 / total_run as f64 * 100.0 } else { 0.0 };

    println!();
    println!("═══════════════════════════════════════════════════════");
    println!("  Test262 Results for libjs");
    println!("═══════════════════════════════════════════════════════");
    println!("  Total:    {}", total);
    println!("  Passed:   {} ({:.1}%)", passed, pass_rate);
    println!("  Failed:   {}", failed);
    println!("  Skipped:  {}", skipped);
    if timed_out > 0 {
    println!("  Timeout:  {} (>{}s)", timed_out, timeout_secs);
    }
    println!("  Time:     {:.1}s", elapsed);
    println!("═══════════════════════════════════════════════════════");

    // Print failures
    if !errors.is_empty() {
        println!();
        if verbose {
            println!("All {} failures:", errors.len());
            for (path, reason) in &errors {
                println!("  FAIL  {}  — {}", path, reason);
            }
        } else {
            println!("First {} failures:", errors.len().min(50));
            for (path, reason) in errors.iter().take(50) {
                println!("  FAIL  {}  — {}", path, reason);
            }
            if errors.len() > 50 {
                println!("  ... and {} more (use --verbose to show all)", errors.len() - 50);
            }
        }
    }

    // Summary by category
    println!();
    let cat_limit = if verbose { usize::MAX } else { 20 };
    println!("Category breakdown{}:", if verbose { "" } else { " (first 20 failure directories)" });
    let mut cat_fails: HashMap<String, usize> = HashMap::new();
    for (path, _) in &errors {
        let parts: Vec<&str> = path.split('/').collect();
        let cat = if parts.len() >= 2 {
            format!("{}/{}", parts[0], parts[1])
        } else {
            parts[0].to_string()
        };
        *cat_fails.entry(cat).or_insert(0) += 1;
    }
    let mut cat_list: Vec<_> = cat_fails.into_iter().collect();
    cat_list.sort_by(|a, b| b.1.cmp(&a.1));
    for (cat, count) in cat_list.iter().take(cat_limit) {
        println!("  {:>5}  {}", count, cat);
    }

    std::process::exit(if failed > 0 { 1 } else { 0 });
}

/// Recursively collect .js files.
fn collect_js_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_js_files(&path, out);
            } else if path.extension().map(|e| e == "js").unwrap_or(false) {
                out.push(path);
            }
        }
    }
}

/// Parse Test262 frontmatter (between /*--- and ---*/).
fn parse_frontmatter(source: &str) -> HashMap<String, String> {
    let mut meta = HashMap::new();
    let start = match source.find("/*---") {
        Some(i) => i + 5,
        None => return meta,
    };
    let end = match source[start..].find("---*/") {
        Some(i) => start + i,
        None => return meta,
    };
    let yaml = &source[start..end];

    // Simple YAML-like key: value parser
    let mut current_key = String::new();
    let mut current_val = String::new();

    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        // Check if line starts a new key (no leading whitespace and contains ':')
        if !line.starts_with(' ') && !line.starts_with('\t') {
            // Save previous
            if !current_key.is_empty() {
                meta.insert(current_key.clone(), current_val.trim().to_string());
            }
            if let Some(colon) = trimmed.find(':') {
                current_key = trimmed[..colon].trim().to_string();
                current_val = trimmed[colon + 1..].trim().to_string();
            }
        } else {
            // Continuation of previous value
            if !current_val.is_empty() { current_val.push('\n'); }
            current_val.push_str(trimmed);
        }
    }
    if !current_key.is_empty() {
        meta.insert(current_key, current_val.trim().to_string());
    }
    meta
}

/// Parse a simple YAML list: either inline [a, b] or multi-line "- a\n- b".
fn parse_yaml_list(val: &str) -> Vec<String> {
    let trimmed = val.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        // Inline: [compareArray.js, assert.js]
        trimmed[1..trimmed.len() - 1]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        // Multi-line: - item
        trimmed.lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.starts_with("- ") { Some(l[2..].trim().to_string()) }
                else { None }
            })
            .collect()
    }
}

/// Features that we know we don't support — skip these tests.
const SKIP_FEATURES: &[&str] = &[
    "SharedArrayBuffer", "Atomics", "async-iteration",
    "top-level-await", "import-assertions", "import-attributes",
    "dynamic-import", "decorators", "explicit-resource-management",
    "resizable-arraybuffer", "arraybuffer-transfer",
    "Temporal", "ShadowRealm", "symbols-as-weakmap-keys",
    "regexp-lookbehind", "regexp-named-groups", "regexp-unicode-property-escapes",
    "regexp-match-indices", "regexp-v-flag", "regexp-duplicate-named-groups",
    "tail-call-optimization", "iterator-helpers",
    "json-modules", "source-phase-imports",
    "Float16Array", "uint8array-base64",
    "IsHTMLDDA",
    "BigInt",
];

fn should_skip(meta: &HashMap<String, String>) -> bool {
    // Skip tests that require features we don't support
    if let Some(features) = meta.get("features") {
        for feat in SKIP_FEATURES {
            if features.contains(feat) {
                return true;
            }
        }
    }
    // Skip tests marked as "raw" (no harness)
    if let Some(flags) = meta.get("flags") {
        if flags.contains("raw") || flags.contains("noStrict") || flags.contains("onlyStrict") {
            // We run these but don't enforce strict/non-strict distinction yet
            // Actually, let's run them
        }
        if flags.contains("generated") {
            // Generated tests are often too complex
        }
    }
    // Skip eval-dependent tests (we have a basic eval, many tests need full eval)
    false
}

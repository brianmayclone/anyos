//! KUnit — in-kernel unit test framework.
//!
//! Modelled after Linux's KUnit: each *test suite* groups related *test cases*.
//! Tests run synchronously during kernel boot when the `kunit` feature is
//! enabled.  Failures are reported via serial output but do **not** panic or
//! halt the kernel — every suite runs to completion regardless of failures.
//! A final summary line shows the overall pass/fail result.
//!
//! # Adding a new test suite
//!
//! 1. Create a module under `kunit/tests/` that defines a `pub static SUITE: TestSuite`.
//! 2. Register it in `kunit/runner.rs` in the `ALL_SUITES` slice.
//!
//! # Writing a test
//!
//! ```rust
//! fn test_addition(ctx: &mut TestContext) {
//!     ctx.expect_eq(2 + 2, 4, "2+2 == 4");
//!     ctx.expect_true(true, "tautology");
//! }
//! ```

pub mod integration;
pub mod runner;
pub mod tests;

/// Per-test execution context passed to every test function.
///
/// Accumulates pass/fail assertion counts.  Tests **must not** call `panic!`
/// on failure — use `ctx.expect_*` methods so all tests run to completion.
pub struct TestContext {
    suite_name: &'static str,
    test_name: &'static str,
    /// Number of assertions that passed in this test.
    pub passed: u32,
    /// Number of assertions that failed in this test.
    pub failed: u32,
    /// Set if the test skipped itself because a precondition was not met.
    pub skipped: bool,
}

impl TestContext {
    pub fn new(suite_name: &'static str, test_name: &'static str) -> Self {
        Self {
            suite_name,
            test_name,
            passed: 0,
            failed: 0,
            skipped: false,
        }
    }

    /// Mark this test as skipped (a precondition was not met). Counted separately
    /// from pass/fail so a silently-not-run test is visible in the report.
    pub fn skip(&mut self, reason: &str) {
        self.skipped = true;
        crate::serial_println!(
            "    [SKIP] {}::{} — {}",
            self.suite_name,
            self.test_name,
            reason
        );
    }

    /// Assert that `cond` is `true`.
    pub fn expect_true(&mut self, cond: bool, msg: &str) {
        if cond {
            self.passed += 1;
        } else {
            self.failed += 1;
            crate::serial_println!(
                "    [FAIL] {}::{} — {} (expected true, got false)",
                self.suite_name,
                self.test_name,
                msg
            );
        }
    }

    /// Assert that `cond` is `false`.
    pub fn expect_false(&mut self, cond: bool, msg: &str) {
        if !cond {
            self.passed += 1;
        } else {
            self.failed += 1;
            crate::serial_println!(
                "    [FAIL] {}::{} — {} (expected false, got true)",
                self.suite_name,
                self.test_name,
                msg
            );
        }
    }

    /// Assert that `left == right`.
    pub fn expect_eq<T>(&mut self, left: T, right: T, msg: &str)
    where
        T: PartialEq + core::fmt::Debug,
    {
        if left == right {
            self.passed += 1;
        } else {
            self.failed += 1;
            crate::serial_println!(
                "    [FAIL] {}::{} — {} (left={:?}, right={:?})",
                self.suite_name,
                self.test_name,
                msg,
                left,
                right
            );
        }
    }

    /// Assert that `left != right`.
    pub fn expect_ne<T>(&mut self, left: T, right: T, msg: &str)
    where
        T: PartialEq + core::fmt::Debug,
    {
        if left != right {
            self.passed += 1;
        } else {
            self.failed += 1;
            crate::serial_println!(
                "    [FAIL] {}::{} — {} (expected !=, both are {:?})",
                self.suite_name,
                self.test_name,
                msg,
                left
            );
        }
    }

    /// Assert that `value` is `Some(_)` and return the inner value (or `None` on failure).
    pub fn expect_some<T: core::fmt::Debug>(&mut self, value: Option<T>, msg: &str) -> Option<T> {
        match value {
            Some(v) => {
                self.passed += 1;
                Some(v)
            }
            None => {
                self.failed += 1;
                crate::serial_println!(
                    "    [FAIL] {}::{} — {} (expected Some, got None)",
                    self.suite_name,
                    self.test_name,
                    msg
                );
                None
            }
        }
    }

    /// Assert that `value` is `None`.
    pub fn expect_none<T: core::fmt::Debug>(&mut self, value: Option<T>, msg: &str) {
        match value {
            None => {
                self.passed += 1;
            }
            Some(v) => {
                self.failed += 1;
                crate::serial_println!(
                    "    [FAIL] {}::{} — {} (expected None, got Some({:?}))",
                    self.suite_name,
                    self.test_name,
                    msg,
                    v
                );
            }
        }
    }

    /// Assert that `left >= right`.
    pub fn expect_ge<T>(&mut self, left: T, right: T, msg: &str)
    where
        T: PartialOrd + core::fmt::Debug,
    {
        if left >= right {
            self.passed += 1;
        } else {
            self.failed += 1;
            crate::serial_println!(
                "    [FAIL] {}::{} — {} ({:?} < {:?})",
                self.suite_name,
                self.test_name,
                msg,
                left,
                right
            );
        }
    }

    /// Assert that `left > right`.
    pub fn expect_gt<T>(&mut self, left: T, right: T, msg: &str)
    where
        T: PartialOrd + core::fmt::Debug,
    {
        if left > right {
            self.passed += 1;
        } else {
            self.failed += 1;
            crate::serial_println!(
                "    [FAIL] {}::{} — {} ({:?} <= {:?})",
                self.suite_name,
                self.test_name,
                msg,
                left,
                right
            );
        }
    }

    /// Returns `true` if all assertions in this test have passed so far.
    pub fn is_ok(&self) -> bool {
        self.failed == 0
    }
}

/// A single named test case.
pub struct TestCase {
    pub name: &'static str,
    pub run: fn(&mut TestContext),
}

/// A test suite groups related test cases under one name.
pub struct TestSuite {
    pub name: &'static str,
    pub cases: &'static [TestCase],
}

// ── Headless completion signal ────────────────────────────────────────────────

/// Unit-phase assertion-failure count, recorded by `runner::run_unit_tests` so
/// the later integration phase can compute an overall pass/fail for the QEMU
/// completion signal. (The two phases run at different boot points.)
pub static KUNIT_UNIT_FAILED: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// Print the machine-readable completion line and ask QEMU to exit with a status
/// that encodes overall pass/fail, so a headless runner keys off the exit code
/// instead of scraping the serial log. Called once, at the end of the
/// integration phase (the last kunit step). `integration_failed` is that phase's
/// assertion-failure count; the unit phase's count is read from
/// [`KUNIT_UNIT_FAILED`].
///
/// QEMU's `isa-debug-exit` device (`-device isa-debug-exit,iobase=0xf4,iosize=4`)
/// exits with `(value << 1) | 1`, so:
///   - all pass → write `0x10` → QEMU exit code 33
///   - any fail → write `0x11` → QEMU exit code 35
///
/// If the device is absent (e.g. a normal full-image run without it), the port
/// write is a harmless no-op and the kernel simply continues to boot.
pub fn report_and_exit(integration_failed: u32) {
    use core::sync::atomic::Ordering;
    let unit_failed = KUNIT_UNIT_FAILED.load(Ordering::Acquire);
    let total_failed = unit_failed.saturating_add(integration_failed);
    let rc = if total_failed == 0 { 0u32 } else { 1u32 };
    crate::serial_println!(
        "KUNIT-DONE rc={} (unit_fail={}, integ_fail={})",
        rc,
        unit_failed,
        integration_failed
    );
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let value: u32 = if total_failed == 0 { 0x10 } else { 0x11 };
        crate::arch::x86::port::outl(0xF4, value);
    }
}

/// Run a slice of test suites, calling `before_case` before each case (the
/// integration runner uses this to publish its shared context). Prints the
/// per-case `[PASS]`/`[FAIL]`/`[SKIP]` and per-suite `[suite]`/`[OK]`/`[FAIL]`
/// lines the headless harness greps for, and returns
/// `(assertions_passed, assertions_failed, suites_passed, suites_failed, cases_skipped)`.
///
/// One shared loop for both the unit and integration runners so the two cannot
/// drift.
pub(crate) fn run_suite_array(
    suites: &[&TestSuite],
    mut before_case: impl FnMut(),
) -> (u32, u32, u32, u32, u32) {
    let mut total_pass = 0u32;
    let mut total_fail = 0u32;
    let mut suites_pass = 0u32;
    let mut suites_fail = 0u32;
    let mut total_skipped = 0u32;

    for suite in suites {
        crate::serial_println!("");
        crate::serial_println!("  [suite] {}", suite.name);

        let mut cases_pass = 0u32;
        let mut cases_fail = 0u32;
        let mut cases_skip = 0u32;

        for case in suite.cases {
            before_case();
            let mut ctx = TestContext::new(suite.name, case.name);
            (case.run)(&mut ctx);

            total_pass += ctx.passed;
            total_fail += ctx.failed;

            if ctx.skipped {
                // `skip()` already printed the [SKIP] line with its reason.
                cases_skip += 1;
                total_skipped += 1;
            } else if ctx.is_ok() {
                crate::serial_println!("    [PASS] {}", case.name);
                cases_pass += 1;
            } else {
                crate::serial_println!(
                    "    [FAIL] {} — {} assertion(s) failed",
                    case.name,
                    ctx.failed
                );
                cases_fail += 1;
            }
        }

        let total_cases = cases_pass + cases_fail + cases_skip;
        if cases_fail == 0 {
            crate::serial_println!(
                "  [OK]   {} — {}/{} tests passed{}",
                suite.name,
                cases_pass,
                total_cases,
                if cases_skip > 0 { " (some skipped)" } else { "" }
            );
            suites_pass += 1;
        } else {
            crate::serial_println!(
                "  [FAIL] {} — {}/{} tests failed",
                suite.name,
                cases_fail,
                total_cases
            );
            suites_fail += 1;
        }
    }

    (total_pass, total_fail, suites_pass, suites_fail, total_skipped)
}

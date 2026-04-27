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
}

impl TestContext {
    pub fn new(suite_name: &'static str, test_name: &'static str) -> Self {
        Self {
            suite_name,
            test_name,
            passed: 0,
            failed: 0,
        }
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

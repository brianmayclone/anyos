//! KUnit test runner — collects all suites and runs them.

use super::{TestContext, TestSuite};

/// All registered test suites, in execution order.
///
/// To add a new suite, append a reference here and create the corresponding
/// module under `kunit/tests/`.
static ALL_SUITES: &[&TestSuite] = &[
    &super::tests::alloc_tests::SUITE,
    &super::tests::sync_tests::SUITE,
    &super::tests::net_tests::SUITE,
    &super::tests::memory_tests::SUITE,
];

/// Run every registered test suite and print a summary to serial output.
///
/// Called from `kernel_main` after the heap allocator is initialized.
/// Never panics — all suites always run to completion.
pub fn run_all() {
    crate::serial_println!("");
    crate::serial_println!("============================================================");
    crate::serial_println!("  KUnit — kernel self-tests");
    crate::serial_println!("============================================================");

    let mut total_assertions_pass: u32 = 0;
    let mut total_assertions_fail: u32 = 0;
    let mut suites_pass: u32 = 0;
    let mut suites_fail: u32 = 0;

    for suite in ALL_SUITES {
        let (sp, sf) = run_suite(suite, &mut total_assertions_pass, &mut total_assertions_fail);
        suites_pass += sp;
        suites_fail += sf;
    }

    let total_suites = suites_pass + suites_fail;
    let total_assertions = total_assertions_pass + total_assertions_fail;

    crate::serial_println!("");
    crate::serial_println!("============================================================");
    if total_assertions_fail == 0 {
        crate::serial_println!(
            "  KUnit: ALL PASS — {} suite(s), {} assertion(s)",
            total_suites, total_assertions
        );
    } else {
        crate::serial_println!(
            "  KUnit: FAIL — {}/{} assertion(s) failed, {}/{} suite(s) failed",
            total_assertions_fail, total_assertions,
            suites_fail, total_suites
        );
    }
    crate::serial_println!("============================================================");
    crate::serial_println!("");
}

/// Run a single suite, return `(1, 0)` if it passed, `(0, 1)` if it failed.
fn run_suite(
    suite: &TestSuite,
    total_pass: &mut u32,
    total_fail: &mut u32,
) -> (u32, u32) {
    crate::serial_println!("");
    crate::serial_println!("  [suite] {}", suite.name);

    let mut cases_pass: u32 = 0;
    let mut cases_fail: u32 = 0;

    for case in suite.cases {
        let mut ctx = TestContext::new(suite.name, case.name);
        (case.run)(&mut ctx);

        *total_pass += ctx.passed;
        *total_fail += ctx.failed;

        if ctx.is_ok() {
            crate::serial_println!("    [PASS] {}", case.name);
            cases_pass += 1;
        } else {
            crate::serial_println!(
                "    [FAIL] {} — {} assertion(s) failed",
                case.name, ctx.failed
            );
            cases_fail += 1;
        }
    }

    if cases_fail == 0 {
        crate::serial_println!(
            "  [OK]   {} — {}/{} tests passed",
            suite.name, cases_pass, cases_pass
        );
        (1, 0)
    } else {
        crate::serial_println!(
            "  [FAIL] {} — {}/{} tests failed",
            suite.name, cases_fail, cases_pass + cases_fail
        );
        (0, 1)
    }
}

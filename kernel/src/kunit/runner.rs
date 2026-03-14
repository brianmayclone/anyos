//! KUnit test runner — two phases matching Linux's testing model:
//!
//! 1. **Unit tests** (`run_unit_tests`): pure algorithm/data-structure tests
//!    with no hardware dependency.  Run after `memory::heap::init()`.
//!
//! 2. **Integration tests** (`run_integration_tests`): verify real hardware
//!    state after ACPI, APIC, IRQ, scheduler, and timer have been initialized.
//!    Run after `arch::hal::enable_interrupts()` and `scheduler::init()`.

use super::{TestContext, TestSuite};

// ── Unit test suites ──────────────────────────────────────────────────────────

static UNIT_SUITES: &[&TestSuite] = &[
    &super::tests::alloc_tests::SUITE,
    &super::tests::sync_tests::SUITE,
    &super::tests::net_tests::SUITE,
    &super::tests::net_types_tests::SUITE,
    &super::tests::memory_tests::SUITE,
    &super::tests::heap_stress_tests::SUITE,
    &super::tests::vma_tests::SUITE,
    &super::tests::crypto_tests::SUITE,
    &super::tests::datetime_tests::SUITE,
    &super::tests::capabilities_tests::SUITE,
    &super::tests::syscall_safety_tests::SUITE,
    &super::tests::ipc_tests::SUITE,
];

/// Run all pure unit tests.  Call after `memory::heap::init()`.
pub fn run_unit_tests() {
    crate::serial_println!("");
    crate::serial_println!("============================================================");
    crate::serial_println!("  KUnit — unit tests");
    crate::serial_println!("============================================================");

    let (p, f, sp, sf) = run_suites(UNIT_SUITES);
    print_summary("unit", p, f, sp, sf);
}

/// Run all integration tests.  Call after hardware init + scheduler init.
pub fn run_integration_tests() {
    super::integration::run_all();
}

// ── Compat alias (keeps existing call sites working) ─────────────────────────

/// Alias for `run_unit_tests` — kept for backwards compatibility with any
/// existing `kunit::runner::run_all()` call sites.
pub fn run_all() {
    run_unit_tests();
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Run a slice of suites and return `(total_pass, total_fail, suites_pass, suites_fail)`.
fn run_suites(suites: &[&TestSuite]) -> (u32, u32, u32, u32) {
    let mut total_pass: u32 = 0;
    let mut total_fail: u32 = 0;
    let mut suites_pass: u32 = 0;
    let mut suites_fail: u32 = 0;

    for suite in suites {
        crate::serial_println!("");
        crate::serial_println!("  [suite] {}", suite.name);

        let mut cases_pass: u32 = 0;
        let mut cases_fail: u32 = 0;

        for case in suite.cases {
            let mut ctx = TestContext::new(suite.name, case.name);
            (case.run)(&mut ctx);

            total_pass += ctx.passed;
            total_fail += ctx.failed;

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
            suites_pass += 1;
        } else {
            crate::serial_println!(
                "  [FAIL] {} — {}/{} tests failed",
                suite.name, cases_fail, cases_pass + cases_fail
            );
            suites_fail += 1;
        }
    }

    (total_pass, total_fail, suites_pass, suites_fail)
}

fn print_summary(kind: &str, total_pass: u32, total_fail: u32, sp: u32, sf: u32) {
    crate::serial_println!("");
    crate::serial_println!("============================================================");
    let total = total_pass + total_fail;
    if total_fail == 0 {
        crate::serial_println!(
            "  KUnit {}: ALL PASS — {} suite(s), {} assertion(s)",
            kind, sp + sf, total
        );
    } else {
        crate::serial_println!(
            "  KUnit {}: FAIL — {}/{} assertions failed, {}/{} suites failed",
            kind, total_fail, total, sf, sp + sf
        );
    }
    crate::serial_println!("============================================================");
    crate::serial_println!("");
}

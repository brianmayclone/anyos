//! KUnit integration tests — Linux-style boot-time hardware verification.
//!
//! Unlike the pure unit tests in `kunit/tests/`, integration tests run
//! **after** real subsystems have been initialized and verify that the actual
//! hardware state matches expectations.  This mirrors Linux's approach:
//!
//! - `kunit/tests/`       → algorithm / data-structure unit tests (no hardware)
//! - `kunit/integration/` → subsystem integration tests (real hardware state)
//!
//! # Call sites in `kernel_main`
//!
//! ```text
//! Phase 4  (after heap)          → kunit::runner::run_unit_tests()
//! Phase 7+ (after interrupts)    → kunit::runner::run_integration_tests()
//! ```
//!
//! Integration tests receive a shared [`IntegrationCtx`] that carries the
//! results of earlier init phases (e.g. the parsed ACPI tables).

pub mod hardware_tests;
pub mod scheduler_tests;
pub mod stability_tests;

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::sync::spinlock::Spinlock;

// ── Shared context ────────────────────────────────────────────────────────────

/// Context passed to every integration test suite.
///
/// Populated by `kernel_main` via [`set_context`] before
/// `run_integration_tests()` is called.
#[derive(Clone)]
pub struct IntegrationCtx {
    /// Whether ACPI was found and parsed successfully.
    pub acpi_present: bool,
    /// LAPIC physical base address from ACPI (0 if ACPI absent).
    pub lapic_address: u32,
    /// Number of logical processors reported by ACPI.
    pub processor_count: usize,
    /// Number of I/O APICs reported by ACPI.
    pub ioapic_count: usize,
    /// Number of interrupt source overrides (ISOs) in ACPI MADT.
    pub iso_count: usize,
}

impl IntegrationCtx {
    pub const fn empty() -> Self {
        Self {
            acpi_present: false,
            lapic_address: 0,
            processor_count: 0,
            ioapic_count: 0,
            iso_count: 0,
        }
    }
}

static CTX: Spinlock<IntegrationCtx> = Spinlock::new(IntegrationCtx::empty());

/// Called from `kernel_main` (gated by `#[cfg(feature = "kunit")]`) after
/// ACPI init to store boot-phase results for integration tests.
pub fn set_context(ctx: IntegrationCtx) {
    *CTX.lock() = ctx;
}

fn get_context() -> IntegrationCtx {
    CTX.lock().clone()
}

// ── Integration runner ────────────────────────────────────────────────────────

/// All registered integration test suites.
static ALL_INTEGRATION_SUITES: &[&TestSuite] = &[
    &scheduler_tests::SUITE,
    &hardware_tests::SUITE,
    &stability_tests::SUITE,
];

/// Run all integration test suites and print a summary.
///
/// Called after interrupts are enabled and all hardware subsystems
/// (ACPI, APIC, IRQ, PIT, scheduler) have been initialized.
pub fn run_all() {
    let ctx = get_context();

    crate::serial_println!("");
    crate::serial_println!("============================================================");
    crate::serial_println!("  KUnit — integration tests (hardware state)");
    crate::serial_println!("============================================================");

    let mut total_pass: u32 = 0;
    let mut total_fail: u32 = 0;
    let mut suites_pass: u32 = 0;
    let mut suites_fail: u32 = 0;

    for suite in ALL_INTEGRATION_SUITES {
        crate::serial_println!("");
        crate::serial_println!("  [suite] {}", suite.name);

        let mut cases_pass: u32 = 0;
        let mut cases_fail: u32 = 0;

        for case in suite.cases {
            // Integration tests receive the context via a thread-local-style
            // slot so their signature matches the standard `fn(&mut TestContext)`.
            CURRENT_CTX.lock().replace(ctx.clone());
            let mut test_ctx = TestContext::new(suite.name, case.name);
            (case.run)(&mut test_ctx);

            total_pass += test_ctx.passed;
            total_fail += test_ctx.failed;

            if test_ctx.is_ok() {
                crate::serial_println!("    [PASS] {}", case.name);
                cases_pass += 1;
            } else {
                crate::serial_println!(
                    "    [FAIL] {} — {} assertion(s) failed",
                    case.name, test_ctx.failed
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

    crate::serial_println!("");
    crate::serial_println!("============================================================");
    let total = total_pass + total_fail;
    if total_fail == 0 {
        crate::serial_println!(
            "  KUnit integration: ALL PASS — {} suite(s), {} assertion(s)",
            suites_pass + suites_fail, total
        );
    } else {
        crate::serial_println!(
            "  KUnit integration: FAIL — {}/{} assertions failed, {}/{} suites failed",
            total_fail, total, suites_fail, suites_pass + suites_fail
        );
    }
    crate::serial_println!("============================================================");
    crate::serial_println!("");
}

// ── Context slot (per-test context injection) ─────────────────────────────────

/// Slot used to pass `IntegrationCtx` into test functions whose signature
/// is `fn(&mut TestContext)` (matching the standard `TestCase` type).
static CURRENT_CTX: Spinlock<Option<IntegrationCtx>> = Spinlock::new(None);

/// Retrieve the current integration context from within a test function.
pub fn current_ctx() -> IntegrationCtx {
    CURRENT_CTX
        .lock()
        .clone()
        .unwrap_or_else(IntegrationCtx::empty)
}

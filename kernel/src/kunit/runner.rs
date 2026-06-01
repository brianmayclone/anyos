//! KUnit test runner — two phases matching Linux's testing model:
//!
//! 1. **Unit tests** (`run_unit_tests`): pure algorithm/data-structure tests
//!    with no hardware dependency.  Run after `memory::heap::init()`.
//!
//! 2. **Integration tests** (`run_integration_tests`): verify real hardware
//!    state after ACPI, APIC, IRQ, scheduler, and timer have been initialized.
//!    Run after `arch::hal::enable_interrupts()` and `scheduler::init()`.

use super::TestSuite;

// ── Unit test suites ──────────────────────────────────────────────────────────

static UNIT_SUITES: &[&TestSuite] = &[
    &super::tests::alloc_tests::SUITE,
    &super::tests::blockcache_tests::SUITE,
    &super::tests::sync_tests::SUITE,
    &super::tests::net_tests::SUITE,
    &super::tests::net_types_tests::SUITE,
    &super::tests::memory_tests::SUITE,
    &super::tests::physmap_tests::SUITE,
    &super::tests::buddy_tests::SUITE,
    &super::tests::heap_stress_tests::SUITE,
    &super::tests::vma_tests::SUITE,
    &super::tests::crypto_tests::SUITE,
    &super::tests::datetime_tests::SUITE,
    &super::tests::capabilities_tests::SUITE,
    &super::tests::syscall_safety_tests::SUITE,
    &super::tests::user_access_tests::SUITE,
    &super::tests::ipc_tests::SUITE,
    &super::tests::vfs_readahead_tests::SUITE,
];

/// Run all pure unit tests.  Call after `memory::heap::init()`.
pub fn run_unit_tests() {
    crate::serial_println!("");
    crate::serial_println!("============================================================");
    crate::serial_println!("  KUnit — unit tests");
    crate::serial_println!("============================================================");

    let (p, f, sp, sf, sk) = super::run_suite_array(UNIT_SUITES, || {});
    print_summary("unit", p, f, sp, sf, sk);

    // Record the unit-phase failure count for the overall completion signal
    // emitted at the end of the integration phase (see `kunit::report_and_exit`).
    super::KUNIT_UNIT_FAILED.store(f, core::sync::atomic::Ordering::Release);
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

fn print_summary(kind: &str, total_pass: u32, total_fail: u32, sp: u32, sf: u32, skipped: u32) {
    crate::serial_println!("");
    crate::serial_println!("============================================================");
    let total = total_pass + total_fail;
    if total_fail == 0 {
        crate::serial_println!(
            "  KUnit {}: ALL PASS — {} suite(s), {} assertion(s){}",
            kind,
            sp + sf,
            total,
            if skipped > 0 { ", some skipped" } else { "" }
        );
    } else {
        crate::serial_println!(
            "  KUnit {}: FAIL — {}/{} assertions failed, {}/{} suites failed",
            kind,
            total_fail,
            total,
            sf,
            sp + sf
        );
    }
    crate::serial_println!("============================================================");
    crate::serial_println!("");
}

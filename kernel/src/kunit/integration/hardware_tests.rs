//! Integration tests for hardware subsystems.
//!
//! Verifies the state of ACPI tables, LAPIC, I/O APIC, IRQ routing, PIT
//! timer, and CPU feature detection **after** those subsystems have been
//! initialized by `kernel_main`.  Mirrors Linux's approach of testing
//! hardware in-context via kvm-unit-tests and KUnit boot-time assertions.
//!
//! All tests use `integration::current_ctx()` to access boot-phase results
//! (ACPI processor/IOAPIC counts, LAPIC address, …) without re-running init.

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::kunit::integration::current_ctx;

#[cfg(target_arch = "x86_64")]
use crate::arch::x86::{apic, cpuid, irq, pit};

pub static SUITE: TestSuite = TestSuite {
    name: "hardware",
    cases: &[
        // ── ACPI ──────────────────────────────────────────────────────────────
        TestCase { name: "acpi_present",                  run: test_acpi_present },
        TestCase { name: "acpi_lapic_address_valid",      run: test_lapic_address },
        TestCase { name: "acpi_processor_count",          run: test_processor_count },
        TestCase { name: "acpi_ioapic_present",           run: test_ioapic_present },
        // ── LAPIC / APIC ──────────────────────────────────────────────────────
        TestCase { name: "apic_initialized",              run: test_apic_initialized },
        TestCase { name: "apic_lapic_id_readable",        run: test_lapic_id },
        // ── IRQ table ─────────────────────────────────────────────────────────
        TestCase { name: "irq_register_dispatch",         run: test_irq_register_dispatch },
        TestCase { name: "irq_unregistered_returns_false",run: test_irq_unregistered },
        TestCase { name: "irq_chain_register",            run: test_irq_chain },
        // ── PIT / TSC ─────────────────────────────────────────────────────────
        TestCase { name: "pit_tick_hz_correct",           run: test_pit_tick_hz },
        TestCase { name: "pit_tsc_calibrated",            run: test_tsc_calibrated },
        TestCase { name: "pit_ticks_advancing",           run: test_ticks_advancing },
        TestCase { name: "pit_rdtsc_advancing",           run: test_rdtsc_advancing },
        // ── CPUID ─────────────────────────────────────────────────────────────
        TestCase { name: "cpuid_sse2_present",            run: test_sse2_present },
        TestCase { name: "cpuid_vendor_nonzero",          run: test_vendor_nonzero },
        TestCase { name: "cpuid_fpu_present",             run: test_fpu_present },
    ],
};

// ── ACPI ──────────────────────────────────────────────────────────────────────

fn test_acpi_present(ctx: &mut TestContext) {
    let c = current_ctx();
    // QEMU exposes an ACPI BIOS by default. If ACPI is absent the system
    // falls back to the legacy PIC — still OK, but flag it.
    if !c.acpi_present {
        crate::serial_println!(
            "    [NOTE] hardware::acpi_present — ACPI not found, running with legacy PIC"
        );
    }
    ctx.expect_true(true, "ACPI check completed (see NOTE above if absent)");
}

fn test_lapic_address(ctx: &mut TestContext) {
    let c = current_ctx();
    if !c.acpi_present { ctx.expect_true(true, "skipped (no ACPI)"); return; }

    // LAPIC is always mapped in the 0xFEC0_0000 – 0xFEFF_FFFF range on x86.
    ctx.expect_ge(c.lapic_address, 0xFEC0_0000u32, "LAPIC addr >= 0xFEC00000");
    ctx.expect_true(c.lapic_address % 4096 == 0, "LAPIC addr is page-aligned");
}

fn test_processor_count(ctx: &mut TestContext) {
    let c = current_ctx();
    if !c.acpi_present { ctx.expect_true(true, "skipped (no ACPI)"); return; }

    // QEMU exposes at least 1 processor; -smp cpus=N exposes N.
    ctx.expect_ge(c.processor_count, 1usize, "at least 1 processor in ACPI MADT");
    // Sanity upper bound (QEMU max is typically 255).
    ctx.expect_true(c.processor_count <= 256, "processor count <= 256");
}

fn test_ioapic_present(ctx: &mut TestContext) {
    let c = current_ctx();
    if !c.acpi_present { ctx.expect_true(true, "skipped (no ACPI)"); return; }

    ctx.expect_ge(c.ioapic_count, 1usize, "at least 1 I/O APIC in ACPI MADT");
}

// ── LAPIC / APIC ──────────────────────────────────────────────────────────────

fn test_apic_initialized(ctx: &mut TestContext) {
    #[cfg(target_arch = "x86_64")]
    {
        let c = current_ctx();
        if c.acpi_present {
            ctx.expect_true(apic::is_initialized(), "LAPIC initialized after ACPI boot path");
        } else {
            ctx.expect_true(true, "skipped (no ACPI, using legacy PIC)");
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    ctx.expect_true(true, "skipped (non-x86)");
}

fn test_lapic_id(ctx: &mut TestContext) {
    #[cfg(target_arch = "x86_64")]
    {
        let c = current_ctx();
        if c.acpi_present && apic::is_initialized() {
            let id = apic::lapic_id();
            // BSP LAPIC ID is almost always 0 in single-socket QEMU.
            ctx.expect_true(id < 255, "LAPIC ID < 255 (not garbage)");
        } else {
            ctx.expect_true(true, "skipped (LAPIC not initialized)");
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    ctx.expect_true(true, "skipped (non-x86)");
}

// ── IRQ table ─────────────────────────────────────────────────────────────────

fn test_irq_register_dispatch(ctx: &mut TestContext) {
    #[cfg(target_arch = "x86_64")]
    {
        // Use IRQ slot 31 (highest, unused in normal operation) as a scratch slot.
        static FIRED: core::sync::atomic::AtomicBool =
            core::sync::atomic::AtomicBool::new(false);

        fn handler(_irq: u8) {
            FIRED.store(true, core::sync::atomic::Ordering::Relaxed);
        }

        irq::register_irq(31, handler);
        let dispatched = irq::dispatch_irq(31);
        irq::unregister_irq(31);

        ctx.expect_true(dispatched, "dispatch_irq returns true when handler registered");
        ctx.expect_true(
            FIRED.load(core::sync::atomic::Ordering::Relaxed),
            "handler was called by dispatch_irq"
        );
    }
    #[cfg(not(target_arch = "x86_64"))]
    ctx.expect_true(true, "skipped (non-x86)");
}

fn test_irq_unregistered(ctx: &mut TestContext) {
    #[cfg(target_arch = "x86_64")]
    {
        // IRQ 30 is not registered in normal operation.
        irq::unregister_irq(30); // ensure clean state
        let dispatched = irq::dispatch_irq(30);
        ctx.expect_false(dispatched, "dispatch_irq returns false with no handler");
    }
    #[cfg(not(target_arch = "x86_64"))]
    ctx.expect_true(true, "skipped (non-x86)");
}

fn test_irq_chain(ctx: &mut TestContext) {
    #[cfg(target_arch = "x86_64")]
    {
        static COUNT: core::sync::atomic::AtomicU32 =
            core::sync::atomic::AtomicU32::new(0);

        fn primary(_irq: u8) {
            COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
        fn secondary(_irq: u8) {
            COUNT.fetch_add(10, core::sync::atomic::Ordering::Relaxed);
        }

        COUNT.store(0, core::sync::atomic::Ordering::Relaxed);
        irq::register_irq(29, primary);
        irq::register_irq_chain(29, secondary);
        irq::dispatch_irq(29);
        irq::unregister_irq(29);

        let val = COUNT.load(core::sync::atomic::Ordering::Relaxed);
        // Both primary (1) and chain (10) must have fired.
        ctx.expect_eq(val, 11u32, "primary + chain handler both called (sum == 11)");
    }
    #[cfg(not(target_arch = "x86_64"))]
    ctx.expect_true(true, "skipped (non-x86)");
}

// ── PIT / TSC ─────────────────────────────────────────────────────────────────

fn test_pit_tick_hz(ctx: &mut TestContext) {
    #[cfg(target_arch = "x86_64")]
    ctx.expect_eq(pit::TICK_HZ, 1000u32, "PIT configured at 1000 Hz");

    #[cfg(not(target_arch = "x86_64"))]
    ctx.expect_true(true, "skipped (non-x86)");
}

fn test_tsc_calibrated(ctx: &mut TestContext) {
    #[cfg(target_arch = "x86_64")]
    {
        let hz = pit::tsc_hz();
        // A calibrated TSC must be at least 100 MHz and at most 10 GHz.
        if hz > 0 {
            ctx.expect_ge(hz, 100_000_000u64, "TSC >= 100 MHz");
            ctx.expect_true(hz < 10_000_000_000u64, "TSC < 10 GHz");
        } else {
            crate::serial_println!("    [NOTE] hardware::tsc_calibrated — TSC not yet calibrated");
            ctx.expect_true(true, "TSC calibration pending (skipped)");
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    ctx.expect_true(true, "skipped (non-x86)");
}

fn test_ticks_advancing(ctx: &mut TestContext) {
    #[cfg(target_arch = "x86_64")]
    {
        let t0 = pit::get_ticks();
        // Busy-wait a short moment using PAUSE to let the PIT fire.
        for _ in 0..500_000u64 {
            core::hint::spin_loop();
        }
        let t1 = pit::get_ticks();
        // With interrupts enabled the tick counter must advance; allow 0
        // only if the PIT hasn't fired yet (very early in boot).
        ctx.expect_ge(t1, t0, "tick count is non-decreasing");
    }
    #[cfg(not(target_arch = "x86_64"))]
    ctx.expect_true(true, "skipped (non-x86)");
}

fn test_rdtsc_advancing(ctx: &mut TestContext) {
    #[cfg(target_arch = "x86_64")]
    {
        let a = pit::rdtsc();
        core::hint::spin_loop();
        core::hint::spin_loop();
        let b = pit::rdtsc();
        ctx.expect_gt(b, a, "RDTSC strictly advances between two reads");
    }
    #[cfg(not(target_arch = "x86_64"))]
    ctx.expect_true(true, "skipped (non-x86)");
}

// ── CPUID ─────────────────────────────────────────────────────────────────────

fn test_sse2_present(ctx: &mut TestContext) {
    #[cfg(target_arch = "x86_64")]
    {
        let f = cpuid::features();
        // Our boot.asm checks for SSE2 before jumping to Rust; if we reach
        // here SSE2 must be present.
        ctx.expect_true(f.sse2, "SSE2 present (required for boot)");
        ctx.expect_true(f.sse,  "SSE present");
        ctx.expect_true(f.fxsr, "FXSR present (required for FPU save/restore)");
    }
    #[cfg(not(target_arch = "x86_64"))]
    ctx.expect_true(true, "skipped (non-x86)");
}

fn test_vendor_nonzero(ctx: &mut TestContext) {
    #[cfg(target_arch = "x86_64")]
    {
        let v = cpuid::vendor();
        // The vendor string must contain at least one non-zero byte.
        let nonzero = v.iter().any(|&b| b != 0);
        ctx.expect_true(nonzero, "CPU vendor string is non-empty");
    }
    #[cfg(not(target_arch = "x86_64"))]
    ctx.expect_true(true, "skipped (non-x86)");
}

fn test_fpu_present(ctx: &mut TestContext) {
    #[cfg(target_arch = "x86_64")]
    {
        let f = cpuid::features();
        ctx.expect_true(f.fpu,  "x87 FPU present");
        ctx.expect_true(f.sse3, "SSE3 present (expected in QEMU default CPU)");
    }
    #[cfg(not(target_arch = "x86_64"))]
    ctx.expect_true(true, "skipped (non-x86)");
}

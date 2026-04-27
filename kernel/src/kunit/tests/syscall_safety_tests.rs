//! Syscall boundary safety tests.
//!
//! These are pure unit tests (no hardware, no scheduler) that verify the
//! syscall-adjacent plumbing won't crash or produce garbage for adversarial
//! inputs.  In a kernel, a single unguarded integer overflow in syscall
//! dispatch is enough to escalate privileges or corrupt kernel state.

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::task::capabilities::{
    parse_capabilities, required_cap, CAP_ALL, CAP_AUDIO, CAP_COMPOSITOR, CAP_DEBUG, CAP_DEVICE,
    CAP_DLL, CAP_EVENT, CAP_MANAGE_PERMS, CAP_NETWORK, CAP_PIPE, CAP_PROCESS, CAP_SYSTEM,
    CAP_THREAD,
};

pub static SUITE: TestSuite = TestSuite {
    name: "syscall::safety",
    cases: &[
        TestCase {
            name: "required_cap_never_panics_low",
            run: test_no_panic_low,
        },
        TestCase {
            name: "required_cap_never_panics_high",
            run: test_no_panic_high,
        },
        TestCase {
            name: "required_cap_extreme_values",
            run: test_extreme_values,
        },
        TestCase {
            name: "required_cap_returns_valid_cap",
            run: test_valid_cap_bits,
        },
        TestCase {
            name: "cap_check_deny_partial",
            run: test_deny_partial,
        },
        TestCase {
            name: "cap_check_deny_none",
            run: test_deny_none,
        },
        TestCase {
            name: "cap_check_allow_exact",
            run: test_allow_exact,
        },
        TestCase {
            name: "cap_all_allows_everything",
            run: test_cap_all_allows_all,
        },
        TestCase {
            name: "parse_caps_no_panic_garbage",
            run: test_parse_garbage,
        },
        TestCase {
            name: "cap_bits_non_overlapping",
            run: test_bits_non_overlapping,
        },
        TestCase {
            name: "required_cap_zero_always_passes",
            run: test_zero_always_passes,
        },
    ],
};

// ── required_cap never panics ─────────────────────────────────────────────────

fn test_no_panic_low(ctx: &mut TestContext) {
    // Scan syscall numbers 0..512 — must never panic, must return a value
    // that is either 0 or a subset of CAP_ALL.
    let mut all_valid = true;
    for n in 0u32..512 {
        let cap = required_cap(n);
        if cap != 0 && (cap & !CAP_ALL) != 0 {
            all_valid = false;
            crate::serial_println!(
                "    required_cap({}) returned invalid bits: {:#010x}",
                n,
                cap
            );
        }
    }
    ctx.expect_true(
        all_valid,
        "required_cap(0..512) always returns 0 or subset of CAP_ALL",
    );
}

fn test_no_panic_high(ctx: &mut TestContext) {
    // Sparse sample of high syscall numbers — no crash, no undefined bits.
    let high_numbers: &[u32] = &[
        512,
        1000,
        0x7FFF,
        0xFFFF,
        0x0001_0000,
        0x00FF_FFFF,
        0x7FFF_FFFF,
        0xFFFF_FFFF,
    ];
    let mut all_valid = true;
    for &n in high_numbers {
        let cap = required_cap(n);
        if cap != 0 && (cap & !CAP_ALL) != 0 {
            all_valid = false;
        }
    }
    ctx.expect_true(all_valid, "required_cap(high) returns 0 or valid cap");
}

fn test_extreme_values(ctx: &mut TestContext) {
    // The catch-all `_` arm of required_cap returns 0.
    // Unknown syscalls must not be accidentally granted capabilities.
    ctx.expect_eq(required_cap(u32::MAX), 0u32, "required_cap(MAX) == 0");
    ctx.expect_eq(
        required_cap(0xDEAD_BEEF),
        0u32,
        "required_cap(DEAD_BEEF) == 0",
    );
    ctx.expect_eq(
        required_cap(0x1234_5678),
        0u32,
        "required_cap(12345678) == 0",
    );
}

// ── required_cap returns only valid bits ──────────────────────────────────────

fn test_valid_cap_bits(ctx: &mut TestContext) {
    // Every return value must be a subset of CAP_ALL (bits 0..14).
    // A value with bits above bit 14 set indicates a programming error
    // (wrong constant, bit-shift overflow) that would silently bypass
    // capability checks because the thread's CapSet could never match.
    let mut bad_count = 0u32;
    for n in 0u32..1000 {
        let cap = required_cap(n);
        if (cap & !CAP_ALL) != 0 {
            bad_count += 1;
        }
    }
    ctx.expect_eq(
        bad_count,
        0u32,
        "no required_cap value has bits outside CAP_ALL",
    );
}

// ── Capability check logic ────────────────────────────────────────────────────

fn test_deny_partial(ctx: &mut TestContext) {
    // (thread_caps & required) == required is the check pattern.
    // A thread with only half the required bits must be denied.
    let required = CAP_NETWORK | CAP_AUDIO;
    let partial = CAP_NETWORK; // missing AUDIO
    ctx.expect_false(
        (partial & required) == required,
        "partial caps denied: CAP_NETWORK only, needs NETWORK|AUDIO",
    );
}

fn test_deny_none(ctx: &mut TestContext) {
    // Zero capabilities denies everything except required==0.
    let required = CAP_PROCESS;
    ctx.expect_false(
        (0u32 & required) == required,
        "zero caps denied for CAP_PROCESS",
    );
    // Edge case: required==0 must always pass.
    ctx.expect_true((0u32 & 0u32) == 0u32, "zero caps passes for required==0");
}

fn test_allow_exact(ctx: &mut TestContext) {
    // A thread with exactly the required caps passes.
    let required = CAP_PIPE | CAP_EVENT;
    ctx.expect_true((required & required) == required, "exact match allowed");
    // A superset also passes.
    ctx.expect_true(
        (CAP_ALL & required) == required,
        "superset (CAP_ALL) allowed",
    );
}

fn test_cap_all_allows_all(ctx: &mut TestContext) {
    // CAP_ALL must satisfy every individual capability requirement.
    let individual_caps: &[u32] = &[
        CAP_NETWORK,
        CAP_AUDIO,
        CAP_PROCESS,
        CAP_PIPE,
        CAP_EVENT,
        CAP_COMPOSITOR,
        CAP_SYSTEM,
        CAP_DLL,
        CAP_THREAD,
        CAP_DEVICE,
        CAP_MANAGE_PERMS,
        CAP_DEBUG,
    ];
    let mut all_ok = true;
    for &cap in individual_caps {
        if (CAP_ALL & cap) != cap {
            all_ok = false;
        }
    }
    ctx.expect_true(all_ok, "CAP_ALL satisfies every individual capability");
}

// ── parse_capabilities with adversarial input ─────────────────────────────────

fn test_parse_garbage(ctx: &mut TestContext) {
    // Garbage strings must not panic and must return a valid subset of CAP_ALL.
    // A very long unknown capability name (512 'x' chars).
    let long_unknown = "x".repeat(512);
    let garbage_inputs: &[&str] = &[
        "",
        ",,,,,",
        "FILESYSTEM",          // wrong case
        "filesystem,,network", // double comma
        "  filesystem  ",      // spaces
        long_unknown.as_str(), // very long unknown cap
        "filesystem,network,audio,display,device,process,pipe,shm,event,compositor,system,dll,thread,manage_perms,debug,UNKNOWN,extra",
    ];
    let mut all_valid = true;
    for &input in garbage_inputs {
        let cap = parse_capabilities(input);
        if (cap & !CAP_ALL) != 0 {
            all_valid = false;
            crate::serial_println!(
                "    parse_capabilities(…) returned invalid bits: {:#010x}",
                cap
            );
        }
    }
    ctx.expect_true(
        all_valid,
        "parse_capabilities never returns bits outside CAP_ALL",
    );
}

// ── Individual cap bits are distinct ─────────────────────────────────────────

fn test_bits_non_overlapping(ctx: &mut TestContext) {
    // Every individual capability must occupy exactly one bit and must not
    // overlap with any other capability. Overlapping bits cause privilege
    // escalation: granting "audio" accidentally also grants "network".
    let caps: &[(&str, u32)] = &[
        ("CAP_NETWORK", CAP_NETWORK),
        ("CAP_AUDIO", CAP_AUDIO),
        ("CAP_PROCESS", CAP_PROCESS),
        ("CAP_PIPE", CAP_PIPE),
        ("CAP_EVENT", CAP_EVENT),
        ("CAP_COMPOSITOR", CAP_COMPOSITOR),
        ("CAP_SYSTEM", CAP_SYSTEM),
        ("CAP_DLL", CAP_DLL),
        ("CAP_THREAD", CAP_THREAD),
        ("CAP_DEVICE", CAP_DEVICE),
        ("CAP_MANAGE_PERMS", CAP_MANAGE_PERMS),
        ("CAP_DEBUG", CAP_DEBUG),
    ];
    // Each must be a power of two.
    let mut all_pow2 = true;
    for &(name, cap) in caps {
        if cap == 0 || (cap & (cap - 1)) != 0 {
            all_pow2 = false;
            crate::serial_println!("    {} is not a power of two: {:#010x}", name, cap);
        }
    }
    ctx.expect_true(all_pow2, "each capability is a single bit (power of two)");

    // Each pair must not share bits.
    let mut no_overlap = true;
    for i in 0..caps.len() {
        for j in (i + 1)..caps.len() {
            if caps[i].1 & caps[j].1 != 0 {
                no_overlap = false;
                crate::serial_println!("    {} and {} overlap!", caps[i].0, caps[j].0);
            }
        }
    }
    ctx.expect_true(no_overlap, "no two capabilities share a bit");
}

// ── Zero required_cap always passes ──────────────────────────────────────────

fn test_zero_always_passes(ctx: &mut TestContext) {
    // required_cap == 0 means "always allowed".
    // Verify the check formula (thread_caps & 0) == 0 holds for any thread_caps.
    for &caps in &[0u32, 1u32, CAP_ALL, 0xFFFF_FFFFu32] {
        ctx.expect_true(
            (caps & 0u32) == 0u32,
            "any thread passes when required == 0",
        );
    }
}

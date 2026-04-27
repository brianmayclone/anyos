//! Unit tests for the capability / permission system.
//!
//! `parse_capabilities` and `required_cap` are pure table-lookup functions —
//! no hardware, heap, or scheduler needed.

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::task::capabilities::{
    parse_capabilities, required_cap, CAP_ALL, CAP_AUDIO, CAP_AUTO_GRANTED, CAP_COMPOSITOR,
    CAP_DEBUG, CAP_DEFAULT, CAP_DEVICE, CAP_DLL, CAP_EVENT, CAP_FILESYSTEM, CAP_MANAGE_PERMS,
    CAP_NETWORK, CAP_PIPE, CAP_PROCESS, CAP_SHM, CAP_SYSTEM, CAP_THREAD,
};

pub static SUITE: TestSuite = TestSuite {
    name: "task::capabilities",
    cases: &[
        TestCase {
            name: "parse_single_cap",
            run: test_parse_single,
        },
        TestCase {
            name: "parse_multiple_caps",
            run: test_parse_multiple,
        },
        TestCase {
            name: "parse_unknown_ignored",
            run: test_parse_unknown,
        },
        TestCase {
            name: "parse_empty_string",
            run: test_parse_empty,
        },
        TestCase {
            name: "cap_all_has_all_bits",
            run: test_cap_all,
        },
        TestCase {
            name: "cap_default_subset",
            run: test_cap_default,
        },
        TestCase {
            name: "cap_auto_granted_subset",
            run: test_cap_auto_granted,
        },
        TestCase {
            name: "required_cap_basic_syscalls_zero",
            run: test_basic_syscalls,
        },
        TestCase {
            name: "required_cap_fs_always_allowed",
            run: test_fs_always_allowed,
        },
        TestCase {
            name: "required_cap_network",
            run: test_net_syscalls,
        },
        TestCase {
            name: "required_cap_audio",
            run: test_audio_syscalls,
        },
        TestCase {
            name: "required_cap_display",
            run: test_display_syscalls,
        },
        TestCase {
            name: "cap_check_logic",
            run: test_cap_check_logic,
        },
    ],
};

// ── parse_capabilities ────────────────────────────────────────────────────────

fn test_parse_single(ctx: &mut TestContext) {
    ctx.expect_eq(
        parse_capabilities("filesystem"),
        CAP_FILESYSTEM,
        "filesystem",
    );
    ctx.expect_eq(parse_capabilities("network"), CAP_NETWORK, "network");
    ctx.expect_eq(parse_capabilities("audio"), CAP_AUDIO, "audio");
    ctx.expect_eq(parse_capabilities("device"), CAP_DEVICE, "device");
    ctx.expect_eq(parse_capabilities("process"), CAP_PROCESS, "process");
    ctx.expect_eq(parse_capabilities("pipe"), CAP_PIPE, "pipe");
    ctx.expect_eq(parse_capabilities("shm"), CAP_SHM, "shm");
    ctx.expect_eq(parse_capabilities("event"), CAP_EVENT, "event");
    ctx.expect_eq(
        parse_capabilities("compositor"),
        CAP_COMPOSITOR,
        "compositor",
    );
    ctx.expect_eq(parse_capabilities("system"), CAP_SYSTEM, "system");
    ctx.expect_eq(parse_capabilities("dll"), CAP_DLL, "dll");
    ctx.expect_eq(parse_capabilities("thread"), CAP_THREAD, "thread");
    ctx.expect_eq(
        parse_capabilities("manage_perms"),
        CAP_MANAGE_PERMS,
        "manage_perms",
    );
    ctx.expect_eq(parse_capabilities("debug"), CAP_DEBUG, "debug");
}

fn test_parse_multiple(ctx: &mut TestContext) {
    let caps = parse_capabilities("filesystem,network,audio");
    ctx.expect_eq(
        caps,
        CAP_FILESYSTEM | CAP_NETWORK | CAP_AUDIO,
        "filesystem+network+audio",
    );

    let all_str = "filesystem,network,audio,display,device,process,pipe,shm,event,\
                   compositor,system,dll,thread,manage_perms,debug";
    ctx.expect_eq(
        parse_capabilities(all_str),
        CAP_ALL,
        "all caps string == CAP_ALL",
    );
}

fn test_parse_unknown(ctx: &mut TestContext) {
    // Unknown capability names are silently ignored.
    ctx.expect_eq(parse_capabilities("bogus"), 0u32, "unknown == 0");
    ctx.expect_eq(
        parse_capabilities("filesystem,bogus"),
        CAP_FILESYSTEM,
        "unknown ignored",
    );
    ctx.expect_eq(parse_capabilities("FILESYSTEM"), 0u32, "case-sensitive");
}

fn test_parse_empty(ctx: &mut TestContext) {
    ctx.expect_eq(parse_capabilities(""), 0u32, "empty string == 0");
    ctx.expect_eq(parse_capabilities(","), 0u32, "comma-only == 0");
}

// ── Capability constants ──────────────────────────────────────────────────────

fn test_cap_all(ctx: &mut TestContext) {
    // CAP_ALL must have all 15 bits set.
    ctx.expect_eq(CAP_ALL, (1u32 << 15) - 1, "CAP_ALL == 0x7FFF");
    // Every individual cap must be a subset of CAP_ALL.
    for (name, cap) in &[
        ("filesystem", CAP_FILESYSTEM),
        ("network", CAP_NETWORK),
        ("audio", CAP_AUDIO),
        ("device", CAP_DEVICE),
        ("process", CAP_PROCESS),
        ("pipe", CAP_PIPE),
        ("shm", CAP_SHM),
        ("event", CAP_EVENT),
        ("compositor", CAP_COMPOSITOR),
        ("system", CAP_SYSTEM),
        ("dll", CAP_DLL),
        ("thread", CAP_THREAD),
        ("manage_perms", CAP_MANAGE_PERMS),
        ("debug", CAP_DEBUG),
    ] {
        ctx.expect_eq(*cap & CAP_ALL, *cap, name);
    }
}

fn test_cap_default(ctx: &mut TestContext) {
    // CAP_DEFAULT must include at least filesystem and process.
    ctx.expect_eq(
        CAP_DEFAULT & CAP_FILESYSTEM,
        CAP_FILESYSTEM,
        "default includes filesystem",
    );
    ctx.expect_eq(
        CAP_DEFAULT & CAP_PROCESS,
        CAP_PROCESS,
        "default includes process",
    );
    // Default must not include privileged caps.
    ctx.expect_eq(CAP_DEFAULT & CAP_SYSTEM, 0u32, "default excludes system");
    ctx.expect_eq(CAP_DEFAULT & CAP_DEBUG, 0u32, "default excludes debug");
    ctx.expect_eq(
        CAP_DEFAULT & CAP_MANAGE_PERMS,
        0u32,
        "default excludes manage_perms",
    );
}

fn test_cap_auto_granted(ctx: &mut TestContext) {
    // Auto-granted caps must be a subset of ALL.
    ctx.expect_eq(
        CAP_AUTO_GRANTED & !CAP_ALL,
        0u32,
        "auto_granted subset of CAP_ALL",
    );
    // DLL + thread should always be auto-granted.
    ctx.expect_eq(CAP_AUTO_GRANTED & CAP_DLL, CAP_DLL, "dll auto-granted");
    ctx.expect_eq(
        CAP_AUTO_GRANTED & CAP_THREAD,
        CAP_THREAD,
        "thread auto-granted",
    );
}

// ── required_cap ─────────────────────────────────────────────────────────────

fn test_basic_syscalls(ctx: &mut TestContext) {
    // Syscalls in the range typically used for EXIT, GETPID, YIELD, SLEEP
    // require no capabilities (return 0).
    // Syscall numbers 0-3 are universally unrestricted.
    for n in [0u32, 1, 2, 3] {
        ctx.expect_eq(required_cap(n), 0u32, "basic syscall requires no cap");
    }
}

fn test_fs_always_allowed(ctx: &mut TestContext) {
    // By design, all filesystem syscalls (open/read/write/close/stat/…) are
    // "always allowed" — required_cap returns 0 for them.  Every app needs
    // basic file access; CAP_FILESYSTEM gates the capability string parsing
    // only, not the syscall dispatch.
    use crate::syscall;
    ctx.expect_eq(
        required_cap(syscall::SYS_OPEN),
        0u32,
        "SYS_OPEN always allowed",
    );
    ctx.expect_eq(
        required_cap(syscall::SYS_READ),
        0u32,
        "SYS_READ always allowed",
    );
    ctx.expect_eq(
        required_cap(syscall::SYS_WRITE),
        0u32,
        "SYS_WRITE always allowed",
    );
    ctx.expect_eq(
        required_cap(syscall::SYS_CLOSE),
        0u32,
        "SYS_CLOSE always allowed",
    );
    ctx.expect_eq(
        required_cap(syscall::SYS_READDIR),
        0u32,
        "SYS_READDIR always allowed",
    );
    ctx.expect_eq(
        required_cap(syscall::SYS_STAT),
        0u32,
        "SYS_STAT always allowed",
    );
}

fn test_net_syscalls(ctx: &mut TestContext) {
    let mut found = false;
    for n in 0..200u32 {
        if required_cap(n) & CAP_NETWORK != 0 {
            found = true;
            break;
        }
    }
    ctx.expect_true(found, "at least one syscall requires CAP_NETWORK");
}

fn test_audio_syscalls(ctx: &mut TestContext) {
    let mut found = false;
    for n in 0..200u32 {
        if required_cap(n) & CAP_AUDIO != 0 {
            found = true;
            break;
        }
    }
    ctx.expect_true(found, "at least one syscall requires CAP_AUDIO");
}

fn test_display_syscalls(ctx: &mut TestContext) {
    // At least one syscall must require the display/compositor capability.
    let mut found = false;
    for n in 0..200u32 {
        if required_cap(n) & CAP_COMPOSITOR != 0 {
            found = true;
            break;
        }
    }
    ctx.expect_true(found, "at least one syscall requires CAP_COMPOSITOR");
}

fn test_cap_check_logic(ctx: &mut TestContext) {
    // Simulate the kernel's capability-check pattern:
    // (thread_caps & required) == required.
    let required = CAP_FILESYSTEM | CAP_NETWORK;

    // Thread with both caps: allowed.
    let thread_caps_ok = CAP_ALL;
    ctx.expect_true(
        (thread_caps_ok & required) == required,
        "thread with all caps passes check",
    );

    // Thread with only one cap: denied.
    let thread_caps_partial = CAP_FILESYSTEM;
    ctx.expect_false(
        (thread_caps_partial & required) == required,
        "thread with only filesystem cap fails network check",
    );

    // Thread with no caps: denied.
    ctx.expect_false((0u32 & required) == required, "thread with no caps denied");
}

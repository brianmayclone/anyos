//! ARM64 PSCI detection and invocation.
//!
//! Parses the DTB once at boot to determine the PSCI conduit (`hvc` vs `smc`)
//! and to cache the MPIDR targets for each logical CPU.

use core::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};

use crate::arch::hal::MAX_CPUS;

const FDT_MAGIC: u32 = 0xD00D_FEED;
const FDT_BEGIN_NODE: u32 = 0x1;
const FDT_END_NODE: u32 = 0x2;
const FDT_PROP: u32 = 0x3;
const FDT_NOP: u32 = 0x4;
const FDT_END: u32 = 0x9;

const CONDUIT_HVC: u8 = 0;
const CONDUIT_SMC: u8 = 1;

static CONDUIT: AtomicU8 = AtomicU8::new(CONDUIT_HVC);
static CPU_COUNT: AtomicUsize = AtomicUsize::new(0);
static CPU_MPIDS: [AtomicU64; MAX_CPUS] = {
    const INIT: AtomicU64 = AtomicU64::new(u64::MAX);
    [INIT; MAX_CPUS]
};

#[inline]
fn be32(p: *const u8) -> u32 {
    unsafe {
        let b = core::slice::from_raw_parts(p, 4);
        ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | (b[3] as u32)
    }
}

#[inline]
fn align4(ptr: *const u8) -> *const u8 {
    let align = ptr as usize & 0x3;
    if align == 0 {
        ptr
    } else {
        unsafe { ptr.add(4 - align) }
    }
}

#[derive(Clone, Copy)]
struct NodeInfo<'a> {
    name: &'a [u8],
    depth: i32,
}

fn node_name<'a>(ptr: &mut *const u8) -> &'a [u8] {
    let name_ptr = *ptr;
    let mut len = 0usize;
    while unsafe { *name_ptr.add(len) } != 0 {
        len += 1;
    }
    let padded = (len + 1 + 3) & !3;
    *ptr = unsafe { name_ptr.add(padded) };
    unsafe { core::slice::from_raw_parts(name_ptr, len) }
}

fn prop_name<'a>(strings: *const u8, nameoff: usize) -> &'a [u8] {
    let p = unsafe { strings.add(nameoff) };
    let mut len = 0usize;
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    unsafe { core::slice::from_raw_parts(p, len) }
}

fn parse_reg_value(data_ptr: *const u8, len: usize) -> Option<u64> {
    match len {
        4 => Some(be32(data_ptr) as u64),
        l if l >= 8 => {
            Some(((be32(data_ptr) as u64) << 32) | be32(unsafe { data_ptr.add(4) }) as u64)
        }
        _ => None,
    }
}

pub fn init() {
    for slot in &CPU_MPIDS {
        slot.store(u64::MAX, Ordering::Relaxed);
    }
    CPU_COUNT.store(0, Ordering::Relaxed);
    CONDUIT.store(CONDUIT_HVC, Ordering::Relaxed);

    let dtb_phys = super::boot::dtb_addr();
    if dtb_phys == 0 {
        return;
    }

    let base = dtb_phys as *const u8;
    if be32(base) != FDT_MAGIC {
        return;
    }

    let off_struct = be32(unsafe { base.add(8) }) as usize;
    let off_strings = be32(unsafe { base.add(12) }) as usize;
    let strings = unsafe { base.add(off_strings) };
    let mut ptr = unsafe { base.add(off_struct) };

    let mut depth: i32 = 0;
    let mut node_stack: [Option<NodeInfo>; 8] = [None; 8];
    let mut in_psci = false;
    let mut in_cpu_node = false;
    let mut cpu_slot = 0usize;

    loop {
        ptr = align4(ptr);
        let token = be32(ptr);
        ptr = unsafe { ptr.add(4) };

        match token {
            FDT_BEGIN_NODE => {
                let name = node_name(&mut ptr);
                depth += 1;

                let idx = depth as usize;
                if idx < node_stack.len() {
                    node_stack[idx] = Some(NodeInfo { name, depth });
                }

                let parent = if depth > 1 {
                    node_stack.get((depth - 1) as usize).and_then(|n| *n)
                } else {
                    None
                };

                in_psci = depth == 1 && name.starts_with(b"psci");
                in_cpu_node = false;
                if let Some(parent) = parent {
                    if parent.depth == 1 && parent.name == b"cpus" && name.starts_with(b"cpu") {
                        in_cpu_node = true;
                    }
                }
            }
            FDT_END_NODE => {
                if in_cpu_node {
                    in_cpu_node = false;
                } else if in_psci {
                    in_psci = false;
                }
                if depth > 0 {
                    let idx = depth as usize;
                    if idx < node_stack.len() {
                        node_stack[idx] = None;
                    }
                    depth -= 1;
                }
            }
            FDT_PROP => {
                let prop_len = be32(ptr) as usize;
                ptr = unsafe { ptr.add(4) };
                let nameoff = be32(ptr) as usize;
                ptr = unsafe { ptr.add(4) };
                let data_ptr = ptr;
                ptr = unsafe { ptr.add((prop_len + 3) & !3) };

                let pname = prop_name(strings, nameoff);

                if in_psci && pname == b"method" && prop_len >= 3 {
                    let method = unsafe { core::slice::from_raw_parts(data_ptr, prop_len) };
                    if method.starts_with(b"smc") {
                        CONDUIT.store(CONDUIT_SMC, Ordering::Relaxed);
                    } else if method.starts_with(b"hvc") {
                        CONDUIT.store(CONDUIT_HVC, Ordering::Relaxed);
                    }
                } else if in_cpu_node && pname == b"reg" && cpu_slot < MAX_CPUS {
                    if let Some(mpidr) = parse_reg_value(data_ptr, prop_len) {
                        CPU_MPIDS[cpu_slot].store(mpidr, Ordering::Relaxed);
                        cpu_slot += 1;
                    }
                }
            }
            FDT_NOP => {}
            FDT_END | _ => break,
        }
    }

    CPU_COUNT.store(cpu_slot, Ordering::Relaxed);
}

pub fn conduit_name() -> &'static str {
    match CONDUIT.load(Ordering::Relaxed) {
        CONDUIT_SMC => "smc",
        _ => "hvc",
    }
}

#[inline]
pub fn prefers_hvc() -> bool {
    CONDUIT.load(Ordering::Relaxed) != CONDUIT_SMC
}

#[inline]
pub fn set_prefer_hvc(hvc: bool) {
    CONDUIT.store(
        if hvc { CONDUIT_HVC } else { CONDUIT_SMC },
        Ordering::Relaxed,
    );
}

pub fn logical_cpu_id_from_mpidr(mpidr: u64) -> usize {
    let count = CPU_COUNT.load(Ordering::Relaxed).min(MAX_CPUS);
    for cpu in 0..count {
        if CPU_MPIDS[cpu].load(Ordering::Relaxed) == mpidr {
            return cpu;
        }
    }
    (mpidr & 0xFF) as usize
}

pub fn cpu_target_mpidr(cpu: usize) -> u64 {
    if cpu < MAX_CPUS {
        let cached = CPU_MPIDS[cpu].load(Ordering::Relaxed);
        if cached != u64::MAX {
            return cached;
        }
    }
    cpu as u64
}

pub fn call(function_id: u64, arg0: u64, arg1: u64, arg2: u64) -> i64 {
    if prefers_hvc() {
        call_hvc(function_id, arg0, arg1, arg2)
    } else {
        call_smc(function_id, arg0, arg1, arg2)
    }
}

pub fn call_hvc(function_id: u64, arg0: u64, arg1: u64, arg2: u64) -> i64 {
    let result: i64;
    unsafe {
        core::arch::asm!(
            "mov x0, {fn_id}",
            "mov x1, {arg0}",
            "mov x2, {arg1}",
            "mov x3, {arg2}",
            ".inst 0xd4000002",
            "mov {result}, x0",
            fn_id = in(reg) function_id,
            arg0 = in(reg) arg0,
            arg1 = in(reg) arg1,
            arg2 = in(reg) arg2,
            result = lateout(reg) result,
            out("x0") _, out("x1") _, out("x2") _, out("x3") _,
            options(nostack),
        );
    }
    result
}

pub fn call_smc(function_id: u64, arg0: u64, arg1: u64, arg2: u64) -> i64 {
    let result: i64;
    unsafe {
        core::arch::asm!(
            "mov x0, {fn_id}",
            "mov x1, {arg0}",
            "mov x2, {arg1}",
            "mov x3, {arg2}",
            ".inst 0xd4000003",
            "mov {result}, x0",
            fn_id = in(reg) function_id,
            arg0 = in(reg) arg0,
            arg1 = in(reg) arg1,
            arg2 = in(reg) arg2,
            result = lateout(reg) result,
            out("x0") _, out("x1") _, out("x2") _, out("x3") _,
            options(nostack),
        );
    }
    result
}

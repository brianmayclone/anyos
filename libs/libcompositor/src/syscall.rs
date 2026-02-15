//! Raw syscall interface for libcompositor DLL.
//!
//! DLLs cannot use anyos_std (no global allocator, no entry point).
//! Raw SYSCALL instruction with anyOS x86-64 calling convention:
//!   RAX = syscall number
//!   RBX = arg1, R10 = arg2, RDX = arg3, RSI = arg4, RDI = arg5
//!   Return value in RAX
//!   Clobbers: RCX (user RIP), R11 (user RFLAGS)
//!
//! NOTE: LLVM reserves RBX on x86_64. We push/pop it inside the asm block.

// Syscall numbers (must match kernel/src/syscall/mod.rs)
const SYS_GETPID: u64 = 6;
const SYS_SLEEP: u64 = 8;
const SYS_SCREEN_SIZE: u64 = 72;
const SYS_SHM_CREATE: u64 = 140;
const SYS_SHM_MAP: u64 = 141;
const SYS_SHM_UNMAP: u64 = 142;
const SYS_SHM_DESTROY: u64 = 143;
const SYS_EVT_CHAN_CREATE: u64 = 63;
const SYS_EVT_CHAN_SUBSCRIBE: u64 = 64;
const SYS_EVT_CHAN_EMIT: u64 = 65;
const SYS_EVT_CHAN_POLL: u64 = 66;
const SYS_EVT_CHAN_EMIT_TO: u64 = 69;

#[inline(always)]
unsafe fn syscall0(n: u64) -> u64 {
    let result: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") n => result,
        out("rcx") _,
        out("r11") _,
    );
    result
}

#[inline(always)]
unsafe fn syscall1(n: u64, a1: u64) -> u64 {
    let result: u64;
    core::arch::asm!(
        "push rbx",
        "mov rbx, {a1}",
        "syscall",
        "pop rbx",
        a1 = in(reg) a1,
        inlateout("rax") n => result,
        out("rcx") _,
        out("r11") _,
    );
    result
}

#[inline(always)]
unsafe fn syscall2(n: u64, a1: u64, a2: u64) -> u64 {
    let result: u64;
    core::arch::asm!(
        "push rbx",
        "mov rbx, {a1}",
        "syscall",
        "pop rbx",
        a1 = in(reg) a1,
        inlateout("rax") n => result,
        in("r10") a2,
        out("rcx") _,
        out("r11") _,
    );
    result
}

#[inline(always)]
unsafe fn syscall3(n: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let result: u64;
    core::arch::asm!(
        "push rbx",
        "mov rbx, {a1}",
        "syscall",
        "pop rbx",
        a1 = in(reg) a1,
        inlateout("rax") n => result,
        in("r10") a2,
        in("rdx") a3,
        out("rcx") _,
        out("r11") _,
    );
    result
}

// ── IPC Wrappers ─────────────────────────────────────────────────────────────

pub fn get_tid() -> u32 {
    unsafe { syscall0(SYS_GETPID) as u32 }
}

pub fn sleep(ms: u32) {
    unsafe { syscall1(SYS_SLEEP, ms as u64); }
}

pub fn screen_size(out_w: *mut u32, out_h: *mut u32) {
    let mut buf = [0u32; 2];
    unsafe { syscall1(SYS_SCREEN_SIZE, buf.as_mut_ptr() as u64); }
    unsafe {
        *out_w = buf[0];
        *out_h = buf[1];
    }
}

pub fn shm_create(size: u32) -> u32 {
    unsafe { syscall1(SYS_SHM_CREATE, size as u64) as u32 }
}

pub fn shm_map(shm_id: u32) -> u64 {
    unsafe { syscall1(SYS_SHM_MAP, shm_id as u64) }
}

pub fn shm_unmap(shm_id: u32) -> u32 {
    unsafe { syscall1(SYS_SHM_UNMAP, shm_id as u64) as u32 }
}

pub fn shm_destroy(shm_id: u32) -> u32 {
    unsafe { syscall1(SYS_SHM_DESTROY, shm_id as u64) as u32 }
}

pub fn evt_chan_create(name_ptr: *const u8, name_len: u32) -> u32 {
    unsafe { syscall2(SYS_EVT_CHAN_CREATE, name_ptr as u64, name_len as u64) as u32 }
}

pub fn evt_chan_subscribe(channel_id: u32, filter: u32) -> u32 {
    unsafe { syscall2(SYS_EVT_CHAN_SUBSCRIBE, channel_id as u64, filter as u64) as u32 }
}

pub fn evt_chan_emit(channel_id: u32, event: *const [u32; 5]) {
    unsafe { syscall2(SYS_EVT_CHAN_EMIT, channel_id as u64, event as u64); }
}

pub fn evt_chan_poll(channel_id: u32, sub_id: u32, buf: *mut [u32; 5]) -> bool {
    unsafe { syscall3(SYS_EVT_CHAN_POLL, channel_id as u64, sub_id as u64, buf as u64) != 0 }
}

pub fn evt_chan_emit_to(channel_id: u32, sub_id: u32, event: *const [u32; 5]) {
    unsafe { syscall3(SYS_EVT_CHAN_EMIT_TO, channel_id as u64, sub_id as u64, event as u64); }
}

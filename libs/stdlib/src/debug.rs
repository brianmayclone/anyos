//! Debug / trace API for anyTrace.
//!
//! Provides userspace wrappers for the debug syscalls (300-313).
//! All functions require `CAP_DEBUG`.

use crate::raw::*;

// ---- Debug event types ----

/// Thread hit a software breakpoint (INT3).
pub const EVENT_BREAKPOINT: u32 = 1;
/// Thread completed a single-step (#DB with TF).
pub const EVENT_SINGLE_STEP: u32 = 2;
/// Thread exited while debug-attached.
pub const EVENT_EXIT: u32 = 3;

// ---- Types ----

/// CPU register state (160 bytes = 20 x u64).
///
/// Layout matches `CpuContext` in the kernel (first 160 bytes):
///   rax, rbx, rcx, rdx, rsi, rdi, rbp, r8, r9, r10, r11, r12, r13, r14, r15,
///   rsp, rip, rflags, cr3
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DebugRegs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rsp: u64,
    pub rip: u64,
    pub rflags: u64,
    pub cr3: u64,
    _reserved: u64,
}

impl Default for DebugRegs {
    fn default() -> Self {
        Self {
            rax: 0, rbx: 0, rcx: 0, rdx: 0,
            rsi: 0, rdi: 0, rbp: 0,
            r8: 0, r9: 0, r10: 0, r11: 0,
            r12: 0, r13: 0, r14: 0, r15: 0,
            rsp: 0, rip: 0, rflags: 0, cr3: 0,
            _reserved: 0,
        }
    }
}

/// Debug event received from the kernel.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DebugEvent {
    /// Event type: `EVENT_BREAKPOINT`, `EVENT_SINGLE_STEP`, or `EVENT_EXIT`.
    pub event_type: u32,
    /// Address associated with the event (RIP at breakpoint/step, exit code for exit).
    pub addr: u64,
}

/// A contiguous region in the target's virtual address space.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MemoryRegion {
    /// Start address (inclusive).
    pub start: u64,
    /// End address (exclusive).
    pub end: u64,
    /// Page table flags (P, RW, US, NX etc.).
    pub flags: u64,
}

/// Extended thread information (128 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ThreadInfoEx {
    pub parent_tid: u32,
    pub state: u32,
    pub priority: u32,
    pub cpu_ticks: u32,
    pub last_cpu: u32,
    pub user_pages: u32,
    pub brk: u32,
    pub mmap_next: u32,
    pub rip: u64,
    pub rsp: u64,
    pub cr3: u64,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
    pub capabilities: u32,
    pub uid: u16,
    pub gid: u16,
    pub debug_attached_by: u32,
    pub name: [u8; 32],
    pub arch_mode: u32,
    pub _reserved: [u8; 8],
}

impl Default for ThreadInfoEx {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}

// ---- API ----

/// Attach to a running thread as debugger.
///
/// The target thread is suspended. Returns `true` on success.
pub fn attach(tid: u32) -> bool {
    syscall1(SYS_DEBUG_ATTACH, tid as u64) == 0
}

/// Detach from a previously attached thread.
///
/// All breakpoints are removed and the thread resumes.
pub fn detach(tid: u32) -> bool {
    syscall1(SYS_DEBUG_DETACH, tid as u64) == 0
}

/// Suspend a debug-attached thread.
pub fn suspend(tid: u32) -> bool {
    syscall1(SYS_DEBUG_SUSPEND, tid as u64) == 0
}

/// Resume a suspended debug-attached thread.
pub fn resume(tid: u32) -> bool {
    syscall1(SYS_DEBUG_RESUME, tid as u64) == 0
}

/// Read the target thread's register state.
///
/// Returns `true` if registers were read successfully.
pub fn get_regs(tid: u32, regs: &mut DebugRegs) -> bool {
    let buf = regs as *mut DebugRegs as u64;
    let size = core::mem::size_of::<DebugRegs>() as u32;
    let ret = syscall3(SYS_DEBUG_GET_REGS, tid as u64, buf, size as u64);
    ret != u32::MAX
}

/// Write register state to the target thread.
///
/// The kernel validates RIP/RSP (must be in user-space) and masks RFLAGS.
pub fn set_regs(tid: u32, regs: &DebugRegs) -> bool {
    let buf = regs as *const DebugRegs as u64;
    let size = core::mem::size_of::<DebugRegs>() as u32;
    syscall3(SYS_DEBUG_SET_REGS, tid as u64, buf, size as u64) == 0
}

/// Read memory from the target thread's address space.
///
/// Returns the number of bytes read, or 0 on error.
pub fn read_mem(tid: u32, addr: u64, buf: &mut [u8]) -> usize {
    let len = buf.len().min(4096) as u32;
    let ret = syscall4(
        SYS_DEBUG_READ_MEM,
        tid as u64,
        addr,
        len as u64,
        buf.as_mut_ptr() as u64,
    );
    if ret == u32::MAX { 0 } else { ret as usize }
}

/// Write memory into the target thread's address space.
///
/// Returns the number of bytes written, or 0 on error.
pub fn write_mem(tid: u32, addr: u64, data: &[u8]) -> usize {
    let len = data.len().min(256) as u32;
    let ret = syscall4(
        SYS_DEBUG_WRITE_MEM,
        tid as u64,
        addr,
        len as u64,
        data.as_ptr() as u64,
    );
    if ret == u32::MAX { 0 } else { ret as usize }
}

/// Set a software breakpoint (INT3) at the given address.
pub fn set_breakpoint(tid: u32, addr: u64) -> bool {
    syscall2(SYS_DEBUG_SET_BREAKPOINT, tid as u64, addr) == 0
}

/// Clear a software breakpoint, restoring the original byte.
pub fn clear_breakpoint(tid: u32, addr: u64) -> bool {
    syscall2(SYS_DEBUG_CLR_BREAKPOINT, tid as u64, addr) == 0
}

/// Single-step: execute one instruction and suspend.
///
/// This sets RFLAGS.TF and resumes the thread. After one instruction,
/// the CPU triggers #DB and the thread is auto-suspended with a
/// `EVENT_SINGLE_STEP` event.
pub fn single_step(tid: u32) -> bool {
    syscall1(SYS_DEBUG_SINGLE_STEP, tid as u64) == 0
}

/// Poll for a debug event on the target thread.
///
/// Non-blocking. Returns the event type (1=breakpoint, 2=single-step, 3=exit)
/// or 0 if no event is pending.
pub fn wait_event(tid: u32, event: &mut DebugEvent) -> u32 {
    let buf = event as *mut DebugEvent as u64;
    let size = core::mem::size_of::<DebugEvent>() as u32;
    syscall3(SYS_DEBUG_WAIT_EVENT, tid as u64, buf, size as u64)
}

/// Get the target thread's virtual memory map.
///
/// Returns the number of regions written to the buffer.
pub fn get_memory_map(tid: u32, regions: &mut [MemoryRegion]) -> usize {
    let buf = regions.as_mut_ptr() as u64;
    let size = (regions.len() * core::mem::size_of::<MemoryRegion>()) as u32;
    let ret = syscall3(SYS_DEBUG_GET_MEM_MAP, tid as u64, buf, size as u64);
    if ret == u32::MAX { 0 } else { ret as usize }
}

/// Get extended information about a thread.
///
/// Returns `true` if the info was read successfully.
pub fn thread_info_ex(tid: u32, info: &mut ThreadInfoEx) -> bool {
    let buf = info as *mut ThreadInfoEx as u64;
    let size = core::mem::size_of::<ThreadInfoEx>() as u32;
    let ret = syscall3(SYS_THREAD_INFO_EX, tid as u64, buf, size as u64);
    ret != u32::MAX
}

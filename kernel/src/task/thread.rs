//! Thread data structure and lifecycle state management.
//!
//! Each thread has a unique TID, a kernel stack, a saved CPU context for context switching,
//! and optional per-process state (page directory, heap break, arguments).

use crate::memory::address::PhysAddr;
use crate::task::context::CpuContext;
use alloc::boxed::Box;
use alloc::vec;

static mut NEXT_TID: u32 = 1;

/// Architecture mode for user-space threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchMode {
    /// Native 64-bit long mode (CS=0x2B).
    Native64 = 0,
    /// 32-bit compatibility mode under long mode (CS=0x1B).
    Compat32 = 1,
}

/// Execution state of a thread in the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// Eligible to be picked by the scheduler.
    Ready,
    /// Currently executing on the CPU.
    Running,
    /// Waiting for an event (e.g. waitpid) and not schedulable.
    Blocked,
    /// Finished execution; awaiting reaping by the scheduler.
    Terminated,
}

/// Saved FPU/SSE state for FXSAVE/FXRSTOR (512 bytes, 16-byte aligned).
#[repr(C, align(16))]
pub struct FxState {
    pub data: [u8; 512],
}

impl FxState {
    /// Create a new FxState with default values (all exceptions masked).
    pub fn new_default() -> Self {
        let mut s = FxState { data: [0u8; 512] };
        // FCW (x87 control word) at offset 0: 0x037F = all x87 exceptions masked
        s.data[0] = 0x7F;
        s.data[1] = 0x03;
        // MXCSR (SSE control/status) at offset 24: 0x1F80 = all SSE exceptions masked
        s.data[24] = 0x80;
        s.data[25] = 0x1F;
        s
    }
}

impl Default for FxState {
    fn default() -> Self {
        Self::new_default()
    }
}

/// A kernel or user thread with its own stack, saved context, and process metadata.
pub struct Thread {
    pub tid: u32,
    pub state: ThreadState,
    pub context: CpuContext,
    pub kernel_stack: Box<[u8]>,
    pub priority: u8,
    pub name: [u8; 32],
    pub exit_code: Option<u32>,
    pub waiting_tid: Option<u32>,
    pub is_user: bool,
    /// Per-process page directory (None for kernel threads that share the kernel PD).
    pub page_directory: Option<PhysAddr>,
    /// Current program break (end of data/heap segment) for user processes.
    pub brk: u32,
    /// Command-line arguments (null-terminated string, set at spawn time).
    pub args: [u8; 256],
    /// Pipe ID for stdout redirection (0 = no pipe, write to serial).
    pub stdout_pipe: u32,
    /// CPU ticks consumed by this thread (incremented each scheduler tick while running).
    pub cpu_ticks: u32,
    /// Architecture mode for user threads (Native64 or Compat32).
    pub arch_mode: ArchMode,
    /// Saved FPU/SSE register state (512 bytes, FXSAVE format).
    pub fpu_state: FxState,
    /// PIT tick at which a sleeping thread should be woken (None = not sleeping).
    pub wake_at_tick: Option<u32>,
    /// PIT tick at which this thread was terminated (for auto-reap grace period).
    pub terminated_at_tick: Option<u32>,
    /// True if this thread shares its page directory with another thread (intra-process child).
    /// When true, sys_exit must NOT destroy the page directory.
    pub pd_shared: bool,
    /// Last CPU this thread ran on (for affinity when re-queuing after wake/unblock).
    pub last_cpu: usize,
}

/// Size of each thread's kernel-mode stack.
const KERNEL_STACK_SIZE: usize = 128 * 1024; // 128 KiB per thread

/// Magic canary value placed at the bottom of each kernel stack.
/// If this gets overwritten, the stack has overflowed.
pub const STACK_CANARY: u64 = 0xDEAD_BEEF_CAFE_BABE;

impl Thread {
    /// Create a new kernel thread that will begin executing at `entry`.
    ///
    /// The thread is initialized in the `Ready` state with its own kernel stack
    /// and a CPU context pointing to the entry function.
    pub fn new(entry: extern "C" fn(), priority: u8, name: &str) -> Self {
        let tid = unsafe {
            let t = NEXT_TID;
            NEXT_TID += 1;
            t
        };

        // Allocate kernel stack on the heap directly (NOT via Box::new which
        // would create a 16 KiB temporary on the current stack — fatal when
        // called from a syscall where the kernel stack is only 16 KiB).
        let stack: Box<[u8]> = vec![0u8; KERNEL_STACK_SIZE].into_boxed_slice();
        let stack_top = stack.as_ptr() as u64 + KERNEL_STACK_SIZE as u64;

        // Write canary at the bottom of the stack to detect overflow
        unsafe {
            *(stack.as_ptr() as *mut u64) = STACK_CANARY;
        }

        // Set up initial context so that when we "switch" to this thread,
        // it starts executing at `entry`.
        // RSP is set to stack_top - 8 for proper 16-byte ABI alignment:
        // the push+ret in context_switch results in RSP = (stack_top - 8)
        // at function entry, which satisfies RSP % 16 == 8.
        let mut context = CpuContext::default();
        context.rip = entry as *const () as u64;
        context.rsp = stack_top - 8;
        context.rbp = stack_top;
        context.rflags = 0x202; // IF (interrupts enabled) + reserved bit 1
        // Use the current page directory (all kernel threads share same address space)
        unsafe { core::arch::asm!("mov {}, cr3", out(reg) context.cr3); }

        // Copy name
        let mut name_buf = [0u8; 32];
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        name_buf[..len].copy_from_slice(&bytes[..len]);

        Thread {
            tid,
            state: ThreadState::Ready,
            context,
            kernel_stack: stack as Box<[u8]>,
            priority,
            name: name_buf,
            exit_code: None,
            waiting_tid: None,
            is_user: false,
            page_directory: None,
            brk: 0,
            args: [0u8; 256],
            stdout_pipe: 0,
            cpu_ticks: 0,
            arch_mode: ArchMode::Native64,
            fpu_state: FxState::new_default(),
            wake_at_tick: None,
            terminated_at_tick: None,
            pd_shared: false,
            last_cpu: 0,
        }
    }

    /// Return the top (highest address) of this thread's kernel stack.
    pub fn kernel_stack_top(&self) -> u64 {
        self.kernel_stack.as_ptr() as u64 + self.kernel_stack.len() as u64
    }

    /// Return the bottom (lowest address) of this thread's kernel stack.
    pub fn kernel_stack_bottom(&self) -> u64 {
        self.kernel_stack.as_ptr() as u64
    }

    /// Check if the stack canary is intact. Returns false if the stack overflowed.
    pub fn check_stack_canary(&self) -> bool {
        unsafe { *(self.kernel_stack.as_ptr() as *const u64) == STACK_CANARY }
    }

    /// Return the thread name as a UTF-8 string slice.
    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..len]).unwrap_or("???")
    }
}

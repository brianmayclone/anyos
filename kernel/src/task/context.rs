//! Thread CPU context for cooperative/preemptive context switching (x86-64).
//!
//! Defines the register state saved and restored by `context_switch.asm` and provides
//! the FFI declaration for the assembly-level context switch routine.

/// CPU context saved/restored during a context switch.
/// Must match the layout expected by context_switch.asm (160 bytes total).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CpuContext {
    pub rax: u64,       // offset 0
    pub rbx: u64,       // offset 8
    pub rcx: u64,       // offset 16
    pub rdx: u64,       // offset 24
    pub rsi: u64,       // offset 32
    pub rdi: u64,       // offset 40
    pub rbp: u64,       // offset 48
    pub r8: u64,        // offset 56
    pub r9: u64,        // offset 64
    pub r10: u64,       // offset 72
    pub r11: u64,       // offset 80
    pub r12: u64,       // offset 88
    pub r13: u64,       // offset 96
    pub r14: u64,       // offset 104
    pub r15: u64,       // offset 112
    pub rsp: u64,       // offset 120
    pub rip: u64,       // offset 128
    pub rflags: u64,    // offset 136
    pub cr3: u64,       // offset 144
    /// Set to 1 by context_switch.asm after saving all registers.
    /// Set to 0 by schedule_inner before releasing the lock.
    /// pick_next skips threads with save_complete == 0 to prevent
    /// another CPU from restoring a partially-saved context.
    pub save_complete: u64, // offset 152
}

impl Default for CpuContext {
    fn default() -> Self {
        CpuContext {
            rax: 0, rbx: 0, rcx: 0, rdx: 0,
            rsi: 0, rdi: 0, rbp: 0,
            r8: 0, r9: 0, r10: 0, r11: 0,
            r12: 0, r13: 0, r14: 0, r15: 0,
            rsp: 0, rip: 0, rflags: 0, cr3: 0,
            save_complete: 1, // New contexts are valid from the start
        }
    }
}

extern "C" {
    /// Low-level context switch implemented in assembly.
    /// Saves current context to `old`, loads context from `new`.
    pub fn context_switch(old: *mut CpuContext, new: *const CpuContext);
}

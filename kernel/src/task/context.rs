//! Thread CPU context for cooperative/preemptive context switching (x86-64).
//!
//! Defines the register state saved and restored by `context_switch.asm` and provides
//! the FFI declaration for the assembly-level context switch routine.

/// CPU context saved/restored during a context switch.
/// Must match the layout expected by context_switch.asm (152 bytes total).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuContext {
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
}

extern "C" {
    /// Low-level context switch implemented in assembly.
    /// Saves current context to `old`, loads context from `new`.
    pub fn context_switch(old: *mut CpuContext, new: *const CpuContext);
}

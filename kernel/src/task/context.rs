//! Thread CPU context for cooperative/preemptive context switching.
//!
//! Defines the register state saved and restored by `context_switch.asm` and provides
//! the FFI declaration for the assembly-level context switch routine.

/// CPU context saved/restored during a context switch.
/// Must match the layout expected by context_switch.asm.
#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct CpuContext {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub esi: u32,
    pub edi: u32,
    pub ebp: u32,
    pub esp: u32,
    pub eip: u32,
    pub eflags: u32,
    pub cr3: u32,
}

extern "C" {
    /// Low-level context switch implemented in assembly.
    /// Saves current context to `old`, loads context from `new`.
    pub fn context_switch(old: *mut CpuContext, new: *const CpuContext);
}

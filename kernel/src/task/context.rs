//! Thread CPU context for cooperative/preemptive context switching (x86-64).
//!
//! Defines the register state saved and restored by `context_switch.asm` and provides
//! the FFI declaration for the assembly-level context switch routine.

/// CPU context saved/restored during a context switch.
/// Must match the layout expected by context_switch.asm (184 bytes total).
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
    /// Magic canary written by context_switch.asm after saving.
    /// Verified before loading. If wrong, the CpuContext memory was
    /// externally overwritten (heap corruption, buffer overflow, etc.).
    pub canary: u64,        // offset 160  (CANARY_MAGIC)
    /// XOR checksum of register fields [0..144] (offsets 0 through 144, 19 u64s).
    /// Excludes save_complete (changes frequently) and canary (constant).
    /// Computed after save, verified before load. Detects partial corruption
    /// even if the canary survives (e.g., single-field overwrite).
    pub checksum: u64,      // offset 168
}

/// Magic value for the CpuContext integrity canary.
/// Chosen to be unlikely as a heap allocator value, PTE, or small integer.
pub const CANARY_MAGIC: u64 = 0xCAFE_BABE_DEAD_BEEF;

impl Default for CpuContext {
    fn default() -> Self {
        let mut ctx = CpuContext {
            rax: 0, rbx: 0, rcx: 0, rdx: 0,
            rsi: 0, rdi: 0, rbp: 0,
            r8: 0, r9: 0, r10: 0, r11: 0,
            r12: 0, r13: 0, r14: 0, r15: 0,
            rsp: 0, rip: 0, rflags: 0, cr3: 0,
            save_complete: 1, // New contexts are valid from the start
            canary: CANARY_MAGIC,
            checksum: 0,
        };
        ctx.checksum = ctx.compute_checksum();
        ctx
    }
}

impl CpuContext {
    /// Compute XOR checksum of register fields (offsets 0..144, 19 u64s).
    /// Excludes save_complete (offset 152) which changes outside context_switch.
    pub fn compute_checksum(&self) -> u64 {
        let p = self as *const Self as *const u64;
        let mut xor: u64 = 0;
        for i in 0..19 {
            xor ^= unsafe { *p.add(i) };
        }
        xor
    }

    /// Verify canary and checksum integrity. Returns Ok(()) or Err with diagnosis.
    pub fn verify_integrity(&self) -> Result<(), &'static str> {
        if self.canary != CANARY_MAGIC {
            return Err("canary corrupt");
        }
        if self.checksum != self.compute_checksum() {
            return Err("checksum mismatch");
        }
        Ok(())
    }
}

extern "C" {
    /// Low-level context switch implemented in assembly.
    /// Saves current context to `old`, loads context from `new`.
    pub fn context_switch(old: *mut CpuContext, new: *const CpuContext);
}

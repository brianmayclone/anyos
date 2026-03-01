//! Thread CPU context for cooperative/preemptive context switching (AArch64).
//!
//! Defines the register state saved and restored by `context_switch.S` and provides
//! the FFI declaration for the assembly-level context switch routine.

/// CPU context saved/restored during a context switch.
/// Must match the layout expected by context_switch.S.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CpuContext {
    /// General-purpose registers X0-X30 (X30 = LR).
    pub x: [u64; 31],       // offset 0   (248 bytes)
    /// Stack Pointer (SP_EL0 for user threads, SP_EL1 for kernel).
    pub sp: u64,             // offset 248
    /// Program Counter (saved ELR_EL1).
    pub pc: u64,             // offset 256
    /// Saved processor state (SPSR_EL1).
    pub pstate: u64,         // offset 264
    /// User page table base (TTBR0_EL1).
    pub ttbr0: u64,          // offset 272
    /// Thread pointer (TPIDR_EL0, used for TLS).
    pub tpidr: u64,          // offset 280
    /// Set to 1 by context_switch.S after saving all registers.
    /// Set to 0 by schedule_inner before releasing the lock.
    /// pick_next skips threads with save_complete == 0 to prevent
    /// another CPU from restoring a partially-saved context.
    pub save_complete: u64,  // offset 288
    /// Magic canary written by context_switch.S after saving.
    /// Verified before loading. If wrong, the CpuContext memory was
    /// externally overwritten (heap corruption, buffer overflow, etc.).
    pub canary: u64,         // offset 296
    /// XOR checksum of register fields. Excludes save_complete and canary.
    /// Computed after save, verified before load.
    pub checksum: u64,       // offset 304
}

/// Magic value for the CpuContext integrity canary.
pub const CANARY_MAGIC: u64 = 0xCAFE_BABE_DEAD_BEEF;

/// Number of u64 fields to include in checksum (x[31] + sp + pc + pstate + ttbr0 + tpidr = 36).
const CHECKSUM_FIELDS: usize = 36;

impl Default for CpuContext {
    fn default() -> Self {
        let mut ctx = CpuContext {
            x: [0; 31],
            sp: 0,
            pc: 0,
            pstate: 0,
            ttbr0: 0,
            tpidr: 0,
            save_complete: 1, // New contexts are valid from the start
            canary: CANARY_MAGIC,
            checksum: 0,
        };
        ctx.checksum = ctx.compute_checksum();
        ctx
    }
}

impl CpuContext {
    // ── Platform-agnostic accessors ────────────────────────────────────

    /// Get the program counter (RIP on x86, PC on ARM64).
    #[inline] pub fn get_pc(&self) -> u64 { self.pc }
    /// Set the program counter.
    #[inline] pub fn set_pc(&mut self, val: u64) { self.pc = val; }
    /// Get the stack pointer (RSP on x86, SP on ARM64).
    #[inline] pub fn get_sp(&self) -> u64 { self.sp }
    /// Set the stack pointer.
    #[inline] pub fn set_sp(&mut self, val: u64) { self.sp = val; }
    /// Get the page table base (CR3 on x86, TTBR0 on ARM64).
    #[inline] pub fn get_page_table(&self) -> u64 { self.ttbr0 }
    /// Set the page table base.
    #[inline] pub fn set_page_table(&mut self, val: u64) { self.ttbr0 = val; }
    /// Get the processor flags (RFLAGS on x86, PSTATE on ARM64).
    #[inline] pub fn get_flags(&self) -> u64 { self.pstate }
    /// Set the processor flags.
    #[inline] pub fn set_flags(&mut self, val: u64) { self.pstate = val; }

    // ── Checksum/integrity ─────────────────────────────────────────────

    /// Compute XOR checksum of register fields.
    /// Excludes save_complete (changes outside context_switch).
    pub fn compute_checksum(&self) -> u64 {
        let p = self as *const Self as *const u64;
        let mut xor: u64 = 0;
        for i in 0..CHECKSUM_FIELDS {
            xor ^= unsafe { *p.add(i) };
        }
        xor
    }

    /// Verify canary and checksum integrity.
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

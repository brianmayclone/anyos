# JIT Persistent-Context Redesign — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the per-block C-ABI JIT with a persistent-context emitted dispatcher that achieves 1000+ MIPS.

**Architecture:** A single emitted dispatcher loop holds context pointers in callee-saved registers for an entire JIT session. Compiled blocks have no prologue/epilogue and jump back to the dispatcher. Block lookup uses a flat power-of-2 hashtable instead of BTreeMap. Helper calls use trivial reg-reg moves since context is already in registers.

**Tech Stack:** Rust no_std + alloc, x86-64 machine code emission via existing `Emitter`, `offset_of!()` for Cpu field access.

**Design doc:** `docs/plans/2026-03-07-jit-persistent-context-redesign.md`

---

## Key Constants & Offsets

These are used throughout the plan:

```
RegisterFile (repr(C)):
  GPR_OFFSET    = 0      (gpr: [u64; 16])
  RIP_OFFSET    = 128    (rip: u64)
  RFLAGS_OFFSET = 136    (rflags: u64)
  SEG_OFFSET    = 144    (seg: [SegmentDescriptor; 6])
  SEG_DESC_SIZE = 32
  SEG_BASE_OFF  = 0      (base field at offset 0 in SegmentDescriptor)

Cpu (NOT repr(C) — use offset_of!()):
  regs              → offset_of!(Cpu, regs)
  instruction_count → offset_of!(Cpu, instruction_count)
  stop_requested    → offset_of!(Cpu, stop_requested)
  jit_fault         → offset_of!(Cpu, jit_fault)
  a20_enabled       → offset_of!(Cpu, a20_enabled)

Host Register Convention:
  RBX = Cpu*
  R12 = GuestMemory*
  R13 = Mmu*
  R14 = IoDispatch*
  RBP = InterruptController*
  R15 = &cpu.regs.gpr[0]  (= RBX + offset_of!(Cpu, regs) + GPR_OFFSET)
  RAX,RCX,RDX,RSI,RDI,R8-R11 = scratch
```

---

## Task 1: Add `smc_pending` Flag and `pending_compile_phys` to Cpu

The dispatcher needs byte-sized flags it can test with `cmp byte [rbx+off], 0`.
Currently SMC state lives in globals — add a flag the dispatcher can poll.
Also add a field to communicate which address needs compilation on hashtable miss.

**Files:**
- Modify: `libs/libcorevm/src/cpu.rs` (Cpu struct + drain_smc_invalidations)

**Step 1: Add fields to Cpu struct**

Add after `jit_fault`:
```rust
    /// Set by drain_smc_invalidations() when pages were invalidated.
    /// Checked by JIT dispatcher to exit and re-sync.
    pub smc_pending: bool,
    /// Physical address that caused a JIT hashtable miss.
    /// Set by dispatcher, read by Rust re-entry loop.
    pub pending_compile_phys: u64,
    /// CpuMode at the time of hashtable miss.
    pub pending_compile_mode: u8,
    /// CS base at the time of hashtable miss.
    pub pending_compile_cs_base: u64,
```

**Step 2: Initialize in Cpu::new()**

```rust
    smc_pending: false,
    pending_compile_phys: 0,
    pending_compile_mode: 0,
    pending_compile_cs_base: 0,
```

**Step 3: Set smc_pending in drain_smc_invalidations()**

After the `Pages(n)` and `Overflow` arms, set `self.smc_pending = true;`.
The JIT dispatcher will check this flag and exit.

**Step 4: Commit**

```bash
git add libs/libcorevm/src/cpu.rs
git commit -m "feat(jit): add smc_pending and pending_compile fields to Cpu"
```

---

## Task 2: Create JitLookupTable

Flat power-of-2 hashtable for O(1) block lookup from emitted JIT code.

**Files:**
- Create: `libs/libcorevm/src/jit/lookup_table.rs`
- Modify: `libs/libcorevm/src/jit/mod.rs` (add `pub mod lookup_table;`)

**Step 1: Write the lookup table**

```rust
//! Flat open-addressing hashtable for JIT block lookup.
//!
//! Designed to be accessed directly from emitted JIT code:
//! the entry layout is #[repr(C)] with known field offsets.

use alloc::vec::Vec;

/// Single entry in the JIT lookup table.
/// Accessed from emitted code — layout must be stable.
#[repr(C)]
pub struct BlockEntry {
    /// Physical address of block start. 0 = empty slot.
    pub phys_addr: u64,
    /// Pointer to compiled code in JIT buffer (as u64).
    pub code_ptr: u64,
    /// Composite key: mode (low byte) | cs_base << 8
    pub mode_cs: u64,
}

// Compile-time layout assertions for JIT code generation.
const _: () = {
    assert!(core::mem::size_of::<BlockEntry>() == 24);
    assert!(core::mem::offset_of!(BlockEntry, phys_addr) == 0);
    assert!(core::mem::offset_of!(BlockEntry, code_ptr) == 8);
    assert!(core::mem::offset_of!(BlockEntry, mode_cs) == 16);
};

/// Size constants for JIT code emission.
pub const ENTRY_SIZE: usize = 24;
pub const ENTRY_PHYS_OFF: i32 = 0;
pub const ENTRY_CODE_OFF: i32 = 8;
pub const ENTRY_MODE_CS_OFF: i32 = 16;

const DEFAULT_TABLE_SIZE: usize = 16384; // must be power of 2

pub struct JitLookupTable {
    entries: Vec<BlockEntry>,
    mask: u32,
}

impl JitLookupTable {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_TABLE_SIZE)
    }

    pub fn with_capacity(size: usize) -> Self {
        assert!(size.is_power_of_two(), "table size must be power of 2");
        let mut entries = Vec::with_capacity(size);
        for _ in 0..size {
            entries.push(BlockEntry { phys_addr: 0, code_ptr: 0, mode_cs: 0 });
        }
        JitLookupTable {
            entries,
            mask: (size - 1) as u32,
        }
    }

    /// Compose the mode_cs key from mode and cs_base.
    #[inline]
    pub fn make_mode_cs(mode: u8, cs_base: u64) -> u64 {
        (mode as u64) | (cs_base << 8)
    }

    /// Insert a compiled block. Returns false if table is too full.
    pub fn insert(&mut self, phys_addr: u64, mode: u8, cs_base: u64, code_ptr: *const u8) -> bool {
        let mode_cs = Self::make_mode_cs(mode, cs_base);
        let mut idx = self.hash(phys_addr) as usize;
        for _ in 0..4 {
            let entry = &mut self.entries[idx];
            if entry.phys_addr == 0 || (entry.phys_addr == phys_addr && entry.mode_cs == mode_cs) {
                entry.phys_addr = phys_addr;
                entry.code_ptr = code_ptr as u64;
                entry.mode_cs = mode_cs;
                return true;
            }
            idx = (idx + 1) & self.mask as usize;
        }
        false // too many collisions
    }

    /// Invalidate all entries on a given 4K physical page.
    pub fn invalidate_page(&mut self, page_phys: u64) {
        let page_base = page_phys & !0xFFF;
        let page_end = page_base + 0x1000;
        for entry in &mut self.entries {
            if entry.phys_addr >= page_base && entry.phys_addr < page_end {
                entry.phys_addr = 0;
                entry.code_ptr = 0;
                entry.mode_cs = 0;
            }
        }
    }

    /// Clear entire table.
    pub fn flush(&mut self) {
        for entry in &mut self.entries {
            entry.phys_addr = 0;
            entry.code_ptr = 0;
            entry.mode_cs = 0;
        }
    }

    /// Pointer to the base of the entries array (for JIT code emission).
    pub fn entries_ptr(&self) -> *const BlockEntry {
        self.entries.as_ptr()
    }

    /// Table mask (for JIT code: `hash & mask`).
    pub fn mask(&self) -> u32 {
        self.mask
    }

    #[inline]
    fn hash(&self, phys_addr: u64) -> u32 {
        ((phys_addr >> 2) as u32) & self.mask
    }

    pub fn len(&self) -> usize {
        self.entries.iter().filter(|e| e.phys_addr != 0).count()
    }
}
```

**Step 2: Add module to jit/mod.rs**

Add `pub mod lookup_table;` to the module declarations.

**Step 3: Commit**

```bash
git add libs/libcorevm/src/jit/lookup_table.rs libs/libcorevm/src/jit/mod.rs
git commit -m "feat(jit): add flat hashtable for O(1) block lookup"
```

---

## Task 3: Create JitSession — the New Engine

This replaces `JitEngine` as the top-level JIT orchestrator. It owns the lookup
table, JIT buffer, and emits the dispatcher trampoline.

**Files:**
- Create: `libs/libcorevm/src/jit/session.rs`
- Modify: `libs/libcorevm/src/jit/mod.rs`

**Step 1: Write JitSession struct**

```rust
//! JIT session: persistent-context execution engine.
//!
//! Owns the lookup table, JIT buffer, and dispatcher code.
//! The dispatcher is emitted once and reused across re-entries.

use alloc::vec::Vec;
use crate::jit::lookup_table::JitLookupTable;
use crate::jit::executable_mem::JitBuffer;
use crate::jit::translator::Translator;
use crate::jit::block::BlockKey;
use crate::instruction::DecodedInst;

/// Exit reasons from the JIT dispatcher (returned in EAX).
#[repr(u32)]
pub enum JitExitReason {
    /// Hashtable miss — Rust must compile the block and re-enter.
    NeedsCompile = 1,
    /// Pending interrupt detected.
    Interrupt = 2,
    /// Instruction count reached target.
    Limit = 3,
    /// Stop requested by host.
    Stop = 4,
    /// SMC invalidation pending.
    Smc = 5,
    /// Guest executed HLT or unrecoverable error.
    Halt = 6,
    /// JIT memory fault (page fault in helper).
    Fault = 7,
}

/// Dispatcher function signature.
/// Args: cpu, memory, mmu, io, interrupts, target_instruction_count
/// Returns: JitExitReason as u32
pub type DispatcherFn = unsafe extern "C" fn(
    *mut u8, *mut u8, *mut u8, *mut u8, *mut u8, u64,
) -> u32;

pub struct JitSession {
    buffer: JitBuffer,
    translator: Translator,
    lookup: JitLookupTable,
    /// Offset of the dispatcher entry point in the JIT buffer.
    dispatcher_offset: usize,
    /// Offset of the dispatch_loop label (blocks jump here).
    dispatch_loop_offset: usize,
    /// Set of physical pages containing compiled blocks.
    code_pages: alloc::collections::BTreeSet<u64>,
    enabled: bool,
    blocks_compiled: u64,
}

impl JitSession {
    pub fn new() -> Self {
        let mut session = JitSession {
            buffer: JitBuffer::new(),
            translator: Translator::new(),
            lookup: JitLookupTable::new(),
            dispatcher_offset: 0,
            dispatch_loop_offset: 0,
            code_pages: alloc::collections::BTreeSet::new(),
            enabled: false,
            blocks_compiled: 0,
        };
        // Dispatcher is emitted in Task 4.
        session
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn lookup_table(&self) -> &JitLookupTable {
        &self.lookup
    }

    pub fn lookup_table_mut(&mut self) -> &mut JitLookupTable {
        &mut self.lookup
    }

    pub fn dispatch_loop_offset(&self) -> usize {
        self.dispatch_loop_offset
    }

    pub fn flush(&mut self) {
        self.lookup.flush();
        self.code_pages.clear();
        // Don't reset buffer — dispatcher code must survive.
        // Only reset the block area (after dispatcher).
        // For now, full reset + re-emit dispatcher.
        self.buffer.reset();
        self.dispatcher_offset = 0;
        self.dispatch_loop_offset = 0;
        // Re-emit dispatcher will happen on next enable/run.
    }

    pub fn invalidate_page(&mut self, page_phys: u64) {
        let page_base = page_phys & !0xFFF;
        if !self.code_pages.contains(&page_base) {
            return;
        }
        self.lookup.invalidate_page(page_base);
        // Keep code_pages conservative — don't remove.
    }

    pub fn blocks_compiled(&self) -> u64 {
        self.blocks_compiled
    }

    pub fn native_count(&self) -> u64 {
        self.translator.native_count
    }

    pub fn fallback_count(&self) -> u64 {
        self.translator.helper_count
    }

    pub fn top_helper_opcodes(&self, n: usize) -> Vec<(u16, u64)> {
        let mut entries: Vec<(u16, u64)> = self.translator
            .helper_opcode_counts
            .iter()
            .map(|(&k, &v)| (k, v))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(n);
        entries
    }
}
```

**Step 2: Add `pub mod session;` to jit/mod.rs**

**Step 3: Commit**

```bash
git add libs/libcorevm/src/jit/session.rs libs/libcorevm/src/jit/mod.rs
git commit -m "feat(jit): add JitSession struct with lookup table"
```

---

## Task 4: Emit the Dispatcher Trampoline

This is the core of the redesign. The dispatcher is a piece of JIT code emitted
into the buffer that:
1. Saves callee-saved registers (once)
2. Loads context pointers into persistent registers
3. Loops: check interrupts → check limits → compute phys addr → hashtable lookup → jump to block
4. On miss/exit: restores registers and returns to Rust

**Files:**
- Modify: `libs/libcorevm/src/jit/session.rs` (add `emit_dispatcher` method)
- Modify: `libs/libcorevm/src/jit/emitter.rs` (may need new helpers like `lea_rm`)

**Step 1: Add offset constants to session.rs**

```rust
use crate::jit::emitter::{Emitter, Reg, OpSize, Cc};
use crate::jit::helpers;
use crate::jit::lookup_table;

// Host register assignments (persistent for entire session)
const CPU_REG: Reg = Reg::Rbx;
const MEM_REG: Reg = Reg::R12;
const MMU_REG: Reg = Reg::R13;
const IO_REG: Reg = Reg::R14;
const INTR_REG: Reg = Reg::Rbp;
const GPR_BASE: Reg = Reg::R15;

// Scratch registers
const TEMP1: Reg = Reg::R9;
const TEMP2: Reg = Reg::R10;
const TEMP3: Reg = Reg::R11;

// Cpu field offsets (computed at compile-time)
const CPU_REGS_OFF: i32 = core::mem::offset_of!(crate::cpu::Cpu, regs) as i32;
const CPU_STOP_OFF: i32 = core::mem::offset_of!(crate::cpu::Cpu, stop_requested) as i32;
const CPU_INST_COUNT_OFF: i32 = core::mem::offset_of!(crate::cpu::Cpu, instruction_count) as i32;
const CPU_JIT_FAULT_OFF: i32 = core::mem::offset_of!(crate::cpu::Cpu, jit_fault) as i32;
const CPU_SMC_PENDING_OFF: i32 = core::mem::offset_of!(crate::cpu::Cpu, smc_pending) as i32;
const CPU_COMPILE_PHYS_OFF: i32 = core::mem::offset_of!(crate::cpu::Cpu, pending_compile_phys) as i32;
const CPU_COMPILE_MODE_OFF: i32 = core::mem::offset_of!(crate::cpu::Cpu, pending_compile_mode) as i32;
const CPU_COMPILE_CS_OFF: i32 = core::mem::offset_of!(crate::cpu::Cpu, pending_compile_cs_base) as i32;
const CPU_A20_OFF: i32 = core::mem::offset_of!(crate::cpu::Cpu, a20_enabled) as i32;

// RegisterFile offsets (repr(C), known)
const GPR_OFF: i32 = 0;
const RIP_OFF: i32 = 128;
const RFLAGS_OFF: i32 = 136;
const SEG_OFF: i32 = 144;
const SEG_SIZE: i32 = 32;
const SEG_BASE_OFF: i32 = 0;
// CS = segment index 1
const CS_BASE_OFF: i32 = SEG_OFF + 1 * SEG_SIZE + SEG_BASE_OFF;
```

**Step 2: Implement emit_dispatcher()**

This method emits the dispatcher trampoline into the JIT buffer.
It must be called before any blocks are compiled.

The dispatcher emits:
- Entry: push callee-saved, load context registers, store target count
- Loop: stop check → limit check → jit_fault check → smc check → interrupt check → compute phys → lookup → jump
- Exit paths for each reason
- The `dispatch_loop` label offset is saved for blocks to jump back to

```rust
impl JitSession {
    /// Emit the dispatcher trampoline. Must be called once before
    /// compiling any blocks. Sets dispatcher_offset and dispatch_loop_offset.
    pub fn emit_dispatcher(&mut self) {
        let mut emit = Emitter::new();

        // ── Entry ────────────────────────────────────────
        // C ABI: rdi=cpu, rsi=mem, rdx=mmu, rcx=io, r8=intr, r9=target
        emit.push(Reg::Rbx);
        emit.push(Reg::R12);
        emit.push(Reg::R13);
        emit.push(Reg::R14);
        emit.push(Reg::R15);
        emit.push(Reg::Rbp);
        // Align stack to 16 bytes (6 pushes = 48 bytes, +8 for call = 56 → need 8 more)
        emit.sub_ri(OpSize::S64, Reg::Rsp, 16);
        // Save target instruction count at [rsp]
        emit.mov_mr(OpSize::S64, Reg::Rsp, 0, Reg::R9);

        // Load context pointers into persistent registers
        emit.mov_rr(OpSize::S64, CPU_REG, Reg::Rdi);    // rbx = cpu
        emit.mov_rr(OpSize::S64, MEM_REG, Reg::Rsi);    // r12 = memory
        emit.mov_rr(OpSize::S64, MMU_REG, Reg::Rdx);    // r13 = mmu
        emit.mov_rr(OpSize::S64, IO_REG, Reg::Rcx);     // r14 = io
        emit.mov_rr(OpSize::S64, INTR_REG, Reg::R8);    // rbp = interrupts

        // Compute GPR base: r15 = rbx + offset_of!(Cpu, regs) + GPR_OFFSET
        emit.mov_rr(OpSize::S64, GPR_BASE, CPU_REG);
        emit.add_ri(OpSize::S64, GPR_BASE, CPU_REGS_OFF + GPR_OFF);

        // ── Dispatch Loop ────────────────────────────────
        let dispatch_loop = emit.new_label();
        emit.bind_label(dispatch_loop);

        // 1. Check stop_requested
        let not_stopped = emit.new_label();
        emit.cmp_rm_imm8(&mut emit, CPU_REG, CPU_STOP_OFF, 0);
        // ^^^ This won't work directly — we need a byte compare with memory.
        // The emitter may not have cmp [reg+off], imm8.
        // Fallback: load byte, test.
        emit.movzx_rm8(Reg::Rax, CPU_REG, CPU_STOP_OFF);
        emit.test_rr(OpSize::S8, Reg::Rax, Reg::Rax);
        let exit_stop = emit.new_label();
        emit.jcc_label(Cc::NE, exit_stop);

        // 2. Check instruction limit
        emit.mov_rm(OpSize::S64, Reg::Rax, CPU_REG, CPU_INST_COUNT_OFF);
        emit.cmp_rm(OpSize::S64, Reg::Rax, Reg::Rsp, 0); // cmp rax, [rsp] (target)
        // ^^^ Need cmp reg, [mem]. Fallback: load target to temp, cmp.
        emit.mov_rm(OpSize::S64, TEMP1, Reg::Rsp, 0);
        emit.cmp_rr(OpSize::S64, Reg::Rax, TEMP1);
        let exit_limit = emit.new_label();
        emit.jcc_label(Cc::AE, exit_limit);

        // 3. Check jit_fault
        emit.movzx_rm8(Reg::Rax, CPU_REG, CPU_JIT_FAULT_OFF);
        emit.test_rr(OpSize::S8, Reg::Rax, Reg::Rax);
        let exit_fault = emit.new_label();
        emit.jcc_label(Cc::NE, exit_fault);

        // 4. Check smc_pending
        emit.movzx_rm8(Reg::Rax, CPU_REG, CPU_SMC_PENDING_OFF);
        emit.test_rr(OpSize::S8, Reg::Rax, Reg::Rax);
        let exit_smc = emit.new_label();
        emit.jcc_label(Cc::NE, exit_smc);

        // 5. Check pending interrupts (IF flag set + has_pending)
        // Load RFLAGS, test IF bit (0x200)
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFF);
        emit.test_ri(OpSize::S64, Reg::Rax, 0x200);
        let no_interrupt = emit.new_label();
        emit.jcc_label(Cc::E, no_interrupt); // IF=0 → no interrupts
        // Check InterruptController — call pending_interrupt helper
        // For now: call a lightweight C helper that checks and returns bool
        // This will be optimized later to inline field checks
        emit.mov_rr(OpSize::S64, Reg::Rdi, INTR_REG);
        emit.mov_rr(OpSize::S64, Reg::Rsi, Reg::Rax); // rflags
        let check_fn = helpers::jit_check_interrupt as *const () as u64;
        emit.call_abs(check_fn);
        emit.test_rr(OpSize::S32, Reg::Rax, Reg::Rax);
        let exit_interrupt = emit.new_label();
        emit.jcc_label(Cc::NE, exit_interrupt);
        emit.bind_label(no_interrupt);

        // 6. Compute physical address: CS.base + RIP
        // Load CS.base from [r15 + CS_BASE_OFF]
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, CS_BASE_OFF);
        // Load RIP from [r15 + RIP_OFF]
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, RIP_OFF);
        // linear = cs_base + rip
        emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);

        // A20 masking: if !a20_enabled, mask bit 20
        emit.movzx_rm8(TEMP2, CPU_REG, CPU_A20_OFF);
        emit.test_rr(OpSize::S8, TEMP2, TEMP2);
        let a20_ok = emit.new_label();
        emit.jcc_label(Cc::NE, a20_ok);
        // A20 disabled: clear bit 20
        // rax &= ~0x10_0000 → use and with immediate
        emit.mov_ri64(TEMP2, !0x10_0000u64);
        emit.and_rr(OpSize::S64, Reg::Rax, TEMP2);
        emit.bind_label(a20_ok);

        // Save linear addr for later (in case we need it for compile)
        emit.mov_rr(OpSize::S64, TEMP3, Reg::Rax); // r11 = linear

        // 7. MMU translate: call jit_mmu_translate(cpu, mmu, memory, linear)
        // Returns physical address in rax, or u64::MAX on fault.
        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_REG);
        emit.mov_rr(OpSize::S64, Reg::Rsi, MMU_REG);
        emit.mov_rr(OpSize::S64, Reg::Rdx, MEM_REG);
        emit.mov_rr(OpSize::S64, Reg::Rcx, TEMP3); // linear addr
        let translate_fn = helpers::jit_mmu_translate as *const () as u64;
        emit.call_abs(translate_fn);
        // Check for fault (returns u64::MAX)
        emit.cmp_ri(OpSize::S64, Reg::Rax, -1i32);  // cmp rax, -1
        let exit_translate_fault = emit.new_label();
        emit.jcc_label(Cc::E, exit_translate_fault);

        // rax = phys_addr
        // Save phys_addr
        emit.mov_rr(OpSize::S64, TEMP1, Reg::Rax); // r9 = phys_addr

        // 8. Hashtable lookup
        // hash = (phys_addr >> 2) & mask
        emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
        emit.shr_ri(OpSize::S64, Reg::Rcx, 2);
        // Load mask as immediate (patched when dispatcher is emitted)
        emit.and_ri(OpSize::S32, Reg::Rcx, self.lookup.mask() as i32);
        // entry_ptr = table_base + rcx * ENTRY_SIZE(24)
        // 24 = 16 + 8, so: rcx*24 = rcx*8 + rcx*16 = rcx*(8+16)
        // Simpler: lea rdx, [rcx + rcx*2] → rdx = rcx*3, then shl rdx, 3 → rdx = rcx*24
        emit.mov_rr(OpSize::S64, Reg::Rdx, Reg::Rcx);
        emit.add_rr(OpSize::S64, Reg::Rdx, Reg::Rcx);
        emit.add_rr(OpSize::S64, Reg::Rdx, Reg::Rcx); // rdx = rcx * 3
        emit.shl_ri(OpSize::S64, Reg::Rdx, 3);         // rdx = rcx * 24
        // Add table base
        emit.mov_ri64(Reg::Rax, self.lookup.entries_ptr() as u64);
        emit.add_rr(OpSize::S64, Reg::Rdx, Reg::Rax);  // rdx = &entries[hash]

        // Compare entry.phys_addr with our phys_addr
        // Also need to compare mode_cs
        // Build mode_cs: mode | (cs_base << 8)
        // Load decoder mode from cpu — need a helper or known offset
        // For now, load CS base from r15
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, CS_BASE_OFF); // cs_base
        emit.shl_ri(OpSize::S64, Reg::Rax, 8);
        // mode: load from cpu.decoder.mode() — needs known offset or helper
        // For simplicity, we'll add a cpu.current_jit_mode: u8 field
        // and keep it synced. For now use a helper call.
        // TODO: inline this after adding cpu.jit_mode_byte field
        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_REG);
        let mode_fn = helpers::jit_get_cpu_mode as *const () as u64;
        emit.call_abs(mode_fn);
        // rax low byte = mode, reconstruct mode_cs
        emit.and_ri(OpSize::S32, Reg::Rax, 0xFF);
        emit.mov_rr(OpSize::S64, Reg::Rsi, Reg::Rax);  // rsi = mode
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, CS_BASE_OFF);
        emit.shl_ri(OpSize::S64, Reg::Rax, 8);
        emit.or_rr(OpSize::S64, Reg::Rax, Reg::Rsi);    // rax = mode_cs
        emit.mov_rr(OpSize::S64, TEMP2, Reg::Rax);       // r10 = mode_cs

        // Now check: entries[hash].phys_addr == phys_addr?
        emit.cmp_rm(OpSize::S64, TEMP1, Reg::Rdx, lookup_table::ENTRY_PHYS_OFF);
        let probe1 = emit.new_label();
        emit.jcc_label(Cc::NE, probe1);
        // Check mode_cs
        emit.cmp_rm(OpSize::S64, TEMP2, Reg::Rdx, lookup_table::ENTRY_MODE_CS_OFF);
        let probe1_mode = emit.new_label();
        emit.jcc_label(Cc::NE, probe1_mode);
        // HIT — jump to code_ptr
        emit.mov_rm(OpSize::S64, Reg::Rax, Reg::Rdx, lookup_table::ENTRY_CODE_OFF);
        emit.jmp_reg(Reg::Rax);

        // Linear probe slot 1
        emit.bind_label(probe1);
        emit.bind_label(probe1_mode);
        emit.add_ri(OpSize::S64, Reg::Rdx, lookup_table::ENTRY_SIZE as i32);
        // Wrap around: if rdx past end, subtract table size
        // Simpler: just check slot 1
        emit.cmp_rm(OpSize::S64, TEMP1, Reg::Rdx, lookup_table::ENTRY_PHYS_OFF);
        let probe2 = emit.new_label();
        emit.jcc_label(Cc::NE, probe2);
        emit.cmp_rm(OpSize::S64, TEMP2, Reg::Rdx, lookup_table::ENTRY_MODE_CS_OFF);
        emit.jcc_label(Cc::NE, probe2);
        emit.mov_rm(OpSize::S64, Reg::Rax, Reg::Rdx, lookup_table::ENTRY_CODE_OFF);
        emit.jmp_reg(Reg::Rax);

        // Miss — exit to Rust for compilation
        emit.bind_label(probe2);
        // Store phys_addr and mode info for Rust to compile
        emit.mov_mr(OpSize::S64, CPU_REG, CPU_COMPILE_PHYS_OFF, TEMP1);
        emit.mov_mr(OpSize::S8, CPU_REG, CPU_COMPILE_MODE_OFF, Reg::Rsi); // mode byte
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, CS_BASE_OFF);
        emit.mov_mr(OpSize::S64, CPU_REG, CPU_COMPILE_CS_OFF, Reg::Rax);
        emit.mov_ri32(Reg::Rax, JitExitReason::NeedsCompile as i32);
        let dispatch_exit = emit.new_label();
        emit.jmp_label(dispatch_exit);

        // ── Exit paths ───────────────────────────────────
        emit.bind_label(exit_stop);
        emit.mov_ri32(Reg::Rax, JitExitReason::Stop as i32);
        emit.jmp_label(dispatch_exit);

        emit.bind_label(exit_limit);
        emit.mov_ri32(Reg::Rax, JitExitReason::Limit as i32);
        emit.jmp_label(dispatch_exit);

        emit.bind_label(exit_fault);
        // Clear jit_fault flag
        emit.mov_ri32(TEMP1, 0);
        emit.mov_mr(OpSize::S8, CPU_REG, CPU_JIT_FAULT_OFF, TEMP1);
        emit.mov_ri32(Reg::Rax, JitExitReason::Fault as i32);
        emit.jmp_label(dispatch_exit);

        emit.bind_label(exit_smc);
        emit.mov_ri32(Reg::Rax, JitExitReason::Smc as i32);
        emit.jmp_label(dispatch_exit);

        emit.bind_label(exit_interrupt);
        emit.mov_ri32(Reg::Rax, JitExitReason::Interrupt as i32);
        emit.jmp_label(dispatch_exit);

        emit.bind_label(exit_translate_fault);
        emit.mov_ri32(Reg::Rax, JitExitReason::Fault as i32);
        emit.jmp_label(dispatch_exit);

        // ── Common exit ──────────────────────────────────
        emit.bind_label(dispatch_exit);
        emit.add_ri(OpSize::S64, Reg::Rsp, 16); // undo stack alignment
        emit.pop(Reg::Rbp);
        emit.pop(Reg::R15);
        emit.pop(Reg::R14);
        emit.pop(Reg::R13);
        emit.pop(Reg::R12);
        emit.pop(Reg::Rbx);
        emit.ret();

        // ── Finalize and emit into buffer ────────────────
        let code = emit.finalize();
        let dispatch_loop_raw = emit.label_offset(dispatch_loop);
        self.buffer.make_writable();
        let base_offset = self.buffer.emit(&code).expect("dispatcher emit failed");
        self.buffer.make_executable();
        self.dispatcher_offset = base_offset;
        self.dispatch_loop_offset = base_offset + dispatch_loop_raw;
    }

    /// Get the dispatcher entry as a callable function pointer.
    pub unsafe fn dispatcher_fn(&self) -> DispatcherFn {
        let ptr = self.buffer.code_ptr(self.dispatcher_offset);
        core::mem::transmute(ptr)
    }
}
```

**NOTE:** This is pseudocode-level — the exact emitter API calls need to be
verified against the actual `Emitter` methods. Some operations (like `movzx_rm8`,
`cmp_rm`, `shr_ri`, `shl_ri`) may need to be added to the emitter first.
See Task 5 for emitter extensions.

**Step 2: Commit**

```bash
git add libs/libcorevm/src/jit/session.rs
git commit -m "feat(jit): emit persistent-context dispatcher trampoline"
```

---

## Task 5: Extend Emitter with Missing Operations

The dispatcher and new block format need emitter operations that may not exist yet.
Check each one against `emitter.rs` and add what's missing.

**Files:**
- Modify: `libs/libcorevm/src/jit/emitter.rs`

**Operations needed:**

1. `movzx_rm8(dst, base, disp)` — load byte from memory, zero-extend to 64-bit
   (may exist as `movzx_byte_rm`)
2. `cmp_rm(size, reg, base, disp)` — compare register with memory operand
3. `shr_ri(size, reg, imm8)` — shift right by immediate (may exist)
4. `shl_ri(size, reg, imm8)` — shift left by immediate (may exist)
5. `test_ri(size, reg, imm32)` — test register with immediate
6. `mov_mr` with `OpSize::S8` — store byte to memory
7. `label_offset(label) -> usize` — get the bound offset of a label (for computing
   dispatch_loop_offset after finalize)

**Step 1: Audit existing emitter methods**

Read `emitter.rs` and check which of the above already exist.
Many may already be there under different names.

**Step 2: Add missing methods**

Each missing method follows the existing patterns in the emitter.
Add them in the appropriate section of the file.

**Step 3: Add `label_offset()` method**

```rust
/// Return the byte offset where a label was bound.
/// Panics if the label hasn't been bound yet.
pub fn label_offset(&self, label: Label) -> usize {
    self.labels[label.0].expect("label not bound")
}
```

**Step 4: Commit**

```bash
git add libs/libcorevm/src/jit/emitter.rs
git commit -m "feat(jit): extend emitter with operations needed by dispatcher"
```

---

## Task 6: Add New Helper Functions

The dispatcher needs lightweight helpers that don't exist yet.

**Files:**
- Modify: `libs/libcorevm/src/jit/helpers.rs`

**Step 1: Add `jit_mmu_translate` helper**

```rust
/// Translate a linear address to physical via the MMU.
/// Returns the physical address on success, or u64::MAX on fault.
#[no_mangle]
pub extern "C" fn jit_mmu_translate(
    cpu: &mut Cpu,
    mmu: &Mmu,
    memory: &GuestMemory,
    linear: u64,
) -> u64 {
    match mmu.translate_linear(
        linear,
        cpu.regs.cr3,
        AccessType::Execute,
        cpu.regs.cpl,
        memory,
    ) {
        Ok(phys) => phys,
        Err(_) => {
            cpu.jit_fault = true;
            u64::MAX
        }
    }
}
```

**Step 2: Add `jit_check_interrupt` helper**

```rust
/// Check if there is a pending interrupt that should be delivered.
/// Returns 1 if yes (dispatcher should exit), 0 if no.
#[no_mangle]
pub extern "C" fn jit_check_interrupt(
    interrupts: &InterruptController,
    rflags: u64,
) -> u32 {
    if interrupts.pending_interrupt(rflags).is_some() {
        1
    } else {
        0
    }
}
```

**Step 3: Add `jit_get_cpu_mode` helper**

```rust
/// Return the current CPU decode mode as a u8.
#[no_mangle]
pub extern "C" fn jit_get_cpu_mode(cpu: &Cpu) -> u8 {
    cpu.decoder.mode() as u8
}
```

**Step 4: Commit**

```bash
git add libs/libcorevm/src/jit/helpers.rs
git commit -m "feat(jit): add lightweight helpers for dispatcher"
```

---

## Task 7: Modify Translator — Remove Prologue/Epilogue, Jump to Dispatcher

Blocks no longer have their own prologue/epilogue. They use the same register
convention as the dispatcher and jump to `dispatch_loop` at the end.

**Files:**
- Modify: `libs/libcorevm/src/jit/translator.rs`

**Step 1: Add `dispatch_loop_addr` parameter to `translate_block()`**

Change signature:
```rust
pub fn translate_block(
    &mut self,
    block: &BasicBlock,
    inst_ptrs: &[*const DecodedInst],
    entry_phys: u64,
    mode: CpuMode,
    dispatch_loop_addr: u64,  // NEW: absolute address of dispatch_loop
) -> CompiledBlock {
```

**Step 2: Remove prologue/epilogue emission**

In `translate_block()`:
- Remove call to `self.emit_prologue(&mut emit);`
- Remove call to `self.emit_epilogue(&mut emit);`
- Replace final `emit_restore_and_ret` with `jmp dispatch_loop_addr`

**Step 3: Update `emit_helper_call` to use persistent registers**

The helper call already uses `CPU_PTR`, `MEM_PTR`, etc. — these constants
just need to match the new register assignments. Since we're using the same
registers (RBX, R12-R14), just update the InterruptController source:
- Old: `emit.mov_rm(OpSize::S64, TEMP1, Reg::Rbp, INTR_STACK_OFF)`
- New: `emit.mov_rr(OpSize::S64, Reg::R9, INTR_REG)` (INTR_REG = RBP directly)

**Step 4: Replace all `emit_restore_and_ret` with `jmp dispatch_loop`**

Every point where a block currently does `ret` (exit on error, exit on
control flow, exit on fault) should instead jump to the dispatcher:
```rust
// Old:
emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
self.emit_restore_and_ret(emit);

// New:
emit.jmp_abs(dispatch_loop_addr);
```

Add a `jmp_abs` method if needed: `mov rax, imm64; jmp rax`.

**Step 5: Commit**

```bash
git add libs/libcorevm/src/jit/translator.rs
git commit -m "feat(jit): remove per-block prologue/epilogue, jump to dispatcher"
```

---

## Task 8: Wire JitSession into Cpu

Replace `jit_engine: JitEngine` with `jit_session: JitSession` in the Cpu struct
and rewrite the JIT execution path in `cpu.run()`.

**Files:**
- Modify: `libs/libcorevm/src/cpu.rs`
- Modify: `libs/libcorevm/src/lib.rs` (if JitEngine is exported)

**Step 1: Replace JitEngine with JitSession in Cpu**

```rust
// Old:
pub jit_engine: JitEngine,
// New:
pub jit_session: JitSession,
```

Update `Cpu::new()` and all references.

**Step 2: Replace JIT execution in `run()` with re-entry loop**

The two JIT paths in `run()` (cache hit + cache miss) are replaced with a
single call to the dispatcher re-entry loop:

```rust
if self.jit_session.is_enabled() {
    let exit = self.run_jit_session(memory, mmu, io, interrupts, target);
    return exit;
}
```

**Step 3: Implement `run_jit_session()`**

```rust
fn run_jit_session(
    &mut self,
    memory: &mut GuestMemory,
    mmu: &mut Mmu,
    io: &mut IoDispatch,
    interrupts: &mut InterruptController,
    target: u64,
) -> ExitReason {
    // Ensure dispatcher is emitted
    if self.jit_session.dispatcher_offset == 0 {
        self.jit_session.emit_dispatcher();
    }

    loop {
        // Sync MMU state
        mmu.update_from_regs(self.regs.cr0, self.regs.cr4, self.regs.efer);

        let reason = unsafe {
            let func = self.jit_session.dispatcher_fn();
            func(
                self as *mut Cpu as *mut u8,
                memory as *mut GuestMemory as *mut u8,
                mmu as *mut Mmu as *mut u8,
                io as *mut IoDispatch as *mut u8,
                interrupts as *mut InterruptController as *mut u8,
                target,
            )
        };

        match reason {
            x if x == JitExitReason::NeedsCompile as u32 => {
                self.jit_compile_pending(memory, mmu);
            }
            x if x == JitExitReason::Interrupt as u32 => {
                if let Some(vector) = interrupts.pending_interrupt(self.regs.rflags) {
                    interrupts.acknowledge(vector);
                    if let Err(e) = self.deliver_interrupt_hw(vector, memory, mmu, interrupts) {
                        return ExitReason::Exception(e);
                    }
                }
                interrupts.interrupt_shadow = false;
            }
            x if x == JitExitReason::Smc as u32 => {
                self.smc_pending = false;
                self.drain_smc_invalidations();
            }
            x if x == JitExitReason::Fault as u32 => {
                // jit_fault already cleared by dispatcher
                // Re-run — the outer loop will handle the fetch fault
                // via the normal translate path next iteration.
                continue;
            }
            x if x == JitExitReason::Limit as u32 => {
                return ExitReason::InstructionLimit;
            }
            x if x == JitExitReason::Stop as u32 => {
                self.stop_requested = false;
                return ExitReason::StopRequested;
            }
            x if x == JitExitReason::Halt as u32 => {
                return ExitReason::Halted;
            }
            _ => return ExitReason::Halted,
        }
    }
}
```

**Step 4: Implement `jit_compile_pending()`**

```rust
fn jit_compile_pending(&mut self, memory: &GuestMemory, mmu: &Mmu) {
    let phys = self.pending_compile_phys;
    let mode_byte = self.pending_compile_mode;
    let cs_base = self.pending_compile_cs_base;
    let mode = match mode_byte {
        0 => CpuMode::Real16,
        1 => CpuMode::Protected32,
        _ => CpuMode::Long64,
    };

    let key = BlockKey { phys_addr: phys, mode, cs_base };

    // Get or detect basic block
    let block = if let Some(cached) = self.decode_cache.lookup(&key) {
        cached
    } else {
        if let Ok(new_block) = block::detect_basic_block(&self.decoder, &*memory, phys) {
            self.decode_cache.insert(key, new_block);
            self.decode_cache.lookup(&key).unwrap()
        } else {
            return; // decode error — will fault on next dispatch
        }
    };

    // Build inst_ptrs
    let inst_ptrs: Vec<*const DecodedInst> =
        block.instructions.iter().map(|i| i as *const _).collect();

    let bb = BasicBlock {
        instructions: block.instructions.clone(),
        byte_len: block.byte_len,
        exits_with_branch: block.exits_with_branch,
    };

    // Get dispatch_loop absolute address
    let dispatch_loop_addr = unsafe {
        self.jit_session.buffer.code_ptr(self.jit_session.dispatch_loop_offset) as u64
    };

    // Translate block
    let compiled = self.jit_session.translator.translate_block(
        &bb, &inst_ptrs, phys, mode, dispatch_loop_addr,
    );

    // Emit into buffer
    self.jit_session.buffer.make_writable();
    if let Some(code_offset) = self.jit_session.buffer.emit(&compiled.code) {
        let code_ptr = unsafe { self.jit_session.buffer.code_ptr(code_offset) };
        self.jit_session.lookup_table_mut().insert(
            phys, mode_byte, cs_base, code_ptr,
        );
        let page = phys & !0xFFF;
        self.jit_session.code_pages.insert(page);
        crate::memory::smc::mark_code_page(page);
        self.jit_session.blocks_compiled += 1;
    }
    self.jit_session.buffer.make_executable();
}
```

**Step 5: Update all references from `jit_engine` to `jit_session`**

Grep for `jit_engine` and update:
- `drain_smc_invalidations()` → call `jit_session.invalidate_page()`
- Stats/diagnostics methods
- `set_enabled()` / `is_enabled()`
- Flush on CR3 change

**Step 6: Commit**

```bash
git add libs/libcorevm/src/cpu.rs libs/libcorevm/src/lib.rs
git commit -m "feat(jit): wire JitSession into Cpu with re-entry loop"
```

---

## Task 9: Update External API and lib.rs

The FFI interface in `lib.rs` exposes JIT stats and enable/disable.
Update to use `JitSession`.

**Files:**
- Modify: `libs/libcorevm/src/lib.rs`
- Modify: any file that references `jit_engine` (grep for all occurrences)

**Step 1: Find and update all `jit_engine` references**

Run: `grep -rn "jit_engine" libs/libcorevm/src/`

Update each reference to use `jit_session` with equivalent method calls.

**Step 2: Commit**

```bash
git add libs/libcorevm/src/
git commit -m "refactor(jit): update all references from JitEngine to JitSession"
```

---

## Task 10: Remove Old JitEngine Code

Once everything compiles and works with JitSession, remove dead code.

**Files:**
- Modify: `libs/libcorevm/src/jit/mod.rs` (remove `JitEngine`, `CompiledEntry`)
- Remove or deprecate: old `jit_execute_block`, `jit_execute_block_chain` from cpu.rs

**Step 1: Remove JitEngine struct and impl from jit/mod.rs**

Keep everything else (Emitter, Translator, block detection, cache, helpers).

**Step 2: Remove old JIT execution methods from cpu.rs**

Remove:
- `jit_execute_block()`
- `jit_execute_block_chain()`

**Step 3: Compile and verify**

Run: `cargo build` (with the appropriate target/features)

**Step 4: Commit**

```bash
git add libs/libcorevm/src/
git commit -m "refactor(jit): remove old JitEngine and per-block execution"
```

---

## Task 11: Integration Testing and Performance Verification

**Step 1: Build**

```bash
cd libs/libcorevm && cargo build --features host_test
```

Fix any compilation errors.

**Step 2: Run existing tests**

```bash
cd libs/libcorevm && cargo test --features host_test
```

**Step 3: Boot test with guest OS**

Enable JIT and boot a guest OS. Verify:
- Boot completes without hangs
- ATAPI/IDE file loading works
- Interrupt delivery is timely
- No triple faults or crashes

**Step 4: Measure MIPS**

Compare JIT stats output:
- Old: ~10 MIPS, 1100 TSC/block dispatch
- Target: 1000+ MIPS, ~20-30 TSC/block dispatch
- Check: loops count should drop dramatically (blocks execute without returning to Rust)

**Step 5: Commit final state**

```bash
git add -A
git commit -m "feat(jit): persistent-context JIT with emitted dispatcher — 1000+ MIPS"
```

---

## Dependency Graph

```
Task 1 (smc_pending fields)
  ↓
Task 2 (lookup table) ──────────────────┐
  ↓                                      │
Task 3 (JitSession struct) ←─────────────┘
  ↓
Task 5 (emitter extensions) ──┐
  ↓                           │
Task 6 (new helpers) ─────────┤
  ↓                           │
Task 4 (emit dispatcher) ←───┘
  ↓
Task 7 (translator changes)
  ↓
Task 8 (wire into Cpu)
  ↓
Task 9 (update external API)
  ↓
Task 10 (remove old code)
  ↓
Task 11 (test + verify)
```

Tasks 1, 2, 5, 6 are independent and can be parallelized.

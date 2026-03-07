# JIT Persistent-Context Redesign

## Problem

The current DBT-JIT achieves only 10 MIPS vs. 50 MIPS for the interpreter. Root cause:
~1100 TSC cycles per block dispatch at ~4.5 instructions/block = ~244 TSC/instruction.

Breakdown of waste per block:
- C-ABI Prologue/Epilogue: 6 push + 6 pop + call jit_get_gpr_ptr = ~220 TSC
- BTreeMap lookup (9983 entries): ~150 TSC
- Helper argument marshalling (6 regs): ~40 TSC per helper call
- Total fixed overhead: ~410+ TSC per block

## Design Decision

**Approach B (Radical)**: New JIT core with persistent register context, emitted dispatcher
loop, flat hashtable lookup, and lightweight helper convention. Rewrite execution layer
completely; reuse emitter primitives.

**Target**: 1000-4000 MIPS.

## Architecture

### 1. Persistent Register Convention

Context pointers stay in callee-saved registers for the entire JIT session.
Save/restore happens once at session entry/exit, not per block.

| Host Register | Usage                          | Lifetime        |
|---------------|--------------------------------|-----------------|
| R15           | GPR base (`&cpu.regs.gpr[0]`)  | Entire session  |
| RBX           | `Cpu*`                         | Entire session  |
| R12           | `GuestMemory*`                 | Entire session  |
| R13           | `Mmu*`                         | Entire session  |
| R14           | `IoDispatch*`                  | Entire session  |
| RBP           | `InterruptController*`         | Entire session  |
| RSP           | Host stack                     | Normal          |
| RAX-RDX, RSI, RDI, R8-R11 | Scratch        | Per instruction |

GPR base computed via `lea r15, [rbx + REGS_GPR_OFFSET]` — no function call.

### 2. Emitted Dispatcher (Trampoline)

A single piece of JIT code emitted once into the JIT buffer. All compiled blocks
jump back to `dispatch_loop` when they finish.

```
dispatch_entry:                    ; called from Rust once per session
    push rbx, r12, r13, r14, r15, rbp
    mov  rbx, rdi                  ; cpu
    mov  r12, rsi                  ; memory
    mov  r13, rdx                  ; mmu
    mov  r14, rcx                  ; io
    mov  rbp, r8                   ; interrupts
    lea  r15, [rbx + REGS_GPR_OFFSET]
    ; Load target instruction count from stack arg or r9
    mov  [rsp - 8], r9             ; save target count

dispatch_loop:
    ; 1. Check stop_requested (byte in Cpu)
    cmp  byte [rbx + STOP_OFFSET], 0
    jne  exit_stop

    ; 2. Check instruction limit
    mov  rax, [rbx + INST_COUNT_OFFSET]
    cmp  rax, [rsp - 8]
    jge  exit_limit

    ; 3. Check pending interrupts
    ; (test interrupt flags — lightweight, ~3 instructions)
    ; jne exit_interrupt

    ; 4. Check SMC pending flag
    cmp  byte [rbx + SMC_PENDING_OFFSET], 0
    jne  exit_smc

    ; 5. Compute physical address
    ;    CS.base + RIP → linear → call MMU translate helper
    mov  rax, [r15 + RIP_OFFSET]
    ; add CS.base, handle A20, call translate
    ; result in rax = phys_addr

    ; 6. Hashtable lookup
    mov  rcx, rax
    shr  rcx, 2
    and  ecx, TABLE_MASK
    lea  rdx, [table_base + rcx * 24]  ; entry ptr
    cmp  [rdx + 0], rax                ; entry.phys_addr == phys_addr?
    jne  probe_or_miss
    ; TODO: also compare mode/cs_base
    jmp  [rdx + 8]                     ; jmp entry.code_ptr

probe_or_miss:
    ; linear probe (1-2 slots), then:
    jmp  exit_compile                  ; → Rust compiles block

exit_stop:
    mov  eax, EXIT_STOP
    jmp  dispatch_exit
exit_limit:
    mov  eax, EXIT_LIMIT
    jmp  dispatch_exit
exit_interrupt:
    mov  eax, EXIT_INTERRUPT
    jmp  dispatch_exit
exit_smc:
    mov  eax, EXIT_SMC
    jmp  dispatch_exit
exit_compile:
    mov  eax, EXIT_NEEDS_COMPILE
    ; Store phys_addr for Rust to know what to compile
    mov  [rbx + PENDING_COMPILE_OFFSET], rax
    jmp  dispatch_exit

dispatch_exit:
    pop  rbp, r15, r14, r13, r12, rbx
    ret                                ; → back to Rust with exit reason in eax
```

### 3. Compiled Blocks (No Prologue/Epilogue)

Blocks are bare instruction sequences. They assume context registers are loaded
and jump back to `dispatch_loop` at the end:

```
block_0x1000:
    ; guest: mov eax, [ebx+4]  → native emit (load, translate, read)
    ; guest: add eax, ecx      → native emit (alu)
    ; guest: mov [ebx+4], eax  → native emit (translate, write)
    ; guest: jnz 0x1020        → advance RIP, jmp dispatch_loop
```

No `ret`, no epilogue. The `dispatch_loop` label address is embedded as an
absolute jump target (known at emit time since dispatcher is emitted first).

### 4. Hashtable (no_std, alloc only)

```rust
#[repr(C)]
pub struct BlockEntry {
    phys_addr: u64,     // 0 = empty
    code_ptr: u64,      // pointer into JIT buffer
    mode_cs: u64,       // CpuMode | (cs_base << 8)
}
// sizeof = 24 bytes

pub struct JitLookupTable {
    entries: Vec<BlockEntry>,  // power-of-2 count
    mask: u32,                  // count - 1
}
```

- Default: 16384 entries = 384 KiB
- Hash: `(phys_addr >> 2) & mask`
- Linear probing, max 3 slots before miss
- Invalidation: set `phys_addr = 0`
- Full flush: `memset` to zero (CR3 change, mode switch)

### 5. Lightweight Helper Convention

Context pointers are already in registers. Helper calls only need to set up
arguments per C ABI with trivial reg-reg moves:

```
; helper_execute_one(inst, cpu, memory, mmu, io, interrupts)
    mov  rdi, <inst_ptr_imm64>    ; 10 bytes — only real setup
    mov  rsi, rbx                 ; 3 bytes — cpu (already there)
    mov  rdx, r12                 ; 3 bytes — memory
    mov  rcx, r13                 ; 3 bytes — mmu
    mov  r8,  r14                 ; 3 bytes — io
    mov  r9,  rbp                 ; 3 bytes — interrupts
    call <helper_addr>            ; 12 bytes (mov rax, imm64; call rax)
    test eax, eax
    jnz  dispatch_loop            ; on error/exit → re-dispatch
```

Cost: ~8-10 cycles (5 reg-reg moves + call). Existing `helper_execute_one`
function unchanged — C ABI compatible.

### 6. Exit/Re-Entry Protocol

| Exit Reason       | Trigger                          | Rust Action                        | Re-Entry      |
|--------------------|----------------------------------|------------------------------------|---------------|
| NEEDS_COMPILE      | Hashtable miss                   | Compile block, insert into table   | Call dispatcher again |
| INTERRUPT          | Pending interrupt flag           | Deliver interrupt to guest         | Call dispatcher again |
| LIMIT              | instruction_count >= target      | Return ExitReason::InstructionLimit| Caller decides |
| STOP               | stop_requested flag              | Return ExitReason::StopRequested   | Caller decides |
| SMC                | smc_pending flag                 | Invalidate table entries, clear    | Call dispatcher again |
| HALT               | HLT via helper                   | Return ExitReason::Halted          | Caller decides |
| FAULT              | jit_fault flag from mem helper   | Inject exception                   | Call dispatcher again |

The Rust-side `run()` loop becomes a simple re-entry loop:

```rust
loop {
    let reason = unsafe { dispatcher_fn(cpu, mem, mmu, io, intr, target) };
    match reason {
        EXIT_NEEDS_COMPILE => { compile_and_insert(cpu.pending_compile_addr); }
        EXIT_INTERRUPT     => { deliver_interrupt(...); }
        EXIT_SMC           => { drain_smc_invalidations(); }
        EXIT_FAULT         => { inject_exception(...); }
        EXIT_LIMIT | EXIT_STOP | EXIT_HALT => return reason.into();
    }
}
```

### 7. SMC Handling

- Memory write helpers mark dirty pages (existing mechanism)
- Dirty pages set `cpu.smc_pending = true`
- Dispatcher checks flag each iteration (~2 cycles)
- On exit: Rust scans hashtable, zeros matching entries
- JIT buffer code is NOT reclaimed (fragmentation acceptable for 4 MiB buffer)
- Full flush on CR3 change or mode switch: zero entire hashtable + reset buffer

### 8. MMU Translation in Dispatcher

The dispatcher needs to translate CS:RIP → physical address. Two options:

**Option A**: Call existing `mmu.translate_linear()` via C ABI helper.
Cost: ~30 cycles but handles all paging modes correctly.

**Option B**: Inline a TLB-like cache in JIT code — check a small direct-mapped
cache first, fall back to helper on miss.

**Decision**: Start with Option A. The translate helper is already correct and
handles PAE, 4-level, 2-level. Optimize later if profiling shows it's hot.

### 9. Interrupt Check in Dispatcher

Must be lightweight. The check boils down to:

```
; IF flag set AND (pic has pending OR lapic has pending) AND no shadow
test word [r15 + RFLAGS_OFFSET], 0x200    ; IF flag
jz   .no_interrupt
cmp  byte [rbp + PENDING_OFFSET], 0       ; InterruptController.has_pending
je   .no_interrupt
jmp  exit_interrupt
.no_interrupt:
```

~3-4 instructions, ~2-4 cycles on the no-interrupt fast path.

### 10. anyOS / no_std Compatibility

- All data structures use `alloc::vec::Vec` or `alloc::collections::*`
- No `std`, no `hashbrown`, no external dependencies
- JIT buffer: existing `JitBuffer` with `make_executable()` API
  - host_test: mmap RWX (already works)
  - anyOS: kernel provides RWX pages (or W^X toggle via syscall)
- Dispatcher code emitted via existing `Emitter` — no `asm!()` macros
- All code generation goes through the same `Emitter::new()` → `finalize()` path

### 11. Performance Budget

| Component              | Old (TSC) | New (TSC) | Notes                    |
|------------------------|-----------|-----------|--------------------------|
| Block lookup           | 150       | 10-15     | Hashtable vs BTreeMap    |
| Prologue/Epilogue      | 220       | 0         | Persistent context       |
| Helper arg setup       | 40        | 5-8       | Reg-reg moves only       |
| Interrupt check        | 0 (Rust)  | 2-4       | Inline in dispatcher     |
| SMC check              | 0 (Rust)  | 2         | Single byte test         |
| Dispatch overhead/block| ~1100     | ~20-30    | 50x reduction            |
| **Per-instruction**    | **~244**  | **~5-10** | At 4.5 inst/block avg    |
| **Projected MIPS**     | **10**    | **1000+** | At 2.5 GHz host          |

### 12. What Gets Replaced vs Reused

**Replaced:**
- `JitEngine` struct (new: `JitSession` with hashtable + dispatcher)
- `jit_execute_block()` / `jit_execute_block_chain()` (new: Rust re-entry loop)
- Block prologue/epilogue emission in `Translator`
- `CompiledEntry` / `compiled` BTreeMap (new: `JitLookupTable`)

**Reused:**
- `Emitter` (all instruction emission primitives)
- `Translator::try_translate_native()` (per-instruction native emission)
- `helper_execute_one()` (C ABI helper, unchanged)
- `jit_mem_read/write` helpers (unchanged)
- `BasicBlock` / `DecodeCache` (block detection still needed)
- `JitBuffer` / `executable_mem.rs` (buffer management)
- SMC dirty page tracking (`memory::smc`)

# JIT v2: Full Translation Without Interpreter Fallback

## Problem

The current JIT uses a hybrid approach: natively translated instructions mixed
with `jit_interpret_one` fallback calls. Each fallback re-decodes the
instruction, re-translates the address, and checks whether RIP advanced
linearly. This overhead makes the JIT slower than the pure interpreter for
blocks with less than ~80% native coverage. In Protected32 mode (where Windows
runs), most blocks fail this threshold because CALL/RET/PUSH/POP and many other
instructions are not natively translated.

Result: ~2.8 MIPS interpreter vs ~1.5 MIPS with JIT enabled.

## Design

### Core Principle

Every instruction in a BasicBlock is translated to native code. No instruction
is left to interpreter fallback. Two translation strategies:

1. **Native emit** — instruction semantics directly in x86-64 machine code
2. **Helper call** — `mov rdi, <ptr_to_DecodedInst>; call helper_execute_one`

The helper receives a pointer to the already-decoded `DecodedInst` in the
decode cache. No re-decode, no address translation, no RIP linearity check.

### Native Instruction Set (Phase 1)

These instructions get direct native code emission:

- MOV (reg-reg, reg-imm, reg-mem, mem-reg)
- ADD, SUB, AND, OR, XOR, CMP (reg-reg, reg-imm, acc-imm)
- TEST (reg-reg)
- INC, DEC
- PUSH reg, POP reg
- CALL rel, RET near
- JMP rel, Jcc rel
- LEA
- SHL, SHR, SAR (reg-imm, reg-1)
- MOVZX, MOVSX (reg-reg)
- XCHG (reg-reg)
- NOP

All other instructions use `helper_execute_one`.

### helper_execute_one

```rust
pub extern "C" fn helper_execute_one(
    inst: *const DecodedInst,
    cpu: &mut Cpu,
    memory: &mut GuestMemory,
    mmu: &mut Mmu,
    io: &mut IoDispatch,
    interrupts: &mut InterruptController,
) -> u32
```

Calls `executor::execute(cpu, &*inst, ...)` directly. The executor handles RIP
advance and all side effects. Returns `JIT_OK` or `JIT_EXIT_BLOCK`.

### Control Flow After Helper Calls

The JIT knows from the DecodedInst whether an instruction modifies control flow
(branches, CALL, RET, INT, IRET, HLT, etc.). For control-flow-modifying
instructions executed via helper, the emitted code always returns
`JIT_EXIT_BLOCK` after the helper call — the block ends and the dispatcher
re-enters at the new RIP.

For non-control-flow helpers (e.g., MUL, DIV, CPUID, FPU ops), execution
continues to the next instruction in the block. The emitted code checks the
helper return value: if `JIT_EXIT_BLOCK`, propagate exit; otherwise continue.

### All CPU Modes Supported

- Real16, Protected32, Long64 all use the same JIT path
- No mode skip — the Real16 guard is removed
- Mode-specific behavior handled by `emit_mask_sp`, `emit_mask_rip` (native)
  and by the executor (helper calls)

### MIN_NATIVE_RATIO_PCT Removed

Since every instruction is translated (native or helper), there is no ratio
check. Every block is compiled.

### Memory Protection

RWX mapping for the JIT buffer. No mprotect toggling needed.

### Pointer Safety (DecodedInst)

Helper calls embed a raw pointer to the `DecodedInst` in the decode cache.
This is safe because:

- `invalidate_page` removes JIT blocks when code pages are modified (SMC)
- `flush` clears all JIT blocks on CR3 change
- Both operations also clear/invalidate the decode cache
- JIT blocks and decode cache entries are always invalidated together

### Existing Infrastructure Retained

- Emitter (x86-64 assembler) — unchanged
- JitBuffer / executable_mem — unchanged (RWX)
- Block detection / BasicBlock / BlockKey — unchanged
- Decode cache — unchanged
- Lazy RFLAGS optimization — unchanged for native instructions

## Expected Performance

- Eliminates re-decode overhead (~50% of current fallback cost)
- Eliminates RIP linearity check overhead
- All blocks compiled (no ratio threshold rejection)
- All CPU modes JIT-accelerated
- Target: 10-30x improvement over interpreter (30-80 MIPS)

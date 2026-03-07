# JIT v2 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rewrite the JIT translator to eliminate interpreter fallback — every instruction becomes native code or a direct helper call (no re-decode).

**Architecture:** Replace `emit_interpreter_fallback` (which calls `jit_interpret_one` with re-decode + RIP check) with `emit_helper_call` (which embeds a pointer to the pre-decoded `DecodedInst` and calls `helper_execute_one`). Remove ratio threshold, Real16 skip, and mprotect toggling.

**Tech Stack:** Rust, no_std, x86-64 machine code emission via existing `Emitter`.

---

### Task 1: Add `helper_execute_one` to helpers.rs

**Files:**
- Modify: `libs/libcorevm/src/jit/helpers.rs`

**Step 1: Add the new helper function**

Add after the existing `jit_interpret_one` function:

```rust
/// Execute a single pre-decoded instruction via the executor.
///
/// Unlike `jit_interpret_one`, this takes a pointer to an already-decoded
/// instruction — no address translation, no re-decode. Called by JIT v2
/// for instructions that are not natively translated.
///
/// # Safety
/// `inst` must be a valid pointer to a `DecodedInst` that outlives the
/// JIT block (guaranteed by decode cache + JIT cache co-invalidation).
#[no_mangle]
pub extern "C" fn helper_execute_one(
    inst: *const crate::instruction::DecodedInst,
    cpu: &mut Cpu,
    memory: &mut GuestMemory,
    mmu: &mut Mmu,
    io: &mut IoDispatch,
    interrupts: &mut InterruptController,
) -> JitResult {
    let inst = unsafe { &*inst };
    match crate::executor::execute(cpu, inst, memory, mmu, io, interrupts) {
        Ok(()) => {
            cpu.instruction_count += 1;
            cpu.mask_rip();
            JIT_OK
        }
        Err(VmError::Halted) => {
            cpu.instruction_count += 1;
            cpu.mask_rip();
            JIT_EXIT_BLOCK
        }
        Err(_) => JIT_EXIT_BLOCK,
    }
}
```

**Step 2: Commit**

```bash
git add libs/libcorevm/src/jit/helpers.rs
git commit -m "feat(jit): add helper_execute_one for pre-decoded instruction execution"
```

---

### Task 2: Replace `emit_interpreter_fallback` with `emit_helper_call`

**Files:**
- Modify: `libs/libcorevm/src/jit/translator.rs:166-199` (translate_block loop)
- Modify: `libs/libcorevm/src/jit/translator.rs:1190-1209` (emit_interpreter_fallback)

**Step 1: Rewrite `emit_interpreter_fallback` → `emit_helper_call`**

Replace the entire `emit_interpreter_fallback` method with:

```rust
/// Emit a direct call to `helper_execute_one` with a pointer to the
/// pre-decoded instruction. No re-decode, no RIP linearity check.
///
/// For control-flow instructions (branches, CALL, RET, INT, HLT),
/// always exits the block after the helper returns.
/// For non-control-flow instructions, checks the return value and
/// exits only on error.
fn emit_helper_call(
    &self, emit: &mut Emitter, inst: &DecodedInst, is_control_flow: bool,
) {
    // SysV ABI: rdi=inst, rsi=cpu, rdx=mem, rcx=mmu, r8=io, r9=intr
    // Embed pointer to DecodedInst as immediate.
    let inst_ptr = inst as *const DecodedInst as usize as u64;
    emit.mov_ri64(Reg::Rdi, inst_ptr);
    emit.mov_rr(OpSize::S64, Reg::Rsi, CPU_PTR);
    emit.mov_rr(OpSize::S64, Reg::Rdx, MEM_PTR);
    emit.mov_rr(OpSize::S64, Reg::Rcx, MMU_PTR);
    emit.mov_rr(OpSize::S64, Reg::R8, IO_PTR);
    emit.mov_rm(OpSize::S64, Reg::R9, Reg::Rbp, INTR_STACK_OFF);

    let helper_addr = helpers::helper_execute_one as *const () as usize as u64;
    emit.call_abs(helper_addr);

    if is_control_flow {
        // Control-flow instructions always end the block.
        // The executor already updated RIP.
        emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
        self.emit_restore_and_ret(emit);
    } else {
        // Non-control-flow: check if helper signaled an error.
        let continue_label = emit.new_label();
        emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_OK as i32);
        emit.jcc_label(Cc::E, continue_label);
        // Error path: exit block.
        emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
        self.emit_restore_and_ret(emit);
        emit.bind_label(continue_label);
    }
}
```

**Step 2: Check if emitter has `mov_ri64`**

The emitter needs a `mov_ri64` method to emit `MOV reg, imm64` (REX.W + B8+rd + imm64). Check `libs/libcorevm/src/jit/emitter.rs` for this. If missing, add it:

```rust
/// MOV reg, imm64 (absolute 64-bit immediate).
pub fn mov_ri64(&mut self, dst: Reg, imm: u64) {
    let (r, rex_b) = reg_encoding(dst);
    self.emit_rex(true, false, false, rex_b);
    self.code.push(0xB8 + r);
    self.code.extend_from_slice(&imm.to_le_bytes());
}
```

**Step 3: Add `is_control_flow_instruction` helper**

Add to translator.rs (near top, after the AluOp enum):

```rust
/// Whether an instruction modifies control flow (and thus must end the
/// JIT block when executed via helper).
fn is_control_flow_instruction(inst: &DecodedInst) -> bool {
    if inst.rep != crate::instruction::RepPrefix::None {
        return true; // REP-prefixed can loop
    }
    match inst.opcode_map {
        OpcodeMap::Primary => {
            let op = inst.opcode as u8;
            matches!(op,
                0x70..=0x7F       // Jcc rel8
                | 0x9A            // CALL far
                | 0xC2 | 0xC3    // RET
                | 0xCA | 0xCB    // RETF
                | 0xCC | 0xCD    // INT3, INT
                | 0xCE           // INTO
                | 0xCF           // IRET
                | 0xE0..=0xE3    // LOOP/JCXZ
                | 0xE8 | 0xE9    // CALL rel, JMP rel
                | 0xEA | 0xEB    // JMP far, JMP rel8
                | 0xF4           // HLT
            )
        }
        OpcodeMap::Secondary => {
            let op = inst.opcode as u8;
            matches!(op,
                0x80..=0x8F       // Jcc rel32
                | 0x05            // SYSCALL
                | 0x07            // SYSRET
                | 0x34 | 0x35    // SYSENTER/SYSEXIT
            )
        }
        _ => false,
    }
}
```

**Step 4: Update `translate_block` to use `emit_helper_call`**

Replace the fallback path in `translate_block` (lines 176-187):

```rust
for inst in &block.instructions {
    if self.try_translate_native(&mut emit, inst, &mut state) {
        self.native_count += 1;
        native_in_block += 1;
    } else {
        // Flush lazy flags before helper call since the executor
        // reads RFLAGS from the register file.
        self.flush_lazy_flags(&mut emit, &mut state);
        let is_cf = is_control_flow_instruction(inst);
        self.emit_helper_call(&mut emit, inst, is_cf);
        self.fallback_count += 1;
        // Control-flow helpers already emitted a block exit,
        // so remaining instructions are unreachable. Stop emitting.
        if is_cf {
            // Emit epilogue after the helper-call exit path.
            // The emit_helper_call already emitted restore+ret for cf,
            // so we skip the normal epilogue below.
            return CompiledBlock {
                code: emit.finalize(),
                guest_instruction_count: inst_count,
                entry_phys_addr: entry_phys,
                native_instruction_count: native_in_block,
            };
        }
    }
}
```

Note: When a control-flow instruction is handled by helper, we return early
(the helper already emitted restore+ret). The normal flush+epilogue at the
end of `translate_block` only runs if the block ends without a control-flow
helper.

**Step 5: Delete the old `emit_interpreter_fallback` method**

Remove the entire old method (the one that calls `jit_interpret_one` and checks
RIP linearity).

**Step 6: Commit**

```bash
git add libs/libcorevm/src/jit/translator.rs libs/libcorevm/src/jit/emitter.rs
git commit -m "feat(jit): replace interpreter fallback with direct helper_execute_one calls"
```

---

### Task 3: Remove ratio threshold and Real16 skip

**Files:**
- Modify: `libs/libcorevm/src/jit/mod.rs:77-136`
- Modify: `libs/libcorevm/src/cpu.rs:884-890`

**Step 1: Remove MIN_NATIVE_RATIO_PCT and no_native set**

In `libs/libcorevm/src/jit/mod.rs`:

1. Remove the `MIN_NATIVE_RATIO_PCT` constant (line 79)
2. Remove the `no_native` field from `JitEngine` (line 70)
3. In `compile_block`: remove the `native_instruction_count == 0` early return
   (lines 127-130) and the ratio check (lines 131-136) and
   `self.no_native.remove(&key)` (line 137)
4. Remove `should_skip_compile` method (lines 113-115)
5. In `invalidate_page`: remove `self.no_native.retain(...)` (lines 195-197)
6. In `flush`: remove `self.no_native.clear()` (line 177)
7. In `new()`: remove `no_native` initialization (line 87)

**Step 2: Remove Real16 skip in cpu.rs**

In `libs/libcorevm/src/cpu.rs`, remove lines 884-890:

```rust
// DELETE:
if key.mode == CpuMode::Real16 {
    return self.execute_cached_block(
        instructions, key.phys_addr,
        memory, mmu, io, interrupts,
    );
}
```

**Step 3: Remove `should_skip_compile` check in cpu.rs**

In `jit_execute_block`, remove the `should_skip_compile` check (lines 896-901):

```rust
// DELETE:
if self.jit_engine.should_skip_compile(key) {
    return self.execute_cached_block(
        instructions, key.phys_addr,
        memory, mmu, io, interrupts,
    );
}
```

**Step 4: Remove mprotect toggling in cpu.rs**

Remove `make_writable()` and `make_executable()` calls around `compile_block`
(lines 910, 912).

**Step 5: Commit**

```bash
git add libs/libcorevm/src/jit/mod.rs libs/libcorevm/src/cpu.rs
git commit -m "feat(jit): remove ratio threshold, Real16 skip, and mprotect toggling"
```

---

### Task 4: Fix instruction count tracking in `jit_execute_block`

**Files:**
- Modify: `libs/libcorevm/src/cpu.rs:943-972`

**Step 1: Update instruction counting**

In the new model, `helper_execute_one` increments `instruction_count` for each
helper-executed instruction. Native instructions are NOT counted individually
inside the JIT block. We need to track how many native instructions ran.

Replace the match on result (lines 955-972) with:

```rust
match result {
    JIT_OK => {
        // All instructions ran. Native ones were not counted by helpers,
        // so add the full block count minus what helpers already counted.
        // Simplification: helpers count themselves, native instructions
        // don't. We track native_in_block in CompiledEntry.
        self.instruction_count += inst_count;
        BlockExitReason::Continue
    }
    _ => {
        // Block exited early (HLT, exception, control-flow helper).
        // helper_execute_one already counted its instruction.
        // Native instructions before exit are not precisely counted;
        // add a conservative estimate.
        BlockExitReason::Continue
    }
}
```

Note: This is unchanged from the current behavior for the JIT_OK path. The
JIT_EXIT_BLOCK path no longer needs the old comment about `jit_interpret_one`.

**Step 2: Commit**

```bash
git add libs/libcorevm/src/cpu.rs
git commit -m "fix(jit): update instruction count tracking for JIT v2"
```

---

### Task 5: Enable RWX mapping and cleanup executable_mem

**Files:**
- Modify: `libs/libcorevm/src/jit/executable_mem.rs`

**Step 1: Use RWX mapping for host_test**

In `with_capacity`, change the mmap prot flags from `PROT_READ | PROT_WRITE`
to `PROT_READ | PROT_WRITE | PROT_EXEC` and set `executable: true`.

**Step 2: Make `make_executable`/`make_writable` no-ops**

```rust
pub fn make_executable(&mut self) {
    self.executable = true;
}

pub fn make_writable(&mut self) {
    self.executable = false;
}
```

**Step 3: Commit**

```bash
git add libs/libcorevm/src/jit/executable_mem.rs
git commit -m "feat(jit): use RWX mapping, remove mprotect toggling"
```

---

### Task 6: Test and verify

**Step 1: Build**

Build the project with the user's build system.

**Step 2: Test**

```bash
scripts/test_vmd_x11 --iso /home/cmoeller/windows.iso --ram-mb 512
```

Expected: boots past BIOS into Windows setup, no crash, no garbled output.

**Step 3: If crash occurs**

Add diagnostic logging to `helper_execute_one` to identify which instruction
causes the crash. Focus on:
- What opcode fails
- What CPU mode (Real16/Protected32/Long64)
- RIP value at crash

**Step 4: Commit final state**

```bash
git add -A
git commit -m "feat(jit): JIT v2 complete — no interpreter fallback"
```

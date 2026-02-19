; =============================================================================
; syscall_fast.asm - SYSCALL/SYSRET entry point for x86-64
; =============================================================================
; SYSCALL convention (64-bit native mode only):
;   RAX = syscall number
;   RBX = arg1, R10 = arg2 (moved from RCX by user), RDX = arg3,
;   RSI = arg4, RDI = arg5
;   Return value in RAX
;
; CPU behavior on SYSCALL:
;   RCX ← RIP (user return address)
;   R11 ← RFLAGS
;   CS  ← STAR[47:32] | 0 (kernel CS)
;   SS  ← STAR[47:32] + 8 (kernel DS)
;   RFLAGS &= ~SFMASK (clears IF, TF, DF)
;
; CPU behavior on SYSRET (o64):
;   RIP ← RCX
;   RFLAGS ← R11 (masked)
;   CS  ← STAR[63:48] + 16 | 3 (user CS 64-bit)
;   SS  ← STAR[63:48] + 8 | 3 (user DS)
;
; NOTE: SYSCALL does NOT change RSP. We use SWAPGS + per-CPU data to
; save user RSP and load kernel RSP.

[BITS 64]

extern syscall_dispatch_64
extern LAPIC_TO_PERCPU

; LAPIC virtual address: LAPIC_VIRT_BASE(0xFFFFFFFFD0100000) + LAPIC_ID(0x20)
; In x86-64 kernel code model, this address fits in sign-extended 32-bit.
%define LAPIC_ID_ADDR 0xFFFFFFFFD0100020

global syscall_fast_entry
syscall_fast_entry:
    ; === Phase 1a: SWAPGS and save user RSP ===
    swapgs                          ; GS.base = kernel per-CPU data
    mov [gs:8], rsp                 ; per_cpu.user_rsp = user RSP

    ; === Phase 1b: Verify PERCPU ownership BEFORE stack switch ===
    ; KERNEL_GS_BASE can get corrupted (QEMU TCG MSR state leak between vCPUs).
    ; If [gs:16] (PERCPU.lapic_id) doesn't match this CPU's hardware LAPIC ID,
    ; we would load the WRONG kernel_rsp and fault. Check now while still on
    ; user stack (safe — user was executing code, stack is valid).
    mov [gs:24], rax                ; save syscall number to PERCPU scratch (no stack needed)
    mov rax, LAPIC_ID_ADDR
    mov eax, [rax]                  ; read LAPIC ID register (UC MMIO, per-CPU)
    shr eax, 24                     ; EAX = this CPU's LAPIC ID (bits 31:24)
    cmp al, byte [gs:16]            ; compare with PERCPU.lapic_id
    jne .repair_percpu              ; mismatch — fix MSR before touching kernel stack
    mov rax, [gs:24]                ; correct — restore syscall number

    ; === Phase 1c: Switch to kernel stack (PERCPU ownership verified) ===
    mov rsp, [gs:0]                 ; RSP = per_cpu.kernel_rsp

    ; Validate kernel RSP: must be in kernel higher-half (bit 63 set).
    ; If PERCPU.kernel_rsp was corrupted (e.g., by wild write or stale value),
    ; using it would cause #PF → #DF (unrecoverable). Detect now and fall back
    ; to the PERCPU repair path which can recover onto the user stack.
    test rsp, rsp
    jns .bad_kernel_rsp             ; RSP is positive (not in higher-half) — corrupt!

    ; === Phase 2: Build SyscallRegs frame (matching INT 0x80 layout) ===
    ; CPU-pushed interrupt frame (emulated for SYSCALL)
    push 0x23                       ; SS  (user data selector, RPL=3)
    push qword [gs:8]              ; RSP (user stack pointer)
    push r11                        ; RFLAGS (saved by SYSCALL in R11)
    push 0x2B                       ; CS  (user code64 selector, RPL=3)
    push rcx                        ; RIP (saved by SYSCALL in RCX)

    ; General-purpose registers (same order as syscall_entry.asm)
    push rax                        ; syscall number
    push rbx                        ; arg1
    push r10                        ; → rcx slot (arg2, moved to R10 by user)
    push rdx                        ; arg3
    push rsi                        ; arg4
    push rdi                        ; arg5
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    ; === Phase 3: Restore GS to user state (clean for context switches) ===
    swapgs                          ; GS.base = user value again

    ; Load kernel data segments (needed for compat mode transitions)
    mov ax, 0x10
    mov ds, ax
    mov es, ax

    ; === Phase 4: Call Rust syscall dispatcher (64-bit path) ===
    ; Re-enable interrupts — SFMASK cleared IF on SYSCALL entry, but syscall
    ; handlers need interrupts for AHCI completion, timer preemption, etc.
    ; (INT 0x80 uses a trap gate that keeps IF=1, so this is consistent.)
    sti

    mov rdi, rsp                    ; arg0 = &SyscallRegs
    call syscall_dispatch_64

    ; Store return value (full 64-bit RAX) in saved RAX position (offset 14*8 = 112)
    mov [rsp + 112], rax

    ; === Phase 5: Restore registers ===
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax

    ; === Phase 6: Return to user space via IRETQ ===
    ; Stack now has: RIP(0), CS(8), RFLAGS(16), RSP(24), SS(32)
    ;
    ; We use IRETQ unconditionally instead of SYSRETQ because:
    ;   1. SYSRETQ has the infamous non-canonical-RIP vulnerability (CVE-2012-0217)
    ;      where a GPF fires in ring 0 on AMD (and certain hypervisor emulations).
    ;   2. Some hypervisors (VirtualBox in NEM/Hyper-V mode) do not set SS.RPL=3
    ;      on SYSRETQ, leaving SS=0x20 instead of 0x23.  User code runs fine
    ;      (64-bit mode ignores SS), but the NEXT interrupt/exception pushes
    ;      SS=0x20 onto the kernel stack, and IRETQ back to ring 3 then faults
    ;      with #GP(0x20) because SS.RPL(0) != target CPL(3).
    ;   3. IRETQ uses our hardcoded frame (SS=0x23, CS=0x2B from Phase 2),
    ;      guaranteeing correct segment selectors on every hypervisor.
    ;   4. The performance difference (SYSRET vs IRETQ) is negligible — a few
    ;      nanoseconds per syscall, irrelevant for a desktop/hobby OS.
    iretq

; =============================================================================
; PERCPU repair path — entered when KERNEL_GS_BASE points to wrong CPU's PERCPU
; =============================================================================
; At entry:
;   EAX = our LAPIC ID (from hardware MMIO read)
;   RSP = user RSP (still on user stack — never switched to kernel stack!)
;   GS_BASE = wrong PERCPU address (from SWAPGS with corrupted KERNEL_GS_BASE)
;   KERNEL_GS_BASE = user's original GS value
;   [gs:24] = original RAX (syscall number) saved to wrong PERCPU's scratch
;   RCX = user RIP, R11 = user RFLAGS, all other regs = user values
;
; Strategy: restore RAX from PERCPU scratch, undo SWAPGS, fix KERNEL_GS_BASE
; via LAPIC_ID → PERCPU lookup table, then retry the entire entry sequence.
; Since we never switched to the kernel stack, no kernel state was corrupted.

.repair_percpu:
    ; Step 1: Restore RAX and undo SWAPGS
    mov rax, [gs:24]                ; restore syscall number from PERCPU scratch
    swapgs                          ; undo: GS_BASE ← user_gs, KGS ← wrong_percpu

    ; Step 2: Fix KERNEL_GS_BASE using LAPIC_TO_PERCPU lookup.
    ; Need ECX, EAX, EDX for wrmsr — save clobbered regs on user stack.
    ; (User stack is valid — the user was executing code at SYSCALL.)
    push rax                        ; save syscall number
    push rcx                        ; save user RIP
    push rdx                        ; save user arg3

    ; Re-read LAPIC ID (EAX was restored to syscall number above)
    mov rax, LAPIC_ID_ADDR
    mov eax, [rax]
    shr eax, 24
    movzx eax, al                   ; EAX = LAPIC ID (zero-extended for table index)

    ; Look up correct PERCPU address
    lea rcx, [rel LAPIC_TO_PERCPU]
    mov rcx, [rcx + rax*8]          ; RCX = &PERCPU[correct_cpu_id]
    test rcx, rcx
    jz .repair_fatal                ; shouldn't happen — no PERCPU for this LAPIC ID

    ; wrmsr(MSR_KERNEL_GS_BASE = 0xC0000102, correct_percpu_addr)
    mov rax, rcx                    ; RAX = correct PERCPU address
    mov rdx, rcx
    shr rdx, 32                     ; EDX:EAX = correct PERCPU address (64-bit)
    mov ecx, 0xC0000102             ; ECX = MSR_KERNEL_GS_BASE
    wrmsr                           ; Fix the corrupted MSR

    ; Step 3: Restore user regs and retry
    pop rdx                         ; restore user arg3
    pop rcx                         ; restore user RIP
    pop rax                         ; restore syscall number
    jmp syscall_fast_entry          ; retry — SWAPGS will now load correct PERCPU

.repair_fatal:
    ; No PERCPU entry found — halt (should never happen after init)
    cli
.repair_halt:
    hlt
    jmp .repair_halt

; =============================================================================
; Bad kernel RSP recovery — PERCPU.kernel_rsp was corrupt (not in higher-half)
; =============================================================================
; At entry:
;   RSP = corrupt value loaded from [gs:0] (NOT in higher-half)
;   GS.base = this CPU's PERCPU (already verified by LAPIC check)
;   RAX = syscall number (restored from PERCPU scratch)
;   RCX = user RIP, R11 = user RFLAGS
;   User RSP saved in [gs:8]
;
; Strategy: log the corruption via serial, switch back to user stack,
; undo SWAPGS, and return to user space with RAX = -EAGAIN (-11).
; The user program sees SYSCALL fail and can retry. This is better than
; a Double Fault which kills the entire system.

.bad_kernel_rsp:
    ; Write diagnostic to serial (0x3F8) — lock-free, no stack needed
    mov dx, 0x3F8
    mov al, '!'
    out dx, al
    mov al, 'S'
    out dx, al
    mov al, 'Y'
    out dx, al
    mov al, 'S'
    out dx, al
    mov al, ' '
    out dx, al
    mov al, 'R'
    out dx, al
    mov al, 'S'
    out dx, al
    mov al, 'P'
    out dx, al
    mov al, 10
    out dx, al

    ; Restore user RSP from PERCPU (saved earlier at [gs:8])
    mov rsp, [gs:8]

    ; Undo SWAPGS (restore user GS)
    swapgs

    ; Return -EAGAIN (errno 11) to user space so the syscall can be retried.
    ; Build a minimal IRETQ frame on the user stack (avoids SYSRET SS.RPL bug).
    ; RCX = user RIP, R11 = user RFLAGS (both still hold SYSCALL-saved values).
    ; RSP = user RSP (loaded from PERCPU before SWAPGS above).
    mov rax, rsp                    ; save original user RSP (before pushes)
    sub rsp, 40                     ; reserve 5 qwords for IRETQ frame
    mov qword [rsp + 32], 0x23     ; SS  (user data, RPL=3)
    mov [rsp + 24], rax             ; RSP (original user RSP)
    mov [rsp + 16], r11             ; RFLAGS
    mov qword [rsp + 8], 0x2B      ; CS  (user code64, RPL=3)
    mov [rsp], rcx                  ; RIP
    mov rax, -11                    ; return -EAGAIN
    iretq

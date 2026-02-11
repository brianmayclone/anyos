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

extern syscall_dispatch

global syscall_fast_entry
syscall_fast_entry:
    ; === Phase 1: Stack switch via SWAPGS (interrupts disabled by SFMASK) ===
    swapgs                          ; GS.base = kernel per-CPU data
    mov [gs:8], rsp                 ; per_cpu.user_rsp = user RSP
    mov rsp, [gs:0]                 ; RSP = per_cpu.kernel_rsp

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

    ; === Phase 4: Call Rust syscall dispatcher ===
    ; Re-enable interrupts — SFMASK cleared IF on SYSCALL entry, but syscall
    ; handlers need interrupts for AHCI completion, timer preemption, etc.
    ; (INT 0x80 uses a trap gate that keeps IF=1, so this is consistent.)
    sti

    mov rdi, rsp                    ; arg0 = &SyscallRegs
    call syscall_dispatch

    ; Store return value in saved RAX position (offset 14*8 = 112)
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

    ; === Phase 6: Return to user space via SYSRET ===
    ; Stack now has: RIP(0), CS(8), RFLAGS(16), RSP(24), SS(32)
    mov rcx, [rsp]                  ; RCX = user RIP (for SYSRETQ)
    mov r11, [rsp + 16]             ; R11 = user RFLAGS (for SYSRETQ)

    ; Validate return RIP: SYSRETQ to non-canonical address causes GPF in ring 0
    bt rcx, 47
    jc .fallback_iretq

    ; Disable interrupts for the critical RSP→SYSRET window: after loading
    ; user RSP we are still in Ring 0, so an interrupt would push its frame
    ; onto the user stack. SYSRET atomically restores RFLAGS (IF=1 from R11).
    cli
    mov rsp, [rsp + 24]
    o64 sysret

.fallback_iretq:
    ; Non-canonical RIP — use IRETQ (safe for any address)
    iretq

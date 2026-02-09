; =============================================================================
; syscall_entry.asm - System call entry point (int 0x80) for x86-64
; =============================================================================
; INT 0x80 convention (used by both 64-bit and 32-bit compat processes):
;   RAX = syscall number
;   RBX = arg1, RCX = arg2, RDX = arg3, RSI = arg4, RDI = arg5
;   Return value in RAX
;
; The dispatcher checks CS to determine if caller is 32-bit compat (CS=0x1B)
; or 64-bit native (CS=0x2B) and adjusts argument extraction accordingly.
;
; CPU pushes on INT: SS, RSP, RFLAGS, CS, RIP (always in 64-bit mode)

[BITS 64]

extern syscall_dispatch

global syscall_entry
syscall_entry:
    ; Save all general-purpose registers
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    ; Load kernel data segment (needed when entering from compat mode)
    mov ax, 0x10
    mov ds, ax
    mov es, ax

    ; Pass pointer to SyscallRegs as first arg (System V ABI: RDI)
    mov rdi, rsp
    call syscall_dispatch

    ; Store return value (RAX) back into the saved RAX on stack.
    ; Stack layout from RSP:
    ;   R15(0) R14(8) R13(16) R12(24) R11(32) R10(40) R9(48) R8(56)
    ;   RBP(64) RDI(72) RSI(80) RDX(88) RCX(96) RBX(104) RAX(112)
    mov [rsp + 112], rax

    ; Restore all general-purpose registers
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

    iretq

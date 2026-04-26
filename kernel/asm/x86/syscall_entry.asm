; =============================================================================
; syscall_entry.asm - System call entry point (int 0x80) for x86-64
; =============================================================================
; 32-bit compatibility-mode user space was removed. The only remaining
; consumer of INT 0x80 is the in-kernel signal-return trampoline mapped
; into user processes (see task::loader), which calls SYS_SIGRETURN via:
;   mov eax, 246 ; int 0x80
; All regular 64-bit programs use the SYSCALL fast path
; (kernel/asm/x86/syscall_fast.asm).
;
; Calling convention (kept as-is for the signal trampoline):
;   EAX = syscall number
;   EBX = arg1, ECX = arg2, EDX = arg3, ESI = arg4, EDI = arg5
;   Return value in EAX
;
; The dispatcher (syscall_dispatch_32) truncates args to u32. In practice
; SYS_SIGRETURN takes no arguments, so the truncation is harmless.
;
; CPU pushes on INT: SS, RSP, RFLAGS, CS, RIP (always in 64-bit mode)

[BITS 64]

extern syscall_dispatch_32

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
    call syscall_dispatch_32

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

    ; Restore user data segment (0x23) before IRETQ.
    ; The entry code sets DS/ES to kernel 0x10; IRETQ does NOT restore DS/ES.
    ; If we leave DS=0x10 (DPL=0), the CPU nulls DS on the CPL 0→3 transition,
    ; causing #GP(0) on the first user-mode memory access.
    test qword [rsp + 8], 3       ; check CS.RPL on stack — returning to ring 3?
    jz .int80_iret_done
    cli
    push rax
    mov ax, 0x23                  ; user data segment (GDT entry 4 | RPL=3)
    mov ds, ax
    mov es, ax
    pop rax
    or qword [rsp + 32], 3       ; sanitise SS (VirtualBox NEM fix)
.int80_iret_done:
    iretq

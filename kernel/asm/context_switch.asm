; =============================================================================
; context_switch.asm - Low-level thread context switch (x86-64)
; =============================================================================
; void context_switch(CpuContext* old, CpuContext* new)
; System V ABI: old in RDI, new in RSI
;
; CpuContext layout (each field u64, total 152 bytes):
;   offset 0:   rax
;   offset 8:   rbx
;   offset 16:  rcx
;   offset 24:  rdx
;   offset 32:  rsi
;   offset 40:  rdi
;   offset 48:  rbp
;   offset 56:  r8
;   offset 64:  r9
;   offset 72:  r10
;   offset 80:  r11
;   offset 88:  r12
;   offset 96:  r13
;   offset 104: r14
;   offset 112: r15
;   offset 120: rsp
;   offset 128: rip
;   offset 136: rflags
;   offset 144: cr3
; =============================================================================

[BITS 64]
global context_switch

context_switch:
    ; --- Save current context to [RDI] ---

    ; Save all general-purpose registers
    mov [rdi + 0],   rax
    mov [rdi + 8],   rbx
    mov [rdi + 16],  rcx
    mov [rdi + 24],  rdx
    mov [rdi + 32],  rsi
    mov [rdi + 40],  rdi
    mov [rdi + 48],  rbp
    mov [rdi + 56],  r8
    mov [rdi + 64],  r9
    mov [rdi + 72],  r10
    mov [rdi + 80],  r11
    mov [rdi + 88],  r12
    mov [rdi + 96],  r13
    mov [rdi + 104], r14
    mov [rdi + 112], r15

    ; Save RSP+8 (past return address). The restore path uses push+ret
    ; which adds an extra stack entry. Saving RSP+8 ensures the caller's
    ; RSP is correct after a round-trip save/restore.
    lea rax, [rsp + 8]
    mov [rdi + 120], rax

    ; Save RIP (return address is at [rsp])
    mov rax, [rsp]
    mov [rdi + 128], rax

    ; Save RFLAGS
    pushfq
    pop rax
    mov [rdi + 136], rax

    ; Save CR3
    mov rax, cr3
    mov [rdi + 144], rax

    ; Mark old context as fully saved. Other CPUs check this flag in
    ; pick_next() before restoring a thread — prevents racing on a
    ; partially-saved CpuContext. x86 TSO guarantees all prior stores
    ; (register saves above) are visible before this store.
    mov qword [rdi + 152], 1

    ; --- Load new context from [RSI] ---

    ; Load CR3 if different (avoid TLB flush if same address space)
    mov rax, [rsi + 144]
    mov rcx, cr3
    cmp rax, rcx
    je .skip_cr3
    mov cr3, rax
.skip_cr3:

    ; Load RFLAGS but keep IF (bit 9) clear to prevent nested timer interrupts
    ; during the rest of the context switch. For resumed threads, IRET will
    ; restore the original IF from the interrupt frame. For new threads, the
    ; entry function must explicitly enable interrupts (sti).
    mov rax, [rsi + 136]
    and rax, 0xFFFFFFFFFFFFFDFF  ; clear IF (bit 9)
    push rax
    popfq

    ; Load general-purpose registers (except RAX and RSI — used as temporaries)
    mov rbx, [rsi + 8]
    mov rcx, [rsi + 16]
    mov rdx, [rsi + 24]
    ; skip rsi (offset 32) — loaded last since we use it as pointer
    mov rdi, [rsi + 40]
    mov rbp, [rsi + 48]
    mov r8,  [rsi + 56]
    mov r9,  [rsi + 64]
    mov r10, [rsi + 72]
    mov r11, [rsi + 80]
    mov r12, [rsi + 88]
    mov r13, [rsi + 96]
    mov r14, [rsi + 104]
    mov r15, [rsi + 112]

    ; Load RSP (switch to new thread's stack)
    mov rsp, [rsi + 120]

    ; Push new RIP for ret (using RAX as temp)
    mov rax, [rsi + 128]
    push rax

    ; Load RAX and RSI (final reads from new context — must be last)
    mov rax, [rsi + 0]
    mov rsi, [rsi + 32]

    ; Jump to new RIP
    ret

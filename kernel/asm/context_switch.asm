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

; Polled serial output: write immediate byte to COM1 (clobbers AL, DX)
%macro SERIAL_PUTC_POLL 1
    mov dx, 0x3FD
%%wait:
    in al, dx
    test al, 0x20
    jz %%wait
    mov dx, 0x3F8
    mov al, %1
    out dx, al
%endmacro

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

    ; --- Validate new context before loading ---
    ; Tighter range validation for heap-corrupted CpuContext.
    ; RSP must be in heap range (>= HEAP_START = 0xFFFFFFFF82000000)
    ;   because all kernel stacks are heap-allocated.
    ; RIP must be in kernel code range (>= KERNEL_VMA = 0xFFFFFFFF80100000
    ;   AND < HEAP_START) because context_switch RIP is always a kernel
    ;   text address (return address from schedule_inner or thread entry).
    ; At this point we have NOT modified RSP or loaded any new state.

    ; Check RSP >= 0xFFFFFFFF82000000 (kernel heap)
    mov rax, [rsi + 120]
    mov rcx, 0xFFFFFFFF82000000
    cmp rax, rcx
    jb .bad_ctx                 ; RSP below heap = corrupt

    ; Check RIP >= 0xFFFFFFFF80100000 (kernel text)
    mov rax, [rsi + 128]
    mov rcx, 0xFFFFFFFF80100000
    cmp rax, rcx
    jb .bad_ctx                 ; RIP below kernel text = corrupt

    ; Check RIP < 0xFFFFFFFF82000000 (must not be in heap/stack area)
    mov rcx, 0xFFFFFFFF82000000
    cmp rax, rcx
    jae .bad_ctx                ; RIP in heap = executing data as code

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

; =========================================================================
; .bad_ctx: Corrupt CpuContext detected — full dump + halt
; =========================================================================
; Reached when RSP/RIP fails range validation.
; We have NOT loaded any new CR3/RSP/RIP — still on old thread's stack/CR3.
; However, the old context has save_complete=1, so another CPU could pick it
; up and start using the same stack. Use register-only I/O (no push/pop).
;
; Prints ALL 20 CpuContext fields to help identify the corruption pattern.
; Then halts this CPU. Other CPUs continue running normally.
; =========================================================================
.bad_ctx:
    cli
    mov r14, rsi               ; save CpuContext pointer in R14

    ; Print "\r\n!CTX CORRUPT ctx="
    SERIAL_PUTC_POLL 0x0D
    SERIAL_PUTC_POLL 0x0A
    SERIAL_PUTC_POLL '!'
    SERIAL_PUTC_POLL 'C'
    SERIAL_PUTC_POLL 'T'
    SERIAL_PUTC_POLL 'X'
    SERIAL_PUTC_POLL ' '
    SERIAL_PUTC_POLL 'C'
    SERIAL_PUTC_POLL 'O'
    SERIAL_PUTC_POLL 'R'
    SERIAL_PUTC_POLL 'R'
    SERIAL_PUTC_POLL 'U'
    SERIAL_PUTC_POLL 'P'
    SERIAL_PUTC_POLL 'T'
    SERIAL_PUTC_POLL ' '
    SERIAL_PUTC_POLL 'c'
    SERIAL_PUTC_POLL 't'
    SERIAL_PUTC_POLL 'x'
    SERIAL_PUTC_POLL '='
    mov r8, r14
    lea r15, [rel .dump_ctx_addr_done]
    jmp .print_hex_r8
.dump_ctx_addr_done:
    SERIAL_PUTC_POLL 0x0D
    SERIAL_PUTC_POLL 0x0A

    ; Dump all 20 fields: offset 0 (rax) through offset 152 (save_complete)
    ; Use R12 as field index (0..19), R14 = CpuContext base (preserved)
    xor r12d, r12d             ; R12 = 0 (first field index)
.dump_field:
    ; Print "  [" offset "] = " value "\r\n"
    SERIAL_PUTC_POLL ' '
    SERIAL_PUTC_POLL ' '
    SERIAL_PUTC_POLL '['

    ; Print offset as 3-digit decimal (max 152)
    mov rax, r12
    shl rax, 3                 ; offset = index * 8
    mov r9, rax                ; save offset in R9 for field read
    ; hundreds
    xor rdx, rdx
    mov rcx, 100
    div rcx                    ; RAX = hundreds, RDX = remainder
    add al, '0'
    mov bl, al
    mov dx, 0x3FD
.dw0:  in al, dx
    test al, 0x20
    jz .dw0
    mov dx, 0x3F8
    mov al, bl
    out dx, al
    ; tens
    mov rax, rdx               ; remainder
    xor rdx, rdx
    mov rcx, 10
    div rcx
    add al, '0'
    mov bl, al
    mov dx, 0x3FD
.dw1:  in al, dx
    test al, 0x20
    jz .dw1
    mov dx, 0x3F8
    mov al, bl
    out dx, al
    ; ones
    add dl, '0'
    mov bl, dl
    mov dx, 0x3FD
.dw2:  in al, dx
    test al, 0x20
    jz .dw2
    mov dx, 0x3F8
    mov al, bl
    out dx, al

    SERIAL_PUTC_POLL ']'
    SERIAL_PUTC_POLL '='

    ; Print field value as 16-nibble hex
    mov r8, [r14 + r9]        ; read CpuContext field at offset R9
    lea r15, [rel .dump_field_done]
    jmp .print_hex_r8
.dump_field_done:
    SERIAL_PUTC_POLL 0x0D
    SERIAL_PUTC_POLL 0x0A

    inc r12
    cmp r12, 20                ; 20 fields (0..19, offsets 0..152)
    jb .dump_field

    ; Halt this CPU forever. Other CPUs continue via their own scheduler.
.ctx_halt:
    hlt
    jmp .ctx_halt

; =========================================================================
; .print_hex_r8: Print R8 as 16-nibble hex via polled COM1
; =========================================================================
; Clobbers: RAX, RBX, RCX, RDX.  Returns via JMP R15 (link register).
; No stack usage — safe even when stack may be raced by another CPU.
; =========================================================================
.print_hex_r8:
    mov rcx, 60                ; bit shift (MSB first)
.ph_loop:
    mov rax, r8
    shr rax, cl
    and al, 0x0F
    cmp al, 10
    jb .ph_digit
    add al, 'a' - 10
    jmp .ph_emit
.ph_digit:
    add al, '0'
.ph_emit:
    mov bl, al                 ; save hex char
    mov dx, 0x3FD
.ph_wait:
    in al, dx
    test al, 0x20
    jz .ph_wait
    mov dx, 0x3F8
    mov al, bl
    out dx, al
    sub rcx, 4
    jge .ph_loop
    jmp r15

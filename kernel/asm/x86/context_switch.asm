; =============================================================================
; context_switch.asm - Low-level thread context switch (x86-64)
; =============================================================================
; void context_switch(CpuContext* old, CpuContext* new)
; System V ABI: old in RDI, new in RSI
;
; CpuContext layout (each field u64, total 176 bytes):
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
;   offset 152: save_complete
;   offset 160: canary   (CANARY_MAGIC = 0xCAFEBABEDEADBEEF)
;   offset 168: checksum (XOR of offsets 0..152)
; =============================================================================

%define CANARY_MAGIC 0xCAFEBABEDEADBEEF

[BITS 64]
global context_switch
extern PCID_NOFLUSH_MASK

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

    ; Write canary magic value (corruption detection)
    mov rax, CANARY_MAGIC
    mov [rdi + 160], rax

    ; Compute XOR checksum of register fields (offsets 0..144, 19 fields).
    ; Excludes save_complete (offset 152) which is modified by Rust scheduler.
    mov rax, [rdi + 0]
    xor rax, [rdi + 8]
    xor rax, [rdi + 16]
    xor rax, [rdi + 24]
    xor rax, [rdi + 32]
    xor rax, [rdi + 40]
    xor rax, [rdi + 48]
    xor rax, [rdi + 56]
    xor rax, [rdi + 64]
    xor rax, [rdi + 72]
    xor rax, [rdi + 80]
    xor rax, [rdi + 88]
    xor rax, [rdi + 96]
    xor rax, [rdi + 104]
    xor rax, [rdi + 112]
    xor rax, [rdi + 120]
    xor rax, [rdi + 128]
    xor rax, [rdi + 136]
    xor rax, [rdi + 144]
    mov [rdi + 168], rax

    ; Mark old context as fully saved — MUST be the LAST store.
    ; Other CPUs check this flag in pick_eligible() before restoring a
    ; thread. All prior stores (registers, canary, checksum) are visible
    ; before this store thanks to x86 TSO (total store order). This
    ; prevents another CPU from loading a context with a stale checksum.
    mov qword [rdi + 152], 1

    ; --- Validate new context before loading ---

    ; 1. Verify canary (detects external memory overwrites)
    mov rax, CANARY_MAGIC
    cmp [rsi + 160], rax
    jne .bad_canary              ; canary destroyed = memory corruption

    ; 2. Verify XOR checksum of register fields (offsets 0..144, 19 fields)
    mov rax, [rsi + 0]
    xor rax, [rsi + 8]
    xor rax, [rsi + 16]
    xor rax, [rsi + 24]
    xor rax, [rsi + 32]
    xor rax, [rsi + 40]
    xor rax, [rsi + 48]
    xor rax, [rsi + 56]
    xor rax, [rsi + 64]
    xor rax, [rsi + 72]
    xor rax, [rsi + 80]
    xor rax, [rsi + 88]
    xor rax, [rsi + 96]
    xor rax, [rsi + 104]
    xor rax, [rsi + 112]
    xor rax, [rsi + 120]
    xor rax, [rsi + 128]
    xor rax, [rsi + 136]
    xor rax, [rsi + 144]
    cmp rax, [rsi + 168]
    jne .bad_checksum            ; fields modified since save = corruption

    ; 3. Range validation for heap-corrupted CpuContext.
    ; RSP must be >= KERNEL_VMA (0xFFFFFFFF80100000) — covers both:
    ;   - boot/idle stacks in BSS area (0xFFFFFFFF803xxxxx)
    ;   - heap-allocated thread stacks (0xFFFFFFFF82xxxxxx+)
    ; RIP must be in kernel code range (>= KERNEL_VMA AND < HEAP_START)
    ;   because context_switch RIP is always a kernel text address.
    ; At this point we have NOT modified RSP or loaded any new state.

    ; Check RSP >= 0xFFFFFFFF80100000 (kernel virtual base)
    mov rax, [rsi + 120]
    mov rcx, 0xFFFFFFFF80100000
    cmp rax, rcx
    jb .bad_ctx                 ; RSP below kernel = corrupt

    ; Check RIP >= 0xFFFFFFFF80100000 (kernel text)
    mov rax, [rsi + 128]
    cmp rax, rcx                ; RCX still = 0xFFFFFFFF80100000
    jb .bad_ctx                 ; RIP below kernel text = corrupt

    ; Check RIP < 0xFFFFFFFF82000000 (must not be in heap/stack area)
    mov rcx, 0xFFFFFFFF82000000
    cmp rax, rcx
    jae .bad_ctx                ; RIP in heap = executing data as code

    ; --- Load new context from [RSI] ---

    ; Load CR3 if different (avoid TLB flush if same address space).
    ; With PCID enabled, context.cr3 = PML4_phys | pcid (bits 0-11).
    ; PCID_NOFLUSH_MASK is (1<<63) when PCID active, 0 otherwise.
    ; Setting bit 63 tells the CPU to preserve TLB entries tagged with
    ; other PCIDs, so switching between processes doesn't destroy each
    ; other's cached translations.
    mov rax, [rsi + 144]
    mov rcx, cr3
    cmp rax, rcx
    je .skip_cr3
    or  rax, [rel PCID_NOFLUSH_MASK]
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
; Corruption handlers — canary, checksum, and range check failures
; =========================================================================
; Each prints a distinguishing label then falls through to the dump.
; At entry RSI = new CpuContext pointer, we're still on old thread's stack.
; Uses register-only I/O (no push/pop) since stack may be raced.
; =========================================================================

.bad_canary:
    cli
    mov r14, rsi
    ; Print "\r\n!CANARY DEAD ctx="
    SERIAL_PUTC_POLL 0x0D
    SERIAL_PUTC_POLL 0x0A
    SERIAL_PUTC_POLL '!'
    SERIAL_PUTC_POLL 'C'
    SERIAL_PUTC_POLL 'A'
    SERIAL_PUTC_POLL 'N'
    SERIAL_PUTC_POLL 'A'
    SERIAL_PUTC_POLL 'R'
    SERIAL_PUTC_POLL 'Y'
    SERIAL_PUTC_POLL ' '
    SERIAL_PUTC_POLL 'D'
    SERIAL_PUTC_POLL 'E'
    SERIAL_PUTC_POLL 'A'
    SERIAL_PUTC_POLL 'D'
    jmp .dump_header_ctx

.bad_checksum:
    cli
    mov r14, rsi
    ; Print "\r\n!CHECKSUM FAIL ctx="
    SERIAL_PUTC_POLL 0x0D
    SERIAL_PUTC_POLL 0x0A
    SERIAL_PUTC_POLL '!'
    SERIAL_PUTC_POLL 'C'
    SERIAL_PUTC_POLL 'H'
    SERIAL_PUTC_POLL 'K'
    SERIAL_PUTC_POLL 'S'
    SERIAL_PUTC_POLL 'U'
    SERIAL_PUTC_POLL 'M'
    SERIAL_PUTC_POLL ' '
    SERIAL_PUTC_POLL 'F'
    SERIAL_PUTC_POLL 'A'
    SERIAL_PUTC_POLL 'I'
    SERIAL_PUTC_POLL 'L'
    jmp .dump_header_ctx

.bad_ctx:
    cli
    mov r14, rsi
    ; Print "\r\n!RSP/RIP BAD  ctx="
    SERIAL_PUTC_POLL 0x0D
    SERIAL_PUTC_POLL 0x0A
    SERIAL_PUTC_POLL '!'
    SERIAL_PUTC_POLL 'R'
    SERIAL_PUTC_POLL 'S'
    SERIAL_PUTC_POLL 'P'
    SERIAL_PUTC_POLL '/'
    SERIAL_PUTC_POLL 'R'
    SERIAL_PUTC_POLL 'I'
    SERIAL_PUTC_POLL 'P'
    SERIAL_PUTC_POLL ' '
    SERIAL_PUTC_POLL 'B'
    SERIAL_PUTC_POLL 'A'
    SERIAL_PUTC_POLL 'D'
    SERIAL_PUTC_POLL ' '

.dump_header_ctx:
    ; Print " ctx="
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

    ; Dump all 22 fields: offset 0 (rax) through offset 168 (checksum)
    ; Use R12 as field index (0..21), R14 = CpuContext base (preserved)
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
    cmp r12, 22                ; 22 fields (0..21, offsets 0..168)
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

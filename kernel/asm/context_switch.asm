; =============================================================================
; context_switch.asm - Low-level thread context switch
; =============================================================================
; void context_switch(CpuContext* old, CpuContext* new)
;
; CpuContext layout (each field u32, total 44 bytes):
;   offset 0:  eax
;   offset 4:  ebx
;   offset 8:  ecx
;   offset 12: edx
;   offset 16: esi
;   offset 20: edi
;   offset 24: ebp
;   offset 28: esp
;   offset 32: eip
;   offset 36: eflags
;   offset 40: cr3
; =============================================================================

[BITS 32]
global context_switch

context_switch:
    ; Arguments: [esp+4] = old CpuContext*, [esp+8] = new CpuContext*
    mov eax, [esp + 4]     ; old context pointer

    ; Save current registers into old context
    mov [eax + 0],  eax    ; eax (will be overwritten, but that's ok)
    mov [eax + 4],  ebx
    mov [eax + 8],  ecx
    mov [eax + 12], edx
    mov [eax + 16], esi
    mov [eax + 20], edi
    mov [eax + 24], ebp

    ; Save ESP+4 (past return address). The restore path uses push+ret
    ; which adds 4 bytes, so saving ESP+4 ensures the caller's ESP is
    ; correct after a round-trip save/restore.
    lea ecx, [esp + 4]
    mov [eax + 28], ecx

    ; Save EIP (return address is at [esp], which is our caller's return addr)
    mov ecx, [esp]         ; Return address
    mov [eax + 32], ecx    ; Save as EIP

    ; Save EFLAGS
    pushfd
    pop ecx
    mov [eax + 36], ecx

    ; Save CR3
    mov ecx, cr3
    mov [eax + 40], ecx

    ; Load new context
    mov eax, [esp + 8]     ; new context pointer

    ; Load CR3 if different (avoid TLB flush if same address space)
    mov ecx, [eax + 40]
    mov edx, cr3
    cmp ecx, edx
    je .skip_cr3
    mov cr3, ecx
.skip_cr3:

    ; Load EFLAGS but keep IF (bit 9) clear to prevent nested timer interrupts
    ; during the rest of the context switch. For resumed threads, IRET will
    ; restore the original IF from the interrupt frame. For new threads, the
    ; entry function must explicitly enable interrupts (sti).
    mov ecx, [eax + 36]
    and ecx, 0xFFFFFDFF     ; clear IF (bit 9)
    push ecx
    popfd

    ; Load registers
    mov ebx, [eax + 4]
    mov ecx, [eax + 8]
    mov edx, [eax + 12]
    mov esi, [eax + 16]
    mov edi, [eax + 20]
    mov ebp, [eax + 24]
    mov esp, [eax + 28]

    ; Push new EIP and "return" to it
    mov eax, [eax + 32]
    push eax
    ret                     ; Jump to new EIP

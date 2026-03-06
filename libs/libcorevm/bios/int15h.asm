; =============================================================================
; int15h.asm — INT 15h: System services (memory map, extended memory)
; =============================================================================

int15h_handler:
    cmp ax, 0xE820
    je i15_e820
    cmp ax, 0xE801
    je i15_e801
    cmp ah, 0x88
    je i15_get_ext_mem
    cmp ah, 0xC0
    je i15_get_config_table
    cmp ax, 0x2401
    je i15_enable_a20
    cmp ax, 0x2402
    je i15_get_a20_status
    cmp ax, 0x2403
    je i15_get_a20_support

    ; Unsupported function.
    mov ah, 0x86
    stc
    iret

; ---------------------------------------------------------------------------
; AX=E820h: Get system memory map.
; ---------------------------------------------------------------------------
i15_e820:
    cmp edx, 0x534D4150
    jne .fail

    push esi
    push ds

    ; The E820 table lives in the BIOS ROM segment (cs:).
    ; memory_detect wrote it with DS=CS; we read it with DS=CS too.
    push cs
    pop ds

    movzx eax, word [e820_count]
    cmp ebx, eax
    jge .e820_fail_pop

    mov eax, ebx
    mov esi, E820_ENTRY_SIZE
    mul esi
    add eax, e820_table         ; ESI = &e820_table[entry_index]
    mov esi, eax

    cmp ecx, 20
    jb .e820_fail_pop

    push cx
    cmp cx, 24
    jb .e820_copy_20
    mov cx, 24
    jmp .e820_copy_ready
.e820_copy_20:
    mov cx, 20
.e820_copy_ready:
    mov dx, cx
    push di
    cld
    rep movsb                   ; DS:SI (segment 0) → ES:DI
    pop di
    cmp dx, 24
    jne .e820_copy_done
    mov dword [es:di + 20], 1   ; ACPI 3.x ext-attrs: entry enabled/valid
.e820_copy_done:
    pop cx

    mov eax, 0x534D4150
    mov edx, 0x534D4150
    inc ebx
    movzx ecx, word [e820_count]
    cmp ebx, ecx
    jl .e820_not_last
    xor ebx, ebx
.e820_not_last:
    movzx ecx, dx

    pop ds
    pop esi
    clc
    iret

.e820_fail_pop:
    pop ds
    pop esi
.fail:
    mov ah, 0x86
    stc
    iret

; ---------------------------------------------------------------------------
; AH=88h: Get extended memory size.
; ---------------------------------------------------------------------------
i15_get_ext_mem:
    push ebx
    mov eax, [cs:ram_size_bytes]
    sub eax, 0x100000
    shr eax, 10
    cmp eax, 0xFFFF
    jbe .ok
    mov eax, 0xFFFF
.ok:
    pop ebx
    clc
    iret

; ---------------------------------------------------------------------------
; AX=E801h: Get extended memory size (two ranges).
; ---------------------------------------------------------------------------
i15_e801:
    push esi
    mov eax, [cs:ram_size_bytes]

    cmp eax, 0x1000000
    jbe .below_16

    mov cx, 15360
    sub eax, 0x1000000
    shr eax, 16
    mov bx, ax
    mov dx, ax
    mov ax, cx
    jmp .done

.below_16:
    sub eax, 0x100000
    shr eax, 10
    mov cx, ax
    xor bx, bx
    xor dx, dx
    jmp .done

.done:
    pop esi
    clc
    iret

; ---------------------------------------------------------------------------
; AH=C0h: Get system configuration table pointer.
; ---------------------------------------------------------------------------
i15_get_config_table:
    mov bx, i15_sys_config_table
    push ax
    mov ax, 0xF000
    mov es, ax
    pop ax
    xor ah, ah
    clc
    iret

i15_sys_config_table:
    dw 8                        ; Table length
    db 0xFC                     ; Model: AT compatible
    db 0x01                     ; Sub-model
    db 0x00                     ; BIOS revision
    db 0x74                     ; Feature byte 1
    db 0x00, 0x00, 0x00, 0x00  ; Feature bytes 2-5

; ---------------------------------------------------------------------------
; AX=2401h: Enable A20 gate (always enabled in libcorevm).
; ---------------------------------------------------------------------------
i15_enable_a20:
    xor ah, ah
    clc
    iret

; ---------------------------------------------------------------------------
; AX=2402h: Get A20 gate status.
; ---------------------------------------------------------------------------
i15_get_a20_status:
    mov al, 1
    xor ah, ah
    clc
    iret

; ---------------------------------------------------------------------------
; AX=2403h: Get A20 gate support.
; ---------------------------------------------------------------------------
i15_get_a20_support:
    mov bx, 0x0003
    xor ah, ah
    clc
    iret

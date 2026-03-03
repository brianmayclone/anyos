; =============================================================================
; int19h.asm — INT 19h: Bootstrap loader
; =============================================================================

int19h_handler:
    sti

    cmp byte [ide_master_present], 0
    je .no_boot

    ; --- Boot from hard disk (MBR) ---
    ; Works for both real HD images and isohybrid ISOs.
    mov si, str_booting_hd
    call bios_print

    ; Load MBR: read LBA 0 to 0x0000:0x7C00.
    mov eax, 0
    mov ecx, 1
    push es
    push word 0x0000
    pop es
    mov di, 0x7C00
    call ide_read_sectors
    pop es
    jc .no_boot

    ; Check boot signature.
    cmp word [0x7C00 + 510], 0xAA55
    jne .no_boot

    ; Valid MBR — jump to it.
    mov dl, 0x80
    jmp 0x0000:0x7C00

.no_boot:
    mov si, str_no_boot
    call bios_print
.halt:
    cli
    hlt
    jmp .halt

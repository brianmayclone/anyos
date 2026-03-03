; =============================================================================
; int19h.asm — INT 19h: Bootstrap loader
; =============================================================================

int19h_handler:
    ; Try boot devices in order: HD, CD, then halt.
    sti

    ; --- Try hard disk (drive 0x80) ---
    cmp byte [ide_master_present], 0
    je .try_cd

    ; Print boot message.
    mov si, str_booting_hd
    call bios_print

    ; Load MBR: read LBA 0 to 0x0000:0x7C00.
    mov eax, 0                  ; LBA 0
    mov ecx, 1                  ; 1 sector
    xor di, di
    push es
    push word 0x0000
    pop es
    mov di, 0x7C00
    call ide_read_sectors
    pop es
    jc .try_cd

    ; Check boot signature.
    cmp word [0x7C00 + 510], 0xAA55
    jne .try_cd

    ; Valid MBR — jump to it.
    mov dl, 0x80                ; Boot drive
    jmp 0x0000:0x7C00

.try_cd:
    ; CD-ROM boot via El Torito would go here (Phase 2).
    ; For now, fall through to no-boot.

.no_boot:
    mov si, str_no_boot
    call bios_print
.halt:
    hlt
    jmp .halt

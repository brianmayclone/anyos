; =============================================================================
; int19h.asm — INT 19h: Bootstrap loader
; =============================================================================

; Temp buffer for El Torito parsing (in free conventional memory).
ELTORITO_BUF    equ 0x8000

int19h_handler:
    ; Try boot devices in order: El Torito CD, HD MBR, then halt.
    sti

    cmp byte [ide_master_present], 0
    je .no_boot

    ; --- Try El Torito CD-ROM boot first ---
    ; Read CD sector 17 (BRVD) = 4 IDE sectors at LBA 68 → 2048 bytes.
    mov eax, 68                 ; CD sector 17 × 4
    mov ecx, 4                  ; 1 CD sector = 4 × 512-byte IDE sectors
    push es
    push word 0x0000
    pop es
    mov di, ELTORITO_BUF
    call ide_read_sectors
    pop es
    jc .try_hd

    ; Check Boot Record Volume Descriptor: type=0x00 + "CD001" at offset 1.
    cmp byte [ELTORITO_BUF], 0x00
    jne .try_hd
    cmp dword [ELTORITO_BUF + 1], 'CD00'
    jne .try_hd
    cmp byte [ELTORITO_BUF + 5], '1'
    jne .try_hd

    ; Get Boot Catalog LBA from BRVD offset 71 (in CD sectors).
    mov eax, [ELTORITO_BUF + 71]
    test eax, eax
    jz .try_hd
    shl eax, 2                  ; Convert CD sectors → IDE sectors

    ; Read the Boot Catalog (1 CD sector = 4 IDE sectors) to temp buffer.
    mov ecx, 4
    push es
    push word 0x0000
    pop es
    mov di, ELTORITO_BUF
    call ide_read_sectors
    pop es
    jc .try_hd

    ; Validate catalog: Validation Entry (offset 0).
    cmp byte [ELTORITO_BUF], 0x01          ; Header ID = 1
    jne .try_hd
    cmp word [ELTORITO_BUF + 30], 0xAA55   ; Key bytes
    jne .try_hd

    ; Parse Default Entry (offset 32).
    cmp byte [ELTORITO_BUF + 32], 0x88     ; Boot indicator = bootable
    jne .try_hd

    ; Load segment (0 = default 0x07C0 → linear address 0x7C00).
    mov bx, [ELTORITO_BUF + 34]
    test bx, bx
    jnz .cd_has_seg
    mov bx, 0x07C0
.cd_has_seg:

    ; Sector count (in 512-byte virtual sectors).
    movzx ecx, word [ELTORITO_BUF + 38]
    test ecx, ecx
    jz .try_hd

    ; Load RBA: starting CD sector of the boot image.
    mov eax, [ELTORITO_BUF + 40]
    shl eax, 2                  ; Convert CD sectors → IDE sectors

    ; Load the boot image to load_segment:0000.
    push es
    mov es, bx
    xor di, di
    call ide_read_sectors
    pop es
    jc .try_hd

    ; El Torito boot succeeded!
    mov si, str_booting_cd
    call bios_print

    ; Jump to the boot image at load_segment:0000.
    mov dl, 0x80                ; Boot drive number
    push bx                     ; Segment
    push word 0x0000            ; Offset
    retf                        ; Far return → jumps to seg:offset

    ; --- Try hard disk MBR boot ---
.try_hd:
    ; Read MBR: LBA 0 to 0x0000:0x7C00.
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

    ; Valid MBR — boot from hard disk.
    mov si, str_booting_hd
    call bios_print
    mov dl, 0x80
    jmp 0x0000:0x7C00

.no_boot:
    mov si, str_no_boot
    call bios_print
.halt:
    cli
    hlt
    jmp .halt

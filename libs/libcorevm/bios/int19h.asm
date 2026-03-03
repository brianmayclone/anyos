; =============================================================================
; int19h.asm — INT 19h: Bootstrap loader
; =============================================================================

; Temp buffer for El Torito parsing (in free conventional memory).
ELTORITO_BUF    equ 0x8000

int19h_handler:
    ; Boot order: check if media is a CD (El Torito), else try MBR.
    sti

    cmp byte [ide_master_present], 0
    je .no_boot

    ; --- Detect media type: check for El Torito signature at CD sector 17 ---
    ; Read CD sector 17 (BRVD) = 4 IDE sectors at LBA 68.
    mov eax, 68
    mov ecx, 4
    push es
    push word 0x0000
    pop es
    mov di, ELTORITO_BUF
    call ide_read_sectors
    pop es
    jc .try_hd

    ; Check "CD001" signature at BRVD offset 1.
    cmp byte [ELTORITO_BUF], 0x00
    jne .try_hd
    cmp dword [ELTORITO_BUF + 1], 'CD00'
    jne .try_hd
    cmp byte [ELTORITO_BUF + 5], '1'
    jne .try_hd

    ; --- This is a CD/ISO — boot via El Torito ---

    ; Get Boot Catalog LBA from BRVD offset 71 (in 2048-byte CD sectors).
    mov eax, [ELTORITO_BUF + 71]
    test eax, eax
    jz .try_hd
    shl eax, 2                  ; CD sectors → IDE 512-byte sectors

    ; Read Boot Catalog to temp buffer.
    mov ecx, 4
    push es
    push word 0x0000
    pop es
    mov di, ELTORITO_BUF
    call ide_read_sectors
    pop es
    jc .try_hd

    ; Validate catalog: Validation Entry at offset 0.
    cmp byte [ELTORITO_BUF], 0x01
    jne .try_hd
    cmp word [ELTORITO_BUF + 30], 0xAA55
    jne .try_hd

    ; Parse Default Entry at offset 32.
    cmp byte [ELTORITO_BUF + 32], 0x88
    jne .try_hd

    ; Load segment (0 = default 0x07C0).
    mov bx, [ELTORITO_BUF + 34]
    test bx, bx
    jnz .cd_has_seg
    mov bx, 0x07C0
.cd_has_seg:

    ; Sector count (512-byte virtual sectors).
    movzx ecx, word [ELTORITO_BUF + 38]
    test ecx, ecx
    jz .try_hd

    ; Load RBA (CD sector of boot image).
    mov eax, [ELTORITO_BUF + 40]
    shl eax, 2                  ; CD sectors → IDE sectors

    ; Load boot image to load_segment:0000.
    push es
    mov es, bx
    xor di, di
    call ide_read_sectors
    pop es
    jc .try_hd

    ; Success — boot from CD-ROM.
    mov si, str_booting_cd
    call bios_print
    mov dl, 0x80
    push bx
    push word 0x0000
    retf

    ; --- Not a CD — try hard disk MBR ---
.try_hd:
    mov eax, 0
    mov ecx, 1
    push es
    push word 0x0000
    pop es
    mov di, 0x7C00
    call ide_read_sectors
    pop es
    jc .no_boot

    cmp word [0x7C00 + 510], 0xAA55
    jne .no_boot

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

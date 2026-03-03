; =============================================================================
; ide.asm — IDE/ATA drive detection
; =============================================================================

; IDE primary channel ports.
IDE_DATA        equ 0x01F0
IDE_ERROR       equ 0x01F1
IDE_SEC_COUNT   equ 0x01F2
IDE_LBA_LO      equ 0x01F3
IDE_LBA_MID     equ 0x01F4
IDE_LBA_HI      equ 0x01F5
IDE_DRIVE_HEAD  equ 0x01F6
IDE_STATUS      equ 0x01F7
IDE_CMD         equ 0x01F7
IDE_ALT_STATUS  equ 0x03F6

; IDE status bits.
IDE_SR_BSY      equ 0x80
IDE_SR_DRDY     equ 0x40
IDE_SR_DRQ      equ 0x08
IDE_SR_ERR      equ 0x01

; IDE commands.
IDE_CMD_IDENTIFY        equ 0xEC
IDE_CMD_READ_SECTORS    equ 0x20
IDE_CMD_READ_SECTORS_EXT equ 0x24

; Drive parameter storage.
ide_master_present: db 0
ide_master_cyls:    dw 0
ide_master_heads:   dw 0
ide_master_spt:     dw 0        ; Sectors per track
ide_master_lba28:   dd 0        ; Total LBA28 sectors
ide_master_lba48:   dd 0, 0     ; Total LBA48 sectors (64-bit)

; 512-byte IDENTIFY buffer (temporarily used during POST).
align 2
ide_identify_buf:   times 256 dw 0

; ---------------------------------------------------------------------------
; ide_detect — Detect IDE master drive on primary channel.
; ---------------------------------------------------------------------------
ide_detect:
    push eax
    push ebx
    push ecx
    push edx
    push di
    push es

    ; Select master drive.
    mov dx, IDE_DRIVE_HEAD
    mov al, 0xA0            ; Master, LBA mode off
    out dx, al

    ; Small delay: read alt status a few times.
    mov dx, IDE_ALT_STATUS
    in al, dx
    in al, dx
    in al, dx
    in al, dx

    ; Check status — if 0xFF, no drive.
    mov dx, IDE_STATUS
    in al, dx
    cmp al, 0xFF
    je .no_drive
    test al, al
    jz .no_drive

    ; Send IDENTIFY command.
    mov dx, IDE_SEC_COUNT
    xor al, al
    out dx, al
    mov dx, IDE_LBA_LO
    out dx, al
    mov dx, IDE_LBA_MID
    out dx, al
    mov dx, IDE_LBA_HI
    out dx, al
    mov dx, IDE_CMD
    mov al, IDE_CMD_IDENTIFY
    out dx, al

    ; Wait for BSY to clear.
    mov dx, IDE_STATUS
    mov cx, 0xFFFF
.wait_bsy:
    in al, dx
    test al, IDE_SR_BSY
    jz .bsy_clear
    dec cx
    jnz .wait_bsy
    jmp .no_drive           ; Timeout
.bsy_clear:
    ; Check that LBA_MID and LBA_HI are still 0 (not ATAPI).
    mov dx, IDE_LBA_MID
    in al, dx
    test al, al
    jnz .no_drive           ; ATAPI or SATA signature
    mov dx, IDE_LBA_HI
    in al, dx
    test al, al
    jnz .no_drive

    ; Wait for DRQ.
    mov dx, IDE_STATUS
    mov cx, 0xFFFF
.wait_drq:
    in al, dx
    test al, IDE_SR_ERR
    jnz .no_drive
    test al, IDE_SR_DRQ
    jnz .drq_set
    dec cx
    jnz .wait_drq
    jmp .no_drive
.drq_set:
    ; Read 256 words of IDENTIFY data.
    mov dx, IDE_DATA
    mov di, ide_identify_buf
    push ds
    pop es
    mov cx, 256
    rep insw

    ; Parse IDENTIFY data.
    ; Word 1: cylinders.
    mov ax, [ide_identify_buf + 2]
    mov [ide_master_cyls], ax
    ; Word 3: heads.
    mov ax, [ide_identify_buf + 6]
    mov [ide_master_heads], ax
    ; Word 6: sectors per track.
    mov ax, [ide_identify_buf + 12]
    mov [ide_master_spt], ax
    ; Words 60-61: total LBA28 sectors.
    mov eax, [ide_identify_buf + 120]
    mov [ide_master_lba28], eax
    ; Words 100-103: total LBA48 sectors (if supported).
    mov eax, [ide_identify_buf + 200]
    mov [ide_master_lba48], eax
    mov eax, [ide_identify_buf + 204]
    mov [ide_master_lba48 + 4], eax

    mov byte [ide_master_present], 1

    ; Update BDA: number of hard disks.
    xor ax, ax
    mov es, ax
    mov byte [es:BDA_NUM_HD], 1

    jmp .done

.no_drive:
    mov byte [ide_master_present], 0
.done:
    pop es
    pop di
    pop edx
    pop ecx
    pop ebx
    pop eax
    ret

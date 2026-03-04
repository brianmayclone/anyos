; =============================================================================
; ide.asm — IDE/ATA drive detection (master + slave)
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

; Master drive parameter storage.
ide_master_present: db 0
ide_master_cyls:    dw 0
ide_master_heads:   dw 0
ide_master_spt:     dw 0        ; Sectors per track
ide_master_lba28:   dd 0        ; Total LBA28 sectors
ide_master_lba48:   dd 0, 0     ; Total LBA48 sectors (64-bit)

; Slave drive parameter storage.
ide_slave_present:  db 0
ide_slave_cyls:     dw 0
ide_slave_heads:    dw 0
ide_slave_spt:      dw 0
ide_slave_lba28:    dd 0
ide_slave_lba48:    dd 0, 0

; Drive select byte for ide_read_sectors / ide_write_sectors.
; 0xE0 = master+LBA, 0xF0 = slave+LBA.
ide_drive_sel:      db 0xE0

; 512-byte IDENTIFY buffer (temporarily used during POST).
align 2
ide_identify_buf:   times 256 dw 0

; ---------------------------------------------------------------------------
; ide_detect — Detect IDE master and slave drives on primary channel.
; ---------------------------------------------------------------------------
ide_detect:
    push eax
    push ebx
    push ecx
    push edx
    push di
    push es

    ; ── Detect master drive ──────────────────────────────────────────────

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
    je .no_master
    test al, al
    jz .no_master

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
.wait_bsy_m:
    in al, dx
    test al, IDE_SR_BSY
    jz .bsy_clear_m
    dec cx
    jnz .wait_bsy_m
    jmp .no_master           ; Timeout
.bsy_clear_m:
    ; Check that LBA_MID and LBA_HI are still 0 (not ATAPI).
    mov dx, IDE_LBA_MID
    in al, dx
    test al, al
    jnz .no_master           ; ATAPI or SATA signature
    mov dx, IDE_LBA_HI
    in al, dx
    test al, al
    jnz .no_master

    ; Wait for DRQ.
    mov dx, IDE_STATUS
    mov cx, 0xFFFF
.wait_drq_m:
    in al, dx
    test al, IDE_SR_ERR
    jnz .no_master
    test al, IDE_SR_DRQ
    jnz .drq_set_m
    dec cx
    jnz .wait_drq_m
    jmp .no_master
.drq_set_m:
    ; Read 256 words of IDENTIFY data.
    mov dx, IDE_DATA
    mov di, ide_identify_buf
    push ds
    pop es
    mov cx, 256
    rep insw

    ; Parse IDENTIFY data.  Store results in the BIOS ROM segment (cs:) so
    ; they survive when the boot image reuses low RAM for its own data.
    ; Word 1: cylinders.
    mov ax, [ide_identify_buf + 2]
    mov [cs:ide_master_cyls], ax
    ; Word 3: heads.
    mov ax, [ide_identify_buf + 6]
    mov [cs:ide_master_heads], ax
    ; Word 6: sectors per track.
    mov ax, [ide_identify_buf + 12]
    mov [cs:ide_master_spt], ax
    ; Words 60-61: total LBA28 sectors.
    mov eax, [ide_identify_buf + 120]
    mov [cs:ide_master_lba28], eax
    ; Words 100-103: total LBA48 sectors (if supported).
    mov eax, [ide_identify_buf + 200]
    mov [cs:ide_master_lba48], eax
    mov eax, [ide_identify_buf + 204]
    mov [cs:ide_master_lba48 + 4], eax

    mov byte [cs:ide_master_present], 1

    ; Update BDA: number of hard disks.
    xor ax, ax
    mov es, ax
    mov byte [es:BDA_NUM_HD], 1

    jmp .detect_slave

.no_master:
    mov byte [cs:ide_master_present], 0
    ; BDA_NUM_HD stays 0.

    ; ── Detect slave drive ───────────────────────────────────────────────

.detect_slave:
    ; Select slave drive.
    mov dx, IDE_DRIVE_HEAD
    mov al, 0xB0            ; Slave, LBA mode off
    out dx, al

    ; Small delay.
    mov dx, IDE_ALT_STATUS
    in al, dx
    in al, dx
    in al, dx
    in al, dx

    ; Check status.
    mov dx, IDE_STATUS
    in al, dx
    cmp al, 0xFF
    je .no_slave
    test al, al
    jz .no_slave

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
.wait_bsy_s:
    in al, dx
    test al, IDE_SR_BSY
    jz .bsy_clear_s
    dec cx
    jnz .wait_bsy_s
    jmp .no_slave
.bsy_clear_s:
    mov dx, IDE_LBA_MID
    in al, dx
    test al, al
    jnz .no_slave
    mov dx, IDE_LBA_HI
    in al, dx
    test al, al
    jnz .no_slave

    ; Wait for DRQ.
    mov dx, IDE_STATUS
    mov cx, 0xFFFF
.wait_drq_s:
    in al, dx
    test al, IDE_SR_ERR
    jnz .no_slave
    test al, IDE_SR_DRQ
    jnz .drq_set_s
    dec cx
    jnz .wait_drq_s
    jmp .no_slave
.drq_set_s:
    ; Read 256 words of IDENTIFY data (reuse buffer).
    mov dx, IDE_DATA
    mov di, ide_identify_buf
    push ds
    pop es
    mov cx, 256
    rep insw

    ; Parse IDENTIFY data for slave.
    mov ax, [ide_identify_buf + 2]
    mov [cs:ide_slave_cyls], ax
    mov ax, [ide_identify_buf + 6]
    mov [cs:ide_slave_heads], ax
    mov ax, [ide_identify_buf + 12]
    mov [cs:ide_slave_spt], ax
    mov eax, [ide_identify_buf + 120]
    mov [cs:ide_slave_lba28], eax
    mov eax, [ide_identify_buf + 200]
    mov [cs:ide_slave_lba48], eax
    mov eax, [ide_identify_buf + 204]
    mov [cs:ide_slave_lba48 + 4], eax

    mov byte [cs:ide_slave_present], 1

    ; Increment BDA hard disk count.
    xor ax, ax
    mov es, ax
    inc byte [es:BDA_NUM_HD]

    jmp .done

.no_slave:
    mov byte [cs:ide_slave_present], 0

.done:
    ; Re-select master drive as default.
    mov dx, IDE_DRIVE_HEAD
    mov al, 0xA0
    out dx, al

    pop es
    pop di
    pop edx
    pop ecx
    pop ebx
    pop eax
    ret

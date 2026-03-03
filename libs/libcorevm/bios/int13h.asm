; =============================================================================
; int13h.asm — INT 13h: Disk BIOS services
; =============================================================================

int13h_handler:
    cmp ah, 0x00
    je .reset_disk
    cmp ah, 0x02
    je .read_sectors_chs
    cmp ah, 0x08
    je .get_drive_params
    cmp ah, 0x15
    je .get_disk_type
    cmp ah, 0x41
    je .check_extensions
    cmp ah, 0x42
    je .extended_read
    cmp ah, 0x43
    je .extended_write
    cmp ah, 0x48
    je .get_ext_params
    ; Unsupported function.
    mov ah, 0x01                ; Invalid function
    stc
    iret

; ---------------------------------------------------------------------------
; AH=00h: Reset disk system.
; ---------------------------------------------------------------------------
.reset_disk:
    xor ah, ah                  ; Success
    clc
    iret

; ---------------------------------------------------------------------------
; AH=02h: Read sectors using CHS.
;   AL = sector count, CH = cylinder low, CL[5:0] = sector, CL[7:6]+CH = cyl,
;   DH = head, DL = drive, ES:BX = buffer.
; ---------------------------------------------------------------------------
.read_sectors_chs:
    push eax
    push ebx
    push ecx
    push edx
    push edi
    push es

    ; Only support drive 0x80 (first HD).
    cmp dl, 0x80
    jne .chs_error

    cmp byte [ide_master_present], 0
    je .chs_error

    ; Convert CHS to LBA: LBA = (C * H + h) * S + (s - 1)
    ; where H = total heads, S = sectors per track.
    movzx eax, ch               ; Cylinder low 8 bits
    mov bl, cl
    shr bl, 6                   ; Cylinder high 2 bits
    movzx ebx, bl
    shl ebx, 8
    or eax, ebx                 ; EAX = cylinder

    movzx ebx, word [ide_master_heads]
    mul ebx                     ; EAX = C * H
    movzx ebx, dh               ; Head
    add eax, ebx                ; EAX = C * H + h

    movzx ebx, word [ide_master_spt]
    mul ebx                     ; EAX = (C*H + h) * S

    mov bl, cl
    and bl, 0x3F                ; Sector (1-based)
    dec bl
    movzx ebx, bl
    add eax, ebx                ; EAX = LBA

    ; Now read using LBA. Sector count was in AL on entry.
    ; We saved original regs, so recover sector count.
    pop es
    push es
    mov ecx, [esp + 20]         ; Original EAX (AL = count)
    movzx ecx, cl               ; ECX = sector count
    ; EAX = LBA, ECX = count, ES:BX was set by caller.
    ; Recover original BX from stack.
    mov edi, [esp + 16]         ; Original EBX
    and edi, 0xFFFF             ; DI = buffer offset (original BX)

    call ide_read_sectors
    jc .chs_error_pop

    pop es
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
    ; AL = sectors read (set by ide_read_sectors).
    xor ah, ah
    clc
    iret

.chs_error_pop:
    pop es
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
.chs_error:
    mov ah, 0x01
    stc
    iret

; ---------------------------------------------------------------------------
; AH=08h: Get drive parameters.
;   DL = drive number.
;   Returns: CH = max cyl low, CL[7:6] = max cyl high, CL[5:0] = max sector,
;            DH = max head, DL = number of drives.
; ---------------------------------------------------------------------------
.get_drive_params:
    cmp dl, 0x80
    jne .gdp_no_drive
    cmp byte [ide_master_present], 0
    je .gdp_no_drive

    push bx
    mov ax, [ide_master_cyls]
    dec ax                      ; Max cylinder (0-based)
    mov ch, al                  ; Low 8 bits of cylinder
    mov cl, ah
    shl cl, 6                   ; High 2 bits into CL[7:6]

    mov ax, [ide_master_spt]
    and al, 0x3F
    or cl, al                   ; Max sector in CL[5:0]

    mov ax, [ide_master_heads]
    dec ax
    mov dh, al                  ; Max head (0-based)

    mov dl, 1                   ; 1 drive
    xor ax, ax                  ; AH = 0 (success)
    mov bl, 0                   ; Drive type (not applicable for HD)
    pop bx
    clc
    iret

.gdp_no_drive:
    mov ah, 0x07                ; Drive parameter error
    mov dl, 0
    stc
    iret

; ---------------------------------------------------------------------------
; AH=15h: Get disk type.
; ---------------------------------------------------------------------------
.get_disk_type:
    cmp dl, 0x80
    jne .gdt_none
    cmp byte [ide_master_present], 0
    je .gdt_none

    mov ah, 0x03                ; Type 3 = hard disk
    ; CX:DX = total sectors.
    mov eax, [ide_master_lba28]
    mov cx, ax
    shr eax, 16
    mov dx, ax
    xchg cx, dx                 ; CX = high, DX = low
    clc
    iret

.gdt_none:
    mov ah, 0x00                ; No drive
    clc
    iret

; ---------------------------------------------------------------------------
; AH=41h: Check INT 13h extensions.
;   BX = 0x55AA, DL = drive.
;   Returns: BX = 0xAA55, AH = version, CX = API bitmap.
; ---------------------------------------------------------------------------
.check_extensions:
    cmp dl, 0x80
    jb .chk_ext_fail
    cmp bx, 0x55AA
    jne .chk_ext_fail

    mov bx, 0xAA55
    mov ah, 0x30                ; Version 3.0
    mov cx, 0x0001              ; Extended disk access supported
    clc
    iret

.chk_ext_fail:
    mov ah, 0x01
    stc
    iret

; ---------------------------------------------------------------------------
; AH=42h: Extended read sectors (LBA).
;   DL = drive, DS:SI = Disk Address Packet (DAP).
;
;   DAP format:
;     Byte 0: size (16)
;     Byte 1: reserved (0)
;     Word 2: sector count
;     Word 4: buffer offset
;     Word 6: buffer segment
;     Qword 8: starting LBA
; ---------------------------------------------------------------------------
.extended_read:
    push eax
    push ebx
    push ecx
    push edx
    push edi
    push es
    push ds
    push si

    ; Only drive 0x80.
    cmp dl, 0x80
    jne .ext_read_fail

    cmp byte [ide_master_present], 0
    je .ext_read_fail

    ; Read DAP fields.
    movzx ecx, word [si + 2]   ; Sector count
    mov di, [si + 4]            ; Buffer offset
    mov ax, [si + 6]            ; Buffer segment
    mov es, ax
    mov eax, [si + 8]           ; LBA low 32 bits (enough for LBA28)

    call ide_read_sectors
    jc .ext_read_fail_pop

    pop si
    pop ds
    pop es
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
    xor ah, ah
    clc
    iret

.ext_read_fail_pop:
    pop si
    pop ds
    pop es
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
.ext_read_fail:
    mov ah, 0x01
    stc
    iret

; ---------------------------------------------------------------------------
; AH=43h: Extended write sectors (LBA). Similar structure to AH=42h.
; ---------------------------------------------------------------------------
.extended_write:
    push eax
    push ebx
    push ecx
    push edx
    push edi
    push es
    push ds
    push si

    cmp dl, 0x80
    jne .ext_write_fail

    cmp byte [ide_master_present], 0
    je .ext_write_fail

    ; Read DAP.
    movzx ecx, word [si + 2]
    mov di, [si + 4]
    mov ax, [si + 6]
    mov es, ax
    mov eax, [si + 8]

    call ide_write_sectors
    jc .ext_write_fail_pop

    pop si
    pop ds
    pop es
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
    xor ah, ah
    clc
    iret

.ext_write_fail_pop:
    pop si
    pop ds
    pop es
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
.ext_write_fail:
    mov ah, 0x01
    stc
    iret

; ---------------------------------------------------------------------------
; AH=48h: Get extended drive parameters.
;   DL = drive, DS:SI = result buffer.
; ---------------------------------------------------------------------------
.get_ext_params:
    cmp dl, 0x80
    jne .gep_fail
    cmp byte [ide_master_present], 0
    je .gep_fail

    ; Buffer size (minimum 26 bytes).
    mov word [si + 0], 26       ; Size of result
    mov word [si + 2], 0x0002   ; Flags: CHS valid

    ; CHS geometry.
    movzx eax, word [ide_master_cyls]
    mov [si + 4], eax           ; Cylinders
    movzx eax, word [ide_master_heads]
    mov [si + 8], eax           ; Heads
    movzx eax, word [ide_master_spt]
    mov [si + 12], eax          ; Sectors per track

    ; Total sectors (64-bit).
    mov eax, [ide_master_lba28]
    mov [si + 16], eax
    mov dword [si + 20], 0      ; High dword

    ; Bytes per sector.
    mov word [si + 24], 512

    xor ah, ah
    clc
    iret

.gep_fail:
    mov ah, 0x01
    stc
    iret

; =============================================================================
; IDE PIO helpers
; =============================================================================

; ---------------------------------------------------------------------------
; ide_read_sectors — Read sectors from IDE master using PIO LBA28.
;   Input:  EAX = start LBA, ECX = sector count, ES:DI = buffer
;   Output: CF set on error
; ---------------------------------------------------------------------------
ide_read_sectors:
    push eax
    push ebx
    push ecx
    push edx
    push edi

.read_loop:
    test ecx, ecx
    jz .read_done

    ; Select drive/LBA bits 24-27.
    push eax
    mov edx, eax
    shr edx, 24
    and dl, 0x0F
    or dl, 0xE0                 ; Master, LBA mode
    mov al, dl
    mov dx, IDE_DRIVE_HEAD
    out dx, al
    pop eax

    ; Sector count = 1.
    push eax
    mov dx, IDE_SEC_COUNT
    mov al, 1
    out dx, al
    pop eax

    ; LBA low byte.
    push eax
    mov dx, IDE_LBA_LO
    out dx, al
    pop eax

    ; LBA mid byte.
    push eax
    shr eax, 8
    mov dx, IDE_LBA_MID
    out dx, al
    pop eax

    ; LBA high byte.
    push eax
    shr eax, 16
    mov dx, IDE_LBA_HI
    out dx, al
    pop eax

    ; Send READ SECTORS command.
    mov dx, IDE_CMD
    mov al, IDE_CMD_READ_SECTORS
    out dx, al

    ; Wait for DRQ.
    mov dx, IDE_STATUS
    push cx
    mov cx, 0xFFFF
.read_wait:
    in al, dx
    test al, IDE_SR_BSY
    jnz .read_wait_cont
    test al, IDE_SR_DRQ
    jnz .read_ready
    test al, IDE_SR_ERR
    jnz .read_err
.read_wait_cont:
    dec cx
    jnz .read_wait
    pop cx
    jmp .read_error

.read_err:
    pop cx
    jmp .read_error

.read_ready:
    pop cx

    ; Read 256 words (512 bytes).
    push cx
    push dx
    mov dx, IDE_DATA
    mov cx, 256
    rep insw
    pop dx
    pop cx

    inc eax                     ; Next LBA
    dec ecx                     ; Decrement count
    jmp .read_loop

.read_done:
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
    clc
    ret

.read_error:
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
    stc
    ret

; ---------------------------------------------------------------------------
; ide_write_sectors — Write sectors to IDE master using PIO LBA28.
;   Input:  EAX = start LBA, ECX = sector count, ES:DI = buffer
;   Output: CF set on error
; ---------------------------------------------------------------------------
ide_write_sectors:
    push eax
    push ebx
    push ecx
    push edx
    push esi

    mov esi, edi                ; Source = ES:SI for outsw

.write_loop:
    test ecx, ecx
    jz .write_done

    ; Select drive/LBA.
    push eax
    mov edx, eax
    shr edx, 24
    and dl, 0x0F
    or dl, 0xE0
    mov al, dl
    mov dx, IDE_DRIVE_HEAD
    out dx, al
    pop eax

    ; Sector count = 1.
    push eax
    mov dx, IDE_SEC_COUNT
    mov al, 1
    out dx, al
    pop eax

    ; LBA bytes.
    push eax
    mov dx, IDE_LBA_LO
    out dx, al
    shr eax, 8
    mov dx, IDE_LBA_MID
    out dx, al
    shr eax, 8
    mov dx, IDE_LBA_HI
    out dx, al
    pop eax

    ; WRITE SECTORS command.
    mov dx, IDE_CMD
    push ax
    mov al, 0x30                ; WRITE SECTORS
    out dx, al
    pop ax

    ; Wait for DRQ.
    mov dx, IDE_STATUS
    push cx
    mov cx, 0xFFFF
.write_wait:
    in al, dx
    test al, IDE_SR_BSY
    jnz .write_wait
    test al, IDE_SR_DRQ
    jnz .write_ready
    test al, IDE_SR_ERR
    jnz .write_err
    dec cx
    jnz .write_wait
    pop cx
    jmp .write_error
.write_err:
    pop cx
    jmp .write_error
.write_ready:
    pop cx

    ; Write 256 words.
    push cx
    push dx
    mov dx, IDE_DATA
    mov cx, 256
    rep outsw
    pop dx
    pop cx

    ; Flush: wait for BSY clear.
    mov dx, IDE_STATUS
    push cx
    mov cx, 0xFFFF
.write_flush:
    in al, dx
    test al, IDE_SR_BSY
    jz .write_flushed
    dec cx
    jnz .write_flush
.write_flushed:
    pop cx

    inc eax
    dec ecx
    jmp .write_loop

.write_done:
    pop esi
    pop edx
    pop ecx
    pop ebx
    pop eax
    clc
    ret

.write_error:
    pop esi
    pop edx
    pop ecx
    pop ebx
    pop eax
    stc
    ret

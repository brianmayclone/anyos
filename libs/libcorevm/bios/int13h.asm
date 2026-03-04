; =============================================================================
; int13h.asm — INT 13h: Disk BIOS services
; =============================================================================

; Last INT 13h status (AH-style code). 0 = success.
int13_last_status: db 0
int13_last_ah:     db 0
int13_last_dl:     db 0
int13_call_count:  dw 0
int13_trace_counter: dw 0
INT13_TRACE_MAX     equ 512

int13h_handler:
    inc word [cs:int13_call_count]
    mov [cs:int13_last_ah], ah
    mov [cs:int13_last_dl], dl

    ; Trace INT 13h calls to serial: "[13 AH=xx DL=xx]".
    push ax
    push bx
    push dx
    mov bl, ah
    mov bh, dl
    call int13_trace_enter
    pop dx
    pop bx
    pop ax

    cmp ah, 0x01
    je .get_last_status
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
    cmp ah, 0x4B
    je .get_cd_emul_status
    ; Unsupported function.
    mov ah, 0x01                ; Invalid function
    stc
    iret

; ---------------------------------------------------------------------------
; AH=01h: Get status of last operation.
; ---------------------------------------------------------------------------
.get_last_status:
    mov ah, [cs:int13_last_status]
    test ah, ah
    jz .gls_ok
    stc
    iret
.gls_ok:
    clc
    iret

; ---------------------------------------------------------------------------
; AH=00h: Reset disk system.
; ---------------------------------------------------------------------------
.reset_disk:
    mov byte [cs:int13_last_status], 0
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

    ; Support first HDD (0x80) and virtual El Torito drive (0xE0).
    cmp dl, 0x80
    je .chs_drive_ok
    cmp dl, 0xE0
    je .chs_drive_ok
    jmp .chs_error

.chs_drive_ok:

    cmp byte [cs:ide_master_present], 0
    je .chs_error

    ; Convert CHS to LBA: LBA = (C * H + h) * S + (s - 1)
    ; where H = total heads, S = sectors per track.
    movzx eax, ch               ; Cylinder low 8 bits
    mov bl, cl
    shr bl, 6                   ; Cylinder high 2 bits
    movzx ebx, bl
    shl ebx, 8
    or eax, ebx                 ; EAX = cylinder

    ; CHS geometry for conversion.
    ; HDD: use IDENTIFY geometry.
    ; CD no-emulation: expose synthetic geometry (64 heads, 32 spt)
    ; consistent with AH=08.
    cmp dl, 0xE0
    jne .chs_geom_hdd
    mov ebx, 64
    jmp .chs_heads_ok
.chs_geom_hdd:
    movzx ebx, word [cs:ide_master_heads]
.chs_heads_ok:
    mul ebx                     ; EAX = C * H
    movzx ebx, dh               ; Head
    add eax, ebx                ; EAX = C * H + h

    cmp dl, 0xE0
    jne .chs_spt_hdd
    mov ebx, 32
    jmp .chs_spt_ok
.chs_spt_hdd:
    movzx ebx, word [cs:ide_master_spt]
.chs_spt_ok:
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
    mov byte [cs:int13_last_status], 0
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
    mov byte [cs:int13_last_status], 0x01
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
    je .gdp_hdd
    cmp dl, 0xE0
    je .gdp_cd
    jmp .gdp_no_drive

.gdp_hdd:
    cmp byte [cs:ide_master_present], 0
    je .gdp_no_drive

    push bx
    mov ax, [cs:ide_master_cyls]
    dec ax                      ; Max cylinder (0-based)
    mov ch, al                  ; Low 8 bits of cylinder
    mov cl, ah
    shl cl, 6                   ; High 2 bits into CL[7:6]

    mov ax, [cs:ide_master_spt]
    and al, 0x3F
    or cl, al                   ; Max sector in CL[5:0]

    mov ax, [cs:ide_master_heads]
    dec ax
    mov dh, al                  ; Max head (0-based)

    mov dl, 1                   ; 1 drive
    mov byte [cs:int13_last_status], 0
    xor ax, ax                  ; AH = 0 (success)
    mov bl, 0                   ; Drive type (not applicable for HD)
    pop bx
    clc
    iret

.gdp_cd:
    cmp byte [cs:ide_master_present], 0
    je .gdp_no_drive

    ; Synthetic CHS for CD no-emulation compatibility.
    ; Heads = 64, SPT = 32, cylinders derived from total sectors.
    push bx
    mov eax, [cs:ide_master_lba28]
    xor edx, edx
    mov ebx, 2048               ; 64 * 32
    div ebx                     ; EAX = cylinders
    test eax, eax
    jz .gdp_cd_cyl_zero
    dec eax
.gdp_cd_cyl_zero:
    cmp eax, 1023
    jbe .gdp_cd_cyl_ok
    mov eax, 1023
.gdp_cd_cyl_ok:

    mov ch, al                  ; Cylinder low 8 bits
    mov cl, ah                  ; Cylinder high 2 bits in AH[1:0]
    and cl, 0x03
    shl cl, 6
    or cl, 32                   ; SPT = 32

    mov dh, 63                  ; Heads = 64 => max head 63
    mov dl, 1                   ; 1 drive
    mov byte [cs:int13_last_status], 0
    xor ax, ax
    mov bl, 0
    pop bx
    clc
    iret

.gdp_no_drive:
    mov byte [cs:int13_last_status], 0x07
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
    cmp byte [cs:ide_master_present], 0
    je .gdt_none

    mov ah, 0x03                ; Type 3 = hard disk
    ; CX:DX = total sectors.
    mov eax, [cs:ide_master_lba28]
    mov cx, ax
    shr eax, 16
    mov dx, ax
    xchg cx, dx                 ; CX = high, DX = low
    mov byte [cs:int13_last_status], 0
    clc
    iret

.gdt_none:
    mov byte [cs:int13_last_status], 0
    mov ah, 0x00                ; No drive
    clc
    iret

; ---------------------------------------------------------------------------
; AH=41h: Check INT 13h extensions.
;   BX = 0x55AA, DL = drive.
;   Returns: BX = 0xAA55, AH = version, CX = API bitmap.
;
;   Supported drives: 0x80 (HDD) and 0xE0 (virtual CD-ROM, El Torito).
; ---------------------------------------------------------------------------
.check_extensions:
    ; Accept 0x80 (HDD) and 0xE0 (virtual CD) only.
    cmp dl, 0x80
    je .chk_ext_ok
    cmp dl, 0xE0
    je .chk_ext_ok
    jmp .chk_ext_fail

.chk_ext_ok:
    cmp bx, 0x55AA
    jne .chk_ext_fail

    mov bx, 0xAA55
    mov ah, 0x30                ; Version 3.0
    mov cx, 0x0001              ; Extended disk access supported
    mov byte [cs:int13_last_status], 0
    clc
    iret

.chk_ext_fail:
    mov byte [cs:int13_last_status], 0x01
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
;
;   For DL=0xE0 (virtual CD-ROM): use standard EDD DAP semantics
;   (LBA + count in 512-byte sectors), same as HDD.  Linux El Torito
;   bootloaders (e.g. ISOLINUX) already convert catalog RBAs to 512-byte
;   units before calling AH=42.
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

    ; BIOS variables use cs: prefix (in the ROM segment) so they are safe
    ; regardless of caller's DS.  Save caller's DS in BX for DAP access.
    mov bx, ds

    cmp byte [cs:ide_master_present], 0
    je .ext_read_fail_pop          ; FIX: was .ext_read_fail — stack already pushed

    cmp dl, 0x80
    je .ext_read_hdd
    cmp dl, 0xE0
    je .ext_read_cd
    jmp .ext_read_fail_pop         ; FIX: was .ext_read_fail — stack already pushed

.ext_read_hdd:
    ; Read DAP fields via caller's DS (saved in BX), using ES as proxy segment.
    push bx
    pop es                          ; ES = caller's DS
    movzx ecx, word [es:si + 2]    ; Sector count (512-byte sectors)
    mov di, [es:si + 4]            ; Buffer offset
    mov bx, [es:si + 6]            ; Buffer segment
    mov eax, [es:si + 8]           ; LBA (512-byte sector units)
    mov es, bx                      ; ES = buffer segment
    call ide_read_sectors
    jc .ext_read_fail_pop
    jmp .ext_read_ok

.ext_read_cd:
    ; CD-ROM: DAP uses 512-byte sectors (EDD semantics).
    push bx
    pop es                          ; ES = caller's DS
    movzx ecx, word [es:si + 2]    ; Sector count (512-byte sectors)
    mov di, [es:si + 4]            ; Buffer offset
    mov bx, [es:si + 6]            ; Buffer segment
    mov eax, [es:si + 8]           ; LBA (512-byte sector units)
    mov es, bx                      ; ES = buffer segment
    call ide_read_sectors
    jc .ext_read_fail_pop

.ext_read_ok:
    pop si
    pop ds
    pop es
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
    mov byte [cs:int13_last_status], 0
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
    mov byte [cs:int13_last_status], 0x01
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

    cmp byte [cs:ide_master_present], 0
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
    mov byte [cs:int13_last_status], 0
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
    mov byte [cs:int13_last_status], 0x01
    mov ah, 0x01
    stc
    iret

; ---------------------------------------------------------------------------
; AH=48h: Get extended drive parameters.
;   DL = drive, DS:SI = result buffer.
; ---------------------------------------------------------------------------
.get_ext_params:
    cmp byte [cs:ide_master_present], 0
    je .gep_fail

    cmp dl, 0x80
    je .gep_hdd
    cmp dl, 0xE0
    je .gep_cd
    jmp .gep_fail

.gep_hdd:
    ; Buffer size (minimum 26 bytes).
    mov word [si + 0], 26       ; Size of result
    mov word [si + 2], 0x0002   ; Flags: CHS valid

    ; CHS geometry.
    movzx eax, word [cs:ide_master_cyls]
    mov [si + 4], eax           ; Cylinders
    movzx eax, word [cs:ide_master_heads]
    mov [si + 8], eax           ; Heads
    movzx eax, word [cs:ide_master_spt]
    mov [si + 12], eax          ; Sectors per track

    ; Total sectors (64-bit, in 512-byte units).
    mov eax, [cs:ide_master_lba28]
    mov [si + 16], eax
    mov dword [si + 20], 0      ; High dword

    ; Bytes per sector.
    mov word [si + 24], 512

    mov byte [cs:int13_last_status], 0
    xor ah, ah
    clc
    iret

.gep_cd:
    ; Virtual CD-ROM: report 2048-byte sectors.
    ; Total capacity in 2048-byte sectors = total_ata_sectors / 4.
    mov word [si + 0], 26       ; Size of result
    mov word [si + 2], 0x0000   ; Flags: no CHS (CD-ROM)
    mov dword [si + 4],  0      ; Cylinders (N/A)
    mov dword [si + 8],  0      ; Heads (N/A)
    mov dword [si + 12], 0      ; Sectors per track (N/A)

    ; Total ISO sectors = ATA sectors / 4.
    mov eax, [cs:ide_master_lba28]
    shr eax, 2
    mov [si + 16], eax
    mov dword [si + 20], 0      ; High dword

    ; Bytes per sector: 2048.
    mov word [si + 24], 2048

    mov byte [cs:int13_last_status], 0
    xor ah, ah
    clc
    iret

.gep_fail:
    mov byte [cs:int13_last_status], 0x01
    mov ah, 0x01
    stc
    iret

; ---------------------------------------------------------------------------
; AH=4Bh: Get/Terminate Disk Emulation Status (El Torito).
;   AL = 0x00 → Terminate disk emulation (we treat as status query too)
;   AL = 0x01 → Return Emulation Status
;   DL = drive number
;
; Fills the 20-byte El Torito Specification Packet at DS:SI:
;   +0   (byte)  packet size = 0x13
;   +1   (byte)  boot media type
;   +2   (byte)  drive number of the emulated drive
;   +3   (byte)  controller index (0)
;   +4   (dword) image start (Load RBA in CD-sector units)
;   +8   (word)  device specification (0)
;   +10  (word)  user buffer segment (0)
;   +12  (word)  load segment
;   +14  (word)  sector count (512-byte virtual sectors)
;   +16  (word)  CHS of emulated drive (0 for no-emulation)
;   +18  (byte)  reserved (0)
; ---------------------------------------------------------------------------
.get_cd_emul_status:
        ; Only support subfunctions 00h (terminate/query) and 01h (status).
        cmp al, 0x00
        je .gcds_sub_ok
        cmp al, 0x01
        je .gcds_sub_ok
        jmp .gcds_fail          ; Nothing pushed yet — skip pops
    .gcds_sub_ok:

    ; Save caller's DS in ES for output-buffer writes (ES:SI).
    push es
    mov ax, ds
    mov es, ax          ; ES = caller's DS

    ; All el_torito_* variables are in the BIOS ROM segment (cs:).
    ; No DS change needed — use cs: prefix for every BIOS variable read.

    ; Caller's DL must match our El Torito drive.
    cmp dl, [cs:el_torito_drive]
    jne .gcds_fail_pop

    cmp byte [cs:el_torito_present], 0
    je .gcds_fail_pop

    ; Fill the 19-byte specification packet at ES:SI (caller's DS:SI).
    mov byte [es:si + 0],  0x13    ; Packet size
    mov al, [cs:el_torito_emul]
    mov [es:si + 1], al            ; Boot media type
    mov al, [cs:el_torito_drive]
    mov [es:si + 2], al            ; Drive number
    mov byte [es:si + 3],  0x00    ; Controller index

    mov eax, [cs:el_torito_rba]
    mov [es:si + 4], eax           ; Load RBA (CD-sector units)

    mov word [es:si + 8],  0x0000  ; Device specification
    mov word [es:si + 10], 0x0000  ; User buffer segment

    mov ax, [cs:el_torito_load_seg]
    mov [es:si + 12], ax           ; Load segment

    mov ax, [cs:el_torito_count]
    mov [es:si + 14], ax           ; Sector count

    mov word [es:si + 16], 0x0000  ; CHS (unused for no-emulation)
    mov byte [es:si + 18], 0x00    ; Reserved

    xor ah, ah
    clc
    mov byte [cs:int13_last_status], 0
    pop es
    iret

.gcds_fail_pop:
    pop es
.gcds_fail:
    mov byte [cs:int13_last_status], 0x01
    mov ah, 0x01
    stc
    iret

; ---------------------------------------------------------------------------
; INT 13h serial tracing helpers
; ---------------------------------------------------------------------------
; Input: BL=function (AH), BH=drive (DL)
int13_trace_enter:
    push ax
    push bx
    push dx

    mov ax, [cs:int13_trace_counter]
    cmp ax, INT13_TRACE_MAX
    jae .done
    inc ax
    mov [cs:int13_trace_counter], ax

    mov al, '['
    call int13_trace_putchar
    mov al, '1'
    call int13_trace_putchar
    mov al, '3'
    call int13_trace_putchar
    mov al, ' '
    call int13_trace_putchar
    mov al, 'A'
    call int13_trace_putchar
    mov al, 'H'
    call int13_trace_putchar
    mov al, '='
    call int13_trace_putchar
    mov al, bl
    call int13_trace_hex8
    mov al, ' '
    call int13_trace_putchar
    mov al, 'D'
    call int13_trace_putchar
    mov al, 'L'
    call int13_trace_putchar
    mov al, '='
    call int13_trace_putchar
    mov al, bh
    call int13_trace_hex8
    mov al, ']'
    call int13_trace_putchar
    mov al, 13
    call int13_trace_putchar
    mov al, 10
    call int13_trace_putchar

.done:
    pop dx
    pop bx
    pop ax
    ret

; Input: AL=value
int13_trace_hex8:
    push ax
    mov ah, al
    shr al, 4
    call int13_trace_nibble
    mov al, ah
    and al, 0x0F
    call int13_trace_nibble
    pop ax
    ret

; Input: AL=nibble (0..15)
int13_trace_nibble:
    cmp al, 10
    jb .digit
    add al, 'A' - 10
    jmp .out
.digit:
    add al, '0'
.out:
    call int13_trace_putchar
    ret

; Input: AL=char
int13_trace_putchar:
    push dx
    push ax

    ; COM1 serial output
    pop ax
    push ax
    call serial_putchar

    ; QEMU/SeaBIOS-style debug port output (vmd also drains this)
    mov dx, 0x0402
    pop ax
    out dx, al

    pop dx
    ret

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

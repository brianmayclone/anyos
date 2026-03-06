; =============================================================================
; int13h.asm — INT 13h: Disk BIOS services
; =============================================================================

; Last INT 13h status (AH-style code). 0 = success.
int13_last_status: db 0
int13_last_ah:     db 0
int13_last_dl:     db 0
int13_call_count:  dw 0
int13_trace_counter: dw 0
INT13_TRACE_MAX     equ 65535

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
    ; Check that the appropriate drive is present.
    cmp dl, 0xE0
    jne .chs_check_master
    cmp byte [cs:ide_slave_present], 0
    je .chs_error
    jmp .chs_present_ok
.chs_check_master:
    cmp byte [cs:ide_master_present], 0
    je .chs_error
.chs_present_ok:

    ; Convert CHS to LBA: LBA = (C * H + h) * S + (s - 1)
    ; where H = total heads, S = sectors per track.
    ;
    ; NOTE: MUL clobbers EDX, so DH (head) and DL (drive) must be read
    ; from the pushed EDX on the stack ([esp+6]=DL, [esp+7]=DH) after
    ; any MUL instruction.
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
    cmp dl, 0xE0                ; DL still valid here (before MUL)
    jne .chs_geom_hdd
    mov ebx, 64
    jmp .chs_heads_ok
.chs_geom_hdd:
    movzx ebx, word [cs:ide_master_heads]
.chs_heads_ok:
    mul ebx                     ; EAX = C * H (EDX clobbered!)
    movzx ebx, byte [esp + 7]  ; DH (head) from pushed EDX
    add eax, ebx                ; EAX = C * H + h

    cmp byte [esp + 6], 0xE0   ; DL (drive) from pushed EDX
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

    ; Set drive select for ide_read_sectors.
    cmp byte [esp + 6], 0xE0   ; DL from pushed EDX
    jne .chs_sel_master
    mov byte [cs:ide_drive_sel], 0xF0  ; Slave
    jmp .chs_sel_done
.chs_sel_master:
    mov byte [cs:ide_drive_sel], 0xE0  ; Master
.chs_sel_done:

    ; Now read using LBA. Sector count was in AL on entry.
    ; We saved original regs, so recover sector count.
    ; Stack frame: push eax(4)+ebx(4)+ecx(4)+edx(4)+edi(4)+es(2) = 22 bytes.
    ; push es is 2 bytes in [BITS 16] mode, so offsets are:
    ;   [esp+0]=ES, [esp+2]=EDI, [esp+6]=EDX, [esp+10]=ECX,
    ;   [esp+14]=EBX, [esp+18]=EAX.
    pop es
    push es
    mov ecx, [esp + 18]         ; Original EAX (AL = count)
    movzx ecx, cl               ; ECX = sector count
    ; EAX = LBA, ECX = count, ES:BX was set by caller.
    ; Recover original BX from stack.
    mov edi, [esp + 14]         ; Original EBX
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
    cmp byte [cs:ide_slave_present], 0
    je .gdp_no_drive

    ; Synthetic CHS for CD no-emulation compatibility.
    ; Heads = 64, SPT = 32, cylinders derived from total sectors.
    push bx
    mov eax, [cs:ide_slave_lba28]
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
    je .gdt_hdd
    cmp dl, 0xE0
    je .gdt_cd
    jmp .gdt_none

.gdt_hdd:
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

.gdt_cd:
    cmp byte [cs:ide_slave_present], 0
    je .gdt_none

    mov ah, 0x03                ; Type 3 = hard disk (CD presented as disk)
    ; CX:DX = total sectors.
    mov eax, [cs:ide_slave_lba28]
    mov cx, ax
    shr eax, 16
    mov dx, ax
    xchg cx, dx
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
;   Supported drives: 0x80 (HDD master) and 0xE0 (CD-ROM slave).
; ---------------------------------------------------------------------------
.check_extensions:
    ; Accept 0x80 (HDD) and 0xE0 (virtual CD) only.
    cmp dl, 0x80
    je .chk_ext_master
    cmp dl, 0xE0
    je .chk_ext_slave
    jmp .chk_ext_fail

.chk_ext_master:
    cmp byte [cs:ide_master_present], 0
    je .chk_ext_fail
    jmp .chk_ext_proceed

.chk_ext_slave:
    cmp byte [cs:ide_slave_present], 0
    je .chk_ext_fail

.chk_ext_proceed:
    cmp bx, 0x55AA
    jne .chk_ext_fail

    mov bx, 0xAA55
    mov ah, 0x30                ; Version 3.0
    mov cx, 0x0001              ; Extended disk access supported
    mov byte [cs:int13_last_status], 0
    clc
    iret

.chk_ext_fail:
    ; Trace: 41h failed for this drive
    push ax
    mov al, '4'
    call int13_trace_putchar
    mov al, '1'
    call int13_trace_putchar
    mov al, 'F'
    call int13_trace_putchar
    mov al, 10
    call int13_trace_putchar
    pop ax

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
;   For DL=0x80 (HDD master): DAP LBA and count are in 512-byte sectors.
;   For DL=0xE0 (virtual CD-ROM slave): the DAP LBA and count are in
;   2048-byte CD sector units (matching the 2048-byte sector size reported
;   by AH=48h). The code below converts them to 512-byte ATA sectors (x4)
;   before calling ide_read_sectors.
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

    cmp dl, 0x80
    je .ext_read_check_master
    cmp dl, 0xE0
    je .ext_read_check_slave
    jmp .ext_read_fail_pop

.ext_read_check_master:
    cmp byte [cs:ide_master_present], 0
    je .ext_read_fail_pop
    jmp .ext_read_hdd

.ext_read_check_slave:
    cmp byte [cs:ide_slave_present], 0
    je .ext_read_slave_missing
    jmp .ext_read_cd

.ext_read_slave_missing:
    ; Trace: slave not present for AH=42h
    push ax
    mov al, '!'
    call int13_trace_putchar
    mov al, 'S'
    call int13_trace_putchar
    mov al, 'L'
    call int13_trace_putchar
    mov al, 10
    call int13_trace_putchar
    pop ax
    jmp .ext_read_fail_pop

.ext_read_hdd:
    ; Read DAP fields via caller's DS (saved in BX), using ES as proxy segment.
    mov byte [cs:ide_drive_sel], 0xE0  ; Master
    push bx
    pop es                          ; ES = caller's DS
    mov ax, [es:si + 4]
    cmp ax, 0xFFFF
    jne .ext_read_hdd_seg
    mov ax, [es:si + 6]
    cmp ax, 0xFFFF
    jne .ext_read_hdd_seg
    cmp byte [es:si + 0], 0x18
    jb .ext_read_hdd_seg
    movzx ecx, word [es:si + 2]    ; Sector count (512-byte sectors)
    mov eax, [es:si + 8]           ; LBA (512-byte sector units)
    mov edi, [es:si + 16]          ; Flat destination address (low dword)
    mov edx, [es:si + 20]          ; High dword must be zero
    test edx, edx
    jnz .ext_read_fail_pop
    xor bx, bx
    mov es, bx                      ; ES = 0 for flat ES:EDI addressing
    call ide_read_sectors_flat
    jc .ext_read_fail_pop
    jmp .ext_read_ok
.ext_read_hdd_seg:
    movzx ecx, word [es:si + 2]    ; Sector count (512-byte sectors)
    mov di, [es:si + 4]            ; Buffer offset
    mov bx, [es:si + 6]            ; Buffer segment
    mov eax, [es:si + 8]           ; LBA (512-byte sector units)
    mov es, bx                      ; ES = buffer segment
    call ide_read_sectors
    jc .ext_read_fail_pop
    jmp .ext_read_ok

.ext_read_cd:
    ; CD-ROM: AH=48h reports 2048-byte CD sectors, so ISOLINUX (and other
    ; El Torito bootloaders) pass LBAs and counts in 2048-byte units.
    ; Convert to 512-byte ATA sectors for ide_read_sectors.
    mov byte [cs:ide_drive_sel], 0xF0  ; Slave
    push bx
    pop es                          ; ES = caller's DS
    mov ax, [es:si + 4]
    cmp ax, 0xFFFF
    jne .ext_read_cd_seg
    mov ax, [es:si + 6]
    cmp ax, 0xFFFF
    jne .ext_read_cd_seg
    cmp byte [es:si + 0], 0x18
    jb .ext_read_cd_seg
    movzx ecx, word [es:si + 2]    ; Sector count (2048-byte CD sectors)
    shl ecx, 2                     ; × 4 → 512-byte ATA sectors
    mov eax, [es:si + 8]           ; LBA (2048-byte CD sector units)
    shl eax, 2                     ; × 4 → ATA LBA (512-byte units)
    mov edi, [es:si + 16]          ; Flat destination address (low dword)
    mov edx, [es:si + 20]          ; High dword must be zero
    test edx, edx
    jnz .ext_read_fail_pop
    xor bx, bx
    mov es, bx                      ; ES = 0 for flat ES:EDI addressing
    call ide_read_sectors_flat
    jc .ext_read_fail_pop
    jmp .ext_read_ok
.ext_read_cd_seg:
    movzx ecx, word [es:si + 2]    ; Sector count (2048-byte CD sectors)
    shl ecx, 2                     ; × 4 → 512-byte ATA sectors
    mov di, [es:si + 4]            ; Buffer offset
    mov bx, [es:si + 6]            ; Buffer segment
    mov eax, [es:si + 8]           ; LBA (2048-byte CD sector units)
    shl eax, 2                     ; × 4 → ATA LBA (512-byte units)

    mov es, bx                      ; ES = buffer segment
    call ide_read_sectors
    jc .ext_read_fail_pop

.ext_read_ok:
    ; Trace success
    push ax
    mov al, '4'
    call int13_trace_putchar
    mov al, '2'
    call int13_trace_putchar
    mov al, 'o'
    call int13_trace_putchar
    mov al, 'k'
    call int13_trace_putchar
    mov al, 10
    call int13_trace_putchar
    pop ax

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
    ; Trace failure
    push ax
    mov al, '4'
    call int13_trace_putchar
    mov al, '2'
    call int13_trace_putchar
    mov al, 'E'
    call int13_trace_putchar
    mov al, 'R'
    call int13_trace_putchar
    mov al, 'R'
    call int13_trace_putchar
    mov al, 10
    call int13_trace_putchar
    pop ax

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
    mov byte [cs:ide_drive_sel], 0xE0  ; Master
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
    cmp dl, 0x80
    je .gep_hdd
    cmp dl, 0xE0
    je .gep_cd
    jmp .gep_fail

.gep_hdd:
    cmp byte [cs:ide_master_present], 0
    je .gep_fail

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
    cmp byte [cs:ide_slave_present], 0
    je .gep_fail

    ; Virtual CD-ROM: report 2048-byte sectors.
    ; Total capacity in 2048-byte sectors = total_ata_sectors / 4.
    mov word [si + 0], 26       ; Size of result
    mov word [si + 2], 0x0000   ; Flags: no CHS (CD-ROM)
    mov dword [si + 4],  0      ; Cylinders (N/A)
    mov dword [si + 8],  0      ; Heads (N/A)
    mov dword [si + 12], 0      ; Sectors per track (N/A)

    ; Total ISO sectors = ATA sectors / 4.
    mov eax, [cs:ide_slave_lba28]
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
    ; Trace: 48h failed
    push ax
    mov al, '4'
    call int13_trace_putchar
    mov al, '8'
    call int13_trace_putchar
    mov al, 'F'
    call int13_trace_putchar
    mov al, 10
    call int13_trace_putchar
    pop ax

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

; Input: AL=char (disabled for performance — immediate return)
int13_trace_putchar:
    ret

; =============================================================================
; IDE PIO helpers
; =============================================================================

; ---------------------------------------------------------------------------
; ide_read_sectors — Read sectors from IDE using PIO LBA28.
;   Input:  EAX = start LBA, ECX = sector count, ES:DI = buffer
;           cs:ide_drive_sel = 0xE0 (master) or 0xF0 (slave)
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
    or dl, [cs:ide_drive_sel]   ; Master (0xE0) or Slave (0xF0)
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
    ; Save EAX (LBA) — the command byte and status polling clobber AL.
    push eax
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
    pop eax
    jmp .read_error

.read_err:
    pop cx
    pop eax
    jmp .read_error

.read_ready:
    pop cx

    ; Read 256 words (512 bytes).
    push di
    push cx
    push dx
    mov dx, IDE_DATA
    mov cx, 256
    rep insw
    pop dx
    pop cx
    pop bx
    cmp di, bx
    jae .read_no_wrap
    ; DI wrapped across 64 KiB boundary: advance ES by 0x1000 paragraphs
    ; so linear destination still advances by +0x10000 bytes.
    mov bx, es
    add bx, 0x1000
    mov es, bx
.read_no_wrap:

    pop eax                     ; Restore EAX (LBA) before incrementing
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
; ide_read_sectors_flat — Read sectors from IDE using PIO LBA28.
;   Input:  EAX = start LBA, ECX = sector count, ES = 0, EDI = flat buffer
;           cs:ide_drive_sel = 0xE0 (master) or 0xF0 (slave)
;   Output: CF set on error
; ---------------------------------------------------------------------------
ide_read_sectors_flat:
    push eax
    push ebx
    push ecx
    push edx
    push edi

.read_loop_flat:
    test ecx, ecx
    jz .read_done_flat

    ; Select drive/LBA bits 24-27.
    push eax
    mov edx, eax
    shr edx, 24
    and dl, 0x0F
    or dl, [cs:ide_drive_sel]
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
    push eax
    mov dx, IDE_CMD
    mov al, IDE_CMD_READ_SECTORS
    out dx, al

    ; Wait for DRQ.
    mov dx, IDE_STATUS
    push cx
    mov cx, 0xFFFF
.read_wait_flat:
    in al, dx
    test al, IDE_SR_BSY
    jnz .read_wait_cont_flat
    test al, IDE_SR_DRQ
    jnz .read_ready_flat
    test al, IDE_SR_ERR
    jnz .read_err_flat
.read_wait_cont_flat:
    dec cx
    jnz .read_wait_flat
    pop cx
    pop eax
    jmp .read_error_flat

.read_err_flat:
    pop cx
    pop eax
    jmp .read_error_flat

.read_ready_flat:
    pop cx

    ; Read 256 words (512 bytes) to flat ES:EDI.
    push ecx
    push dx
    mov dx, IDE_DATA
    mov ecx, 256
    a32 rep insw
    pop dx
    pop ecx

    pop eax
    inc eax
    dec ecx
    jmp .read_loop_flat

.read_done_flat:
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
    clc
    ret

.read_error_flat:
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
    stc
    ret

; ---------------------------------------------------------------------------
; ide_write_sectors — Write sectors to IDE using PIO LBA28.
;   Input:  EAX = start LBA, ECX = sector count, ES:DI = buffer
;           cs:ide_drive_sel = 0xE0 (master) or 0xF0 (slave)
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
    or dl, [cs:ide_drive_sel]   ; Master (0xE0) or Slave (0xF0)
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
    ; Save EAX (LBA) — the command byte, status polling, and flush clobber AL.
    push eax
    mov dx, IDE_CMD
    mov al, 0x30                ; WRITE SECTORS
    out dx, al

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
    pop eax
    jmp .write_error
.write_err:
    pop cx
    pop eax
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

    pop eax                     ; Restore EAX (LBA) before incrementing
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

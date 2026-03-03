; =============================================================================
; int1ah.asm — INT 1Ah: PCI BIOS services + RTC time/date
; =============================================================================

CMOS_ADDR   equ 0x70
CMOS_DATA   equ 0x71

int1ah_handler:
    cmp ah, 0x00
    je .get_tick_count
    cmp ah, 0x01
    je .set_tick_count
    cmp ah, 0x02
    je .get_rtc_time
    cmp ah, 0x03
    je .set_rtc_time
    cmp ah, 0x04
    je .get_rtc_date
    cmp ah, 0x05
    je .set_rtc_date
    cmp ah, 0xB1
    je .pci_bios
    ; Unsupported.
    stc
    iret

; ---------------------------------------------------------------------------
; AH=00h: Get system timer tick count.
;   Returns: CX:DX = tick count, AL = midnight flag.
; ---------------------------------------------------------------------------
.get_tick_count:
    push ds
    xor ax, ax
    mov ds, ax
    mov dx, [BDA_TIMER_COUNT]
    mov cx, [BDA_TIMER_COUNT + 2]
    mov al, [BDA_TIMER_OVERFLOW]
    mov byte [BDA_TIMER_OVERFLOW], 0
    pop ds
    clc
    iret

; ---------------------------------------------------------------------------
; AH=01h: Set system timer tick count.
;   CX:DX = new tick count.
; ---------------------------------------------------------------------------
.set_tick_count:
    push ds
    xor ax, ax
    mov ds, ax
    mov [BDA_TIMER_COUNT], dx
    mov [BDA_TIMER_COUNT + 2], cx
    mov byte [BDA_TIMER_OVERFLOW], 0
    pop ds
    clc
    iret

; ---------------------------------------------------------------------------
; AH=02h: Get RTC time.
;   Returns: CH = hours (BCD), CL = minutes (BCD), DH = seconds (BCD).
; ---------------------------------------------------------------------------
.get_rtc_time:
    ; Read CMOS status register B to check BCD vs binary mode.
    mov al, 0x0B
    out CMOS_ADDR, al
    in al, CMOS_DATA
    push ax                     ; Save status B

    ; Read seconds.
    mov al, 0x00
    out CMOS_ADDR, al
    in al, CMOS_DATA
    mov dh, al                  ; Seconds

    ; Read minutes.
    mov al, 0x02
    out CMOS_ADDR, al
    in al, CMOS_DATA
    mov cl, al                  ; Minutes

    ; Read hours.
    mov al, 0x04
    out CMOS_ADDR, al
    in al, CMOS_DATA
    mov ch, al                  ; Hours

    ; Convert binary to BCD if CMOS is in binary mode (bit 2 of status B).
    pop ax
    test al, 0x04
    jz .rtc_time_done           ; Already BCD

    ; Convert binary to BCD.
    mov al, dh
    call .bin_to_bcd
    mov dh, al
    mov al, cl
    call .bin_to_bcd
    mov cl, al
    mov al, ch
    call .bin_to_bcd
    mov ch, al

.rtc_time_done:
    mov dl, 0                   ; Daylight saving: off
    xor ah, ah
    clc
    iret

; ---------------------------------------------------------------------------
; AH=03h: Set RTC time.
; ---------------------------------------------------------------------------
.set_rtc_time:
    ; Not implemented for now.
    clc
    iret

; ---------------------------------------------------------------------------
; AH=04h: Get RTC date.
;   Returns: CH = century (BCD), CL = year (BCD), DH = month (BCD),
;            DL = day (BCD).
; ---------------------------------------------------------------------------
.get_rtc_date:
    ; Read CMOS status B.
    mov al, 0x0B
    out CMOS_ADDR, al
    in al, CMOS_DATA
    push ax

    ; Day of month.
    mov al, 0x07
    out CMOS_ADDR, al
    in al, CMOS_DATA
    mov dl, al

    ; Month.
    mov al, 0x08
    out CMOS_ADDR, al
    in al, CMOS_DATA
    mov dh, al

    ; Year (2-digit).
    mov al, 0x09
    out CMOS_ADDR, al
    in al, CMOS_DATA
    mov cl, al

    ; Century (reg 0x32, if available — default to 0x20).
    mov al, 0x32
    out CMOS_ADDR, al
    in al, CMOS_DATA
    test al, al
    jnz .have_century
    mov al, 0x20                ; Default: 20xx
.have_century:
    mov ch, al

    ; Convert if binary mode.
    pop ax
    test al, 0x04
    jz .rtc_date_done

    mov al, dl
    call .bin_to_bcd
    mov dl, al
    mov al, dh
    call .bin_to_bcd
    mov dh, al
    mov al, cl
    call .bin_to_bcd
    mov cl, al
    mov al, ch
    call .bin_to_bcd
    mov ch, al

.rtc_date_done:
    xor ah, ah
    clc
    iret

; ---------------------------------------------------------------------------
; AH=05h: Set RTC date.
; ---------------------------------------------------------------------------
.set_rtc_date:
    clc
    iret

; ---------------------------------------------------------------------------
; Binary to BCD conversion. AL = binary value, returns AL = BCD.
; ---------------------------------------------------------------------------
.bin_to_bcd:
    push cx
    xor ah, ah
    mov cl, 10
    div cl                      ; AL = tens, AH = ones
    shl al, 4
    or al, ah
    pop cx
    ret

; =============================================================================
; PCI BIOS services (AH=B1h)
; =============================================================================

.pci_bios:
    cmp al, 0x01
    je .pci_present
    cmp al, 0x02
    je .pci_find_device
    cmp al, 0x03
    je .pci_find_class
    cmp al, 0x08
    je .pci_read_byte
    cmp al, 0x09
    je .pci_read_word
    cmp al, 0x0A
    je .pci_read_dword
    cmp al, 0x0B
    je .pci_write_byte
    cmp al, 0x0C
    je .pci_write_word
    cmp al, 0x0D
    je .pci_write_dword
    ; Unknown PCI function.
    mov ah, 0x81                ; Function not supported
    stc
    iret

; ---------------------------------------------------------------------------
; AX=B101h: PCI BIOS present check.
; ---------------------------------------------------------------------------
.pci_present:
    mov edx, 0x20494350        ; "PCI "
    mov ah, 0x00                ; Success
    mov bh, 0x02                ; PCI revision 2.1 major
    mov bl, 0x10                ; PCI revision 2.1 minor
    mov cl, 0x00                ; Max bus number
    mov al, 0x01                ; Hardware mechanism 1
    clc
    iret

; ---------------------------------------------------------------------------
; AX=B102h: Find PCI device by vendor/device ID.
;   CX = device ID, DX = vendor ID, SI = index (0 = first match).
;   Returns: BH = bus, BL = dev/func.
; ---------------------------------------------------------------------------
.pci_find_device:
    push ds
    push di
    push ax

    mov ax, 0xF000
    mov ds, ax

    mov di, [pci_device_count]
    test di, di
    jz .pci_fd_notfound

    push si
    mov bx, pci_device_table
    xor ax, ax                  ; Match index counter

.pci_fd_loop:
    cmp ax, di
    jge .pci_fd_notfound_pop

    ; Check vendor ID at offset 4.
    cmp dx, [bx + 4]
    jne .pci_fd_next
    ; Check device ID at offset 6.
    cmp cx, [bx + 6]
    jne .pci_fd_next

    ; Match found — check index.
    cmp si, 0
    je .pci_fd_found
    dec si
    jmp .pci_fd_next

.pci_fd_next:
    add bx, PCI_ENTRY_SIZE
    inc ax
    jmp .pci_fd_loop

.pci_fd_found:
    pop si
    ; Build BH = bus, BL = (dev << 3) | func.
    mov bh, [bx + 0]           ; Bus
    mov al, [bx + 1]           ; Device
    shl al, 3
    or al, [bx + 2]            ; Function
    mov bl, al

    pop ax
    pop di
    pop ds
    mov ah, 0x00
    clc
    iret

.pci_fd_notfound_pop:
    pop si
.pci_fd_notfound:
    pop ax
    pop di
    pop ds
    mov ah, 0x86                ; Device not found
    stc
    iret

; ---------------------------------------------------------------------------
; AX=B103h: Find PCI device by class code.
;   ECX = class code (class:subclass:progif), SI = index.
;   Returns: BH = bus, BL = dev/func.
; ---------------------------------------------------------------------------
.pci_find_class:
    push ds
    push di
    push ax

    mov ax, 0xF000
    mov ds, ax

    mov di, [pci_device_count]
    test di, di
    jz .pci_fc_notfound

    push si
    mov bx, pci_device_table
    xor ax, ax

.pci_fc_loop:
    cmp ax, di
    jge .pci_fc_notfound_pop

    ; Build class code from table: class(10) : subclass(9) : progif(8).
    movzx edx, byte [bx + 10]  ; Class
    shl edx, 8
    mov dl, [bx + 9]           ; Subclass
    shl edx, 8
    mov dl, [bx + 8]           ; Prog IF

    cmp edx, ecx
    jne .pci_fc_next

    cmp si, 0
    je .pci_fc_found
    dec si

.pci_fc_next:
    add bx, PCI_ENTRY_SIZE
    inc ax
    jmp .pci_fc_loop

.pci_fc_found:
    pop si
    mov bh, [bx + 0]
    mov al, [bx + 1]
    shl al, 3
    or al, [bx + 2]
    mov bl, al

    pop ax
    pop di
    pop ds
    mov ah, 0x00
    clc
    iret

.pci_fc_notfound_pop:
    pop si
.pci_fc_notfound:
    pop ax
    pop di
    pop ds
    mov ah, 0x86
    stc
    iret

; ---------------------------------------------------------------------------
; AX=B108h: Read PCI config byte.
;   BH = bus, BL = dev/func, DI = register.
;   Returns: CL = byte value.
; ---------------------------------------------------------------------------
.pci_read_byte:
    push eax
    push edx

    call .pci_build_addr
    mov dx, PCI_CONFIG_ADDR
    out dx, eax

    ; Read dword and extract byte.
    mov dx, PCI_CONFIG_DATA
    in eax, dx
    mov ecx, edi
    and cl, 3                   ; Byte offset within dword
    shl cl, 3                   ; Bit shift
    shr eax, cl
    mov cl, al

    pop edx
    pop eax
    mov ah, 0x00
    clc
    iret

; ---------------------------------------------------------------------------
; AX=B109h: Read PCI config word.
;   BH = bus, BL = dev/func, DI = register.
;   Returns: CX = word value.
; ---------------------------------------------------------------------------
.pci_read_word:
    push eax
    push edx

    call .pci_build_addr
    mov dx, PCI_CONFIG_ADDR
    out dx, eax

    mov dx, PCI_CONFIG_DATA
    in eax, dx
    test di, 2
    jz .rw_low
    shr eax, 16
.rw_low:
    mov cx, ax

    pop edx
    pop eax
    mov ah, 0x00
    clc
    iret

; ---------------------------------------------------------------------------
; AX=B10Ah: Read PCI config dword.
;   BH = bus, BL = dev/func, DI = register.
;   Returns: ECX = dword value.
; ---------------------------------------------------------------------------
.pci_read_dword:
    push eax
    push edx

    call .pci_build_addr
    mov dx, PCI_CONFIG_ADDR
    out dx, eax

    mov dx, PCI_CONFIG_DATA
    in eax, dx
    mov ecx, eax

    pop edx
    pop eax
    mov ah, 0x00
    clc
    iret

; ---------------------------------------------------------------------------
; AX=B10Bh: Write PCI config byte.
;   BH = bus, BL = dev/func, DI = register, CL = byte value.
; ---------------------------------------------------------------------------
.pci_write_byte:
    push eax
    push edx

    call .pci_build_addr
    mov dx, PCI_CONFIG_ADDR
    out dx, eax

    ; Read-modify-write.
    mov dx, PCI_CONFIG_DATA
    in eax, dx
    push ecx
    mov ecx, edi
    and cl, 3
    shl cl, 3
    mov edx, 0xFF
    shl edx, cl
    not edx
    and eax, edx               ; Clear target byte
    pop ecx
    push ecx
    movzx edx, cl
    mov ecx, edi
    and cl, 3
    shl cl, 3
    shl edx, cl
    or eax, edx                ; Insert new byte
    pop ecx

    mov dx, PCI_CONFIG_DATA
    out dx, eax

    pop edx
    pop eax
    mov ah, 0x00
    clc
    iret

; ---------------------------------------------------------------------------
; AX=B10Ch: Write PCI config word.
;   BH = bus, BL = dev/func, DI = register, CX = word value.
; ---------------------------------------------------------------------------
.pci_write_word:
    push eax
    push edx

    call .pci_build_addr
    mov dx, PCI_CONFIG_ADDR
    out dx, eax

    mov dx, PCI_CONFIG_DATA
    in eax, dx
    test di, 2
    jz .ww_low
    and eax, 0x0000FFFF
    movzx ecx, cx
    shl ecx, 16
    or eax, ecx
    jmp .ww_write
.ww_low:
    and eax, 0xFFFF0000
    movzx ecx, cx
    or eax, ecx
.ww_write:
    out dx, eax

    pop edx
    pop eax
    mov ah, 0x00
    clc
    iret

; ---------------------------------------------------------------------------
; AX=B10Dh: Write PCI config dword.
;   BH = bus, BL = dev/func, DI = register, ECX = dword value.
; ---------------------------------------------------------------------------
.pci_write_dword:
    push eax
    push edx

    call .pci_build_addr
    mov dx, PCI_CONFIG_ADDR
    out dx, eax

    mov dx, PCI_CONFIG_DATA
    mov eax, ecx
    out dx, eax

    pop edx
    pop eax
    mov ah, 0x00
    clc
    iret

; ---------------------------------------------------------------------------
; Helper: Build PCI config address from BH=bus, BL=dev/func, DI=register.
;   Returns: EAX = config address with enable bit.
; ---------------------------------------------------------------------------
.pci_build_addr:
    movzx eax, bh              ; Bus
    shl eax, 16
    movzx ecx, bl
    shl ecx, 8                 ; Dev/func already in [7:3]/[2:0] format
    or eax, ecx
    movzx ecx, di
    and cx, 0x00FC
    or eax, ecx
    or eax, 0x80000000
    ret

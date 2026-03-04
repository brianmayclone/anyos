; =============================================================================
; pci.asm — PCI bus enumeration via Type 1 configuration (CF8/CFC)
; =============================================================================

PCI_CONFIG_ADDR equ 0x0CF8
PCI_CONFIG_DATA equ 0x0CFC

; PCI device table (filled during enumeration).
; Each entry: bus(1) + dev(1) + func(1) + pad(1) + vendor(2) + device(2)
;           + class(1) + subclass(1) + progif(1) + hdrtype(1) = 12 bytes
PCI_TABLE_MAX   equ 16
PCI_ENTRY_SIZE  equ 12

pci_device_count:   dw 0
pci_device_table:   times (PCI_TABLE_MAX * PCI_ENTRY_SIZE) db 0

; ---------------------------------------------------------------------------
; pci_config_read32 — Read a 32-bit PCI config register.
;   Input:  EAX = config address (with enable bit set)
;   Output: EAX = 32-bit value
; ---------------------------------------------------------------------------
pci_config_read32:
    push dx
    mov dx, PCI_CONFIG_ADDR
    out dx, eax
    mov dx, PCI_CONFIG_DATA
    in eax, dx
    pop dx
    ret

; ---------------------------------------------------------------------------
; pci_config_write32 — Write a 32-bit PCI config register.
;   Input:  EAX = config address, ECX = value to write
; ---------------------------------------------------------------------------
pci_config_write32:
    push dx
    mov dx, PCI_CONFIG_ADDR
    out dx, eax
    mov dx, PCI_CONFIG_DATA
    mov eax, ecx
    out dx, eax
    pop dx
    ret

; ---------------------------------------------------------------------------
; pci_make_addr — Build a PCI config address.
;   Input:  BH = bus, BL[7:3] = device, BL[2:0] = function, DI = register
;   Output: EAX = config address with enable bit
; ---------------------------------------------------------------------------
pci_make_addr:
    push ecx
    movzx eax, bh              ; Bus
    shl eax, 16
    movzx ecx, bl
    shr cl, 3
    and cl, 0x1F               ; Device (5 bits)
    shl ecx, 11
    or eax, ecx
    movzx ecx, bl
    and cl, 0x07               ; Function (3 bits)
    shl ecx, 8
    or eax, ecx
    movzx ecx, di
    and cx, 0x00FC             ; Register (aligned)
    or eax, ecx
    or eax, 0x80000000         ; Enable bit
    pop ecx
    ret

; ---------------------------------------------------------------------------
; pci_enumerate — Scan PCI bus 0 and fill pci_device_table.
;   Output: [pci_device_count] = number of devices found
; ---------------------------------------------------------------------------
pci_enumerate:
    push eax
    push ebx
    push ecx
    push edx
    push edi
    push si
    push ds
    push cs
    pop ds                          ; DS = CS = 0xF000 for BIOS variable writes

    mov si, pci_device_table
    xor cx, cx                  ; Device count = 0
    xor bh, bh                 ; Bus 0

    mov byte [.cur_dev], 0
.dev_loop:
    cmp byte [.cur_dev], 32
    jge .done

    ; Build BL = (device << 3) | function 0.
    mov al, [.cur_dev]
    shl al, 3
    mov bl, al                  ; BL = dev<<3 | func=0

    ; Read vendor/device ID (register 0x00).
    xor di, di
    call pci_make_addr
    call pci_config_read32
    cmp ax, 0xFFFF              ; No device?
    je .next_dev
    cmp ax, 0x0000              ; Invalid?
    je .next_dev

    ; Device found. Check if multi-function.
    push eax                    ; Save vendor/device
    mov di, 0x0C                ; Register 0x0C (header type at byte 0x0E)
    call pci_make_addr
    call pci_config_read32
    shr eax, 16                 ; AL = header type, AH = BIST
    mov dl, al                  ; DL = header type
    pop eax                     ; Restore vendor/device

    ; Enumerate functions.
    mov byte [.cur_func], 0
    test dl, 0x80               ; Multi-function?
    jnz .func_loop
    ; Single function: just store function 0.
    mov byte [.max_func], 1
    jmp .func_loop
.multi_func:
    mov byte [.max_func], 8
.func_loop:
    cmp byte [.cur_func], 8
    jge .next_dev
    test dl, 0x80
    jz .check_max_func
    jmp .do_func
.check_max_func:
    cmp byte [.cur_func], 1
    jge .next_dev

.do_func:
    ; Build BL for this function.
    mov al, [.cur_dev]
    shl al, 3
    or al, [.cur_func]
    mov bl, al

    ; Read vendor/device ID.
    xor di, di
    call pci_make_addr
    call pci_config_read32
    cmp ax, 0xFFFF
    je .next_func
    cmp ax, 0x0000
    je .next_func

    ; Store entry if table not full.
    cmp cx, PCI_TABLE_MAX
    jge .next_func

    push eax
    ; Store bus, dev, func.
    mov byte [si + 0], bh      ; Bus
    mov al, [.cur_dev]
    mov byte [si + 1], al      ; Device
    mov al, [.cur_func]
    mov byte [si + 2], al      ; Function
    mov byte [si + 3], 0       ; Padding
    pop eax
    ; Store vendor/device ID.
    mov [si + 4], ax            ; Vendor ID (low 16)
    shr eax, 16
    mov [si + 6], ax            ; Device ID (high 16)

    ; Read class code (register 0x08).
    push ecx
    mov di, 0x08
    call pci_make_addr
    call pci_config_read32
    mov [si + 8], al            ; Revision ID -> we store class info
    shr eax, 8
    mov [si + 8], al            ; Prog IF
    shr eax, 8
    mov [si + 9], al            ; Subclass
    shr eax, 8
    mov [si + 10], al           ; Class code
    pop ecx

    ; Read header type.
    push ecx
    mov di, 0x0C
    call pci_make_addr
    call pci_config_read32
    shr eax, 16
    mov [si + 11], al           ; Header type
    pop ecx

    add si, PCI_ENTRY_SIZE
    inc cx

.next_func:
    inc byte [.cur_func]
    jmp .func_loop

.next_dev:
    inc byte [.cur_dev]
    jmp .dev_loop

.done:
    mov [pci_device_count], cx

    pop ds
    pop si
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
    ret

; Local variables (in BIOS ROM area — read-only issue, use stack or BDA instead).
; We place these as mutable data in the BIOS image; since the BIOS is loaded
; into RAM (not actual ROM), writes work fine.
.cur_dev:   db 0
.cur_func:  db 0
.max_func:  db 0

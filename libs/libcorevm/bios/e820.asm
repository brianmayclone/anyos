; =============================================================================
; e820.asm — E820 memory map table (built during POST)
; =============================================================================

; Each E820 entry: base(8) + length(8) + type(4) + acpi(4) = 24 bytes.
; We support up to 8 entries.
E820_MAX_ENTRIES    equ 8
E820_ENTRY_SIZE     equ 24

e820_count:         dw 0
e820_table:         times (E820_MAX_ENTRIES * E820_ENTRY_SIZE) db 0

; RAM size in bytes (filled by memory_detect).
ram_size_bytes:     dd 0, 0         ; 64-bit value (low, high)

; ---------------------------------------------------------------------------
; memory_detect — Detect memory size from CMOS and build the E820 table.
;
; The CMOS device in libcorevm pre-populates:
;   Reg 0x17/0x18 = extended memory in KB (1MB-65MB range, capped at 0xFFFF)
;   Reg 0x34/0x35 = memory above 16MB in 64KB units
; ---------------------------------------------------------------------------
memory_detect:
    push es
    push di
    push eax
    push ebx
    push ecx
    push edx
    push ds
    push cs
    pop ds                          ; DS = CS = 0xF000 for BIOS variable writes

    ; Read extended memory (1MB-16MB) from CMOS 0x17/0x18.
    mov al, 0x17
    out 0x70, al
    in al, 0x71
    movzx bx, al               ; Low byte
    mov al, 0x18
    out 0x70, al
    in al, 0x71
    mov ah, al
    mov al, bl                  ; AX = extended mem in KB (up to 0xFFFF = 65535 KB)
    movzx eax, ax
    shl eax, 10                 ; Convert KB to bytes
    mov edx, eax               ; EDX = extended memory bytes (below 16MB portion)

    ; Read memory above 16MB from CMOS 0x34/0x35 (in 64KB units).
    push edx
    mov al, 0x34
    out 0x70, al
    in al, 0x71
    movzx bx, al
    mov al, 0x35
    out 0x70, al
    in al, 0x71
    mov ah, al
    mov al, bl                  ; AX = 64KB units above 16MB
    movzx eax, ax
    shl eax, 16                 ; Convert 64KB units to bytes
    pop edx

    ; Total RAM = 1MB + extended + above_16MB.
    add eax, edx               ; EAX = extended + above_16MB bytes
    add eax, 0x100000          ; Add 1MB base
    mov [ram_size_bytes], eax
    mov dword [ram_size_bytes + 4], 0

    ; Now build the E820 table.
    mov di, e820_table
    xor cx, cx                  ; Entry count

    ; Entry 0: 0x00000 - 0x9FBFF = usable (639 KB conventional).
    mov dword [di + 0], 0x00000000      ; Base low
    mov dword [di + 4], 0x00000000      ; Base high
    mov dword [di + 8], 0x0009FC00      ; Length low (639 KB)
    mov dword [di + 12], 0x00000000     ; Length high
    mov dword [di + 16], 1              ; Type: usable
    mov dword [di + 20], 0              ; ACPI extended
    add di, E820_ENTRY_SIZE
    inc cx

    ; Entry 1: 0x9FC00 - 0x9FFFF = reserved (EBDA, 1 KB).
    mov dword [di + 0], 0x0009FC00
    mov dword [di + 4], 0x00000000
    mov dword [di + 8], 0x00000400      ; 1 KB
    mov dword [di + 12], 0x00000000
    mov dword [di + 16], 2              ; Type: reserved
    mov dword [di + 20], 0
    add di, E820_ENTRY_SIZE
    inc cx

    ; Entry 2: 0xE0000 - 0xFFFFF = reserved (BIOS ROM area).
    mov dword [di + 0], 0x000E0000
    mov dword [di + 4], 0x00000000
    mov dword [di + 8], 0x00020000      ; 128 KB
    mov dword [di + 12], 0x00000000
    mov dword [di + 16], 2              ; Type: reserved
    mov dword [di + 20], 0
    add di, E820_ENTRY_SIZE
    inc cx

    ; Entry 3: 0x100000 - top_of_ram = usable (extended memory).
    mov eax, [ram_size_bytes]
    sub eax, 0x100000                   ; Length = total - 1MB
    mov dword [di + 0], 0x00100000      ; Base: 1 MB
    mov dword [di + 4], 0x00000000
    mov dword [di + 8], eax             ; Length
    mov dword [di + 12], 0x00000000
    mov dword [di + 16], 1              ; Type: usable
    mov dword [di + 20], 0
    add di, E820_ENTRY_SIZE
    inc cx

    ; Entry 4: 0xB0000000 - 0xBFFFFFFF = reserved (PCI MMCONFIG, 256 MB).
    mov dword [di + 0], 0xB0000000
    mov dword [di + 4], 0x00000000
    mov dword [di + 8], 0x10000000      ; 256 MB
    mov dword [di + 12], 0x00000000
    mov dword [di + 16], 2              ; Type: reserved
    mov dword [di + 20], 0
    add di, E820_ENTRY_SIZE
    inc cx

    ; Entry 5: 0xFEC00000 - 0xFECFFFFF = reserved (IO-APIC, 64 KB).
    mov dword [di + 0], 0xFEC00000
    mov dword [di + 4], 0x00000000
    mov dword [di + 8], 0x00010000      ; 64 KB
    mov dword [di + 12], 0x00000000
    mov dword [di + 16], 2              ; Type: reserved
    mov dword [di + 20], 0
    add di, E820_ENTRY_SIZE
    inc cx

    mov [e820_count], cx

    pop ds
    pop edx
    pop ecx
    pop ebx
    pop eax
    pop di
    pop es
    ret

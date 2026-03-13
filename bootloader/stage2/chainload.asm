; =============================================================================
; chainload.asm - Boot entry execution
; =============================================================================

; execute_entry - Execute a boot menu entry
; Input: BX = entry index (0-based)
; For type=0 (kernel): copies params to BOOT_INFO_ADDR+44, returns
; For type=1 (chainload): chainloads, does not return
execute_entry:
    ; Calculate entry pointer: cfg_entries + BX * 128
    movzx eax, bx
    shl eax, 7                     ; * 128
    add eax, cfg_entries            ; ESI = pointer to entry

    ; Check entry type at offset 96
    mov esi, eax
    cmp byte [esi + 96], 0
    je .type_kernel
    cmp byte [esi + 96], 1
    je .type_chainload
    ret                             ; Unknown type, just return

.type_kernel:
    ; Copy 64 bytes of params from entry+32 to BOOT_INFO_ADDR+44
    add esi, 32                     ; Source: entry + 32
    mov edi, BOOT_INFO_ADDR + 44   ; Destination
    mov ecx, 64
    a32 rep movsb
    ret

.type_chainload:
    ; Get disk number (offset 97) and partition (offset 98)
    movzx edx, byte [esi + 97]     ; disk number
    add dl, 0x80                    ; Convert to BIOS drive number
    push dx                         ; Save drive number
    movzx ebx, byte [esi + 98]     ; partition number

    ; Modify kernel_load_dap buffer to point to 0x0000:0x7C00
    mov word [kernel_load_dap + 4], 0x7C00  ; offset
    mov word [kernel_load_dap + 6], 0x0000  ; segment
    mov word [kernel_load_dap + 2], 1       ; 1 sector
    mov dword [kernel_load_dap + 8], 0      ; LBA 0 (MBR)
    mov dword [kernel_load_dap + 12], 0     ; LBA high

    ; Read MBR (sector 0) of target disk to 0x7C00
    pop dx                          ; DL = BIOS drive
    push dx                         ; Save again
    push bx                         ; Save partition number
    mov ah, 0x42
    mov si, kernel_load_dap
    int 0x13
    jc .disk_error

    ; Find partition start LBA from MBR partition table
    pop bx                          ; partition number
    pop dx                          ; drive number
    push dx
    ; Partition entry = 0x7C00 + 0x1BE + partition * 16
    movzx eax, bx
    shl eax, 4                     ; * 16
    add eax, 0x7C00 + 0x1BE
    ; Start LBA at offset +8 within partition entry
    mov eax, [eax + 8]

    ; Load first sector of partition to 0x7C00
    mov dword [kernel_load_dap + 8], eax    ; LBA = partition start
    mov dword [kernel_load_dap + 12], 0
    pop dx                          ; DL = BIOS drive
    push dx                         ; Save for final setup
    mov ah, 0x42
    mov si, kernel_load_dap
    int 0x13
    jc .disk_error

    ; Reset video to text mode
    mov ax, 0x0003
    int 0x10

    ; Set up registers and jump to VBR
    pop dx                          ; DL = BIOS drive number
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00
    jmp 0x0000:0x7C00

.disk_error:
    ; Clean up stack
    add sp, 4                       ; Remove saved bx + dx (or just dx)
    ; Reset video to text mode
    mov ax, 0x0003
    int 0x10
    mov si, .msg_disk_error
    call print_string_16
.halt:
    cli
    hlt
    jmp .halt

.msg_disk_error: db "Chainload: disk read error", 13, 10, 0

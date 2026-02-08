; =============================================================================
; .anyOS Stage 1 Bootloader (MBR)
; =============================================================================
; Loaded by BIOS at 0x0000:0x7C00 (512 bytes)
; Task: Set up segments/stack, load Stage 2 from disk, jump to it
; =============================================================================

[BITS 16]
[ORG 0x7C00]

STAGE2_SEGMENT      equ 0x0000
STAGE2_OFFSET       equ 0x8000
STAGE2_SECTORS      equ 63
STAGE2_START_LBA    equ 1

start:
    ; Disable interrupts during setup
    cli

    ; Set up segment registers
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00          ; Stack grows down from 0x7C00

    ; Re-enable interrupts
    sti

    ; Save boot drive number (BIOS passes it in DL)
    mov [boot_drive], dl

    ; Load Stage 2 using INT 13h Extended Read (LBA addressing)
    mov si, dap
    mov dl, [boot_drive]
    mov ah, 0x42
    int 0x13
    jc .disk_error

    ; Jump to Stage 2
    mov dl, [boot_drive]    ; Pass boot drive to stage 2
    jmp STAGE2_SEGMENT:STAGE2_OFFSET

.disk_error:
    mov si, msg_disk_err
    call print_string
    cli
    hlt

; -----------------------------------------------------------------------------
; print_string - Print null-terminated string at DS:SI via BIOS
; -----------------------------------------------------------------------------
print_string:
    pusha
.loop:
    lodsb
    test al, al
    jz .done
    mov ah, 0x0E
    mov bx, 0x0007
    int 0x10
    jmp .loop
.done:
    popa
    ret

; =============================================================================
; Data
; =============================================================================
boot_drive:     db 0
msg_boot:       db ".anyOS Boot", 13, 10, 0
msg_disk_err:   db "Disk Error!", 13, 10, 0

; Disk Address Packet for INT 13h AH=42h
ALIGN 4
dap:
    db 0x10                     ; DAP size (16 bytes)
    db 0                        ; Reserved
    dw STAGE2_SECTORS           ; Number of sectors to read
    dw STAGE2_OFFSET            ; Buffer offset
    dw STAGE2_SEGMENT           ; Buffer segment
    dd STAGE2_START_LBA         ; Starting LBA (low 32 bits)
    dd 0                        ; Starting LBA (high 32 bits)

; =============================================================================
; Pad to 510 bytes and add boot signature
; =============================================================================
times 510 - ($ - $$) db 0
dw 0xAA55

; =============================================================================
; .anyOS Stage 1 Bootloader (MBR)
; =============================================================================
; Loaded by BIOS at 0x0000:0x7C00 (512 bytes)
; Task: Set up segments/stack, load Stage 2 from disk, jump to it
;
; Supports two boot paths:
;   1. HDD/AHCI: INT 13h AH=42h reads stage2 from disk LBA 1
;   2. CD-ROM (El Torito): BIOS pre-loaded stage1+stage2 contiguously
;      at 0x7C00. Stage2 is at 0x7E00, needs memcpy to 0x8000.
; =============================================================================

[BITS 16]
[ORG 0x7C00]

STAGE2_SEGMENT      equ 0x0000
STAGE2_OFFSET       equ 0x8000
STAGE2_SECTORS      equ 63
STAGE2_START_LBA    equ 1
STAGE2_SIZE         equ STAGE2_SECTORS * 512  ; 32256 bytes

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

    ; Try loading Stage 2 using INT 13h Extended Read (LBA addressing)
    ; This works for HDD/AHCI boot but fails on CD-ROM drives
    mov si, dap
    mov dl, [boot_drive]
    mov ah, 0x42
    int 0x13
    jnc .jump_stage2        ; Success → stage2 is at 0x8000

    ; ---------------------------------------------------------------
    ; INT 13h failed — El Torito CD-ROM boot path
    ; BIOS loaded 64 sectors (stage1+stage2) contiguously at 0x7C00:
    ;   0x7C00 - 0x7DFF : stage1 (512 bytes)
    ;   0x7E00 - 0xFBFF : stage2 (63 * 512 = 32256 bytes)
    ; Need to copy stage2 from 0x7E00 to 0x8000.
    ; Regions overlap (0x8000-0xFBFF), so copy backwards.
    ; ---------------------------------------------------------------
    std                                 ; Direction flag = backwards
    mov cx, STAGE2_SIZE / 2             ; Word count (16128 words)
    mov si, 0x7E00 + STAGE2_SIZE - 2   ; Last word of source  (0xFBFE)
    mov di, STAGE2_OFFSET + STAGE2_SIZE - 2  ; Last word of dest (0xFDFE)
    rep movsw
    cld                                 ; Restore direction flag

.jump_stage2:
    ; Jump to Stage 2
    mov dl, [boot_drive]    ; Pass boot drive to stage 2
    jmp STAGE2_SEGMENT:STAGE2_OFFSET

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

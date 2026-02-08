; =============================================================================
; vesa.asm - VESA VBE mode setup for graphical framebuffer
;
; Called from Stage 2 in real mode (16-bit) before entering protected mode.
; Tries to set 1024x768x32, falls back to 800x600x32 or 640x480x32.
; Stores mode info at VBE_MODE_INFO_ADDR for BootInfo population.
; =============================================================================

; Temp buffers for VBE BIOS calls (free conventional memory)
VBE_CTRL_INFO       equ 0x2200   ; 512 bytes for VBE Controller Info
VBE_MODE_INFO_ADDR  equ 0x2000   ; 256 bytes for VBE Mode Info

; Desired resolutions (tried in order)
WANT_W              equ 1024
WANT_H              equ 768
WANT_BPP            equ 32

FALLBACK1_W         equ 800
FALLBACK1_H         equ 600

FALLBACK2_W         equ 640
FALLBACK2_H         equ 480

; =============================================================================
; setup_vesa - Find and set a VESA VBE mode with linear framebuffer
;
; On success: sets vesa_ok to 1, mode info stored at VBE_MODE_INFO_ADDR
; On failure: sets vesa_ok to 0, boot continues in text mode
; =============================================================================
setup_vesa:
    pusha

    ; --- Step 1: Get VBE Controller Info ---
    mov di, VBE_CTRL_INFO
    mov dword [di], 0x32454256  ; "VBE2" signature → request VBE 2.0+ info
    mov ax, 0x4F00
    int 0x10
    cmp ax, 0x004F
    jne .no_vesa

    ; Check signature is "VESA"
    cmp dword [VBE_CTRL_INFO], 0x41534556
    jne .no_vesa

    ; --- Step 2: Get mode list pointer ---
    ; Mode list is a far pointer at offset 14 (offset) + 16 (segment)
    mov si, [VBE_CTRL_INFO + 14]   ; offset of mode list
    mov ax, [VBE_CTRL_INFO + 16]   ; segment of mode list
    mov es, ax                      ; ES:SI points to mode list

    ; --- Step 3: Scan mode list for best match ---
    ; Try preferred resolution first, then fallbacks
    mov word [best_mode], 0xFFFF
    mov byte [best_match], 0        ; 0=none, 1=fallback2, 2=fallback1, 3=preferred

.scan_modes:
    mov cx, [es:si]                 ; Read mode number
    cmp cx, 0xFFFF                  ; End of list?
    je .done_scan
    add si, 2                       ; Advance to next mode

    ; Get mode info
    push es
    push si
    push cx

    ; ES:DI = buffer for mode info
    push ds
    pop es                          ; ES = DS (our data segment)
    mov di, VBE_MODE_INFO_ADDR
    mov ax, 0x4F01
    int 0x10

    pop cx
    cmp ax, 0x004F
    jne .skip_mode

    ; Check mode attributes (offset 0): must have bit 0 (supported) and bit 4 (graphics) and bit 7 (linear FB)
    mov ax, [VBE_MODE_INFO_ADDR]
    test ax, 0x0091                 ; bits 0, 4, 7
    jz .skip_mode
    ; Make sure all 3 bits are set
    and ax, 0x0091
    cmp ax, 0x0091
    jne .skip_mode

    ; Check BPP (offset 0x19)
    cmp byte [VBE_MODE_INFO_ADDR + 0x19], WANT_BPP
    jne .skip_mode

    ; Check memory model (offset 0x1B): must be 6 (direct color)
    cmp byte [VBE_MODE_INFO_ADDR + 0x1B], 6
    jne .skip_mode

    ; Check resolution
    mov ax, [VBE_MODE_INFO_ADDR + 0x12]   ; width
    mov bx, [VBE_MODE_INFO_ADDR + 0x14]   ; height

    ; Check preferred: 1024x768
    cmp ax, WANT_W
    jne .check_fb1
    cmp bx, WANT_H
    jne .check_fb1
    ; Preferred match!
    cmp byte [best_match], 3
    jge .skip_mode
    mov byte [best_match], 3
    mov [best_mode], cx
    jmp .skip_mode

.check_fb1:
    ; Check fallback 1: 800x600
    cmp ax, FALLBACK1_W
    jne .check_fb2
    cmp bx, FALLBACK1_H
    jne .check_fb2
    cmp byte [best_match], 2
    jge .skip_mode
    mov byte [best_match], 2
    mov [best_mode], cx
    jmp .skip_mode

.check_fb2:
    ; Check fallback 2: 640x480
    cmp ax, FALLBACK2_W
    jne .skip_mode
    cmp bx, FALLBACK2_H
    jne .skip_mode
    cmp byte [best_match], 1
    jge .skip_mode
    mov byte [best_match], 1
    mov [best_mode], cx

.skip_mode:
    pop si
    pop es
    jmp .scan_modes

.done_scan:
    ; Check if we found a mode
    cmp word [best_mode], 0xFFFF
    je .no_vesa

    ; --- Step 4: Get final mode info for the selected mode ---
    mov cx, [best_mode]
    push ds
    pop es
    mov di, VBE_MODE_INFO_ADDR
    mov ax, 0x4F01
    int 0x10
    cmp ax, 0x004F
    jne .no_vesa

    ; --- Step 5: Set the mode with linear framebuffer ---
    mov bx, [best_mode]
    or bx, 0x4000                   ; Bit 14: use linear framebuffer
    mov ax, 0x4F02
    int 0x10
    cmp ax, 0x004F
    jne .no_vesa

    ; Success!
    mov byte [vesa_ok], 1
    popa
    ret

.no_vesa:
    mov byte [vesa_ok], 0
    popa
    ret

; --- Data ---
vesa_ok:    db 0
best_mode:  dw 0xFFFF
best_match: db 0

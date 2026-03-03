; =============================================================================
; video_init.asm — VGA text mode initialization and Bochs VBE support
; =============================================================================

VGA_TEXT_SEG    equ 0xB800
VGA_CRTC_ADDR  equ 0x03D4
VGA_CRTC_DATA  equ 0x03D5
VGA_STATUS     equ 0x03DA

; Bochs VBE I/O ports.
VBE_DISPI_PORT_INDEX    equ 0x01CE
VBE_DISPI_PORT_DATA     equ 0x01CF

; Bochs VBE register indices.
VBE_DISPI_INDEX_ID          equ 0
VBE_DISPI_INDEX_XRES        equ 1
VBE_DISPI_INDEX_YRES        equ 2
VBE_DISPI_INDEX_BPP         equ 3
VBE_DISPI_INDEX_ENABLE      equ 4
VBE_DISPI_INDEX_BANK        equ 5
VBE_DISPI_INDEX_VIRT_WIDTH  equ 6
VBE_DISPI_INDEX_VIRT_HEIGHT equ 7
VBE_DISPI_INDEX_X_OFFSET    equ 8
VBE_DISPI_INDEX_Y_OFFSET    equ 9

; VBE enable flags.
VBE_DISPI_DISABLED          equ 0x00
VBE_DISPI_ENABLED           equ 0x01
VBE_DISPI_LFB_ENABLED      equ 0x40

; LFB base address (VGA PCI BAR0).
VGA_LFB_BASE    equ 0xFD000000

; ---------------------------------------------------------------------------
; video_init — Set VGA mode 03h (80x25 text, 16 colors) and clear screen.
; ---------------------------------------------------------------------------
video_init:
    push es
    push di
    push cx
    push ax

    ; Reset attribute controller flip-flop.
    mov dx, VGA_STATUS
    in al, dx

    ; Disable Bochs VBE (ensure we're in text mode).
    mov dx, VBE_DISPI_PORT_INDEX
    mov ax, VBE_DISPI_INDEX_ENABLE
    out dx, ax
    mov dx, VBE_DISPI_PORT_DATA
    mov ax, VBE_DISPI_DISABLED
    out dx, ax

    ; Clear the text-mode framebuffer (80*25 = 2000 cells).
    mov ax, VGA_TEXT_SEG
    mov es, ax
    xor di, di
    mov ax, 0x0720          ; Space, light gray on black
    mov cx, 2000
    rep stosw

    ; Set cursor position to (0, 0).
    mov dx, VGA_CRTC_ADDR
    mov al, 0x0E            ; Cursor location high
    out dx, al
    mov dx, VGA_CRTC_DATA
    xor al, al
    out dx, al
    mov dx, VGA_CRTC_ADDR
    mov al, 0x0F            ; Cursor location low
    out dx, al
    mov dx, VGA_CRTC_DATA
    xor al, al
    out dx, al

    ; Enable cursor (scanlines 6-7).
    mov dx, VGA_CRTC_ADDR
    mov al, 0x0A            ; Cursor start
    out dx, al
    mov dx, VGA_CRTC_DATA
    mov al, 0x06
    out dx, al
    mov dx, VGA_CRTC_ADDR
    mov al, 0x0B            ; Cursor end
    out dx, al
    mov dx, VGA_CRTC_DATA
    mov al, 0x07
    out dx, al

    pop ax
    pop cx
    pop di
    pop es
    ret

; ---------------------------------------------------------------------------
; vbe_set_mode — Set a Bochs VBE linear framebuffer mode.
;   Input: BX = width, CX = height, DL = bpp (bits per pixel)
; ---------------------------------------------------------------------------
vbe_set_mode:
    push ax
    push dx

    ; Disable VBE first.
    push dx
    mov dx, VBE_DISPI_PORT_INDEX
    mov ax, VBE_DISPI_INDEX_ENABLE
    out dx, ax
    mov dx, VBE_DISPI_PORT_DATA
    mov ax, VBE_DISPI_DISABLED
    out dx, ax
    pop dx

    ; Set X resolution.
    push dx
    mov dx, VBE_DISPI_PORT_INDEX
    mov ax, VBE_DISPI_INDEX_XRES
    out dx, ax
    mov dx, VBE_DISPI_PORT_DATA
    mov ax, bx
    out dx, ax
    pop dx

    ; Set Y resolution.
    push dx
    mov dx, VBE_DISPI_PORT_INDEX
    mov ax, VBE_DISPI_INDEX_YRES
    out dx, ax
    mov dx, VBE_DISPI_PORT_DATA
    mov ax, cx
    out dx, ax
    pop dx

    ; Set BPP.
    push dx
    movzx ax, dl
    push ax
    mov dx, VBE_DISPI_PORT_INDEX
    mov ax, VBE_DISPI_INDEX_BPP
    out dx, ax
    mov dx, VBE_DISPI_PORT_DATA
    pop ax
    out dx, ax
    pop dx

    ; Enable VBE with LFB.
    push dx
    mov dx, VBE_DISPI_PORT_INDEX
    mov ax, VBE_DISPI_INDEX_ENABLE
    out dx, ax
    mov dx, VBE_DISPI_PORT_DATA
    mov ax, VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED
    out dx, ax
    pop dx

    pop dx
    pop ax
    ret

; =============================================================================
; int10h.asm — INT 10h: Video BIOS services
; =============================================================================

int10h_handler:
    cmp ah, 0x00
    je i10_set_mode
    cmp ah, 0x01
    je i10_set_cursor_shape
    cmp ah, 0x02
    je i10_set_cursor_pos
    cmp ah, 0x03
    je i10_get_cursor_pos
    cmp ah, 0x05
    je i10_set_active_page
    cmp ah, 0x06
    je i10_scroll_up
    cmp ah, 0x07
    je i10_scroll_down
    cmp ah, 0x08
    je i10_read_char_attr
    cmp ah, 0x09
    je i10_write_char_attr
    cmp ah, 0x0E
    je i10_tty_output
    cmp ah, 0x0F
    je i10_get_video_mode
    cmp ah, 0x12
    je i10_alt_func_select
    cmp ah, 0x1A
    je i10_display_combo
    cmp ax, 0x4F00
    je i10_vbe_info
    cmp ax, 0x4F01
    je i10_vbe_mode_info
    cmp ax, 0x4F02
    je i10_vbe_set_mode
    cmp ax, 0x4F03
    je i10_vbe_get_mode
    iret

; ---------------------------------------------------------------------------
; AH=00h: Set video mode (AL = mode number).
; ---------------------------------------------------------------------------
i10_set_mode:
    push ds
    push es
    push di
    push cx
    push ax

    xor bx, bx
    mov ds, bx
    mov [BDA_VIDEO_MODE], al

    cmp al, 0x03
    je .mode_03
    cmp al, 0x13
    je .mode_13
    jmp .done

.mode_03:
    call video_init
    mov word [BDA_SCREEN_COLS], 80
    mov byte [BDA_ROWS_MINUS1], 24
    mov word [BDA_VIDEO_PAGE_SZ], 4000
    jmp .done

.mode_13:
    mov bx, 320
    mov cx, 200
    mov dl, 8
    call vbe_set_mode
    mov word [BDA_SCREEN_COLS], 40
    mov byte [BDA_ROWS_MINUS1], 24

.done:
    mov word [BDA_CURSOR_POS], 0
    pop ax
    pop cx
    pop di
    pop es
    pop ds
    iret

; ---------------------------------------------------------------------------
; AH=01h: Set cursor shape. CH = start scanline, CL = end scanline.
; ---------------------------------------------------------------------------
i10_set_cursor_shape:
    push ds
    push dx
    push ax
    xor ax, ax
    mov ds, ax
    mov [BDA_CURSOR_SHAPE], cx

    mov dx, VGA_CRTC_ADDR
    mov al, 0x0A
    out dx, al
    mov dx, VGA_CRTC_DATA
    mov al, ch
    out dx, al
    mov dx, VGA_CRTC_ADDR
    mov al, 0x0B
    out dx, al
    mov dx, VGA_CRTC_DATA
    mov al, cl
    out dx, al

    pop ax
    pop dx
    pop ds
    iret

; ---------------------------------------------------------------------------
; AH=02h: Set cursor position. BH = page, DH = row, DL = column.
; ---------------------------------------------------------------------------
i10_set_cursor_pos:
    push ds
    push ax
    push bx
    xor ax, ax
    mov ds, ax

    movzx bx, bh
    shl bx, 1
    mov [BDA_CURSOR_POS + bx], dx

    cmp bh, 0
    jne .done

    push dx                         ; Save cursor (MUL clobbers DX)
    movzx ax, dh
    mov bx, 80
    mul bx
    pop dx                          ; Restore cursor
    movzx bx, dl
    add ax, bx

    push dx
    push ax
    mov dx, VGA_CRTC_ADDR
    mov al, 0x0E
    out dx, al
    mov dx, VGA_CRTC_DATA
    pop ax
    push ax
    mov al, ah
    out dx, al
    mov dx, VGA_CRTC_ADDR
    mov al, 0x0F
    out dx, al
    mov dx, VGA_CRTC_DATA
    pop ax
    out dx, al
    pop dx

.done:
    pop bx
    pop ax
    pop ds
    iret

; ---------------------------------------------------------------------------
; AH=03h: Get cursor position and shape.
; ---------------------------------------------------------------------------
i10_get_cursor_pos:
    push ds
    push bx
    xor ax, ax
    mov ds, ax
    movzx bx, bh
    shl bx, 1
    mov dx, [BDA_CURSOR_POS + bx]
    mov cx, [BDA_CURSOR_SHAPE]
    pop bx
    pop ds
    iret

; ---------------------------------------------------------------------------
; AH=05h: Set active display page.
; ---------------------------------------------------------------------------
i10_set_active_page:
    push ds
    push ax
    xor bx, bx
    mov ds, bx
    mov [BDA_ACTIVE_PAGE], al
    pop ax
    pop ds
    iret

; ---------------------------------------------------------------------------
; AH=06h: Scroll up.
; ---------------------------------------------------------------------------
i10_scroll_up:
    push ds
    push es
    push si
    push di
    pusha

    mov bp, sp
    mov ax, VGA_TEXT_SEG
    mov ds, ax
    mov es, ax

    ; Clear entire region: fill with space + attribute.
    movzx ax, byte [bp + 8]    ; BH = attribute
    mov ah, al
    mov al, 0x20
    xchg ah, al

    push ax
    movzx si, byte [bp + 10]   ; CH = top row
    mov ax, si
    mov bx, 80
    mul bx
    movzx bx, byte [bp + 11]   ; CL = left col
    add ax, bx
    shl ax, 1
    mov di, ax
    pop ax

    movzx cx, byte [bp + 6]    ; DL = right col
    movzx bx, byte [bp + 11]   ; CL = left col
    sub cx, bx
    inc cx

    movzx bx, byte [bp + 7]    ; DH = bottom row
    movzx si, byte [bp + 10]   ; CH = top row
    sub bx, si
    inc bx

.row:
    push cx
    push di
    rep stosw
    pop di
    add di, 160
    pop cx
    dec bx
    jnz .row

    popa
    pop di
    pop si
    pop es
    pop ds
    iret

; ---------------------------------------------------------------------------
; AH=07h: Scroll down.
; ---------------------------------------------------------------------------
i10_scroll_down:
    jmp i10_scroll_up

; ---------------------------------------------------------------------------
; AH=08h: Read character and attribute at cursor.
; ---------------------------------------------------------------------------
i10_read_char_attr:
    push ds
    push es
    push bx
    push si

    xor si, si
    mov ds, si
    movzx bx, bh
    shl bx, 1
    mov dx, [BDA_CURSOR_POS + bx]

    push dx                         ; Save cursor (MUL clobbers DX)
    movzx ax, dh
    mov bx, 80
    mul bx
    pop dx                          ; Restore cursor
    movzx bx, dl
    add ax, bx
    shl ax, 1
    mov si, ax

    mov ax, VGA_TEXT_SEG
    mov es, ax
    mov ax, [es:si]

    pop si
    pop bx
    pop es
    pop ds
    iret

; ---------------------------------------------------------------------------
; AH=09h: Write character and attribute at cursor.
; ---------------------------------------------------------------------------
i10_write_char_attr:
    push ds
    push es
    push di
    push bx
    push cx
    push dx

    push ax
    xor ax, ax
    mov ds, ax

    movzx di, bh
    shl di, 1
    mov dx, [BDA_CURSOR_POS + di]

    push dx                         ; Save cursor (MUL clobbers DX)
    movzx ax, dh
    push bx
    mov bx, 80
    mul bx
    pop bx
    pop dx                          ; Restore cursor
    push bx
    movzx bx, dl
    add ax, bx
    shl ax, 1
    mov di, ax
    pop bx

    mov ax, VGA_TEXT_SEG
    mov es, ax
    pop ax
    mov ah, bl

    rep stosw

    pop dx
    pop cx
    pop bx
    pop di
    pop es
    pop ds
    iret

; ---------------------------------------------------------------------------
; AH=0Eh: TTY character output.
; ---------------------------------------------------------------------------
i10_tty_output:
    push ds
    push es
    push di
    push bx
    push cx
    push dx
    push ax

    ; Mirror the character to the serial port so all INT 10h text output
    ; (BIOS POST messages and bootloader text) appears in the serial log.
    ; serial_putchar preserves AX and DX, so this is safe here.
    call serial_putchar

    xor bx, bx
    mov ds, bx

    mov dx, [BDA_CURSOR_POS]

    cmp al, 0x0D
    je .cr
    cmp al, 0x0A
    je .lf
    cmp al, 0x08
    je .bs
    cmp al, 0x07
    je .done

    ; Regular character: write to VGA.
    push ax
    push dx                         ; Save cursor (MUL clobbers DX)
    movzx ax, dh
    mov bx, 80
    mul bx
    pop dx                          ; Restore cursor
    movzx bx, dl
    add ax, bx
    shl ax, 1
    mov di, ax
    pop ax

    mov bx, VGA_TEXT_SEG
    mov es, bx
    mov ah, 0x07
    mov [es:di], ax

    inc dl
    cmp dl, 80
    jb .update_cursor
    xor dl, dl
    inc dh

.check_scroll:
    cmp dh, 25
    jb .update_cursor
    mov dh, 24
    call i10_scroll_line_up
    jmp .update_cursor

.cr:
    xor dl, dl
    jmp .update_cursor

.lf:
    inc dh
    jmp .check_scroll

.bs:
    cmp dl, 0
    je .update_cursor
    dec dl
    jmp .update_cursor

.update_cursor:
    mov [BDA_CURSOR_POS], dx

    push ax
    push dx                         ; Save cursor (MUL clobbers DX)
    movzx ax, dh
    mov bx, 80
    mul bx
    pop dx                          ; Restore cursor
    movzx bx, dl
    add ax, bx

    push dx
    push ax
    mov dx, VGA_CRTC_ADDR
    mov al, 0x0E
    out dx, al
    mov dx, VGA_CRTC_DATA
    pop ax
    push ax
    mov al, ah
    out dx, al
    mov dx, VGA_CRTC_ADDR
    mov al, 0x0F
    out dx, al
    mov dx, VGA_CRTC_DATA
    pop ax
    out dx, al
    pop dx
    pop ax

.done:
    pop ax
    pop dx
    pop cx
    pop bx
    pop di
    pop es
    pop ds
    iret

; Scroll the text screen up one line (used by TTY output).
i10_scroll_line_up:
    push ds
    push es
    push si
    push di
    push cx

    mov ax, VGA_TEXT_SEG
    mov ds, ax
    mov es, ax

    mov si, 160
    xor di, di
    mov cx, 80 * 24
    rep movsw

    mov di, 160 * 24
    mov ax, 0x0720
    mov cx, 80
    rep stosw

    pop cx
    pop di
    pop si
    pop es
    pop ds
    ret

; ---------------------------------------------------------------------------
; AH=0Fh: Get video mode.
; ---------------------------------------------------------------------------
i10_get_video_mode:
    push ds
    xor bx, bx
    mov ds, bx
    mov al, [BDA_VIDEO_MODE]
    mov ah, byte [BDA_SCREEN_COLS]
    mov bh, [BDA_ACTIVE_PAGE]
    pop ds
    iret

; ---------------------------------------------------------------------------
; AH=12h: Alternate function select.
; ---------------------------------------------------------------------------
i10_alt_func_select:
    cmp bl, 0x10
    jne .done
    mov bh, 0x00
    mov bl, 0x03
    xor cx, cx
.done:
    iret

; ---------------------------------------------------------------------------
; AH=1Ah: Display combination code.
; ---------------------------------------------------------------------------
i10_display_combo:
    mov al, 0x1A
    mov bl, 0x08
    iret

; =============================================================================
; VBE (VESA BIOS Extensions) functions
; =============================================================================

VBE_MODE_640x480x32     equ 0x0112
VBE_MODE_800x600x32     equ 0x0115
VBE_MODE_1024x768x32    equ 0x0118
VBE_MODE_1280x1024x32   equ 0x011B
VBE_MODE_1280x720x32    equ 0x0140
VBE_MODE_1280x800x32    equ 0x0141
VBE_MODE_1366x768x32    equ 0x0142
VBE_MODE_1400x1050x32   equ 0x0143
VBE_MODE_1600x1200x32   equ 0x0144
VBE_MODE_1920x1080x32   equ 0x0145
VBE_MODE_1920x1200x32   equ 0x0146

str_vbe_oem:    db 'CoreVM VBE', 0

vbe_mode_list:
    dw VBE_MODE_640x480x32
    dw VBE_MODE_800x600x32
    dw VBE_MODE_1024x768x32
    dw VBE_MODE_1280x1024x32
    dw VBE_MODE_1280x720x32
    dw VBE_MODE_1280x800x32
    dw VBE_MODE_1366x768x32
    dw VBE_MODE_1400x1050x32
    dw VBE_MODE_1600x1200x32
    dw VBE_MODE_1920x1080x32
    dw VBE_MODE_1920x1200x32
    dw 0xFFFF

vbe_mi_xres:    dw 0
vbe_mi_yres:    dw 0
vbe_mi_pitch:   dw 0

; VBE mode lookup table: mode_number, xres, yres, pitch (each dw)
vbe_mode_table:
    dw VBE_MODE_640x480x32,    640,  480, 2560
    dw VBE_MODE_800x600x32,    800,  600, 3200
    dw VBE_MODE_1024x768x32,  1024,  768, 4096
    dw VBE_MODE_1280x1024x32, 1280, 1024, 5120
    dw VBE_MODE_1280x720x32,  1280,  720, 5120
    dw VBE_MODE_1280x800x32,  1280,  800, 5120
    dw VBE_MODE_1366x768x32,  1366,  768, 5464
    dw VBE_MODE_1400x1050x32, 1400, 1050, 5600
    dw VBE_MODE_1600x1200x32, 1600, 1200, 6400
    dw VBE_MODE_1920x1080x32, 1920, 1080, 7680
    dw VBE_MODE_1920x1200x32, 1920, 1200, 7680
    dw 0xFFFF, 0, 0, 0

current_vbe_mode:   dw 0x0003

; ---------------------------------------------------------------------------
; AX=4F00h: Return VBE controller info.
; ---------------------------------------------------------------------------
i10_vbe_info:
    push ds
    push si
    push cx
    push di

    mov dword [es:di + 0], 'VESA'
    mov word [es:di + 4], 0x0200
    mov word [es:di + 6], str_vbe_oem
    mov word [es:di + 8], 0xF000
    mov dword [es:di + 10], 0
    mov word [es:di + 14], vbe_mode_list
    mov word [es:di + 16], 0xF000
    mov word [es:di + 18], 256

    mov ax, 0x004F
    pop di
    pop cx
    pop si
    pop ds
    iret

; ---------------------------------------------------------------------------
; AX=4F01h: Return VBE mode info.
; ---------------------------------------------------------------------------
i10_vbe_mode_info:
    push di
    push cx

    ; Zero the buffer.
    push cx
    push di
    xor ax, ax
    mov cx, 128
    rep stosw
    pop di
    pop cx

    and cx, 0x01FF

    ; Look up mode in table
    mov si, vbe_mode_table
.mi_search:
    cmp word [cs:si], 0xFFFF
    je .mi_not_found
    cmp cx, [cs:si]
    je .mi_found
    add si, 8              ; next entry (mode + xres + yres + pitch)
    jmp .mi_search

.mi_not_found:
    mov ax, 0x014F
    pop cx
    pop di
    iret

.mi_found:
    mov ax, [cs:si + 2]
    mov [vbe_mi_xres], ax
    mov ax, [cs:si + 4]
    mov [vbe_mi_yres], ax
    mov ax, [cs:si + 6]
    mov [vbe_mi_pitch], ax

.mi_fill:
    mov word [es:di + 0], 0x009B
    mov ax, [vbe_mi_pitch]
    mov [es:di + 16], ax
    mov ax, [vbe_mi_xres]
    mov [es:di + 18], ax
    mov ax, [vbe_mi_yres]
    mov [es:di + 20], ax
    mov byte [es:di + 22], 8
    mov byte [es:di + 23], 16
    mov byte [es:di + 24], 1
    mov byte [es:di + 25], 32
    mov byte [es:di + 27], 6
    mov byte [es:di + 31], 8
    mov byte [es:di + 32], 16
    mov byte [es:di + 33], 8
    mov byte [es:di + 34], 8
    mov byte [es:di + 35], 8
    mov byte [es:di + 36], 0
    mov byte [es:di + 37], 8
    mov byte [es:di + 38], 24
    mov dword [es:di + 40], VGA_LFB_BASE
    mov ax, [vbe_mi_pitch]
    mov [es:di + 50], ax

    mov ax, 0x004F
    pop cx
    pop di
    iret

; ---------------------------------------------------------------------------
; AX=4F02h: Set VBE mode.
; ---------------------------------------------------------------------------
i10_vbe_set_mode:
    push cx
    push dx

    mov cx, bx
    and cx, 0x01FF

    cmp cx, 0x0003
    je .sm_text

    ; Look up mode in table
    push si
    mov si, vbe_mode_table
.sm_search:
    cmp word [cs:si], 0xFFFF
    je .sm_not_found
    cmp cx, [cs:si]
    je .sm_found
    add si, 8
    jmp .sm_search

.sm_not_found:
    pop si
    mov ax, 0x014F
    jmp .sm_done

.sm_found:
    mov bx, [cs:si + 2]    ; width
    mov cx, [cs:si + 4]    ; height
    pop si
    mov dl, 32
    call vbe_set_mode
    jmp .sm_ok

.sm_text:
    call video_init
    jmp .sm_ok

.sm_ok:
    mov [current_vbe_mode], bx
    mov ax, 0x004F

.sm_done:
    pop dx
    pop cx
    iret

; ---------------------------------------------------------------------------
; AX=4F03h: Get current VBE mode.
; ---------------------------------------------------------------------------
i10_vbe_get_mode:
    mov bx, [current_vbe_mode]
    mov ax, 0x004F
    iret

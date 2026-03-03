; =============================================================================
; bda.asm — BIOS Data Area constants and initialization
; =============================================================================

; BDA absolute addresses (segment 0x0040).
BDA_SEG             equ 0x0040
BDA_COM1            equ 0x0400      ; COM1 base port
BDA_COM2            equ 0x0402
BDA_COM3            equ 0x0404
BDA_COM4            equ 0x0406
BDA_EQUIP           equ 0x0410      ; Equipment word
BDA_CONV_MEM_KB     equ 0x0413      ; Conventional memory in KB
BDA_KBD_FLAGS1      equ 0x0417      ; Keyboard shift flags byte 1
BDA_KBD_FLAGS2      equ 0x0418      ; Keyboard shift flags byte 2
BDA_KBD_BUF_HEAD    equ 0x041A      ; Keyboard buffer head offset (rel to 0x400)
BDA_KBD_BUF_TAIL    equ 0x041C      ; Keyboard buffer tail offset (rel to 0x400)
BDA_KBD_BUF         equ 0x041E      ; Keyboard buffer (32 bytes, 16 entries)
BDA_VIDEO_MODE      equ 0x0449      ; Current video mode
BDA_SCREEN_COLS     equ 0x044A      ; Screen columns (word)
BDA_VIDEO_PAGE_SZ   equ 0x044C      ; Video page size (word)
BDA_CURSOR_POS      equ 0x0450      ; Cursor positions (8 pages, word each)
BDA_CURSOR_SHAPE    equ 0x0460      ; Cursor start/end scanline
BDA_ACTIVE_PAGE     equ 0x0462      ; Active display page
BDA_CRTC_PORT       equ 0x0463      ; CRTC base port (0x3D4 for color)
BDA_TIMER_COUNT     equ 0x046C      ; Timer tick count (dword)
BDA_TIMER_OVERFLOW  equ 0x0470      ; Timer overflow (midnight flag)
BDA_BREAK_FLAG      equ 0x0471      ; Ctrl-Break flag
BDA_SOFT_RESET      equ 0x0472      ; Soft reset flag
BDA_NUM_HD          equ 0x0475      ; Number of hard disks
BDA_KBD_BUF_START   equ 0x0480      ; Keyboard buffer start offset
BDA_KBD_BUF_END     equ 0x0482      ; Keyboard buffer end offset
BDA_ROWS_MINUS1     equ 0x0484      ; Number of rows minus 1

; EBDA location.
EBDA_SEG            equ 0x9FC0      ; EBDA at 0x9FC00
EBDA_SIZE_KB        equ 1           ; 1 KB

; ---------------------------------------------------------------------------
; bda_init — Initialize the BIOS Data Area at 0x0040:0x0000.
; ---------------------------------------------------------------------------
bda_init:
    push es
    push di
    push cx
    push ax

    ; Zero out BDA (256 bytes at 0x0400).
    xor ax, ax
    mov es, ax
    mov di, 0x0400
    mov cx, 128             ; 256 bytes / 2
    rep stosw

    ; COM1 port base.
    mov word [es:BDA_COM1], 0x03F8

    ; Equipment word: bit 1 = FPU present, bits 4-5 = 11 (80x25 color).
    ;   0x0022 = FPU + initial video mode 80x25 color
    mov word [es:BDA_EQUIP], 0x0022

    ; Conventional memory: 639 KB (EBDA takes 1 KB from 640).
    mov word [es:BDA_CONV_MEM_KB], 639

    ; Keyboard buffer pointers (empty buffer).
    mov word [es:BDA_KBD_BUF_HEAD], 0x001E  ; Offset relative to 0x400
    mov word [es:BDA_KBD_BUF_TAIL], 0x001E
    mov word [es:BDA_KBD_BUF_START], 0x001E
    mov word [es:BDA_KBD_BUF_END], 0x003E   ; 0x1E + 32

    ; Video: mode 03h, 80 columns, 25 rows.
    mov byte [es:BDA_VIDEO_MODE], 0x03
    mov word [es:BDA_SCREEN_COLS], 80
    mov word [es:BDA_VIDEO_PAGE_SZ], 4000   ; 80*25*2
    mov byte [es:BDA_ACTIVE_PAGE], 0
    mov word [es:BDA_CRTC_PORT], 0x03D4
    mov byte [es:BDA_ROWS_MINUS1], 24

    ; Cursor shape: scanlines 6-7 (underline cursor).
    mov word [es:BDA_CURSOR_SHAPE], 0x0607

    pop ax
    pop cx
    pop di
    pop es
    ret

; ---------------------------------------------------------------------------
; ebda_init — Initialize the Extended BIOS Data Area at 0x9FC00.
; ---------------------------------------------------------------------------
ebda_init:
    push es
    push di
    push cx
    push ax

    ; Zero EBDA (1 KB).
    mov ax, EBDA_SEG
    mov es, ax
    xor di, di
    xor ax, ax
    mov cx, 512             ; 1024 / 2
    rep stosw

    ; First byte of EBDA = size in KB.
    mov byte [es:0x0000], EBDA_SIZE_KB

    ; Store EBDA segment in BDA at 0x040E.
    xor ax, ax
    mov es, ax
    mov word [es:0x040E], EBDA_SEG

    pop ax
    pop cx
    pop di
    pop es
    ret

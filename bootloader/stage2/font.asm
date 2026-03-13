; =============================================================================
; font.asm - 8x16 bitmap font renderer for VESA framebuffer
; =============================================================================
; Font data at FONT_LOAD_ADDR (0x28000): 95 glyphs (ASCII 32-126),
; 16 bytes/glyph. Each byte = one row, MSB = leftmost pixel.
; VBE mode info block at 0x2000.
; Unreal mode: 32-bit addressing works with a32/addr32 prefix.
; =============================================================================

FONT_LOAD_ADDR equ 0x28000
VBE_MODE_INFO  equ 0x2000

; -----------------------------------------------------------------------------
; draw_char - Draw a single 8x16 glyph
; Input: AL = ASCII char (32-126), EBX = X pixels, ECX = Y pixels,
;        EDX = color (0x00RRGGBB)
; -----------------------------------------------------------------------------
draw_char:
    pusha

    ; Bounds check
    cmp al, 32
    jb .done
    cmp al, 126
    ja .done

    ; Calculate glyph address in ESI
    movzx esi, al
    sub esi, 32
    shl esi, 4              ; * 16 bytes per glyph
    add esi, FONT_LOAD_ADDR

    ; Get framebuffer base (PhysBasePtr at VBE+0x28, dword)
    mov edi, [VBE_MODE_INFO + 0x28]

    ; Get pitch (BytesPerScanLine at VBE+0x10, word)
    movzx eax, word [VBE_MODE_INFO + 0x10]
    ; EAX = pitch

    ; Calculate starting pixel address: fb_base + Y * pitch + X * 4
    ; ECX = Y, EBX = X
    imul ecx, eax           ; ECX = Y * pitch
    add edi, ecx            ; EDI = fb_base + Y * pitch
    lea edi, [edi + ebx*4]  ; EDI = fb_base + Y * pitch + X * 4

    ; EAX = pitch (restore for row advancement)
    movzx ebp, word [VBE_MODE_INFO + 0x10]

    ; Draw 16 rows
    mov ecx, 16
.row_loop:
    a32 mov al, [esi]       ; Load glyph byte
    inc esi

    ; Unrolled bit checks: MSB (bit 7) = leftmost pixel
    test al, 0x80
    jz .skip0
    a32 mov [edi], edx
.skip0:
    test al, 0x40
    jz .skip1
    a32 mov [edi+4], edx
.skip1:
    test al, 0x20
    jz .skip2
    a32 mov [edi+8], edx
.skip2:
    test al, 0x10
    jz .skip3
    a32 mov [edi+12], edx
.skip3:
    test al, 0x08
    jz .skip4
    a32 mov [edi+16], edx
.skip4:
    test al, 0x04
    jz .skip5
    a32 mov [edi+20], edx
.skip5:
    test al, 0x02
    jz .skip6
    a32 mov [edi+24], edx
.skip6:
    test al, 0x01
    jz .skip7
    a32 mov [edi+28], edx
.skip7:
    ; Advance to next row
    add edi, ebp
    dec ecx
    jnz .row_loop

.done:
    popa
    ret

; -----------------------------------------------------------------------------
; draw_string - Draw a null-terminated string
; Input: ESI = string pointer, EBX = X pixels, ECX = Y pixels,
;        EDX = color (0x00RRGGBB)
; -----------------------------------------------------------------------------
draw_string:
    pusha

.loop:
    lodsb
    test al, al
    jz .done
    call draw_char
    add ebx, 8
    jmp .loop

.done:
    popa
    ret

; -----------------------------------------------------------------------------
; draw_string_centered - Draw a string horizontally centered
; Input: ESI = null-terminated string, ECX = Y pixels, EDX = color
; -----------------------------------------------------------------------------
draw_string_centered:
    pusha

    ; Calculate string length
    push esi
    xor eax, eax
.len_loop:
    cmp byte [esi], 0
    je .len_done
    inc eax
    inc esi
    jmp .len_loop
.len_done:
    pop esi

    ; Pixel width = len * 8
    shl eax, 3

    ; Screen width from VBE mode info (XResolution at +0x12, word)
    movzx ebx, word [VBE_MODE_INFO + 0x12]

    ; X = (screen_width - string_pixel_width) / 2
    sub ebx, eax
    shr ebx, 1

    ; Call draw_string (ESI, EBX, ECX, EDX already set)
    call draw_string

    popa
    ret

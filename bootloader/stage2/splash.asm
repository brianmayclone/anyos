; =============================================================================
; splash.asm - Boot splash screen
; =============================================================================

; -----------------------------------------------------------------------------
; show_splash - Clear screen to black + draw centered boot logo
; -----------------------------------------------------------------------------
show_splash:
    pusha

    ; Clear framebuffer to black (VESA init may leave garbage)
    call clear_screen

    ; Check if logo was loaded
    cmp word [logo_sectors], 0
    je .no_logo

    ; Set logo_y to vertically centered position using native logo height
    ; center_y = (screen_h - logo_h) / 2
    movzx eax, word [0x2000 + 0x14]    ; screen height
    a32 mov ebx, [LOGO_LOAD_ADDR + 4]  ; logo height (from header)
    sub eax, ebx
    shr eax, 1
    mov [logo_y], eax

    call draw_logo

.no_logo:
    popa
    ret

; -----------------------------------------------------------------------------
; clear_screen - Fill framebuffer with black (0x00000000)
; Uses REP STOSD for speed
; -----------------------------------------------------------------------------
clear_screen:
    pusha
    mov edi, [0x2000 + 0x28]           ; PhysBasePtr
    movzx eax, word [0x2000 + 0x10]    ; pitch
    movzx ecx, word [0x2000 + 0x14]    ; height
    imul ecx, eax                       ; total bytes = height * pitch
    shr ecx, 2                          ; dwords = bytes / 4
    xor eax, eax                        ; black
    a32 rep stosd
    popa
    ret

; -----------------------------------------------------------------------------
; draw_logo - Draw logo at native pixel size
;   Logo at LOGO_LOAD_ADDR: [width:u32][height:u32][RGB pixels 3 bytes each]
;   Black pixels (0,0,0) are transparent
; -----------------------------------------------------------------------------
draw_logo:
    pusha

    ; Read logo dimensions from header
    a32 mov eax, [LOGO_LOAD_ADDR]       ; logo width
    mov [logo_w], eax
    a32 mov ebx, [LOGO_LOAD_ADDR + 4]  ; logo height
    mov [logo_h], ebx

    ; Compute horizontal center: logo_x = (screen_w - logo_w) / 2
    movzx ecx, word [0x2000 + 0x12]   ; screen width
    sub ecx, eax
    shr ecx, 1
    mov [logo_x], ecx

    ; Get framebuffer info
    mov edi, [0x2000 + 0x28]           ; PhysBasePtr
    movzx ecx, word [0x2000 + 0x10]   ; pitch

    ; Compute start address: fb + logo_y * pitch + logo_x * 4
    mov eax, [logo_y]
    mul ecx                             ; eax = logo_y * pitch (edx:eax, but fits 32-bit)
    add edi, eax
    mov eax, [logo_x]
    shl eax, 2                          ; * 4 bytes per pixel
    add edi, eax
    ; edi = top-left corner of logo destination

    mov [.dst_base], edi
    mov [.pitch], ecx

    ; Source pointer: skip 8-byte header
    mov esi, LOGO_LOAD_ADDR + 8

    ; Loop over source rows
    mov ecx, [logo_h]
    mov [.src_rows_left], ecx

.src_row_loop:
    cmp dword [.src_rows_left], 0
    je .done

    mov edi, [.dst_base]
    mov ecx, [logo_w]

.src_pixel_loop:
    ; Read RGB (3 bytes) from source
    movzx eax, byte [esi]              ; R
    movzx ebx, byte [esi + 1]          ; G
    movzx edx, byte [esi + 2]          ; B
    add esi, 3

    ; Check transparency (black = skip)
    or eax, ebx
    or eax, edx
    jz .skip_pixel

    ; Rebuild pixel: 0x00RRGGBB
    movzx eax, byte [esi - 3]          ; R
    shl eax, 16
    movzx ebx, byte [esi - 2]          ; G
    shl ebx, 8
    or eax, ebx
    movzx ebx, byte [esi - 1]          ; B
    or eax, ebx

    mov [edi], eax
    jmp .next_pixel

.skip_pixel:
.next_pixel:
    add edi, 4                          ; advance 1 pixel * 4 bytes
    dec ecx
    jnz .src_pixel_loop

    ; Advance destination to next row
    mov eax, [.pitch]
    add [.dst_base], eax

    dec dword [.src_rows_left]
    jmp .src_row_loop

.done:
    popa
    ret

; Local data for draw_logo
.dst_base:      dd 0
.pitch:         dd 0
.src_rows_left: dd 0

; -----------------------------------------------------------------------------
; wait_for_input - Wait for timeout or Escape key
;   Returns: AL = 0 (timeout), AL = 0x1B (Escape pressed)
; -----------------------------------------------------------------------------
wait_for_input:
    push bx
    push cx
    push dx

    ; Get starting tick count via INT 1Ah
    xor ah, ah
    int 0x1A                            ; CX:DX = tick count
    mov bx, dx                          ; save low word of start ticks
    mov [.start_hi], cx

    ; Compute target = start + timeout * 18
    movzx eax, word [cfg_timeout]
    mov ecx, 18
    mul ecx                             ; eax = timeout * 18
    add bx, ax                          ; bx = low word of target (may wrap)
    adc word [.start_hi], dx            ; handle carry into high word
    ; .start_hi now = target high word, bx = target low word
    mov [.target_lo], bx
    mov ax, [.start_hi]
    mov [.target_hi], ax

.wait_loop:
    ; Check keyboard (non-blocking)
    mov ah, 0x01
    int 0x16
    jz .no_key

    ; Key is ready, read it
    mov ah, 0x00
    int 0x16
    ; AH = scan code, AL = ASCII
    cmp ah, 0x01                        ; Escape scan code
    jne .wait_loop                      ; ignore other keys
    mov al, 0x1B                        ; return Escape
    jmp .done

.no_key:
    ; Check ticks
    xor ah, ah
    int 0x1A                            ; CX:DX = current ticks

    ; Compare CX:DX >= target_hi:target_lo
    cmp cx, [.target_hi]
    ja .timed_out
    jb .wait_loop
    cmp dx, [.target_lo]
    jb .wait_loop

.timed_out:
    xor al, al                          ; return 0 (timeout)

.done:
    pop dx
    pop cx
    pop bx
    ret

.start_hi:  dw 0
.target_lo: dw 0
.target_hi: dw 0

; =============================================================================
; Public data
; =============================================================================
logo_w:      dd 0
logo_h:      dd 0
logo_x:      dd 0
logo_y:      dd 0

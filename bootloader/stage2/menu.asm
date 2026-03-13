; =============================================================================
; menu.asm - Interactive boot menu
; =============================================================================

; Color constants
MENU_HIGHLIGHT_COLOR equ 0x00AACCFF
MENU_NORMAL_COLOR    equ 0x00888888
MENU_FOOTER_COLOR    equ 0x00666666
MENU_BAR_COLOR       equ 0x00202830

; -----------------------------------------------------------------------------
; show_menu - Display interactive boot menu, return selected entry in AL
; -----------------------------------------------------------------------------
show_menu:
    ; Initialize selected entry
    mov al, [cfg_default]
    mov [menu_selected], al

.redraw:
    ; 2a. Draw gradient background
    call draw_gradient

    ; 2b. If logo_sectors != 0, draw logo at 15% screen height
    cmp word [logo_sectors], 0
    je .no_logo

    ; logo_y = screen_height * 15 / 100
    movzx eax, word [0x2000 + 0x14]    ; screen height
    imul eax, 15
    xor edx, edx
    mov ecx, 100
    div ecx
    mov [logo_y], eax

    call draw_logo

.no_logo:
    ; 2c. Calculate entry list Y = logo_y + logo_h*4 + 32
    mov eax, [logo_y]
    mov ebx, [logo_h]
    shl ebx, 2                          ; logo_h * 4
    add eax, ebx
    add eax, 32
    mov [menu_entry_y], eax

    ; 2d. Draw each entry
    movzx ecx, byte [cfg_count]
    xor ebx, ebx                        ; index = 0

.entry_loop:
    cmp ebx, ecx
    jge .draw_footer

    push ecx
    push ebx

    ; Check if this is the selected entry
    cmp bl, [menu_selected]
    jne .not_selected

    ; Draw highlight bar
    push ebx
    mov ecx, [menu_entry_y]
    mov edx, MENU_BAR_COLOR
    call draw_menu_bar
    pop ebx

    ; Draw name in highlight color
    ; ESI = cfg_entries + index * 128
    movzx eax, bl
    shl eax, 7                          ; * 128
    add eax, cfg_entries
    mov esi, eax
    mov ecx, [menu_entry_y]
    add ecx, 2                          ; small vertical padding within bar
    mov edx, MENU_HIGHLIGHT_COLOR
    call draw_string_centered
    jmp .next_entry

.not_selected:
    ; Draw name in normal color
    movzx eax, bl
    shl eax, 7
    add eax, cfg_entries
    mov esi, eax
    mov ecx, [menu_entry_y]
    add ecx, 2
    mov edx, MENU_NORMAL_COLOR
    call draw_string_centered

.next_entry:
    ; Advance Y by 24 pixels
    add dword [menu_entry_y], 24

    pop ebx
    pop ecx
    inc ebx
    jmp .entry_loop

.draw_footer:
    ; Draw footer at bottom: screen_height - 32
    movzx ecx, word [0x2000 + 0x14]    ; screen height
    sub ecx, 32
    mov esi, menu_footer_str
    mov edx, MENU_FOOTER_COLOR
    call draw_string_centered

.wait_key:
    ; 3. Wait for keypress (blocking)
    mov ah, 0x00
    int 0x16
    ; AH = scan code

    cmp ah, 0x48                        ; Up arrow
    je .key_up
    cmp ah, 0x50                        ; Down arrow
    je .key_down
    cmp ah, 0x1C                        ; Enter
    je .key_enter
    jmp .wait_key                       ; ignore other keys

.key_up:
    cmp byte [menu_selected], 0
    je .redraw                          ; already at top
    dec byte [menu_selected]
    jmp .redraw

.key_down:
    mov al, [cfg_count]
    dec al
    cmp [menu_selected], al
    jge .redraw                         ; already at bottom
    inc byte [menu_selected]
    jmp .redraw

.key_enter:
    mov al, [menu_selected]
    ret

; -----------------------------------------------------------------------------
; draw_menu_bar - Draw highlight bar across full screen width
; Input: ECX = Y position, EDX = bar color
; -----------------------------------------------------------------------------
draw_menu_bar:
    pusha

    ; Get framebuffer info
    mov edi, [0x2000 + 0x28]           ; PhysBasePtr
    movzx eax, word [0x2000 + 0x10]   ; pitch
    mov [.pitch], eax
    movzx ebx, word [0x2000 + 0x12]   ; screen width

    ; Calculate start: fb + Y * pitch
    imul ecx, eax
    add edi, ecx

    ; Fill 20 rows
    mov ecx, 20
.row_loop:
    push ecx
    push edi
    mov ecx, ebx                        ; width in pixels
.pixel_loop:
    mov [edi], edx
    add edi, 4
    dec ecx
    jnz .pixel_loop
    pop edi
    add edi, [.pitch]
    pop ecx
    dec ecx
    jnz .row_loop

    popa
    ret

.pitch: dd 0

; -----------------------------------------------------------------------------
; Menu data
; -----------------------------------------------------------------------------
menu_selected:   db 0
menu_entry_y:    dd 0
menu_footer_str: db 'Up/Down Select   Enter Boot', 0

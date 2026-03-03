; =============================================================================
; boot.asm — Boot device helpers and bios_print utility
; =============================================================================

; ---------------------------------------------------------------------------
; bios_print — Print NUL-terminated string via INT 10h TTY.
;   Input: DS:SI = string pointer (caller must set DS = 0xF000 if needed).
;
;   This function uses direct VGA writes instead of INT 10h to avoid
;   recursion issues during early POST (before IVT is set up).
;   After IVT setup, callers can also use INT 10h directly.
; ---------------------------------------------------------------------------
bios_print:
    push ax
    push bx
    push si

.print_loop:
    ; Use CS: override to read from BIOS segment.
    cs lodsb
    test al, al
    jz .print_done
    mov ah, 0x0E
    mov bx, 0x0007             ; Page 0, light gray
    int 0x10
    jmp .print_loop

.print_done:
    pop si
    pop bx
    pop ax
    ret

; ---------------------------------------------------------------------------
; bios_print_dec16 — Print a 16-bit unsigned decimal number.
;   Input: AX = number to print.
; ---------------------------------------------------------------------------
bios_print_dec16:
    push ax
    push bx
    push cx
    push dx

    xor cx, cx                  ; Digit count
    mov bx, 10

.dec_div:
    xor dx, dx
    div bx                     ; AX = quotient, DX = remainder
    push dx                     ; Save digit
    inc cx
    test ax, ax
    jnz .dec_div

.dec_print:
    pop ax
    add al, '0'
    mov ah, 0x0E
    push bx
    mov bx, 0x0007
    int 0x10
    pop bx
    dec cx
    jnz .dec_print

    pop dx
    pop cx
    pop bx
    pop ax
    ret

; ---------------------------------------------------------------------------
; bios_print_hex16 — Print a 16-bit hex value (4 hex digits).
;   Input: AX = value.
; ---------------------------------------------------------------------------
bios_print_hex16:
    push ax
    mov al, ah          ; High byte first
    call bios_print_hex8
    pop ax
    call bios_print_hex8  ; Low byte (AL from original AX)
    ret

; ---------------------------------------------------------------------------
; bios_print_hex32 — Print a 32-bit hex value (8 hex digits).
;   Input: EAX = value.
; ---------------------------------------------------------------------------
bios_print_hex32:
    push eax
    shr eax, 16
    call bios_print_hex16   ; High 16 bits
    pop eax
    call bios_print_hex16   ; Low 16 bits (AX = lower 16 of original EAX)
    ret

; ---------------------------------------------------------------------------
; bios_print_hex8 — Print an 8-bit hex value.
;   Input: AL = value.
; ---------------------------------------------------------------------------
bios_print_hex8:
    push ax
    push bx
    push cx

    mov cl, al
    shr al, 4
    call .hex_nibble
    mov al, cl
    and al, 0x0F
    call .hex_nibble

    pop cx
    pop bx
    pop ax
    ret

.hex_nibble:
    cmp al, 10
    jb .hex_digit
    add al, 'A' - 10
    jmp .hex_out
.hex_digit:
    add al, '0'
.hex_out:
    mov ah, 0x0E
    mov bx, 0x0007
    int 0x10
    ret

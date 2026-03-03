; =============================================================================
; int16h.asm — INT 16h: Keyboard BIOS services
; =============================================================================

int16h_handler:
    cmp ah, 0x00
    je .wait_key
    cmp ah, 0x01
    je .check_key
    cmp ah, 0x02
    je .get_shift_flags
    cmp ah, 0x10
    je .wait_key              ; Enhanced: same as 00h for us
    cmp ah, 0x11
    je .check_key             ; Enhanced: same as 01h
    cmp ah, 0x12
    je .get_ext_shift_flags
    ; Unsupported.
    iret

; ---------------------------------------------------------------------------
; AH=00h/10h: Wait for keypress.
;   Returns: AH = scan code, AL = ASCII code.
; ---------------------------------------------------------------------------
.wait_key:
    push ds
    push bx
    xor bx, bx
    mov ds, bx

.wait_loop:
    cli
    mov bx, [BDA_KBD_BUF_HEAD]
    cmp bx, [BDA_KBD_BUF_TAIL]
    jne .key_available
    sti
    hlt                         ; Wait for keyboard IRQ
    jmp .wait_loop

.key_available:
    ; Read key from buffer.
    mov ax, [bx + 0x0400]      ; BDA buffer is at 0x400 + offset
    ; Advance head pointer.
    add bx, 2
    cmp bx, [BDA_KBD_BUF_END]
    jb .head_ok
    mov bx, [BDA_KBD_BUF_START]
.head_ok:
    mov [BDA_KBD_BUF_HEAD], bx
    sti

    pop bx
    pop ds
    iret

; ---------------------------------------------------------------------------
; AH=01h/11h: Check keyboard buffer (non-blocking).
;   Returns: ZF=1 if empty, ZF=0 if key available (AX = key, not removed).
; ---------------------------------------------------------------------------
.check_key:
    push ds
    push bx
    xor bx, bx
    mov ds, bx

    cli
    mov bx, [BDA_KBD_BUF_HEAD]
    cmp bx, [BDA_KBD_BUF_TAIL]
    je .buf_empty

    ; Key available: peek without removing.
    mov ax, [bx + 0x0400]
    sti

    pop bx
    pop ds
    ; Clear ZF to indicate key available.
    push bp
    mov bp, sp
    and word [bp + 6], ~0x0040  ; Clear ZF in saved FLAGS on stack
    pop bp
    iret

.buf_empty:
    sti
    pop bx
    pop ds
    ; Set ZF to indicate empty.
    push bp
    mov bp, sp
    or word [bp + 6], 0x0040    ; Set ZF in saved FLAGS on stack
    pop bp
    iret

; ---------------------------------------------------------------------------
; AH=02h: Get shift flags.
;   Returns: AL = shift flags byte 1.
; ---------------------------------------------------------------------------
.get_shift_flags:
    push ds
    xor ax, ax
    mov ds, ax
    mov al, [BDA_KBD_FLAGS1]
    pop ds
    iret

; ---------------------------------------------------------------------------
; AH=12h: Get extended shift flags.
;   Returns: AL = flags byte 1, AH = flags byte 2.
; ---------------------------------------------------------------------------
.get_ext_shift_flags:
    push ds
    xor ax, ax
    mov ds, ax
    mov al, [BDA_KBD_FLAGS1]
    mov ah, [BDA_KBD_FLAGS2]
    pop ds
    iret

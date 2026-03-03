; =============================================================================
; int_misc.asm — Miscellaneous interrupt handlers
; =============================================================================

; ---------------------------------------------------------------------------
; INT 08h: Timer tick handler (IRQ 0).
; ---------------------------------------------------------------------------
timer_tick_handler:
    push ds
    push ax

    xor ax, ax
    mov ds, ax

    inc dword [BDA_TIMER_COUNT]

    cmp dword [BDA_TIMER_COUNT], 1573040
    jb .no_midnight
    mov dword [BDA_TIMER_COUNT], 0
    mov byte [BDA_TIMER_OVERFLOW], 1
.no_midnight:

    mov al, 0x20
    out PIC1_CMD, al

    pop ax
    pop ds
    iret

; ---------------------------------------------------------------------------
; INT 09h: Keyboard interrupt handler (IRQ 1).
; ---------------------------------------------------------------------------
keyboard_irq_handler:
    push ds
    push ax
    push bx
    push cx

    xor ax, ax
    mov ds, ax

    ; Read scancode from PS/2 controller.
    in al, 0x60

    ; Ignore break codes (key release, bit 7 set).
    test al, 0x80
    jnz .eoi

    ; Translate scancode to ASCII via table in BIOS segment (CS: override).
    movzx bx, al               ; BX = scancode index
    cmp bx, scancode_table_size
    jae .eoi

    mov ah, al                  ; AH = scan code
    mov al, [cs:scancode_table + bx]    ; AL = ASCII

    ; If ASCII is 0, skip (non-printable/unmapped key).
    test al, al
    jz .eoi

    ; Store in keyboard buffer (AH=scancode, AL=ASCII).
    mov bx, [BDA_KBD_BUF_TAIL]
    mov cx, bx
    add cx, 2
    cmp cx, [BDA_KBD_BUF_END]
    jb .no_wrap
    mov cx, [BDA_KBD_BUF_START]
.no_wrap:
    cmp cx, [BDA_KBD_BUF_HEAD]
    je .eoi                     ; Buffer full

    mov [bx + 0x0400], ax
    mov [BDA_KBD_BUF_TAIL], cx

.eoi:
    mov al, 0x20
    out PIC1_CMD, al

    pop cx
    pop bx
    pop ax
    pop ds
    iret

; Scancode set 1 to ASCII translation table (US keyboard layout).
scancode_table:
    db 0                        ; 0x00: none
    db 27                       ; 0x01: Escape
    db '1','2','3','4','5','6','7','8','9','0'  ; 0x02-0x0B
    db '-','='                  ; 0x0C-0x0D
    db 8                        ; 0x0E: Backspace
    db 9                        ; 0x0F: Tab
    db 'q','w','e','r','t','y','u','i','o','p'  ; 0x10-0x19
    db '[',']'                  ; 0x1A-0x1B
    db 13                       ; 0x1C: Enter
    db 0                        ; 0x1D: Left Ctrl
    db 'a','s','d','f','g','h','j','k','l'      ; 0x1E-0x26
    db ';',"'"                  ; 0x27-0x28
    db '`'                      ; 0x29: Backtick
    db 0                        ; 0x2A: Left Shift
    db '\'                      ; 0x2B: Backslash
    db 'z','x','c','v','b','n','m'              ; 0x2C-0x32
    db ',','.','/'              ; 0x33-0x35
    db 0                        ; 0x36: Right Shift
    db '*'                      ; 0x37: Keypad *
    db 0                        ; 0x38: Left Alt
    db ' '                      ; 0x39: Space
    db 0                        ; 0x3A: Caps Lock
    times 10 db 0               ; 0x3B-0x44: F1-F10
    times 15 db 0               ; 0x45-0x53: misc keys
scancode_table_size equ ($ - scancode_table)

; ---------------------------------------------------------------------------
; INT 11h: Get equipment list.
; ---------------------------------------------------------------------------
int11h_handler:
    push ds
    xor ax, ax
    mov ds, ax
    mov ax, [BDA_EQUIP]
    pop ds
    iret

; ---------------------------------------------------------------------------
; INT 12h: Get conventional memory size.
; ---------------------------------------------------------------------------
int12h_handler:
    push ds
    xor ax, ax
    mov ds, ax
    mov ax, [BDA_CONV_MEM_KB]
    pop ds
    iret

; ---------------------------------------------------------------------------
; INT 14h: Serial port services.
; ---------------------------------------------------------------------------
int14h_handler:
    cmp ah, 0x00
    je .init
    cmp ah, 0x01
    je .write
    cmp ah, 0x02
    je .read
    cmp ah, 0x03
    je .status
    iret

.init:
    mov ah, 0x60
    mov al, 0x00
    iret

.write:
    push dx
    mov dx, COM1_BASE + 5
.sw_wait:
    push ax
    in al, dx
    test al, 0x20
    pop ax
    jz .sw_wait
    mov dx, COM1_BASE
    out dx, al
    pop dx
    mov ah, 0x60
    iret

.read:
    push dx
    mov dx, COM1_BASE + 5
.sr_wait:
    in al, dx
    test al, 0x01
    jz .sr_wait
    mov dx, COM1_BASE
    in al, dx
    pop dx
    mov ah, 0x00
    iret

.status:
    push dx
    mov dx, COM1_BASE + 5
    in al, dx
    mov ah, al
    mov dx, COM1_BASE + 6
    in al, dx
    pop dx
    iret

; ---------------------------------------------------------------------------
; INT 76h: IDE primary IRQ handler (IRQ 14).
; ---------------------------------------------------------------------------
ide_irq_handler:
    push ax
    push dx
    mov dx, IDE_STATUS
    in al, dx
    mov al, 0x20
    out PIC2_CMD, al
    out PIC1_CMD, al
    pop dx
    pop ax
    iret

; ---------------------------------------------------------------------------
; Generic dummy handlers.
; ---------------------------------------------------------------------------
iret_handler:
    iret

eoi_iret:
    push ax
    mov al, 0x20
    out PIC1_CMD, al
    pop ax
    iret

slave_eoi_iret:
    push ax
    mov al, 0x20
    out PIC2_CMD, al
    out PIC1_CMD, al
    pop ax
    iret

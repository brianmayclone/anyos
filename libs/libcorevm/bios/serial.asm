; =============================================================================
; serial.asm — COM1 (16550 UART) initialization
; =============================================================================

COM1_BASE   equ 0x03F8

; ---------------------------------------------------------------------------
; serial_init — Initialize COM1 at 9600 baud, 8N1.
; ---------------------------------------------------------------------------
serial_init:
    ; Disable interrupts on UART.
    mov dx, COM1_BASE + 1       ; IER
    xor al, al
    out dx, al

    ; Enable DLAB (set baud rate divisor).
    mov dx, COM1_BASE + 3       ; LCR
    mov al, 0x80
    out dx, al

    ; Set divisor to 12 (9600 baud: 115200 / 9600 = 12).
    mov dx, COM1_BASE + 0       ; DLL (DLAB=1)
    mov al, 12
    out dx, al
    mov dx, COM1_BASE + 1       ; DLM (DLAB=1)
    xor al, al
    out dx, al

    ; 8 bits, no parity, 1 stop bit (clear DLAB).
    mov dx, COM1_BASE + 3       ; LCR
    mov al, 0x03
    out dx, al

    ; Enable FIFO, clear buffers, 14-byte threshold.
    mov dx, COM1_BASE + 2       ; FCR
    mov al, 0xC7
    out dx, al

    ; Set DTR + RTS + OUT2.
    mov dx, COM1_BASE + 4       ; MCR
    mov al, 0x0B
    out dx, al

    ret

; ---------------------------------------------------------------------------
; serial_putchar — Write character in AL to COM1.
; ---------------------------------------------------------------------------
serial_putchar:
    push dx
    push ax
    mov ah, al                  ; Save char
    mov dx, COM1_BASE + 5       ; LSR
.wait:
    in al, dx
    test al, 0x20               ; THR empty?
    jz .wait
    mov dx, COM1_BASE           ; THR
    mov al, ah
    out dx, al
    pop ax
    pop dx
    ret

; ---------------------------------------------------------------------------
; serial_puts — Write NUL-terminated string at DS:SI to COM1.
; ---------------------------------------------------------------------------
serial_puts:
    push si
    push ax
.loop:
    lodsb
    test al, al
    jz .done
    call serial_putchar
    jmp .loop
.done:
    pop ax
    pop si
    ret

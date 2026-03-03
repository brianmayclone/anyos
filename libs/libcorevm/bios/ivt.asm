; =============================================================================
; ivt.asm — Interrupt Vector Table setup
; =============================================================================

BIOS_SEG    equ 0xF000

; ---------------------------------------------------------------------------
; ivt_setup — Install all interrupt vectors at 0x0000:0x0000.
; ---------------------------------------------------------------------------
ivt_setup:
    push es
    push di
    push ax
    push cx

    ; First, fill all 256 vectors with the dummy iret handler.
    xor ax, ax
    mov es, ax
    xor di, di
    mov cx, 256
.fill_default:
    mov word [es:di], iret_handler
    mov word [es:di + 2], BIOS_SEG
    add di, 4
    loop .fill_default

    ; Now install specific handlers.

    ; CPU exceptions 0x00-0x07 → iret_handler (already set).

    ; IRQ handlers (master PIC: INT 08h-0Fh).
    mov di, 0x08 * 4
    mov word [es:di], timer_tick_handler
    mov word [es:di + 2], BIOS_SEG

    mov di, 0x09 * 4
    mov word [es:di], keyboard_irq_handler
    mov word [es:di + 2], BIOS_SEG

    ; IRQ 2 (cascade) → eoi_iret.
    mov di, 0x0A * 4
    mov word [es:di], eoi_iret
    mov word [es:di + 2], BIOS_SEG

    ; IRQ 3-7 → eoi_iret.
    mov cx, 5
    mov di, 0x0B * 4
.master_irqs:
    mov word [es:di], eoi_iret
    mov word [es:di + 2], BIOS_SEG
    add di, 4
    loop .master_irqs

    ; BIOS service interrupts.
    mov di, 0x10 * 4
    mov word [es:di], int10h_handler
    mov word [es:di + 2], BIOS_SEG

    mov di, 0x11 * 4
    mov word [es:di], int11h_handler
    mov word [es:di + 2], BIOS_SEG

    mov di, 0x12 * 4
    mov word [es:di], int12h_handler
    mov word [es:di + 2], BIOS_SEG

    mov di, 0x13 * 4
    mov word [es:di], int13h_handler
    mov word [es:di + 2], BIOS_SEG

    mov di, 0x14 * 4
    mov word [es:di], int14h_handler
    mov word [es:di + 2], BIOS_SEG

    mov di, 0x15 * 4
    mov word [es:di], int15h_handler
    mov word [es:di + 2], BIOS_SEG

    mov di, 0x16 * 4
    mov word [es:di], int16h_handler
    mov word [es:di + 2], BIOS_SEG

    mov di, 0x19 * 4
    mov word [es:di], int19h_handler
    mov word [es:di + 2], BIOS_SEG

    mov di, 0x1A * 4
    mov word [es:di], int1ah_handler
    mov word [es:di + 2], BIOS_SEG

    ; Slave PIC IRQs: INT 70h-77h → slave_eoi_iret.
    mov di, 0x70 * 4
    mov cx, 8
.slave_irqs:
    mov word [es:di], slave_eoi_iret
    mov word [es:di + 2], BIOS_SEG
    add di, 4
    loop .slave_irqs

    ; IDE primary IRQ (IRQ 14 = INT 76h on slave PIC, or INT 0Eh remapped).
    ; With our PIC mapping: IRQ 14 → INT 70h + 6 = INT 76h.
    mov di, 0x76 * 4
    mov word [es:di], ide_irq_handler
    mov word [es:di + 2], BIOS_SEG

    pop cx
    pop ax
    pop di
    pop es
    ret

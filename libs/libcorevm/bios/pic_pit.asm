; =============================================================================
; pic_pit.asm — PIC (8259A) remapping and PIT (8254) initialization
; =============================================================================

; PIC ports.
PIC1_CMD    equ 0x20
PIC1_DATA   equ 0x21
PIC2_CMD    equ 0xA0
PIC2_DATA   equ 0xA1

; PIT ports.
PIT_CH0     equ 0x40
PIT_CMD     equ 0x43

; ---------------------------------------------------------------------------
; pic_init — Remap the dual 8259A PIC.
;   Master: IRQ 0-7  -> INT 08h-0Fh
;   Slave:  IRQ 8-15 -> INT 70h-77h
; ---------------------------------------------------------------------------
pic_init:
    ; ICW1: edge-triggered, cascade, ICW4 needed.
    mov al, 0x11
    out PIC1_CMD, al
    out PIC2_CMD, al

    ; ICW2: base interrupt vectors.
    mov al, 0x08            ; Master: INT 08h
    out PIC1_DATA, al
    mov al, 0x70            ; Slave: INT 70h
    out PIC2_DATA, al

    ; ICW3: cascade wiring.
    mov al, 0x04            ; Master: slave on IRQ 2
    out PIC1_DATA, al
    mov al, 0x02            ; Slave: cascade identity 2
    out PIC2_DATA, al

    ; ICW4: 8086 mode, manual EOI.
    mov al, 0x01
    out PIC1_DATA, al
    out PIC2_DATA, al

    ; OCW1: mask all except IRQ 0 (timer), IRQ 1 (keyboard), IRQ 2 (cascade).
    mov al, 0xF8            ; 1111_1000 = unmask IRQ 0,1,2
    out PIC1_DATA, al

    ; Slave: mask all initially.
    mov al, 0xFF
    out PIC2_DATA, al

    ret

; ---------------------------------------------------------------------------
; pic_unmask_irq14 — Unmask IRQ 14 (IDE primary) on slave PIC.
; ---------------------------------------------------------------------------
pic_unmask_irq14:
    in al, PIC2_DATA
    and al, 0xBF            ; Unmask IRQ 14 (slave bit 6)
    out PIC2_DATA, al
    ; Also unmask cascade (IRQ 2) on master if not already.
    in al, PIC1_DATA
    and al, 0xFB            ; Unmask IRQ 2
    out PIC1_DATA, al
    ret

; ---------------------------------------------------------------------------
; pit_init — Program PIT channel 0 as rate generator (~18.2 Hz).
; ---------------------------------------------------------------------------
pit_init:
    ; Channel 0, access lobyte/hibyte, mode 2 (rate generator), binary.
    mov al, 0x34            ; 0011_0100
    out PIT_CMD, al

    ; Divisor = 65536 (0x0000 wraps to 65536) => 1193182/65536 ~ 18.2 Hz.
    mov al, 0x00
    out PIT_CH0, al         ; Low byte
    out PIT_CH0, al         ; High byte
    ret

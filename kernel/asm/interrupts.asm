; =============================================================================
; interrupts.asm - ISR and IRQ stub entries for .anyOS kernel
; =============================================================================
; These stubs save CPU state and call Rust handlers:
;   isr_handler(frame: &InterruptFrame)
;   irq_handler(frame: &InterruptFrame)
; =============================================================================

[BITS 32]

; Rust handlers (defined in kernel/src/arch/x86/idt.rs)
extern isr_handler
extern irq_handler

; =============================================================================
; ISR stubs - CPU Exceptions (INT 0-31)
; =============================================================================

; Macro for exceptions that do NOT push an error code
%macro ISR_NOERRCODE 1
global isr%1
isr%1:
    push dword 0            ; Push dummy error code
    push dword %1           ; Push interrupt number
    jmp isr_common_stub
%endmacro

; Macro for exceptions that DO push an error code automatically
%macro ISR_ERRCODE 1
global isr%1
isr%1:
    ; Error code already pushed by CPU
    push dword %1           ; Push interrupt number
    jmp isr_common_stub
%endmacro

; CPU Exceptions
ISR_NOERRCODE 0     ; #DE Divide Error
ISR_NOERRCODE 1     ; #DB Debug Exception
ISR_NOERRCODE 2     ; NMI
ISR_NOERRCODE 3     ; #BP Breakpoint
ISR_NOERRCODE 4     ; #OF Overflow
ISR_NOERRCODE 5     ; #BR Bound Range Exceeded
ISR_NOERRCODE 6     ; #UD Invalid Opcode
ISR_NOERRCODE 7     ; #NM Device Not Available
ISR_ERRCODE   8     ; #DF Double Fault
ISR_NOERRCODE 9     ; Coprocessor Segment Overrun (legacy)
ISR_ERRCODE   10    ; #TS Invalid TSS
ISR_ERRCODE   11    ; #NP Segment Not Present
ISR_ERRCODE   12    ; #SS Stack-Segment Fault
ISR_ERRCODE   13    ; #GP General Protection Fault
ISR_ERRCODE   14    ; #PF Page Fault
ISR_NOERRCODE 15    ; Reserved
ISR_NOERRCODE 16    ; #MF x87 FP Exception
ISR_ERRCODE   17    ; #AC Alignment Check
ISR_NOERRCODE 18    ; #MC Machine Check
ISR_NOERRCODE 19    ; #XM SIMD FP Exception
ISR_NOERRCODE 20    ; #VE Virtualization Exception
ISR_ERRCODE   21    ; #CP Control Protection Exception
ISR_NOERRCODE 22    ; Reserved
ISR_NOERRCODE 23    ; Reserved
ISR_NOERRCODE 24    ; Reserved
ISR_NOERRCODE 25    ; Reserved
ISR_NOERRCODE 26    ; Reserved
ISR_NOERRCODE 27    ; Reserved
ISR_NOERRCODE 28    ; Reserved
ISR_NOERRCODE 29    ; Reserved
ISR_NOERRCODE 30    ; Reserved
ISR_NOERRCODE 31    ; Reserved

; =============================================================================
; IRQ stubs - Hardware Interrupts (INT 32-47)
; =============================================================================
%macro IRQ 2
global irq%1
irq%1:
    push dword 0            ; Dummy error code
    push dword %2           ; Interrupt number (32 + IRQ#)
    jmp irq_common_stub
%endmacro

IRQ 0,  32      ; PIT Timer
IRQ 1,  33      ; Keyboard
IRQ 2,  34      ; Cascade
IRQ 3,  35      ; COM2
IRQ 4,  36      ; COM1
IRQ 5,  37      ; LPT2
IRQ 6,  38      ; Floppy
IRQ 7,  39      ; LPT1 / Spurious
IRQ 8,  40      ; CMOS RTC
IRQ 9,  41      ; Free / ACPI
IRQ 10, 42      ; Free
IRQ 11, 43      ; Free
IRQ 12, 44      ; PS/2 Mouse
IRQ 13, 45      ; FPU / Coprocessor
IRQ 14, 46      ; Primary ATA
IRQ 15, 47      ; Secondary ATA

; LAPIC / APIC vectors (INT 48-55)
IRQ 16, 48      ; LAPIC Timer
IRQ 17, 49      ; Reserved
IRQ 18, 50      ; Reserved
IRQ 19, 51      ; Reserved
IRQ 20, 52      ; IPI: TLB shootdown
IRQ 21, 53      ; IPI: Halt
IRQ 22, 54      ; Reserved
IRQ 23, 55      ; LAPIC Spurious

; =============================================================================
; Common ISR stub - saves state, calls Rust isr_handler, restores state
; =============================================================================
isr_common_stub:
    ; Save all general-purpose registers
    pushad

    ; Save segment registers
    push ds
    push es
    push fs
    push gs

    ; Load kernel data segment
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    ; Pass pointer to stack frame as argument
    push esp
    call isr_handler
    add esp, 4

    ; Restore segment registers
    pop gs
    pop fs
    pop es
    pop ds

    ; Restore general-purpose registers
    popad

    ; Remove interrupt number and error code from stack
    add esp, 8

    ; Return from interrupt
    iretd

; =============================================================================
; Common IRQ stub - saves state, calls Rust irq_handler, restores state
; =============================================================================
irq_common_stub:
    pushad

    push ds
    push es
    push fs
    push gs

    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    push esp
    call irq_handler
    add esp, 4

    pop gs
    pop fs
    pop es
    pop ds

    popad
    add esp, 8
    iretd

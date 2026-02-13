; =============================================================================
; interrupts.asm - ISR and IRQ stub entries for .anyOS kernel (x86-64)
; =============================================================================
; These stubs save CPU state and call Rust handlers:
;   isr_handler(frame: &InterruptFrame)   — arg in RDI (System V ABI)
;   irq_handler(frame: &InterruptFrame)   — arg in RDI (System V ABI)
;
; In 64-bit mode there is no pushad/popad; all GPRs are pushed individually.
; The CPU always pushes SS and RSP on interrupt (even same-privilege).
; =============================================================================

[BITS 64]

; Rust handlers (defined in kernel/src/arch/x86/idt.rs)
extern isr_handler
extern irq_handler
extern bad_rsp_recovery

; =============================================================================
; ISR stubs - CPU Exceptions (INT 0-31)
; =============================================================================

; Macro for exceptions that do NOT push an error code
%macro ISR_NOERRCODE 1
global isr%1
isr%1:
    push qword 0            ; Push dummy error code
    push qword %1           ; Push interrupt number
    jmp isr_common_stub
%endmacro

; Macro for exceptions that DO push an error code automatically
%macro ISR_ERRCODE 1
global isr%1
isr%1:
    ; Error code already pushed by CPU (64-bit)
    push qword %1           ; Push interrupt number
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
; IRQ stubs - Hardware Interrupts (INT 32-55)
; =============================================================================
%macro IRQ 2
global irq%1
irq%1:
    push qword 0            ; Dummy error code
    push qword %2           ; Interrupt number (32 + IRQ#)
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
; Common ISR stub - saves all GPRs, calls Rust isr_handler, restores state
; =============================================================================
isr_common_stub:
    ; Save all general-purpose registers (no pushad in 64-bit mode)
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    ; Safety net: validate RSP is in kernel higher-half (bit 63 set).
    ; On Ring 3→0 transitions the CPU loads RSP from TSS.RSP0. If RSP0
    ; was transiently corrupt (small positive value), the CPU and our
    ; pushes above wrote into identity-mapped low memory — corrupting
    ; BIOS data, page tables, or AP trampoline. Detect and halt NOW
    ; before the Rust handler causes more damage.
    test rsp, rsp
    jns .bad_rsp

    ; Load kernel data segment (needed when entering from compat mode)
    mov ax, 0x10
    mov ds, ax
    mov es, ax

    ; Pass pointer to InterruptFrame as first arg (System V ABI: RDI)
    mov rdi, rsp
    call isr_handler

    ; Restore all general-purpose registers
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax

    ; Remove interrupt number and error code from stack
    add rsp, 16

    ; Return from interrupt (64-bit IRET)
    iretq

.bad_rsp:
    cli
    ; Write "!ISR RSP\n" to serial (0x3F8) — lock-free, no stack needed
    mov dx, 0x3F8
    mov al, '!'
    out dx, al
    mov al, 'I'
    out dx, al
    mov al, 'S'
    out dx, al
    mov al, 'R'
    out dx, al
    mov al, ' '
    out dx, al
    mov al, 'R'
    out dx, al
    mov al, 'S'
    out dx, al
    mov al, 'P'
    out dx, al
    mov al, 10
    out dx, al
    ; --- RECOVERY: switch to valid kernel stack and call Rust handler ---
    ; This fires on Ring 3→0 transition with corrupt TSS.RSP0.
    ; At this point: KERNEL_GS_BASE = PERCPU (not swapped yet).
    ; SWAPGS gives us access to PERCPU.kernel_rsp at [gs:0].
    swapgs
    mov rsp, [gs:0]             ; RSP = per-CPU kernel_rsp (idle thread stack)
    swapgs                      ; restore GS for normal kernel operation
    ; Validate the loaded RSP is in kernel higher-half
    test rsp, rsp
    jns .recovery_failed
    ; Set up kernel data segments
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    ; Call Rust recovery: kills current thread, sends EOI, enters idle loop.
    ; bad_rsp_recovery() is a divergent function (never returns).
    call bad_rsp_recovery
.recovery_failed:
    hlt
    jmp .recovery_failed

; =============================================================================
; Common IRQ stub - saves all GPRs, calls Rust irq_handler, restores state
; =============================================================================
irq_common_stub:
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    ; Safety net: same RSP validation as ISR stub (see comment above)
    test rsp, rsp
    jns .bad_rsp

    mov ax, 0x10
    mov ds, ax
    mov es, ax

    mov rdi, rsp
    call irq_handler

    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax

    add rsp, 16
    iretq

.bad_rsp:
    cli
    ; Write "!IRQ RSP\n" to serial (0x3F8) — lock-free
    mov dx, 0x3F8
    mov al, '!'
    out dx, al
    mov al, 'I'
    out dx, al
    mov al, 'R'
    out dx, al
    mov al, 'Q'
    out dx, al
    mov al, ' '
    out dx, al
    mov al, 'R'
    out dx, al
    mov al, 'S'
    out dx, al
    mov al, 'P'
    out dx, al
    mov al, 10
    out dx, al
    ; --- RECOVERY: same as ISR bad_rsp above ---
    swapgs
    mov rsp, [gs:0]             ; RSP = per-CPU kernel_rsp
    swapgs
    test rsp, rsp
    jns .irq_recovery_failed
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    call bad_rsp_recovery
.irq_recovery_failed:
    hlt
    jmp .irq_recovery_failed

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
extern bad_kernel_iret_recovery
extern LAPIC_TO_PERCPU

; Global variable to save the corrupt RSP for diagnostics.
; Defined in Rust (scheduler.rs) as #[no_mangle] pub static.
extern BAD_RSP_SAVED

; Saved DS/ES from ISR/IRQ entry — for #GP diagnostics.
; Defined in Rust (idt.rs) as #[no_mangle] pub static mut.
extern SAVED_FAULT_DS
extern SAVED_FAULT_ES

; LAPIC virtual address: LAPIC_VIRT_BASE(0xFFFFFFFFD0100000) + LAPIC_ID(0x20)
%define LAPIC_ID_ADDR 0xFFFFFFFFD0100020

%macro SWITCH_TO_RECOVERY_STACK 1
    ; Save the bad stack pointer before replacing it. This macro deliberately
    ; does not use the current stack: it is used when RSP may point at user
    ; memory or an unmapped guard page.
    mov [rel BAD_RSP_SAVED], rsp

    ; Resolve the current CPU's PERCPU block through the LAPIC lookup table
    ; instead of using SWAPGS. Interrupts can arrive with either user-GS or
    ; kernel-GS active, so blindly swapping here can select the wrong base.
    mov rax, LAPIC_ID_ADDR
    mov eax, [rax]
    shr eax, 24
    movzx eax, al
    lea rbx, [rel LAPIC_TO_PERCPU]
    mov rbx, [rbx + rax*8]
    test rbx, rbx
    jz %1

    mov rsp, [rbx]                 ; PERCPU.kernel_rsp
    test rsp, rsp
    jns %1
%endmacro

%macro VALIDATE_KERNEL_IRET 1
    ; At this point the stack top is the CPU IRETQ frame:
    ;   [rsp+0]=RIP [rsp+8]=CS [rsp+16]=RFLAGS [rsp+24]=RSP [rsp+32]=SS
    ; If a same-ring kernel return frame is corrupt, do not let IRETQ jump
    ; into low identity memory (e.g. RIP=0x3). Hand the exact frame to Rust
    ; while it is still intact.
    push rax
    push rcx
    mov rcx, [rsp + 24]           ; original CS after two pushes
    test rcx, 3
    jnz %%ok                      ; user return gets normal sanitising below
    mov rax, [rsp + 16]           ; original RIP after two pushes
    mov rcx, 0xFFFFFFFF80100000
    cmp rax, rcx
    jb %%bad
    mov rcx, 0xFFFFFFFF82000000
    cmp rax, rcx
    jae %%bad
%%ok:
    pop rcx
    pop rax
    jmp %%done
%%bad:
    cli
    lea rdi, [rsp + 16]           ; pointer to original IRETQ frame
    mov esi, %1                   ; 0=ISR, 1=IRQ
    call bad_kernel_iret_recovery
%%halt:
    hlt
    jmp %%halt
%%done:
%endmacro

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
IRQ 22, 54      ; IPI: Reschedule
IRQ 23, 55      ; Reserved APIC slot

; MSI slots (vectors 56-87) — all route through irq_common_stub
; so PCI MSI devices beyond the first two don't fault with #GP (missing IDT
; gate) the moment they try to fire an interrupt.
%assign n 24
%rep 32
    IRQ n, (32 + n)
    %assign n n+1
%endrep

; =============================================================================
; Common ISR stub - saves all GPRs, calls Rust isr_handler, restores state
; =============================================================================
isr_common_stub:
    ; If the CPU entered this stub on a user/low stack, do not push anything.
    ; A push at RSP=...f000 would fault at RSP-8 and turn the original fault
    ; into a double fault before Rust can recover.
    test rsp, rsp
    jns .bad_rsp

    ; Interrupts and exceptions do not perform SWAPGS for us. If this frame
    ; came from ring 3, GS.base is still the user's GS (usually 0). Normalize
    ; to kernel PERCPU before any Rust code or scheduler path can run.
    test qword [rsp + 24], 3       ; common frame: [int,err,rip,cs,...]
    jz .isr_gs_ready
    swapgs
.isr_gs_ready:

    ; Capture DS/ES BEFORE overwriting — for #GP diagnostics.
    ; In 64-bit long mode, DS base is forced to 0, so RIP-relative
    ; addressing works even when DS holds a null selector (0x0000).
    push rax
    mov ax, ds
    mov word [rel SAVED_FAULT_DS], ax
    mov ax, es
    mov word [rel SAVED_FAULT_ES], ax
    pop rax

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

    VALIDATE_KERNEL_IRET 0

    ; Restore user data segment and sanitise SS before IRETQ when returning to ring 3.
    ; The entry code sets DS/ES to kernel 0x10; IRETQ does NOT restore DS/ES.
    ; If we leave DS=0x10 (DPL=0), the CPU nulls DS on the CPL 0→3 transition,
    ; causing #GP(0) on the first user-mode memory access (affects 32-bit compat procs).
    ; Also fix SS.RPL for VirtualBox NEM/Hyper-V (see irq_common_stub comment).
    test qword [rsp + 8], 3        ; check CS.RPL (bits 0-1)
    jz .isr_iret_done              ; kernel return (RPL=0) — no fix needed
    cli
    push rax
    mov ax, 0x23                   ; user data segment (GDT entry 4 | RPL=3)
    mov ds, ax
    mov es, ax
    pop rax
    or qword [rsp + 32], 3         ; force SS.RPL = 3 for user-mode return
    swapgs                         ; restore user GS before IRETQ
.isr_iret_done:
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
    SWITCH_TO_RECOVERY_STACK .recovery_failed
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
    ; Same early guard as ISR: recover before the first push can fault on a
    ; user/low stack.
    test rsp, rsp
    jns .bad_rsp

    ; Hardware IRQs from ring 3 also arrive with user GS still loaded.
    ; Switch to kernel PERCPU for the whole handler/scheduler residency.
    test qword [rsp + 24], 3       ; common frame: [int,err,rip,cs,...]
    jz .irq_gs_ready
    swapgs
.irq_gs_ready:

    ; Capture DS/ES BEFORE overwriting (same as ISR stub)
    push rax
    mov ax, ds
    mov word [rel SAVED_FAULT_DS], ax
    mov ax, es
    mov word [rel SAVED_FAULT_ES], ax
    pop rax

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

    VALIDATE_KERNEL_IRET 1

    ; Restore user data segment and sanitise SS before IRETQ (same fix as isr_common_stub).
    test qword [rsp + 8], 3        ; check CS.RPL
    jz .irq_iret_done
    cli
    push rax
    mov ax, 0x23                   ; user data segment (GDT entry 4 | RPL=3)
    mov ds, ax
    mov es, ax
    pop rax
    or qword [rsp + 32], 3         ; force SS.RPL = 3
    swapgs                         ; restore user GS before IRETQ
.irq_iret_done:
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
    SWITCH_TO_RECOVERY_STACK .irq_recovery_failed
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    call bad_rsp_recovery
.irq_recovery_failed:
    hlt
    jmp .irq_recovery_failed

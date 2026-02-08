; =============================================================================
; protected_mode.asm - GDT setup and switch to 32-bit protected mode
; =============================================================================

; =============================================================================
; GDT for protected mode
; =============================================================================
ALIGN 8
pm_gdt:
    ; Entry 0: Null descriptor (required)
    dq 0

    ; Entry 1 (0x08): Kernel Code Segment
    ; Base=0, Limit=0xFFFFF (4GiB with 4K granularity), 32-bit, ring 0, execute/read
    dw 0xFFFF               ; Limit low (bits 0-15)
    dw 0x0000               ; Base low (bits 0-15)
    db 0x00                 ; Base mid (bits 16-23)
    db 10011010b            ; Access: Present=1, DPL=00, S=1, Type=1010 (code, exec/read)
    db 11001111b            ; Flags: G=1(4K), D=1(32-bit), L=0, AVL=0 | Limit high=0xF
    db 0x00                 ; Base high (bits 24-31)

    ; Entry 2 (0x10): Kernel Data Segment
    ; Base=0, Limit=0xFFFFF (4GiB with 4K granularity), 32-bit, ring 0, read/write
    dw 0xFFFF
    dw 0x0000
    db 0x00
    db 10010010b            ; Access: Present=1, DPL=00, S=1, Type=0010 (data, read/write)
    db 11001111b
    db 0x00

    ; Entry 3 (0x18): User Code Segment (for later, ring 3)
    dw 0xFFFF
    dw 0x0000
    db 0x00
    db 11111010b            ; Access: Present=1, DPL=11, S=1, Type=1010 (code, exec/read)
    db 11001111b
    db 0x00

    ; Entry 4 (0x20): User Data Segment (for later, ring 3)
    dw 0xFFFF
    dw 0x0000
    db 0x00
    db 11110010b            ; Access: Present=1, DPL=11, S=1, Type=0010 (data, read/write)
    db 11001111b
    db 0x00

    ; Entry 5 (0x28): TSS (filled later by kernel)
    dq 0
pm_gdt_end:

pm_gdt_descriptor:
    dw pm_gdt_end - pm_gdt - 1     ; Size of GDT - 1
    dd pm_gdt                        ; Linear address of GDT

; Segment selectors
KERNEL_CODE_SEG equ 0x08
KERNEL_DATA_SEG equ 0x10

; =============================================================================
; enter_protected_mode - Switch from real mode to 32-bit protected mode
; =============================================================================
enter_protected_mode:
    ; Disable interrupts (no IDT set up yet for PM)
    cli

    ; Load the GDT
    lgdt [pm_gdt_descriptor]

    ; Set PE (Protection Enable) bit in CR0
    mov eax, cr0
    or eax, 1
    mov cr0, eax

    ; Far jump to flush pipeline and load CS with kernel code selector
    jmp KERNEL_CODE_SEG:pm_entry

; =============================================================================
; 32-bit Protected Mode Entry
; =============================================================================
[BITS 32]
pm_entry:
    ; Set up all data segment registers
    mov ax, KERNEL_DATA_SEG
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    ; Set up kernel stack (at 2 MiB, growing downward)
    mov esp, 0x00200000

    ; Clear BSS of kernel (we don't know exact bounds yet, kernel will do it)

    ; Push BootInfo address as argument to kernel_main (cdecl calling convention)
    push dword BOOT_INFO_ADDR

    ; Jump to kernel entry point at its physical load address
    ; (Kernel is loaded at 0x100000, entry point is at the start of .text)
    ; The linker script sets VMA to 0xC0100000 but LMA to 0x100000
    ; Before paging is enabled, we must jump to the physical address
    call KERNEL_LOAD_PHYS

    ; Should never reach here
    cli
.halt:
    hlt
    jmp .halt

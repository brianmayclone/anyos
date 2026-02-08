; =============================================================================
; ap_trampoline.asm — AP startup trampoline for SMP
; =============================================================================
; This code is copied to physical address 0x8000 at runtime.
; The AP starts in 16-bit real mode (CS:IP = 0x0800:0000).
;
; Communication area at 0x7F00:
;   0x7F00: u32 — stack pointer (virtual address)
;   0x7F04: u32 — CR3 (page directory physical address)
;   0x7F08: u32 — (reserved)
;   0x7F0C: u32 — entry point (Rust function pointer, virtual address)
;   0x7F10: u8  — AP ready flag
;   0x7F14: u32 — cpu_id
; =============================================================================

[BITS 16]
[ORG 0x8000]

global ap_trampoline_start
global ap_trampoline_end

ap_trampoline_start:
    cli
    xor     ax, ax
    mov     ds, ax
    mov     es, ax
    mov     ss, ax

    ; Load our temporary GDT
    lgdt    [ap_gdt_desc]

    ; Enable protected mode
    mov     eax, cr0
    or      al, 1
    mov     cr0, eax

    ; Far jump to 32-bit code — flush pipeline
    jmp     dword 0x08:ap_pm_entry

[BITS 32]
ap_pm_entry:
    ; Load data segments
    mov     ax, 0x10
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     gs, ax
    mov     ss, ax

    ; Load page directory from communication area
    mov     eax, [0x7F04]
    mov     cr3, eax

    ; Enable paging
    mov     eax, cr0
    or      eax, 0x80000000
    mov     cr0, eax

    ; Load stack pointer (virtual address set by BSP)
    mov     esp, [0x7F00]

    ; Jump to Rust entry point (virtual address set by BSP)
    mov     eax, [0x7F0C]
    jmp     eax

; Padding to align GDT
align 16

; =============================================================================
; Temporary GDT for the AP trampoline (3 entries)
; =============================================================================
ap_gdt:
    ; Entry 0: Null descriptor
    dq  0x0000000000000000
    ; Entry 1 (0x08): Code segment — base=0, limit=4G, 32-bit, ring 0
    dq  0x00CF9A000000FFFF
    ; Entry 2 (0x10): Data segment — base=0, limit=4G, 32-bit, ring 0
    dq  0x00CF92000000FFFF

ap_gdt_desc:
    dw  ap_gdt_desc - ap_gdt - 1   ; limit
    dd  ap_gdt                       ; base address

ap_trampoline_end:

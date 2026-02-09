; =============================================================================
; ap_trampoline.asm — AP startup trampoline for SMP (64-bit long mode)
; =============================================================================
; This code is copied to physical address 0x8000 at runtime.
; The AP starts in 16-bit real mode (CS:IP = 0x0800:0000).
;
; Flow: real mode → 32-bit protected mode → enable PAE + LME → enable paging
;       → long mode → far jump to 64-bit code → jump to Rust entry.
;
; Communication area at 0x7F00:
;   0x7F00: u64 — stack pointer (virtual address)
;   0x7F08: u64 — CR3 (PML4 physical address)
;   0x7F10: u64 — entry point (Rust function pointer, virtual address)
;   0x7F18: u8  — AP ready flag
;   0x7F1C: u32 — cpu_id
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

    ; Load our temporary GDT (includes 32-bit and 64-bit code segments)
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

    ; Enable PAE (CR4 bit 5) — required for long mode
    mov     eax, cr4
    or      eax, (1 << 5)
    mov     cr4, eax

    ; Set IA32_EFER.LME (Long Mode Enable, bit 8 of MSR 0xC0000080)
    mov     ecx, 0xC0000080
    rdmsr
    or      eax, (1 << 8)
    wrmsr

    ; Load PML4 from communication area into CR3
    mov     eax, [0x7F08]           ; Lower 32 bits of CR3 (PML4 phys < 4GB)
    mov     cr3, eax

    ; Enable paging (CR0.PG) — this activates long mode!
    mov     eax, cr0
    or      eax, 0x80000000
    mov     cr0, eax

    ; Far jump to 64-bit long mode code (GDT entry 3 = 0x18 = 64-bit code)
    jmp     dword 0x18:ap_lm_entry

[BITS 64]
ap_lm_entry:
    ; Reload data segments with 64-bit data selector
    mov     ax, 0x10
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     gs, ax
    mov     ss, ax

    ; Load 64-bit stack pointer from communication area
    mov     rsp, [0x7F00]

    ; Jump to Rust entry point (64-bit virtual address)
    mov     rax, [0x7F10]
    jmp     rax

; Padding to align GDT
align 16

; =============================================================================
; Temporary GDT for the AP trampoline (4 entries)
; =============================================================================
ap_gdt:
    ; Entry 0: Null descriptor
    dq  0x0000000000000000
    ; Entry 1 (0x08): 32-bit Code segment — base=0, limit=4G, ring 0
    dq  0x00CF9A000000FFFF
    ; Entry 2 (0x10): Data segment — base=0, limit=4G, ring 0
    dq  0x00CF92000000FFFF
    ; Entry 3 (0x18): 64-bit Code segment — L=1, D=0, P=1, DPL=0, type=0xA
    dq  0x00209A0000000000

ap_gdt_desc:
    dw  ap_gdt_desc - ap_gdt - 1   ; limit
    dd  ap_gdt                       ; base address

ap_trampoline_end:

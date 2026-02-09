; =============================================================================
; protected_mode.asm - GDT setup, switch to protected mode, then to long mode
;
; Flow:
;   1. Load GDT and enter 32-bit protected mode
;   2. Build 4-level page tables (identity + higher-half + framebuffer)
;   3. Enable PAE, set IA32_EFER.LME, enable paging -> long mode active
;   4. Far jump to 64-bit code, jump to kernel
; =============================================================================

; Page table physical addresses (all below Stage 2 at 0x8000)
PML4_ADDR       equ 0x4000      ; PML4 (top-level, 512 entries x 8 = 4 KiB)
PDPT_LOW_ADDR   equ 0x5000      ; PDPT for identity map (PML4[0])
PD_LOW_ADDR     equ 0x6000      ; PD for first 1 GiB (2 MiB pages, identity map)
PDPT_HIGH_ADDR  equ 0x7000      ; PDPT for higher-half (PML4[511])
PD_FB_ADDR      equ 0x3000      ; PD for 3-4 GiB range (framebuffer identity map)

; Page table entry flags
PT_PRESENT      equ 0x01
PT_RW           equ 0x02
PT_PS           equ 0x80        ; Page Size (2 MiB for PD entries)
PT_BASE_FLAGS   equ (PT_PRESENT | PT_RW)
PT_PAGE_FLAGS   equ (PT_PRESENT | PT_RW | PT_PS)

; MSR addresses
MSR_EFER        equ 0xC0000080
EFER_LME_BIT    equ 8           ; Long Mode Enable bit

; =============================================================================
; Boot GDT - 32-bit and 64-bit code segments for mode transitions
; =============================================================================
ALIGN 8
pm_gdt:
    ; Entry 0 (0x00): Null descriptor
    dq 0

    ; Entry 1 (0x08): 32-bit Kernel Code
    ; Used temporarily for protected mode before switching to long mode
    dw 0xFFFF               ; Limit low
    dw 0x0000               ; Base low
    db 0x00                 ; Base mid
    db 10011010b            ; Access: P=1, DPL=00, S=1, Type=1010 (code, exec/read)
    db 11001111b            ; Flags: G=1, D=1, L=0, AVL=0 | Limit high=0xF
    db 0x00                 ; Base high

    ; Entry 2 (0x10): Kernel Data (same for 32/64-bit)
    dw 0xFFFF
    dw 0x0000
    db 0x00
    db 10010010b            ; Access: P=1, DPL=00, S=1, Type=0010 (data, read/write)
    db 11001111b            ; Flags: G=1, D=1, L=0 | Limit high=0xF
    db 0x00

    ; Entry 3 (0x18): 64-bit Kernel Code
    ; Used after long mode is activated
    dw 0x0000               ; Limit low (ignored in 64-bit)
    dw 0x0000               ; Base low (ignored)
    db 0x00                 ; Base mid (ignored)
    db 10011010b            ; Access: P=1, DPL=00, S=1, Type=1010 (code, exec/read)
    db 00100000b            ; Flags: G=0, D=0, L=1, AVL=0 | Limit high=0x0
    db 0x00                 ; Base high (ignored)
pm_gdt_end:

pm_gdt_descriptor:
    dw pm_gdt_end - pm_gdt - 1     ; Size of GDT - 1
    dd pm_gdt                        ; Linear address of GDT

; Segment selectors
KERNEL_CODE32_SEG equ 0x08
KERNEL_DATA_SEG   equ 0x10
KERNEL_CODE64_SEG equ 0x18

; =============================================================================
; enter_protected_mode - Switch to protected mode, build page tables, enter
;                        long mode, and jump to kernel
; =============================================================================
enter_protected_mode:
    ; Disable interrupts
    cli

    ; Load the GDT
    lgdt [pm_gdt_descriptor]

    ; Set PE (Protection Enable) bit in CR0
    mov eax, cr0
    or eax, 1
    mov cr0, eax

    ; Far jump to 32-bit protected mode code
    jmp KERNEL_CODE32_SEG:pm_entry

; =============================================================================
; 32-bit Protected Mode - build 4-level page tables, switch to long mode
; =============================================================================
[BITS 32]
pm_entry:
    ; Set up data segments
    mov ax, KERNEL_DATA_SEG
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax
    mov esp, 0x00200000             ; Temporary stack at 2 MiB

    ; --- Clear all page table pages (5 x 4 KiB = 20 KiB) ---
    ; Pages are contiguous: 0x3000 (PD_FB), 0x4000 (PML4), 0x5000 (PDPT_LOW),
    ;                        0x6000 (PD_LOW), 0x7000 (PDPT_HIGH)
    mov edi, PD_FB_ADDR             ; Start at 0x3000
    xor eax, eax
    mov ecx, (5 * 4096 / 4)        ; 5120 dwords = 20 KiB
    rep stosd

    ; --- PML4 entries ---
    ; PML4[0] -> PDPT_LOW (identity map for low memory + framebuffer)
    mov dword [PML4_ADDR + 0*8],     PDPT_LOW_ADDR | PT_BASE_FLAGS
    mov dword [PML4_ADDR + 0*8 + 4], 0
    ; PML4[511] -> PDPT_HIGH (higher-half kernel at 0xFFFFFFFF80000000)
    mov dword [PML4_ADDR + 511*8],     PDPT_HIGH_ADDR | PT_BASE_FLAGS
    mov dword [PML4_ADDR + 511*8 + 4], 0

    ; --- PDPT_LOW entries ---
    ; PDPT_LOW[0] -> PD_LOW (covers 0x00000000 - 0x3FFFFFFF = first 1 GiB)
    mov dword [PDPT_LOW_ADDR + 0*8],     PD_LOW_ADDR | PT_BASE_FLAGS
    mov dword [PDPT_LOW_ADDR + 0*8 + 4], 0
    ; PDPT_LOW[3] -> PD_FB (covers 0xC0000000 - 0xFFFFFFFF = 3-4 GiB, framebuffer)
    mov dword [PDPT_LOW_ADDR + 3*8],     PD_FB_ADDR | PT_BASE_FLAGS
    mov dword [PDPT_LOW_ADDR + 3*8 + 4], 0

    ; --- PD_LOW: identity map first 16 MiB with 2 MiB pages ---
    xor eax, eax                    ; Starting physical address = 0
    mov edi, PD_LOW_ADDR
    mov ecx, 8                      ; 8 entries x 2 MiB = 16 MiB
.pd_low_loop:
    mov ebx, eax
    or ebx, PT_PAGE_FLAGS
    mov [edi], ebx
    mov dword [edi + 4], 0
    add eax, 0x200000               ; Next 2 MiB
    add edi, 8
    dec ecx
    jnz .pd_low_loop

    ; --- PDPT_HIGH: higher-half kernel ---
    ; 0xFFFFFFFF80000000 -> PML4[511], PDPT[510], PD[0]
    ; Point PDPT_HIGH[510] -> same PD_LOW (reuses identity map PD)
    mov dword [PDPT_HIGH_ADDR + 510*8],     PD_LOW_ADDR | PT_BASE_FLAGS
    mov dword [PDPT_HIGH_ADDR + 510*8 + 4], 0

    ; --- Map framebuffer (identity map at its physical address) ---
    ; BootInfo.framebuffer_addr is at BOOT_INFO_ADDR + 12
    mov eax, [BOOT_INFO_ADDR + 12]
    test eax, eax
    jz .no_fb_map

    ; Calculate PD index within the 3-4 GiB range:
    ; PD index = (phys_addr & 0x3FFFFFFF) >> 21
    mov ebx, eax
    and ebx, 0x3FFFFFFF
    shr ebx, 21                     ; PD index

    ; Map 8 x 2 MiB pages (16 MiB) for framebuffer VRAM
    and eax, 0xFFE00000             ; 2 MiB-align the address
    mov ecx, 8
    lea edi, [PD_FB_ADDR + ebx*8]
.fb_map_loop:
    mov ebx, eax
    or ebx, PT_PAGE_FLAGS
    mov [edi], ebx
    mov dword [edi + 4], 0
    add eax, 0x200000
    add edi, 8
    dec ecx
    jnz .fb_map_loop
.no_fb_map:

    ; --- Enable PAE (CR4 bit 5) ---
    mov eax, cr4
    or eax, (1 << 5)               ; CR4.PAE
    mov cr4, eax

    ; --- Load PML4 into CR3 ---
    mov eax, PML4_ADDR
    mov cr3, eax

    ; --- Enable Long Mode via IA32_EFER MSR ---
    mov ecx, MSR_EFER
    rdmsr
    or eax, (1 << EFER_LME_BIT)    ; Set LME
    wrmsr

    ; --- Enable paging (activates long mode since LME is set) ---
    mov eax, cr0
    or eax, (1 << 31)              ; CR0.PG
    mov cr0, eax

    ; --- Far jump to 64-bit code segment ---
    jmp KERNEL_CODE64_SEG:lm_entry

; =============================================================================
; 64-bit Long Mode Entry
; =============================================================================
[BITS 64]
lm_entry:
    ; Set up data segments
    mov ax, KERNEL_DATA_SEG
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    ; Temporary stack
    mov rsp, 0x00200000

    ; Pass BootInfo address in RDI (System V AMD64 ABI)
    mov edi, BOOT_INFO_ADDR

    ; Jump to kernel entry point at physical address
    ; (identity-mapped, kernel is loaded at 0x100000)
    mov rax, KERNEL_LOAD_PHYS
    jmp rax

    ; Should never reach here
    cli
.halt:
    hlt
    jmp .halt

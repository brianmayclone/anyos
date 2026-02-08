; =============================================================================
; boot.asm - Kernel boot stub
;
; This code runs BEFORE kernel_main. It sets up initial paging so the
; higher-half kernel (linked at VMA 0xC0100000) can execute.
;
; Flow:
;   1. Stage 2 loads kernel flat binary at physical 0x100000 and calls here
;   2. We set up a page directory with:
;      - Identity map: 0x00000000 -> 0x00000000 (first 16 MB)
;      - Higher-half:  0xC0000000 -> 0x00000000 (16 MB)
;   3. Enable PSE (4 MB pages) and paging
;   4. Jump to higher-half address space
;   5. Zero BSS, set up stack, call kernel_main
; =============================================================================

[BITS 32]

; Higher-half kernel offset
KERNEL_VMA      equ 0xC0000000

; Kernel stack: placed ABOVE BSS to avoid overlap as the kernel grows.
; Must match KERNEL_STACK_SIZE in kernel/src/memory/physical.rs.
KERNEL_STACK_SIZE equ 0x10000  ; 64 KiB

; Page directory at physical 0x4000 (safe: below Stage 2 at 0x8000,
; above BIOS data area at 0x500, not overlapping BootInfo at 0x9000
; or memory map at 0x1000)
BOOT_PAGE_DIR   equ 0x00004000

; 4 MB page directory entry flags
PDE_PRESENT     equ 0x01
PDE_RW          equ 0x02
PDE_PS          equ 0x80       ; Page Size = 4 MB
PDE_FLAGS       equ (PDE_PRESENT | PDE_RW | PDE_PS)

; Serial port for debug output
COM1            equ 0x3F8

; Linker-provided symbols (virtual addresses)
extern kernel_main
extern _bss_start
extern _bss_end

section .text.boot
global _boot_start

; =============================================================================
; _boot_start - MUST be at the very first byte of .text.boot!
; Stage 2 calls physical 0x100000 which maps to the start of this section.
; Called by Stage 2 with boot_info_addr on the stack (cdecl)
; =============================================================================
_boot_start:
    ; --- FIRST: Reload DS and ES to ensure valid segment descriptors ---
    ; This must happen before any memory access through DS/ES!
    mov ax, 0x10                ; KERNEL_DATA_SEG
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    ; Debug: output 'B' to serial port (I/O ports don't need DS)
    mov dx, COM1 + 5
.wait_b:
    in al, dx
    test al, 0x20
    jz .wait_b
    mov al, 'B'
    mov dx, COM1
    out dx, al

    ; Write to VGA text buffer as visual indicator (now safe, DS is set)
    mov dword [0xB8000], 0x0F420F4F  ; "OB" in white on black
    mov dword [0xB8004], 0x0F540F4F  ; "OT" in white on black

    ; Stack layout from Stage 2:
    ;   [esp]   = return address (from 'call KERNEL_LOAD_PHYS')
    ;   [esp+4] = boot_info_addr (BOOT_INFO_ADDR = 0x9000)
    mov esi, [esp + 4]          ; Save boot_info_addr in ESI (uses SS, safe)

    ; Debug: output '1'
    call serial_char_1

    ; --- Clear page directory (1024 dwords = 4 KB) ---
    mov edi, BOOT_PAGE_DIR
    xor eax, eax
    mov ecx, 1024
    rep stosd

    ; Debug: output '2'
    call serial_char_2

    ; --- Identity map first 16 MB (PDEs 0..3) ---
    mov dword [BOOT_PAGE_DIR + 0*4], 0x00000000 | PDE_FLAGS
    mov dword [BOOT_PAGE_DIR + 1*4], 0x00400000 | PDE_FLAGS
    mov dword [BOOT_PAGE_DIR + 2*4], 0x00800000 | PDE_FLAGS
    mov dword [BOOT_PAGE_DIR + 3*4], 0x00C00000 | PDE_FLAGS

    ; --- Map higher-half: 0xC0000000+ -> 0x00000000+ (PDEs 768..771) ---
    mov dword [BOOT_PAGE_DIR + 768*4], 0x00000000 | PDE_FLAGS
    mov dword [BOOT_PAGE_DIR + 769*4], 0x00400000 | PDE_FLAGS
    mov dword [BOOT_PAGE_DIR + 770*4], 0x00800000 | PDE_FLAGS
    mov dword [BOOT_PAGE_DIR + 771*4], 0x00C00000 | PDE_FLAGS

    ; --- Identity-map framebuffer if available (from BootInfo at [esi]) ---
    ; BootInfo.framebuffer_addr is at offset 12
    mov eax, [esi + 12]            ; framebuffer_addr
    test eax, eax
    jz .no_fb_map
    ; Map as 4 MB PSE page: PDE index = addr >> 22
    mov ebx, eax
    shr ebx, 22                    ; PDE index
    and eax, 0xFFC00000            ; 4 MB-align the address
    or eax, PDE_FLAGS
    mov [BOOT_PAGE_DIR + ebx*4], eax
.no_fb_map:

    ; Debug: output '3'
    call serial_char_3

    ; --- Enable PSE (Page Size Extension) in CR4 ---
    mov eax, cr4
    or eax, 0x10                ; CR4.PSE = bit 4
    mov cr4, eax

    ; --- Load page directory into CR3 ---
    mov eax, BOOT_PAGE_DIR
    mov cr3, eax

    ; --- Enable paging (PG bit in CR0) ---
    mov eax, cr0
    or eax, 0x80000000          ; CR0.PG = bit 31
    mov cr0, eax

    ; --- Paging is now active ---
    ; Identity map keeps us running at physical addresses.
    ; Jump to higher-half trampoline using absolute virtual address.
    mov eax, higher_half_entry
    jmp eax

; --- Serial debug helpers (in .text.boot so they're identity-mapped) ---
serial_char_1:
    push eax
    mov dx, COM1 + 5
.w: in al, dx
    test al, 0x20
    jz .w
    mov al, '1'
    mov dx, COM1
    out dx, al
    pop eax
    ret

serial_char_2:
    push eax
    mov dx, COM1 + 5
.w: in al, dx
    test al, 0x20
    jz .w
    mov al, '2'
    mov dx, COM1
    out dx, al
    pop eax
    ret

serial_char_3:
    push eax
    mov dx, COM1 + 5
.w: in al, dx
    test al, 0x20
    jz .w
    mov al, '3'
    mov dx, COM1
    out dx, al
    pop eax
    ret

; =============================================================================
; higher_half_entry - Runs in higher-half virtual address space
; =============================================================================
section .text
higher_half_entry:
    ; Zero the BSS section FIRST (before setting up the stack, since the
    ; old hardcoded stack at 0xC0200000 overlapped with BSS as kernel grew).
    mov edi, _bss_start
    mov ecx, _bss_end
    sub ecx, edi
    shr ecx, 2                  ; byte count -> dword count
    xor eax, eax
    rep stosd

    ; Set up kernel stack ABOVE BSS so they never overlap.
    ; Stack grows down from (_bss_end + KERNEL_STACK_SIZE).
    mov esp, _bss_end
    add esp, KERNEL_STACK_SIZE

    ; Debug: output '5' for higher-half reached
    mov dx, COM1 + 5
.wait_tx:
    in al, dx
    test al, 0x20
    jz .wait_tx
    mov al, '5'
    mov dx, COM1
    out dx, al

    ; Call kernel_main(boot_info_addr)
    ; ESI still holds the boot_info_addr from Stage 2
    push esi
    call kernel_main

    ; kernel_main should never return, but just in case:
    cli
boot_halt:
    hlt
    jmp boot_halt

; =============================================================================
; .anyOS Stage 2 Bootloader
; =============================================================================
; Loaded by Stage 1 at 0x0000:0x8000 (real mode, 16-bit)
; Tasks:
;   1. Enable A20 line
;   2. Query E820 memory map
;   3. Load kernel from disk to 0x100000 (1 MiB)
;   4. Set up GDT and switch to 32-bit protected mode
;   5. Fill BootInfo structure and jump to kernel
; =============================================================================

[BITS 16]
[ORG 0x8000]

; Jump over data area (jmp short = 2 bytes, lands at offset 20+)
    jmp short stage2_entry

; Data area patched by mkimage.py at offsets 2-19:
; MUST start at byte offset 2 (immediately after jmp short)
kernel_sectors:     dw 0        ; offset 2-3:  number of kernel sectors
kernel_start_lba:   dd 0        ; offset 4-7:  starting LBA of kernel
config_lba:         dw 0        ; offset 8-9:  boot.cfg LBA
config_sectors:     dw 0        ; offset 10-11: boot.cfg sector count
logo_lba:           dw 0        ; offset 12-13: logo LBA
logo_sectors:       dw 0        ; offset 14-15: logo sector count
font_lba:           dw 0        ; offset 16-17: font LBA
font_sectors:       dw 0        ; offset 18-19: font sector count

; =============================================================================
; Constants
; =============================================================================
BOOT_INFO_ADDR      equ 0x9000
MEMORY_MAP_ADDR     equ 0x1000
MEMORY_MAP_COUNT    equ 0x0FF0  ; u32 count stored here
KERNEL_LOAD_PHYS    equ 0x100000 ; 1 MiB mark
TEMP_BUFFER         equ 0x10000  ; Temporary load buffer (64 KiB)
TEMP_BUFFER_SEG     equ 0x1000   ; Segment for temp buffer
LOGO_LOAD_ADDR      equ 0x20000
FONT_LOAD_ADDR      equ 0x28000
CONFIG_LOAD_ADDR    equ 0x30000

stage2_entry:
    ; Save boot drive (passed in DL from stage 1)
    mov [boot_drive], dl

    ; Step 1: Enable A20 line
    call enable_a20

    ; Step 2: Get memory map
    call get_memory_map

    ; Step 3: Enter unreal mode for high memory access
    call enter_unreal_mode

    ; Step 4: Set VESA VBE graphical mode (must be in real mode)
    call setup_vesa

    ; Step 5: If VESA ok, load logo and font
    cmp byte [vesa_ok], 1
    jne .skip_logo_font

    mov ax, [logo_lba]
    mov cx, [logo_sectors]
    test cx, cx
    jz .skip_logo
    mov edi, LOGO_LOAD_ADDR
    call load_sectors
.skip_logo:

    mov ax, [font_lba]
    mov cx, [font_sectors]
    test cx, cx
    jz .skip_logo_font
    mov edi, FONT_LOAD_ADDR
    call load_sectors
.skip_logo_font:

    ; Step 6: Load boot.cfg
    mov ax, [config_lba]
    mov cx, [config_sectors]
    test cx, cx
    jz .skip_config
    mov edi, CONFIG_LOAD_ADDR
    call load_sectors
.skip_config:

    ; Step 7: Parse config
    call parse_config

    ; Step 8: If VESA ok, show splash and wait for input
    cmp byte [vesa_ok], 1
    jne .no_splash
    call show_splash
    call wait_for_input         ; AL = 0 (timeout/enter) or 0x1B (escape)
    cmp al, 0x1B
    jne .boot_default
    call show_menu              ; AL = selected entry index
    jmp .execute
.no_splash:
.boot_default:
    mov al, [cfg_default]
.execute:
    ; AL = entry index to boot
    call execute_entry

    ; Step 9: Load kernel to high memory (0x100000)
    call load_kernel

    ; Step 10: Fill BootInfo structure
    call fill_boot_info

    ; Step 11: Switch to protected mode and jump to kernel
    call enter_protected_mode
    ; Does not return

; =============================================================================
; Data
; =============================================================================
boot_drive:     db 0
msg_stage2:     db "Stage 2: Starting...", 13, 10, 0
msg_a20:        db "  A20 line enabled", 13, 10, 0
msg_memmap:     db "  Memory map obtained", 13, 10, 0
msg_unreal:     db "  Unreal mode active", 13, 10, 0
msg_kernel:     db "  Kernel loaded at 1MB", 13, 10, 0
msg_vesa_ok:    db "  VESA VBE mode set", 13, 10, 0
msg_vesa_fail:  db "  VESA not available (text mode)", 13, 10, 0
msg_pm:         db "  Entering protected mode...", 13, 10, 0

; =============================================================================
; Print string in 16-bit real mode (DS:SI = null-terminated string)
; =============================================================================
print_string_16:
    pusha
.loop:
    lodsb
    test al, al
    jz .done
    mov ah, 0x0E
    mov bx, 0x0007
    int 0x10
    jmp .loop
.done:
    popa
    ret

; =============================================================================
; fill_boot_info - Populate BootInfo struct at BOOT_INFO_ADDR
; =============================================================================
fill_boot_info:
    pusha
    mov di, BOOT_INFO_ADDR

    ; magic = 0x414E594F ("ANYO")
    mov dword [di + 0], 0x414E594F

    ; memory_map_addr
    mov dword [di + 4], MEMORY_MAP_ADDR

    ; memory_map_count
    mov eax, [MEMORY_MAP_COUNT]
    mov [di + 8], eax

    ; framebuffer fields from VESA VBE mode info (or zero if no VESA)
    cmp byte [vesa_ok], 1
    jne .no_fb
    ; Read from VBE Mode Info Block (stored at VBE_MODE_INFO_ADDR = 0x2000)
    mov eax, [0x2000 + 0x28]       ; PhysBasePtr (framebuffer address)
    mov [di + 12], eax
    movzx eax, word [0x2000 + 0x10] ; BytesPerScanLine (pitch)
    mov [di + 16], eax
    movzx eax, word [0x2000 + 0x12] ; XResolution (width)
    mov [di + 20], eax
    movzx eax, word [0x2000 + 0x14] ; YResolution (height)
    mov [di + 24], eax
    mov al, [0x2000 + 0x19]         ; BitsPerPixel (bpp)
    mov [di + 28], al
    jmp .fb_done
.no_fb:
    mov dword [di + 12], 0      ; framebuffer_addr
    mov dword [di + 16], 0      ; framebuffer_pitch
    mov dword [di + 20], 0      ; framebuffer_width
    mov dword [di + 24], 0      ; framebuffer_height
    mov byte  [di + 28], 0      ; framebuffer_bpp
.fb_done:

    ; boot_drive
    mov al, [boot_drive]
    mov [di + 29], al

    ; padding
    mov word [di + 30], 0

    ; kernel_phys_start
    mov dword [di + 32], KERNEL_LOAD_PHYS

    ; kernel_phys_end = kernel_phys_start + kernel_sectors * 512
    movzx eax, word [kernel_sectors]
    shl eax, 9                  ; * 512
    add eax, KERNEL_LOAD_PHYS
    mov [di + 36], eax

    ; rsdp_addr = 0 (BIOS path discovers RSDP by scanning memory)
    mov dword [di + 40], 0

    ; Zero boot_params (64 bytes at offset 44)
    push edi
    push ecx
    push eax
    lea edi, [BOOT_INFO_ADDR + 44]
    mov ecx, 16                     ; 16 dwords = 64 bytes
    xor eax, eax
    a32 rep stosd
    pop eax
    pop ecx
    pop edi

    popa
    ret

; =============================================================================
; Include sub-modules
; =============================================================================
%include "a20.asm"
%include "memory_map.asm"
%include "disk.asm"
%include "vesa.asm"
%include "config.asm"
%include "font.asm"
%include "splash.asm"
%include "menu.asm"
%include "chainload.asm"
%include "protected_mode.asm"

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

; Debug macro: write a single char to serial port 0x3F8
%macro SERIAL_OUT 1
    push ax
    push dx
    mov dx, 0x3F8
    mov al, %1
    out dx, al
    pop dx
    pop ax
%endmacro

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

; exFAT runtime data (HDD boot path):
FS_START_LBA        equ 128        ; exFAT volume starts at sector 128
kernel_file_size:   dd 0           ; Kernel size in bytes (set by exfat_read_file)
boot_is_cdrom:      db 0           ; 1 if booting from CD-ROM
_boot_dir_cluster:  dd 0           ; Saved /boot/ directory cluster

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

    ; Debug: init serial port 0x3F8 (115200 baud, 8N1)
    mov dx, 0x3FB
    mov al, 0x80            ; DLAB on
    out dx, al
    mov dx, 0x3F8
    mov al, 1               ; divisor low = 1 (115200)
    out dx, al
    mov dx, 0x3F9
    xor al, al              ; divisor high = 0
    out dx, al
    mov dx, 0x3FB
    mov al, 0x03            ; 8N1, DLAB off
    out dx, al

    SERIAL_OUT '1'           ; Step 1: A20

    ; Step 1: Enable A20 line
    call enable_a20

    SERIAL_OUT '2'           ; Step 2: memmap

    ; Step 2: Get memory map
    call get_memory_map

    SERIAL_OUT '3'           ; Step 3: unreal

    ; Step 3: Enter unreal mode for high memory access
    call enter_unreal_mode

    SERIAL_OUT '4'           ; Step 4: VESA

    ; Step 4: Set VESA VBE graphical mode (must be in real mode)
    call setup_vesa

    SERIAL_OUT '5'           ; Step 5: load files

    ; Detect CD-ROM vs HDD via INT 13h AH=48h
    mov word [drive_params_buf], 0x1E
    mov ah, 0x48
    mov dl, [boot_drive]
    mov si, drive_params_buf
    int 0x13
    jc .hdd_boot                ; No EDD -> assume HDD

    mov ax, [drive_params_buf + 0x18]
    cmp ax, 1024
    ja .cdrom_boot              ; > 1024 bytes/sector -> CD-ROM

.hdd_boot:
    mov byte [boot_is_cdrom], 0
    jmp .exfat_load

.cdrom_boot:
    mov byte [boot_is_cdrom], 1
    ; CD-ROM: use existing raw-sector loading (patched LBAs)
    mov ax, [logo_lba]
    mov cx, [logo_sectors]
    test cx, cx
    jz .cd_skip_logo
    SERIAL_OUT 'L'
    mov edi, LOGO_LOAD_ADDR
    call load_sectors
.cd_skip_logo:
    mov ax, [font_lba]
    mov cx, [font_sectors]
    test cx, cx
    jz .cd_skip_font
    SERIAL_OUT 'F'
    mov edi, FONT_LOAD_ADDR
    call load_sectors
.cd_skip_font:
    mov ax, [config_lba]
    mov cx, [config_sectors]
    test cx, cx
    jz .cd_skip_config
    SERIAL_OUT 'C'
    mov edi, CONFIG_LOAD_ADDR
    call load_sectors
.cd_skip_config:
    jmp .files_loaded

.exfat_load:
    ; HDD: use exFAT filesystem
    mov eax, FS_START_LBA
    call exfat_init
    jc .exfat_fatal

    ; Load kernel: /System/krnl64 (FATAL)
    mov ebx, str_system
    call exfat_open_dir
    jc .kernel_fatal

    mov ebx, str_krnl64
    mov edi, KERNEL_LOAD_PHYS
    call exfat_read_file
    jc .kernel_fatal
    mov [kernel_file_size], eax
    SERIAL_OUT 'K'

    ; Load boot files from /boot/ (all optional)
    call exfat_reset_to_root
    mov ebx, str_boot_dir
    call exfat_open_dir
    jc .skip_boot_files

    ; Save /boot/ cluster for reuse between file loads
    mov eax, [exfat_cur_dir_cluster]
    mov [_boot_dir_cluster], eax

    ; boot.cfg
    mov ebx, str_bootcfg
    mov edi, CONFIG_LOAD_ADDR
    call exfat_read_file
    jc .exfat_skip_cfg
    SERIAL_OUT 'C'
.exfat_skip_cfg:

    ; logo.bin (only if VESA available)
    cmp byte [vesa_ok], 1
    jne .exfat_skip_logo
    mov eax, [_boot_dir_cluster]
    mov [exfat_cur_dir_cluster], eax
    mov ebx, str_logobin
    mov edi, LOGO_LOAD_ADDR
    call exfat_read_file
    jc .exfat_skip_logo
    mov word [logo_sectors], 1     ; Signal splash/menu that logo is loaded
    SERIAL_OUT 'L'
.exfat_skip_logo:

    ; font.bin
    mov eax, [_boot_dir_cluster]
    mov [exfat_cur_dir_cluster], eax
    mov ebx, str_fontbin
    mov edi, FONT_LOAD_ADDR
    call exfat_read_file
    jc .exfat_skip_font
    SERIAL_OUT 'F'
.exfat_skip_font:
    jmp .files_loaded

.skip_boot_files:
    jmp .files_loaded

.exfat_fatal:
    mov si, msg_exfat_err
    call print_string_16
    cli
    hlt

.kernel_fatal:
    mov si, msg_kernel_err
    call print_string_16
    cli
    hlt

.files_loaded:
    SERIAL_OUT '6'           ; Step 6: config

    SERIAL_OUT '7'           ; Step 7: parse

    ; Step 7: Parse config
    call parse_config

    SERIAL_OUT '8'           ; Step 8: splash

    ; Step 8: If VESA ok, show splash and wait for input
    cmp byte [vesa_ok], 1
    jne .no_splash
    SERIAL_OUT 'S'           ; show_splash
    call show_splash
    SERIAL_OUT 'W'           ; wait_for_input
    call wait_for_input         ; AL = 0 (timeout/enter) or 0x1B (escape)
    cmp al, 0x1B
    jne .boot_default
    call show_menu              ; AL = selected entry index
    jmp .execute
.no_splash:
.boot_default:
    movzx ax, byte [cfg_default]
.execute:
    ; Save selected entry index
    push ax

    SERIAL_OUT '9'           ; Step 9: load kernel

    ; Step 9: Load kernel (CD-ROM only -- HDD already loaded via exFAT)
    cmp byte [boot_is_cdrom], 0
    je .skip_raw_kernel_load
    call load_kernel
.skip_raw_kernel_load:

    SERIAL_OUT 'B'           ; Step 10: boot info

    ; Step 10: Fill BootInfo structure (zeros boot_params)
    call fill_boot_info

    SERIAL_OUT 'E'           ; execute_entry (writes params AFTER fill_boot_info)

    ; Step 10b: Execute entry — copies params to BootInfo AFTER zeroing
    pop ax
    movzx bx, al               ; execute_entry expects BX = entry index
    call execute_entry

    SERIAL_OUT 'P'           ; Step 11: protected mode

    ; Step 11: Switch to protected mode and jump to kernel
    call enter_protected_mode
    ; Does not return

; =============================================================================
; Data
; =============================================================================
boot_drive:     db 0
str_system:       db "System", 0
str_krnl64:       db "krnl64", 0
str_boot_dir:     db "boot", 0
str_bootcfg:      db "boot.cfg", 0
str_logobin:      db "logo.bin", 0
str_fontbin:      db "font.bin", 0
msg_exfat_err:    db "FATAL: Bad exFAT!", 13, 10, 0
msg_kernel_err:   db "FATAL: No kernel!", 13, 10, 0
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

    ; kernel_phys_end: depends on boot path
    cmp byte [boot_is_cdrom], 0
    jne .kend_cdrom
    mov eax, [kernel_file_size]
    jmp .kend_done
.kend_cdrom:
    movzx eax, word [kernel_sectors]
    shl eax, 9
.kend_done:
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

    ; EDID data (128 bytes at offset 108) + edid_valid (1 byte at offset 236)
    push esi
    push edi
    push ecx
    push eax
    cmp byte [edid_ok], 1
    jne .no_edid_copy
    ; Copy 128 bytes from EDID_BUF_ADDR to BOOT_INFO_ADDR + 108
    mov esi, EDID_BUF_ADDR
    lea edi, [BOOT_INFO_ADDR + 108]
    mov ecx, 32                     ; 32 dwords = 128 bytes
    a32 rep movsd
    mov byte [BOOT_INFO_ADDR + 236], 1   ; edid_valid = 1
    jmp .edid_copy_done
.no_edid_copy:
    ; Zero the EDID area (128 bytes)
    lea edi, [BOOT_INFO_ADDR + 108]
    mov ecx, 32
    xor eax, eax
    a32 rep stosd
    mov byte [BOOT_INFO_ADDR + 236], 0   ; edid_valid = 0
.edid_copy_done:
    ; Zero padding bytes 237-239
    mov byte [BOOT_INFO_ADDR + 237], 0
    mov byte [BOOT_INFO_ADDR + 238], 0
    mov byte [BOOT_INFO_ADDR + 239], 0
    pop eax
    pop ecx
    pop edi
    pop esi

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
%include "exfat.asm"
%include "protected_mode.asm"

; =============================================================================
; post.asm — Power-On Self Test (POST) entry point
; =============================================================================

; ---------------------------------------------------------------------------
; post_entry — Main BIOS initialization. Called from reset vector.
; ---------------------------------------------------------------------------
post_entry:
    cli

    ; Set up segments.
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00              ; Stack below boot sector load area

    ; Initialize PIC (remap IRQs).
    call pic_init

    ; Initialize PIT (timer at ~18.2 Hz).
    call pit_init

    ; Install all interrupt vectors.
    call ivt_setup

    ; Initialize BDA and EBDA.
    call bda_init
    call ebda_init

    ; Initialize serial port (COM1).
    call serial_init

    ; Initialize VGA text mode (clear screen, set cursor).
    call video_init

    ; Enable interrupts (timer ticks start).
    sti

    ; Print POST banner.
    mov si, str_banner
    call bios_print

    ; Always print last INT13 diagnostic state.
    mov si, str_i13_diag
    call bios_print
    mov ax, [cs:int13_call_count]
    call bios_print_dec16
    mov si, str_cd_reentry_ah
    call bios_print
    mov al, [cs:int13_last_ah]
    call bios_print_hex8
    mov si, str_cd_reentry_dl
    call bios_print
    mov al, [cs:int13_last_dl]
    call bios_print_hex8
    mov si, str_cd_reentry_st
    call bios_print
    mov al, [cs:int13_last_status]
    call bios_print_hex8
    mov si, str_crlf
    call bios_print

    ; If we already handed off to an El Torito boot image and reached POST
    ; again, show diagnostic context directly on screen.
    cmp byte [cs:el_torito_handoff], 0
    je .no_cd_reentry_diag

    mov si, str_cd_reentry
    call bios_print

    mov si, str_cd_reentry_ah
    call bios_print
    mov al, [cs:int13_last_ah]
    call bios_print_hex8

    mov si, str_cd_reentry_dl
    call bios_print
    mov al, [cs:int13_last_dl]
    call bios_print_hex8

    mov si, str_cd_reentry_st
    call bios_print
    mov al, [cs:int13_last_status]
    call bios_print_hex8

    mov si, str_crlf
    call bios_print

    ; Clear marker so diagnostics print only once per failed handoff.
    mov byte [cs:el_torito_handoff], 0

.no_cd_reentry_diag:

    ; Detect memory and build E820 table.
    call memory_detect

    ; Print memory size.
    mov si, str_memory
    call bios_print
    mov eax, [cs:ram_size_bytes]
    shr eax, 20                 ; Convert bytes to MB
    call bios_print_dec16
    mov si, str_mb
    call bios_print

    ; Enumerate PCI bus.
    call pci_enumerate

    ; Print PCI device count.
    mov si, str_pci_scan
    call bios_print
    mov ax, [cs:pci_device_count]
    call bios_print_dec16
    mov si, str_pci_device
    call bios_print

    ; Detect IDE drives.
    call ide_detect

    ; Unmask IDE IRQ (IRQ 14) now that handler is installed.
    call pic_unmask_irq14

    ; Print IDE status.
    mov si, str_ide_master
    call bios_print
    cmp byte [cs:ide_master_present], 0
    je .no_ide
    mov si, str_ide_found
    call bios_print
    jmp .ide_done
.no_ide:
    mov si, str_ide_none
    call bios_print
.ide_done:

    ; Run BIOS self-test diagnostics.
    call bios_selftest

    ; Print blank line.
    mov si, str_crlf
    call bios_print

    ; Start boot sequence.
    int 0x19

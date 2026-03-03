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

    ; Detect memory and build E820 table.
    call memory_detect

    ; Print memory size.
    mov si, str_memory
    call bios_print
    mov eax, [ram_size_bytes]
    shr eax, 20                 ; Convert bytes to MB
    call bios_print_dec16
    mov si, str_mb
    call bios_print

    ; Enumerate PCI bus.
    call pci_enumerate

    ; Print PCI device count.
    mov si, str_pci_scan
    call bios_print
    movzx ax, word [pci_device_count]
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
    cmp byte [ide_master_present], 0
    je .no_ide
    mov si, str_ide_found
    call bios_print
    jmp .ide_done
.no_ide:
    mov si, str_ide_none
    call bios_print
.ide_done:

    ; Print blank line.
    mov si, str_crlf
    call bios_print

    ; Start boot sequence.
    int 0x19

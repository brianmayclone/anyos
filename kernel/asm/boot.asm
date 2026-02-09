; =============================================================================
; boot.asm - Kernel boot stub (64-bit long mode)
;
; This code runs AFTER the Stage 2 bootloader has set up long mode with
; 4-level paging (identity + higher-half). Control arrives here in 64-bit
; mode at the physical load address (0x100000), identity-mapped.
;
; Flow:
;   1. Stage 2 jumps here in 64-bit mode with RDI = boot_info_addr
;   2. Jump to higher-half virtual address space
;   3. Zero BSS, set up stack, call kernel_main
; =============================================================================

[BITS 64]

; Kernel stack: placed ABOVE BSS to avoid overlap as the kernel grows.
; Must match KERNEL_STACK_SIZE in kernel/src/memory/physical.rs.
KERNEL_STACK_SIZE equ 0x10000  ; 64 KiB

; Serial port for debug output
COM1            equ 0x3F8

; Linker-provided symbols (virtual addresses)
extern kernel_main
extern _bss_start
extern _bss_end

section .text.boot
global _boot_start

; =============================================================================
; _boot_start - Entry from Stage 2 (64-bit long mode)
;
; Called with RDI = boot_info_addr (physical address, e.g. 0x9000)
; Paging is already enabled: identity map + higher-half map active.
; =============================================================================
_boot_start:
    ; Save boot_info_addr (RDI) in RSI before we use RDI for rep stosq
    mov rsi, rdi

    ; Debug: output 'B' to serial port
    mov dx, COM1 + 5
.wait_b:
    in al, dx
    test al, 0x20
    jz .wait_b
    mov al, 'B'
    mov dx, COM1
    out dx, al

    ; Debug: output '6' (64-bit mode reached)
    mov dx, COM1 + 5
.wait_6:
    in al, dx
    test al, 0x20
    jz .wait_6
    mov al, '6'
    mov dx, COM1
    out dx, al

    ; Jump to higher-half virtual address space
    ; This absolute address is resolved by the linker to the VMA
    mov rax, higher_half_entry
    jmp rax

; =============================================================================
; higher_half_entry - Runs in higher-half virtual address space
; =============================================================================
section .text
higher_half_entry:
    ; Zero the BSS section (before setting up the stack)
    mov rdi, _bss_start
    mov rcx, _bss_end
    sub rcx, rdi
    shr rcx, 3                  ; byte count -> qword count
    xor rax, rax
    rep stosq

    ; Handle remaining bytes if BSS size not multiple of 8
    mov rdi, _bss_start
    mov rcx, _bss_end
    sub rcx, rdi
    and rcx, 7
    xor rax, rax
    rep stosb

    ; Set up kernel stack ABOVE BSS so they never overlap.
    ; Stack grows down from (_bss_end + KERNEL_STACK_SIZE).
    mov rsp, _bss_end
    add rsp, KERNEL_STACK_SIZE

    ; Ensure 16-byte stack alignment (required by System V AMD64 ABI)
    and rsp, -16

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
    ; RSI holds the boot_info_addr saved earlier
    mov rdi, rsi
    call kernel_main

    ; kernel_main should never return, but just in case:
    cli
boot_halt:
    hlt
    jmp boot_halt

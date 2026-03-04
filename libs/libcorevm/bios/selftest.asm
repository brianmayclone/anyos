; =============================================================================
; selftest.asm — BIOS Self-Test (mini POST diagnostics)
; =============================================================================
; Exercises critical BIOS subsystems and reports pass/fail for each.
; Called from POST after hardware init, before boot.
; =============================================================================

selftest_pass_count:  dw 0
selftest_fail_count:  dw 0
selftest_total:       dw 0

; ---------------------------------------------------------------------------
; bios_selftest — Run all self-tests and print results.
; ---------------------------------------------------------------------------
bios_selftest:
    push eax
    push ebx
    push ecx
    push edx
    push esi
    push edi
    push ds
    push es

    mov word [cs:selftest_pass_count], 0
    mov word [cs:selftest_fail_count], 0
    mov word [cs:selftest_total], 0

    mov si, str_st_header
    call bios_print

    ; --- Test 1: Low RAM read/write ---
    call selftest_ram_rw

    ; --- Test 2: BIOS variable persistence (cs: segment) ---
    call selftest_bios_vars

    ; --- Test 3: INT 15h AH=88h extended memory ---
    call selftest_int15_88

    ; --- Test 4: INT 15h AX=E820h memory map ---
    call selftest_int15_e820

    ; --- Test 5: INT 15h A20 gate ---
    call selftest_a20

    ; --- Test 6: INT 1Ah AH=02h RTC time ---
    call selftest_rtc_time

    ; --- Test 7: INT 1Ah AX=B101h PCI BIOS ---
    call selftest_pci_bios

    ; --- Test 8: PCI enumeration count ---
    call selftest_pci_enum

    ; --- Test 9: IDE detection ---
    call selftest_ide

    ; --- Test 10: Timer tick (BDA) ---
    call selftest_timer

    ; --- Test 11: IVT integrity ---
    call selftest_ivt

    ; --- Summary ---
    mov si, str_st_summary
    call bios_print
    mov ax, [cs:selftest_pass_count]
    call bios_print_dec16
    mov si, str_st_slash
    call bios_print
    mov ax, [cs:selftest_total]
    call bios_print_dec16
    mov si, str_st_passed
    call bios_print

    ; Print FAIL warning if any failures.
    mov ax, [cs:selftest_fail_count]
    test ax, ax
    jz .no_failures
    mov si, str_st_warn
    call bios_print
    mov ax, [cs:selftest_fail_count]
    call bios_print_dec16
    mov si, str_st_failed
    call bios_print
.no_failures:
    mov si, str_crlf
    call bios_print

    pop es
    pop ds
    pop edi
    pop esi
    pop edx
    pop ecx
    pop ebx
    pop eax
    ret

; ---------------------------------------------------------------------------
; selftest_print_ok — Print " OK" and bump pass counter.
; ---------------------------------------------------------------------------
selftest_print_ok:
    mov si, str_st_ok
    call bios_print
    inc word [cs:selftest_pass_count]
    inc word [cs:selftest_total]
    ret

; ---------------------------------------------------------------------------
; selftest_print_fail — Print " FAIL" and bump fail counter.
; ---------------------------------------------------------------------------
selftest_print_fail:
    mov si, str_st_fail
    call bios_print
    inc word [cs:selftest_fail_count]
    inc word [cs:selftest_total]
    ret

; ===========================================================================
; Test 1: RAM read/write — write patterns to low RAM and verify.
; ===========================================================================
selftest_ram_rw:
    mov si, str_st_ram
    call bios_print

    ; Use address 0x0500 (safe free area above BDA/IVT).
    xor ax, ax
    mov ds, ax

    ; Test pattern 1: 0x55AA
    mov word [0x0500], 0x55AA
    cmp word [0x0500], 0x55AA
    jne .ram_fail

    ; Test pattern 2: 0xAA55
    mov word [0x0500], 0xAA55
    cmp word [0x0500], 0xAA55
    jne .ram_fail

    ; Test pattern 3: walking byte at 0x0502-0x0509
    mov byte [0x0502], 0x01
    mov byte [0x0503], 0x02
    mov byte [0x0504], 0x04
    mov byte [0x0505], 0x08
    cmp byte [0x0502], 0x01
    jne .ram_fail
    cmp byte [0x0503], 0x02
    jne .ram_fail
    cmp byte [0x0504], 0x04
    jne .ram_fail
    cmp byte [0x0505], 0x08
    jne .ram_fail

    ; Test 32-bit write/read
    mov dword [0x0508], 0xDEADBEEF
    cmp dword [0x0508], 0xDEADBEEF
    jne .ram_fail

    call selftest_print_ok
    ret
.ram_fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 2: BIOS variable persistence — write to cs: and read back.
;         This directly tests that the BIOS ROM area is writable (our fix).
; ===========================================================================
selftest_bios_vars:
    mov si, str_st_biosvar
    call bios_print

    ; Save original value.
    mov ax, [cs:.test_var]
    push ax

    ; Write test pattern.
    mov word [cs:.test_var], 0xCAFE
    cmp word [cs:.test_var], 0xCAFE
    jne .bv_fail

    ; Write second pattern.
    mov word [cs:.test_var], 0xBEEF
    cmp word [cs:.test_var], 0xBEEF
    jne .bv_fail

    ; Restore and pass.
    pop ax
    mov [cs:.test_var], ax
    call selftest_print_ok
    ret
.bv_fail:
    pop ax
    call selftest_print_fail
    mov si, str_st_biosvar_hint
    call bios_print
    ret

.test_var: dw 0

; ===========================================================================
; Test 3: INT 15h AH=88h — Get extended memory size.
; ===========================================================================
selftest_int15_88:
    mov si, str_st_int15_88
    call bios_print

    clc
    mov ah, 0x88
    int 0x15
    jc .i88_fail

    ; AX = extended memory in KB. Should be > 0 for any usable system.
    test ax, ax
    jz .i88_fail

    ; Print result.
    call selftest_print_ok
    mov si, str_st_lparen
    call bios_print
    call bios_print_dec16
    mov si, str_st_kb
    call bios_print
    ret
.i88_fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 4: INT 15h AX=E820h — Memory map.
; ===========================================================================
selftest_int15_e820:
    mov si, str_st_e820
    call bios_print

    ; Set up buffer at 0x0600 in ES:DI.
    xor ax, ax
    mov es, ax
    mov di, 0x0600
    xor ebx, ebx               ; Start at entry 0
    mov edx, 0x534D4150         ; 'SMAP'
    mov ecx, 24
    mov eax, 0xE820
    int 0x15

    jc .e820_fail
    cmp eax, 0x534D4150         ; Signature returned?
    jne .e820_fail

    ; Count entries by iterating.
    xor cx, cx
    inc cx                      ; Already got entry 0
.e820_loop:
    test ebx, ebx
    jz .e820_done               ; EBX=0 means last entry
    mov di, 0x0600
    mov edx, 0x534D4150
    mov ecx, 24
    mov eax, 0xE820
    int 0x15
    jc .e820_done
    inc cx
    jmp .e820_loop

.e820_done:
    ; CX = number of entries. Should be >= 4 (conv + EBDA + ROM + ext).
    cmp cx, 4
    jb .e820_fail

    push cx
    call selftest_print_ok
    mov si, str_st_lparen
    call bios_print
    pop ax
    call bios_print_dec16
    mov si, str_st_entries
    call bios_print
    ret
.e820_fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 5: A20 gate status.
; ===========================================================================
selftest_a20:
    mov si, str_st_a20
    call bios_print

    mov ax, 0x2402
    int 0x15
    jc .a20_fail

    ; AL should be 1 (A20 enabled) in our hypervisor.
    cmp al, 1
    jne .a20_fail

    call selftest_print_ok
    ret
.a20_fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 6: RTC time (INT 1Ah AH=02h).
; ===========================================================================
selftest_rtc_time:
    mov si, str_st_rtc
    call bios_print

    mov ah, 0x02
    int 0x1A
    jc .rtc_fail

    ; Print time: CH=hours, CL=minutes, DH=seconds (all BCD).
    call selftest_print_ok
    mov si, str_st_lparen
    call bios_print

    mov al, ch
    call bios_print_hex8
    mov si, str_st_colon
    call bios_print
    mov al, cl
    call bios_print_hex8
    mov si, str_st_colon
    call bios_print
    mov al, dh
    call bios_print_hex8

    mov si, str_st_rparen
    call bios_print
    ret
.rtc_fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 7: PCI BIOS present (INT 1Ah AX=B101h).
; ===========================================================================
selftest_pci_bios:
    mov si, str_st_pci_bios
    call bios_print

    mov ax, 0xB101
    int 0x1A
    jc .pcib_fail

    ; EDX should be "PCI " = 0x20494350.
    cmp edx, 0x20494350
    jne .pcib_fail

    ; AH should be 0 (success).
    test ah, ah
    jnz .pcib_fail

    ; BH.BL = version (2.1 = BH=2, BL=0x10).
    push bx
    call selftest_print_ok
    mov si, str_st_lparen
    call bios_print
    mov si, str_st_pci_ver
    call bios_print
    pop bx
    mov al, bh
    call bios_print_hex8
    mov si, str_st_dot
    call bios_print
    mov al, bl
    call bios_print_hex8
    mov si, str_st_rparen
    call bios_print
    ret
.pcib_fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 8: PCI device enumeration.
; ===========================================================================
selftest_pci_enum:
    mov si, str_st_pci_enum
    call bios_print

    mov ax, [cs:pci_device_count]
    ; Even 0 devices is valid; but the count should be < PCI_TABLE_MAX.
    cmp ax, PCI_TABLE_MAX
    ja .pcie_fail

    push ax
    call selftest_print_ok
    mov si, str_st_lparen
    call bios_print
    pop ax
    call bios_print_dec16
    mov si, str_st_devs
    call bios_print
    ret
.pcie_fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 9: IDE detection result.
; ===========================================================================
selftest_ide:
    mov si, str_st_ide
    call bios_print

    ; Check if ide_master_present is 0 or 1 (valid values).
    mov al, [cs:ide_master_present]
    cmp al, 1
    ja .ide_fail

    call selftest_print_ok
    mov si, str_st_lparen
    call bios_print

    cmp byte [cs:ide_master_present], 0
    je .ide_not_present
    mov si, str_ide_found
    jmp .ide_print_status
.ide_not_present:
    mov si, str_ide_none
.ide_print_status:
    call bios_print
    mov si, str_st_rparen_nocrlf
    call bios_print
    mov si, str_crlf
    call bios_print
    ret
.ide_fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 10: Timer tick — read BDA timer, wait briefly, verify it increments.
; ===========================================================================
selftest_timer:
    mov si, str_st_timer
    call bios_print

    ; Read initial tick count.
    mov ah, 0x00
    int 0x1A                    ; CX:DX = tick count
    mov bx, dx                  ; Save low word

    ; Spin-wait for tick to change (up to ~64K iterations).
    mov cx, 0xFFFF
.timer_wait:
    push cx
    mov ah, 0x00
    int 0x1A
    pop cx
    cmp dx, bx                  ; Changed?
    jne .timer_ok
    dec cx
    jnz .timer_wait

    ; Timed out — timer did not tick.
    call selftest_print_fail
    ret
.timer_ok:
    call selftest_print_ok
    ret

; ===========================================================================
; Test 11: IVT integrity — check key interrupt vectors are non-zero.
; ===========================================================================
selftest_ivt:
    mov si, str_st_ivt
    call bios_print

    xor ax, ax
    mov ds, ax

    ; Check INT 10h vector (video).
    mov eax, [0x10 * 4]
    test eax, eax
    jz .ivt_fail

    ; Check INT 13h vector (disk).
    mov eax, [0x13 * 4]
    test eax, eax
    jz .ivt_fail

    ; Check INT 15h vector (system).
    mov eax, [0x15 * 4]
    test eax, eax
    jz .ivt_fail

    ; Check INT 16h vector (keyboard).
    mov eax, [0x16 * 4]
    test eax, eax
    jz .ivt_fail

    ; Check INT 19h vector (boot).
    mov eax, [0x19 * 4]
    test eax, eax
    jz .ivt_fail

    ; Check INT 1Ah vector (PCI/RTC).
    mov eax, [0x1A * 4]
    test eax, eax
    jz .ivt_fail

    call selftest_print_ok
    ret
.ivt_fail:
    call selftest_print_fail
    ret

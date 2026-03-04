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

    call selftest_ram_rw
    call selftest_bios_vars
    call selftest_int15_88
    call selftest_int15_e820
    call selftest_a20
    call selftest_a20_wire
    call selftest_rtc_time
    call selftest_pci_bios
    call selftest_pci_enum
    call selftest_ide
    call selftest_int13_read
    call selftest_timer
    call selftest_ivt
    call selftest_vga_mem
    call selftest_cpuid
    call selftest_pmode
    call selftest_extmem

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
; Helpers: print OK/FAIL, bump counters, print CRLF.
; ---------------------------------------------------------------------------
selftest_print_ok:
    mov si, str_st_ok
    call bios_print
    mov si, str_crlf
    call bios_print
    inc word [cs:selftest_pass_count]
    inc word [cs:selftest_total]
    ret

; Print " OK" WITHOUT newline (for tests that append details).
selftest_print_ok_detail:
    mov si, str_st_ok
    call bios_print
    inc word [cs:selftest_pass_count]
    inc word [cs:selftest_total]
    ret

selftest_print_fail:
    mov si, str_st_fail
    call bios_print
    mov si, str_crlf
    call bios_print
    inc word [cs:selftest_fail_count]
    inc word [cs:selftest_total]
    ret

; Print " FAIL" WITHOUT newline (for tests that append details).
selftest_print_fail_detail:
    mov si, str_st_fail
    call bios_print
    inc word [cs:selftest_fail_count]
    inc word [cs:selftest_total]
    ret

; ===========================================================================
; Test 1: RAM read/write
; ===========================================================================
selftest_ram_rw:
    mov si, str_st_ram
    call bios_print

    xor ax, ax
    mov ds, ax

    mov word [0x0500], 0x55AA
    cmp word [0x0500], 0x55AA
    jne .fail

    mov word [0x0500], 0xAA55
    cmp word [0x0500], 0xAA55
    jne .fail

    mov byte [0x0502], 0x01
    mov byte [0x0503], 0x02
    mov byte [0x0504], 0x04
    mov byte [0x0505], 0x08
    cmp byte [0x0502], 0x01
    jne .fail
    cmp byte [0x0503], 0x02
    jne .fail
    cmp byte [0x0504], 0x04
    jne .fail
    cmp byte [0x0505], 0x08
    jne .fail

    mov dword [0x0508], 0xDEADBEEF
    cmp dword [0x0508], 0xDEADBEEF
    jne .fail

    call selftest_print_ok
    ret
.fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 2: BIOS variable persistence (cs: writable)
; ===========================================================================
selftest_bios_vars:
    mov si, str_st_biosvar
    call bios_print

    mov ax, [cs:.test_var]
    push ax

    mov word [cs:.test_var], 0xCAFE
    cmp word [cs:.test_var], 0xCAFE
    jne .fail

    mov word [cs:.test_var], 0xBEEF
    cmp word [cs:.test_var], 0xBEEF
    jne .fail

    pop ax
    mov [cs:.test_var], ax
    call selftest_print_ok
    ret
.fail:
    pop ax
    call selftest_print_fail_detail
    mov si, str_st_biosvar_hint
    call bios_print
    ret

.test_var: dw 0

; ===========================================================================
; Test 3: INT 15h AH=88h — extended memory size
; ===========================================================================
selftest_int15_88:
    mov si, str_st_int15_88
    call bios_print

    clc
    mov ah, 0x88
    int 0x15
    jc .fail

    test ax, ax
    jz .fail

    push ax
    call selftest_print_ok_detail
    mov si, str_st_lparen
    call bios_print
    pop ax
    call bios_print_dec16
    mov si, str_st_kb
    call bios_print
    ret
.fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 4: INT 15h AX=E820h — memory map
; ===========================================================================
selftest_int15_e820:
    mov si, str_st_e820
    call bios_print

    ; Use BP as entry counter (not clobbered by INT 15h).
    push bp
    xor bp, bp

    xor ax, ax
    mov es, ax
    mov di, 0x0600
    xor ebx, ebx
    mov edx, 0x534D4150
    mov ecx, 24
    mov eax, 0xE820
    int 0x15

    jc .fail
    cmp eax, 0x534D4150
    jne .fail
    inc bp                      ; Entry 0 counted

.loop:
    test ebx, ebx
    jz .done
    mov di, 0x0600
    mov edx, 0x534D4150
    mov ecx, 24
    mov eax, 0xE820
    int 0x15
    jc .done
    inc bp
    jmp .loop

.done:
    cmp bp, 4
    jb .fail_pop

    mov ax, bp
    pop bp
    push ax
    call selftest_print_ok_detail
    mov si, str_st_lparen
    call bios_print
    pop ax
    call bios_print_dec16
    mov si, str_st_entries
    call bios_print
    ret
.fail_pop:
    pop bp
.fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 5: A20 gate status (BIOS function)
; ===========================================================================
selftest_a20:
    mov si, str_st_a20
    call bios_print

    mov ax, 0x2402
    int 0x15
    jc .fail

    cmp al, 1
    jne .fail

    call selftest_print_ok
    ret
.fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 6: A20 line wire test — verify address line 20 works.
; ===========================================================================
selftest_a20_wire:
    mov si, str_st_a20_wire
    call bios_print

    xor ax, ax
    mov ds, ax
    mov ax, 0xFFFF
    mov es, ax

    ; DS:0x0500 = phys 0x00500
    ; ES:0x0510 = phys 0x100500 (A20 on) or 0x00500 (A20 off)
    mov al, [ds:0x0500]
    push ax
    mov al, [es:0x0510]
    push ax

    mov byte [ds:0x0500], 0x00
    mov byte [es:0x0510], 0xFF

    cmp byte [ds:0x0500], 0x00
    jne .fail

    cmp byte [es:0x0510], 0xFF
    jne .fail

    pop ax
    mov [es:0x0510], al
    pop ax
    mov [ds:0x0500], al
    call selftest_print_ok
    ret
.fail:
    pop ax
    mov [es:0x0510], al
    pop ax
    mov [ds:0x0500], al
    call selftest_print_fail
    ret

; ===========================================================================
; Test 7: RTC time
; ===========================================================================
selftest_rtc_time:
    mov si, str_st_rtc
    call bios_print

    mov ah, 0x02
    int 0x1A
    jc .fail

    push cx
    push dx
    call selftest_print_ok_detail
    mov si, str_st_lparen
    call bios_print

    pop dx
    pop cx
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
.fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 8: PCI BIOS present
; ===========================================================================
selftest_pci_bios:
    mov si, str_st_pci_bios
    call bios_print

    mov ax, 0xB101
    int 0x1A
    jc .fail

    cmp edx, 0x20494350
    jne .fail

    test ah, ah
    jnz .fail

    push bx
    call selftest_print_ok_detail
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
.fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 9: PCI device enumeration
; ===========================================================================
selftest_pci_enum:
    mov si, str_st_pci_enum
    call bios_print

    mov ax, [cs:pci_device_count]
    cmp ax, PCI_TABLE_MAX
    ja .fail

    push ax
    call selftest_print_ok_detail
    mov si, str_st_lparen
    call bios_print
    pop ax
    call bios_print_dec16
    mov si, str_st_devs
    call bios_print
    ret
.fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 10: IDE detection
; ===========================================================================
selftest_ide:
    mov si, str_st_ide
    call bios_print

    mov al, [cs:ide_master_present]
    cmp al, 1
    ja .fail

    call selftest_print_ok_detail

    cmp byte [cs:ide_master_present], 0
    je .not_present
    mov si, str_st_lparen
    call bios_print
    mov si, str_st_yes
    call bios_print
    mov si, str_st_rparen
    call bios_print
    ret
.not_present:
    mov si, str_st_lparen
    call bios_print
    mov si, str_st_no
    call bios_print
    mov si, str_st_rparen
    call bios_print
    ret
.fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 11: INT 13h disk read
; ===========================================================================
selftest_int13_read:
    mov si, str_st_int13_rd
    call bios_print

    cmp byte [cs:ide_master_present], 0
    je .skip

    xor ax, ax
    mov es, ax
    mov bx, 0x0600
    mov ax, 0x0201              ; AH=02 read, AL=1 sector
    mov cx, 0x0001              ; C=0, S=1
    mov dh, 0x00                ; H=0
    mov dl, 0x80                ; First HD
    int 0x13
    jc .fail

    ; Check MBR signature.
    cmp word [es:0x07FE], 0xAA55
    je .mbr
    call selftest_print_ok
    ret
.mbr:
    call selftest_print_ok_detail
    mov si, str_st_lparen
    call bios_print
    mov si, str_st_mbr_sig
    call bios_print
    mov si, str_st_rparen
    call bios_print
    ret

.skip:
    call selftest_print_ok_detail
    mov si, str_st_lparen
    call bios_print
    mov si, str_st_skipped
    call bios_print
    mov si, str_st_rparen
    call bios_print
    ret

.fail:
    push ax
    call selftest_print_fail_detail
    mov si, str_st_lparen
    call bios_print
    mov si, str_st_ah_eq
    call bios_print
    pop ax
    mov al, ah
    call bios_print_hex8
    mov si, str_st_rparen
    call bios_print
    ret

; ===========================================================================
; Test 12: Timer tick — verify PIT fires IRQ 0 and BDA timer increments.
; ===========================================================================
selftest_timer:
    mov si, str_st_timer
    call bios_print

    ; Read BDA timer count directly (faster than INT 1Ah).
    xor ax, ax
    mov ds, ax
    mov eax, [BDA_TIMER_COUNT]
    mov ebx, eax                ; Save initial value

    ; Spin waiting for a tick.  PIT fires at ~18.2 Hz (every ~55ms).
    ; With wall-clock-based PIT, we just need enough iterations for
    ; ~55ms of real time to elapse.  Use a generous limit.
    mov ecx, 0x00FFFFFF
.wait:
    cmp [BDA_TIMER_COUNT], ebx
    jne .ok
    dec ecx
    jnz .wait

    call selftest_print_fail
    ret
.ok:
    call selftest_print_ok
    ret

; ===========================================================================
; Test 13: IVT integrity
; ===========================================================================
selftest_ivt:
    mov si, str_st_ivt
    call bios_print

    xor ax, ax
    mov ds, ax

    mov eax, [0x10 * 4]        ; INT 10h (video)
    test eax, eax
    jz .fail
    mov eax, [0x13 * 4]        ; INT 13h (disk)
    test eax, eax
    jz .fail
    mov eax, [0x15 * 4]        ; INT 15h (system)
    test eax, eax
    jz .fail
    mov eax, [0x16 * 4]        ; INT 16h (keyboard)
    test eax, eax
    jz .fail
    mov eax, [0x19 * 4]        ; INT 19h (boot)
    test eax, eax
    jz .fail
    mov eax, [0x1A * 4]        ; INT 1Ah (PCI/RTC)
    test eax, eax
    jz .fail

    call selftest_print_ok
    ret
.fail:
    call selftest_print_fail
    ret

; ===========================================================================
; Test 14: VGA text mode memory at 0xB8000
; ===========================================================================
selftest_vga_mem:
    mov si, str_st_vga
    call bios_print

    mov ax, 0xB800
    mov es, ax

    ; Use off-screen position (row 24, col 79).
    mov ax, [es:3998]
    push ax

    mov word [es:3998], 0x4E54
    cmp word [es:3998], 0x4E54
    jne .fail

    mov word [es:3998], 0x0742
    cmp word [es:3998], 0x0742
    jne .fail

    pop ax
    mov [es:3998], ax
    call selftest_print_ok
    ret
.fail:
    pop ax
    mov [es:3998], ax
    call selftest_print_fail
    ret

; ===========================================================================
; Test 15: CPUID
; ===========================================================================
selftest_cpuid:
    mov si, str_st_cpuid
    call bios_print

    ; Check CPUID availability via EFLAGS.ID (bit 21).
    pushfd
    pop eax
    mov ecx, eax
    xor eax, (1 << 21)
    push eax
    popfd
    pushfd
    pop eax
    push ecx
    popfd
    xor eax, ecx
    test eax, (1 << 21)
    jz .fail

    ; CPUID EAX=0: vendor string.
    xor eax, eax
    cpuid

    push eax                    ; Max level
    push ecx                    ; Vendor[8..11]
    push edx                    ; Vendor[4..7]
    push ebx                    ; Vendor[0..3]

    call selftest_print_ok_detail
    mov si, str_st_lparen
    call bios_print

    ; Print vendor: EBX EDX ECX.
    pop eax
    call .print_4chars
    pop eax
    call .print_4chars
    pop eax
    call .print_4chars

    mov si, str_st_comma_lv
    call bios_print
    pop eax
    call bios_print_hex8

    mov si, str_st_rparen
    call bios_print
    ret

.fail:
    call selftest_print_fail
    ret

; Print 4 ASCII chars from EAX (low byte first).
.print_4chars:
    push eax
    push ecx
    push bx
    mov cx, 4
.p4c_loop:
    push ax
    mov ah, 0x0E
    mov bx, 0x0007
    int 0x10
    pop ax
    shr eax, 8
    dec cx
    jnz .p4c_loop
    pop bx
    pop ecx
    pop eax
    ret

; ===========================================================================
; Test 16: Protected mode — enter and return to real mode.
;   Critical for ISOLINUX/Linux which switch to PM immediately.
;
;   GDT layout:
;     0x08 — Code 16-bit, base=0xF0000, limit=0xFFFF  (BIOS ROM offsets)
;     0x10 — Data flat,    base=0,       limit=4GB     (all physical memory)
;   The code segment base matches CS=0xF000 in real mode, so the same
;   label offsets work in both real mode and protected mode.
; ===========================================================================
selftest_pmode:
    mov si, str_st_pmode
    call bios_print

    cli

    mov [cs:.save_ds], ds
    mov [cs:.save_es], es
    mov [cs:.save_ss], ss
    mov [cs:.save_sp], sp

    ; Load GDT.
    lgdt [cs:.gdt_ptr]

    ; Set CR0.PE.
    mov eax, cr0
    or al, 1
    mov cr0, eax

    ; Far jump to enter PM.  Selector 0x08 has base=0xF0000, so the
    ; 16-bit offset .in_pm maps to physical 0xF0000 + .in_pm — correct.
    jmp 0x08:.in_pm

.in_pm:
    ; 16-bit protected mode.  DS/ES/SS use selector 0x10 (base=0, flat).
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax

    ; Write marker to prove PM memory access works (DS base=0 → flat).
    mov dword [0x0510], 0x504D4F4B  ; "PMOK"

    ; Return to real mode: clear CR0.PE.
    mov eax, cr0
    and al, 0xFE
    mov cr0, eax

    ; Far jump reloads CS with real-mode segment 0xF000.
    jmp 0xF000:.back_real

.back_real:
    mov ax, [cs:.save_ds]
    mov ds, ax
    mov ax, [cs:.save_es]
    mov es, ax
    mov ax, [cs:.save_ss]
    mov ss, ax
    mov sp, [cs:.save_sp]

    sti

    ; Verify the marker.
    xor ax, ax
    mov ds, ax
    cmp dword [0x0510], 0x504D4F4B
    jne .fail

    call selftest_print_ok
    ret
.fail:
    call selftest_print_fail
    ret

.save_ds: dw 0
.save_es: dw 0
.save_ss: dw 0
.save_sp: dw 0

; Minimal GDT shared by PM test and extmem test.
align 8
.gdt:
    dq 0                        ; Null descriptor (0x00)

    ; Code 16-bit (0x08): base=0xF0000, limit=0xFFFF, exec/read.
    ; Base matches the real-mode CS=0xF000 so label offsets are the same.
    dw 0xFFFF                   ; Limit 15:0
    dw 0x0000                   ; Base 15:0
    db 0x0F                     ; Base 23:16  (0x0F → base = 0x0F_0000)
    db 10011010b                ; P=1, DPL=0, S=1, Type=Execute/Read
    db 0x00                     ; G=0, D=0 (16-bit), Limit 19:16=0
    db 0x00                     ; Base 31:24

    ; Data flat (0x10): base=0, limit=4GB, read/write.
    dw 0xFFFF                   ; Limit 15:0
    dw 0x0000                   ; Base 15:0
    db 0x00                     ; Base 23:16
    db 10010010b                ; P=1, DPL=0, S=1, Type=Read/Write
    db 0x8F                     ; G=1, B=0, Limit 19:16=0xF → 4GB
    db 0x00                     ; Base 31:24
.gdt_end:

.gdt_ptr:
    dw .gdt_end - .gdt - 1
    dd .gdt + 0xF0000           ; Physical address of GDT

; ===========================================================================
; Test 17: Extended memory — read/write above 1MB via protected mode.
;   Uses 16-bit PM with the flat data segment (base=0, limit=4GB) and
;   the a32 address-size prefix to reach addresses above 0xFFFF.
; ===========================================================================
selftest_extmem:
    mov si, str_st_extmem
    call bios_print

    cmp dword [cs:ram_size_bytes], 0x200000
    jb .skip

    cli

    mov [cs:.save_ds], ds
    mov [cs:.save_es], es
    mov [cs:.save_ss], ss
    mov [cs:.save_sp], sp

    ; Load GDT (shared with PM test).
    lgdt [cs:selftest_pmode.gdt_ptr]

    ; Enter protected mode.
    mov eax, cr0
    or al, 1
    mov cr0, eax
    jmp 0x08:.in_pm

.in_pm:
    ; Flat data segment for 4GB access.
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax

    ; Write test patterns at physical 1MB using a32 address override.
    a32 mov dword [0x100000], 0xE740CAFE
    a32 mov dword [0x100004], 0x12345678

    ; Read back into low memory (results at 0x0520/0x0524).
    a32 mov eax, [0x100000]
    mov [0x0520], eax
    a32 mov eax, [0x100004]
    mov [0x0524], eax

    ; Clean up.
    a32 mov dword [0x100000], 0
    a32 mov dword [0x100004], 0

    ; Return to real mode.
    mov eax, cr0
    and al, 0xFE
    mov cr0, eax
    jmp 0xF000:.back_real

.back_real:
    mov ax, [cs:.save_ds]
    mov ds, ax
    mov ax, [cs:.save_es]
    mov es, ax
    mov ax, [cs:.save_ss]
    mov ss, ax
    mov sp, [cs:.save_sp]
    sti

    ; Verify results from low memory.
    xor ax, ax
    mov ds, ax
    cmp dword [0x0520], 0xE740CAFE
    jne .fail
    cmp dword [0x0524], 0x12345678
    jne .fail

    call selftest_print_ok
    ret

.skip:
    call selftest_print_ok_detail
    mov si, str_st_lparen
    call bios_print
    mov si, str_st_skipped
    call bios_print
    mov si, str_st_rparen
    call bios_print
    ret
.fail:
    call selftest_print_fail
    ret

.save_ds: dw 0
.save_es: dw 0
.save_ss: dw 0
.save_sp: dw 0

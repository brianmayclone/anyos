; =============================================================================
; a20.asm - Enable A20 address line
; =============================================================================

; Try multiple methods to enable A20
enable_a20:
    pusha

    ; Check if A20 is already enabled
    call check_a20
    cmp ax, 1
    je .done

    ; Method 1: BIOS INT 15h
    mov ax, 0x2401
    int 0x15
    call check_a20
    cmp ax, 1
    je .done

    ; Method 2: Keyboard controller
    call a20_keyboard
    call check_a20
    cmp ax, 1
    je .done

    ; Method 3: Fast A20 (port 0x92)
    in al, 0x92
    or al, 2
    and al, 0xFE           ; Don't reset CPU (bit 0)
    out 0x92, al
    call check_a20
    cmp ax, 1
    je .done

    ; All methods failed - print error and halt
    mov si, msg_a20_fail
    call print_string_16
    cli
    hlt

.done:
    popa
    ret

; Enable A20 via keyboard controller
a20_keyboard:
    cli
    call a20_wait_input
    mov al, 0xAD            ; Disable keyboard
    out 0x64, al
    call a20_wait_input
    mov al, 0xD0            ; Read output port
    out 0x64, al
    call a20_wait_output
    in al, 0x60
    push ax
    call a20_wait_input
    mov al, 0xD1            ; Write output port
    out 0x64, al
    call a20_wait_input
    pop ax
    or al, 2               ; Enable A20 bit
    out 0x60, al
    call a20_wait_input
    mov al, 0xAE            ; Re-enable keyboard
    out 0x64, al
    call a20_wait_input
    sti
    ret

a20_wait_input:
    in al, 0x64
    test al, 2
    jnz a20_wait_input
    ret

a20_wait_output:
    in al, 0x64
    test al, 1
    jz a20_wait_output
    ret

; Check if A20 is enabled using wrap-around test
; Returns AX=1 if enabled, AX=0 if disabled
check_a20:
    pushf
    push ds
    push es
    push di
    push si

    xor ax, ax
    mov es, ax              ; ES = 0x0000
    mov di, 0x0500

    not ax
    mov ds, ax              ; DS = 0xFFFF
    mov si, 0x0510          ; 0xFFFF:0x0510 = 0x100500 (or 0x0500 if A20 off)

    ; Save original bytes
    mov al, [es:di]
    push ax
    mov al, [ds:si]
    push ax

    ; Write test values
    mov byte [es:di], 0x00
    mov byte [ds:si], 0xFF

    ; Check if they wrap around
    cmp byte [es:di], 0xFF

    ; Restore original bytes
    pop ax
    mov [ds:si], al
    pop ax
    mov [es:di], al

    ; Set return value
    mov ax, 0
    je .done                ; If equal, A20 is disabled (wraps)
    mov ax, 1               ; A20 is enabled (no wrap)

.done:
    pop si
    pop di
    pop es
    pop ds
    popf
    ret

msg_a20_fail: db "FATAL: Cannot enable A20!", 13, 10, 0

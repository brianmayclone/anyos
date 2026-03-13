; =============================================================================
; chainload.asm - Boot entry execution
; =============================================================================

; execute_entry - Execute a boot menu entry
; Input: BX = entry index (0-based)
; For type=0 (kernel): copies params to BOOT_INFO_ADDR+44, returns
; For type=1 (chainload): chainloads, does not return
execute_entry:
    ; Calculate entry pointer: cfg_entries + BX * 128
    movzx eax, bx
    shl eax, 7                     ; * 128
    add eax, cfg_entries            ; ESI = pointer to entry

    ; Check entry type at offset 96
    mov esi, eax
    cmp byte [esi + 96], 0
    je .type_kernel
    cmp byte [esi + 96], 1
    je .type_chainload
    ret                             ; Unknown type, just return

.type_kernel:
    ; Check if params == "custom" (interactive input)
    a32 cmp dword [esi + 32], 'cust'
    jne .copy_params
    a32 cmp word [esi + 36], 'om'
    jne .copy_params
    a32 cmp byte [esi + 38], 0
    jne .copy_params

    ; Interactive: prompt user for custom boot params
    call read_custom_params
    ret

.copy_params:
    ; Copy 64 bytes of params from entry+32 to BOOT_INFO_ADDR+44
    add esi, 32                     ; Source: entry + 32
    mov edi, BOOT_INFO_ADDR + 44   ; Destination
    mov ecx, 64
    a32 rep movsb
    ret

.type_chainload:
    ; Get disk number (offset 97) and partition (offset 98)
    movzx edx, byte [esi + 97]     ; disk number
    add dl, 0x80                    ; Convert to BIOS drive number
    push dx                         ; Save drive number
    movzx ebx, byte [esi + 98]     ; partition number

    ; Modify kernel_load_dap buffer to point to 0x0000:0x7C00
    mov word [kernel_load_dap + 4], 0x7C00  ; offset
    mov word [kernel_load_dap + 6], 0x0000  ; segment
    mov word [kernel_load_dap + 2], 1       ; 1 sector
    mov dword [kernel_load_dap + 8], 0      ; LBA 0 (MBR)
    mov dword [kernel_load_dap + 12], 0     ; LBA high

    ; Read MBR (sector 0) of target disk to 0x7C00
    pop dx                          ; DL = BIOS drive
    push dx                         ; Save again
    push bx                         ; Save partition number
    mov ah, 0x42
    mov si, kernel_load_dap
    int 0x13
    jc .disk_error

    ; Find partition start LBA from MBR partition table
    pop bx                          ; partition number
    pop dx                          ; drive number
    push dx
    ; Partition entry = 0x7C00 + 0x1BE + partition * 16
    movzx eax, bx
    shl eax, 4                     ; * 16
    add eax, 0x7C00 + 0x1BE
    ; Start LBA at offset +8 within partition entry
    mov eax, [eax + 8]

    ; Load first sector of partition to 0x7C00
    mov dword [kernel_load_dap + 8], eax    ; LBA = partition start
    mov dword [kernel_load_dap + 12], 0
    pop dx                          ; DL = BIOS drive
    push dx                         ; Save for final setup
    mov ah, 0x42
    mov si, kernel_load_dap
    int 0x13
    jc .disk_error

    ; Reset video to text mode
    mov ax, 0x0003
    int 0x10

    ; Set up registers and jump to VBR
    pop dx                          ; DL = BIOS drive number
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00
    jmp 0x0000:0x7C00

.disk_error:
    ; Clean up stack
    add sp, 4                       ; Remove saved bx + dx (or just dx)
    ; Reset video to text mode
    mov ax, 0x0003
    int 0x10
    mov si, .msg_disk_error
    call print_string_16
.halt:
    cli
    hlt
    jmp .halt

.msg_disk_error: db "Chainload: disk read error", 13, 10, 0

; -----------------------------------------------------------------------------
; read_custom_params - Draw input prompt on screen, read keyboard into buffer
;   Writes result to BOOT_INFO_ADDR+44 (max 63 chars + null)
; -----------------------------------------------------------------------------
read_custom_params:
    pusha

    ; Clear screen and draw prompt
    call clear_screen

    ; Draw prompt text centered
    mov esi, .prompt_str
    mov ecx, 300                    ; Y position
    mov edx, 0x00AACCFF            ; light blue
    call draw_string_centered

    ; Draw hint below
    mov esi, .hint_str
    mov ecx, 324                    ; Y = 300 + 24
    mov edx, 0x00666666            ; gray
    call draw_string_centered

    ; Initialize input buffer and cursor
    mov edi, BOOT_INFO_ADDR + 44
    xor ecx, ecx                   ; cursor position (char count)

    ; Draw initial cursor
    call .draw_input_line

.input_loop:
    ; Wait for keypress (blocking)
    mov ah, 0x00
    int 0x16
    ; AH = scan code, AL = ASCII

    ; Enter = done
    cmp ah, 0x1C
    je .input_done

    ; Backspace
    cmp ah, 0x0E
    je .backspace

    ; Escape = cancel (empty params)
    cmp ah, 0x01
    je .input_cancel

    ; Printable ASCII? (32-126)
    cmp al, 32
    jb .input_loop
    cmp al, 126
    ja .input_loop

    ; Max 63 chars
    cmp ecx, 63
    jge .input_loop

    ; Store character
    a32 mov [edi + ecx], al
    inc ecx

    ; Redraw input line
    call .draw_input_line
    jmp .input_loop

.backspace:
    test ecx, ecx
    jz .input_loop
    dec ecx
    a32 mov byte [edi + ecx], 0

    ; Redraw input line
    call .draw_input_line
    jmp .input_loop

.input_cancel:
    ; Zero out params
    xor ecx, ecx

.input_done:
    ; Null-terminate
    a32 mov byte [edi + ecx], 0

    popa
    ret

; -- Draw the input line at Y=360, centered --
.draw_input_line:
    push ecx
    push esi
    push edx

    ; First clear the input area (draw black bar at Y=356, 20px tall)
    push ecx
    push edi
    mov edi, [0x2000 + 0x28]       ; PhysBasePtr
    movzx eax, word [0x2000 + 0x10]; pitch
    mov ebx, eax                    ; save pitch
    imul eax, 356                   ; Y * pitch
    add edi, eax
    mov ecx, 20                     ; 20 rows
.clear_bar:
    push ecx
    push edi
    movzx ecx, word [0x2000 + 0x12]; screen width
    xor eax, eax                    ; black
.clear_px:
    a32 mov [edi], eax
    add edi, 4
    dec ecx
    jnz .clear_px
    pop edi
    add edi, ebx                    ; next row
    pop ecx
    dec ecx
    jnz .clear_bar
    pop edi
    pop ecx

    ; Draw the current input text
    ; Null-terminate at cursor position for drawing
    push ecx
    a32 mov byte [edi + ecx], 0     ; temporary null-terminate
    mov esi, edi                    ; ESI = input buffer
    mov ecx, 360                    ; Y position
    mov edx, 0x00FFFFFF            ; white
    call draw_string_centered
    pop ecx

    ; Draw cursor character '_' after the text
    ; (We just drew centered text, so cursor is tricky.
    ;  For simplicity, draw the full string + '_' as one unit)
    a32 mov byte [edi + ecx], '_'   ; append cursor
    push ecx
    inc ecx
    a32 mov byte [edi + ecx], 0     ; null-terminate after cursor
    pop ecx
    mov esi, edi
    push ecx
    mov ecx, 360
    mov edx, 0x00FFFFFF
    call draw_string_centered
    pop ecx
    ; Remove cursor from buffer
    a32 mov byte [edi + ecx], 0

    pop edx
    pop esi
    pop ecx
    ret

.prompt_str: db 'Enter boot parameters:', 0
.hint_str:   db 'verbose  res=1920x1080  (Enter to confirm, Esc to cancel)', 0

; =============================================================================
; config.asm - Boot configuration parser
; =============================================================================
; Parses INI-style boot.cfg from CONFIG_LOAD_ADDR (0x30000)
; Entry structure: 128 bytes per entry (max 8 entries)
;   Offset  0-31:  Name (null-terminated)
;   Offset 32-95:  Params (null-terminated, 64 bytes)
;   Byte   96:     type (0=kernel, 1=chainload)
;   Byte   97:     disk number
;   Byte   98:     partition number
;   Byte   99-127: reserved
; =============================================================================

cfg_timeout:    dw 3
cfg_default:    db 0
cfg_count:      db 0
cfg_entries:    times 128*8 db 0

; =============================================================================
; parse_config — main entry point
; =============================================================================
parse_config:
    pusha
    push es

    ; ESI = config buffer (use a32 prefix for 32-bit addressing in unreal mode)
    mov esi, CONFIG_LOAD_ADDR
    mov byte [cfg_count], 0
    mov word [cfg_timeout], 3
    mov byte [cfg_default], 0

.parse_loop:
    ; Check for end of buffer (null byte)
    a32 cmp byte [esi], 0
    je .done

    ; Skip leading whitespace
    call skip_whitespace

    ; Check for null again after whitespace
    a32 cmp byte [esi], 0
    je .done

    ; Check for comment line
    a32 cmp byte [esi], '#'
    je .skip_line

    ; Check for empty line (CR or LF)
    a32 cmp byte [esi], 0x0A
    je .skip_lf
    a32 cmp byte [esi], 0x0D
    je .skip_cr

    ; Check for section header [Name]
    a32 cmp byte [esi], '['
    je .parse_section

    ; Otherwise it's a key=value line
    call parse_key_value
    jmp .parse_loop

.skip_line:
    call advance_to_next_line
    jmp .parse_loop

.skip_lf:
    inc esi
    jmp .parse_loop

.skip_cr:
    inc esi
    ; Skip optional LF after CR
    a32 cmp byte [esi], 0x0A
    jne .parse_loop
    inc esi
    jmp .parse_loop

; ---- Parse [SectionName] ----
.parse_section:
    ; Check if we already have 8 entries
    cmp byte [cfg_count], 8
    jge .skip_line

    inc esi                     ; skip '['

    ; EDI = pointer into current entry's name field
    movzx eax, byte [cfg_count]
    shl eax, 7                  ; * 128
    add eax, cfg_entries
    mov edi, eax

    xor ecx, ecx               ; char count
.copy_name:
    a32 mov al, [esi]
    cmp al, ']'
    je .name_done
    cmp al, 0x0A
    je .name_done
    cmp al, 0x0D
    je .name_done
    cmp al, 0
    je .name_done
    cmp ecx, 31                ; max 31 chars + null
    jge .name_skip_char
    a32 mov [edi], al
    inc edi
    inc ecx
.name_skip_char:
    inc esi
    jmp .copy_name

.name_done:
    ; Null-terminate the name
    a32 mov byte [edi], 0

    ; Skip past ']' if present
    a32 cmp byte [esi], ']'
    jne .name_no_bracket
    inc esi
.name_no_bracket:

    ; Increment entry count
    inc byte [cfg_count]

    ; Advance to next line
    call advance_to_next_line
    jmp .parse_loop

.done:
    ; If no entries, create default "anyOS" entry
    cmp byte [cfg_count], 0
    jne .clamp_default

    ; Write default entry name "anyOS"
    mov edi, cfg_entries
    a32 mov dword [edi], 'anyO'
    a32 mov byte [edi+4], 'S'
    a32 mov byte [edi+5], 0
    ; type=0 (kernel) is already 0 from times fill
    mov byte [cfg_count], 1

.clamp_default:
    ; Clamp cfg_default to cfg_count-1
    mov al, [cfg_count]
    dec al
    cmp [cfg_default], al
    jbe .default_ok
    mov [cfg_default], al
.default_ok:

    pop es
    popa
    ret

; =============================================================================
; parse_key_value — parse a key=value line at [ESI]
; =============================================================================
parse_key_value:
    ; NOTE: no pusha — ESI must remain advanced for the caller's parse loop

    ; Check for global keys first: timeout= and default=
    ; Compare "timeout="
    a32 cmp dword [esi], 'time'
    jne .not_timeout
    a32 cmp dword [esi+4], 'out='
    jne .not_timeout

    add esi, 8
    call parse_decimal
    cmp ax, 3
    jge .timeout_ok
    mov ax, 3
.timeout_ok:
    mov [cfg_timeout], ax
    call advance_to_next_line
    ret

.not_timeout:
    ; Compare "default="
    a32 cmp dword [esi], 'defa'
    jne .not_default
    a32 cmp dword [esi+4], 'ult='
    jne .not_default

    add esi, 8
    call parse_decimal
    mov [cfg_default], al
    call advance_to_next_line
    ret

.not_default:
    ; Entry-level keys — need a current entry
    cmp byte [cfg_count], 0
    je .kv_skip

    ; Calculate current entry base address
    movzx eax, byte [cfg_count]
    dec eax
    shl eax, 7                  ; * 128
    add eax, cfg_entries
    mov ebx, eax               ; EBX = entry base

    ; Check "kernel="
    a32 cmp dword [esi], 'kern'
    jne .not_kernel
    a32 cmp word [esi+4], 'el'
    jne .not_kernel
    a32 cmp byte [esi+6], '='
    jne .not_kernel

    add esi, 7
    ; type = 0 (kernel)
    a32 mov byte [ebx+96], 0
    call advance_to_next_line
    ret

.not_kernel:
    ; Check "chainload="
    a32 cmp dword [esi], 'chai'
    jne .not_chainload
    a32 cmp dword [esi+4], 'nloa'
    jne .not_chainload
    a32 cmp word [esi+8], 'd='
    jne .not_chainload

    add esi, 10
    ; type = 1 (chainload)
    a32 mov byte [ebx+96], 1
    call advance_to_next_line
    ret

.not_chainload:
    ; Check "disk="
    a32 cmp dword [esi], 'disk'
    jne .not_disk
    a32 cmp byte [esi+4], '='
    jne .not_disk

    add esi, 5
    call parse_decimal
    a32 mov [ebx+97], al
    call advance_to_next_line
    ret

.not_disk:
    ; Check "partition="
    a32 cmp dword [esi], 'part'
    jne .not_partition
    a32 cmp dword [esi+4], 'itio'
    jne .not_partition
    a32 cmp word [esi+8], 'n='
    jne .not_partition

    add esi, 10
    call parse_decimal
    a32 mov [ebx+98], al
    call advance_to_next_line
    ret

.not_partition:
    ; Check "params="
    a32 cmp dword [esi], 'para'
    jne .kv_skip
    a32 cmp word [esi+4], 'ms'
    jne .kv_skip
    a32 cmp byte [esi+6], '='
    jne .kv_skip

    add esi, 7
    ; Copy value into params field (offset 32, max 63 chars + null)
    lea edi, [ebx+32]
    xor ecx, ecx
.copy_params:
    a32 mov al, [esi]
    cmp al, 0x0A
    je .params_done
    cmp al, 0x0D
    je .params_done
    cmp al, 0
    je .params_done
    cmp ecx, 63
    jge .params_skip_char
    a32 mov [edi], al
    inc edi
    inc ecx
.params_skip_char:
    inc esi
    jmp .copy_params
.params_done:
    a32 mov byte [edi], 0
    call advance_to_next_line
    ret

.kv_skip:
    call advance_to_next_line
    ret

; =============================================================================
; parse_decimal — parse decimal number from [ESI], result in AX
; =============================================================================
parse_decimal:
    push ebx
    push ecx
    xor eax, eax
    xor ecx, ecx               ; accumulator

.dec_loop:
    a32 mov bl, [esi]
    cmp bl, '0'
    jb .dec_done
    cmp bl, '9'
    ja .dec_done

    ; ecx = ecx * 10 + digit
    imul ecx, ecx, 10
    movzx ebx, bl
    sub ebx, '0'
    add ecx, ebx
    inc esi
    jmp .dec_loop

.dec_done:
    mov eax, ecx
    pop ecx
    pop ebx
    ret

; =============================================================================
; skip_whitespace — skip spaces and tabs at [ESI]
; =============================================================================
skip_whitespace:
.sw_loop:
    a32 cmp byte [esi], ' '
    je .sw_skip
    a32 cmp byte [esi], 0x09
    je .sw_skip
    ret
.sw_skip:
    inc esi
    jmp .sw_loop

; =============================================================================
; advance_to_next_line — move ESI past next LF (or to null)
; =============================================================================
advance_to_next_line:
.atl_loop:
    a32 cmp byte [esi], 0
    je .atl_done
    a32 cmp byte [esi], 0x0A
    je .atl_found_lf
    inc esi
    jmp .atl_loop
.atl_found_lf:
    inc esi                     ; skip past LF
.atl_done:
    ret

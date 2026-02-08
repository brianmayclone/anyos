; =============================================================================
; memory_map.asm - Query BIOS E820 memory map
; =============================================================================
; Stores memory map entries at MEMORY_MAP_ADDR
; Stores entry count at MEMORY_MAP_COUNT
; Each entry: base_addr(u64) + length(u64) + type(u32) + acpi_ext(u32) = 24 bytes

get_memory_map:
    pusha

    mov di, MEMORY_MAP_ADDR ; ES:DI = destination buffer
    xor ebx, ebx           ; Continuation value (0 = start)
    mov dword [mmap_count], 0

.loop:
    mov eax, 0xE820        ; Function number
    mov ecx, 24            ; Buffer size (24 bytes per entry)
    mov edx, 0x534D4150    ; 'SMAP' (must be set each call on some BIOSes)
    int 0x15

    ; Check for error
    jc .done                ; CF set = error or end of list
    cmp eax, 0x534D4150    ; EAX should be 'SMAP' on return
    jne .error

    ; Check if entry is valid (length > 0)
    cmp ecx, 20            ; Minimum valid return is 20 bytes
    jl .skip

    ; Check if length field is zero (skip zero-length entries)
    mov eax, [di + 8]      ; Low 32 bits of length
    or eax, [di + 12]      ; High 32 bits of length
    jz .skip

    ; Valid entry - advance buffer pointer
    add di, 24
    inc dword [mmap_count]

.skip:
    ; Check if this was the last entry
    test ebx, ebx
    jz .done

    jmp .loop

.error:
    mov si, msg_e820_fail
    call print_string_16
    cli
    hlt

.done:
    ; Store count at MEMORY_MAP_COUNT
    mov eax, [mmap_count]
    mov [MEMORY_MAP_COUNT], eax

    popa
    ret

mmap_count:     dd 0
msg_e820_fail:  db "FATAL: E820 memory map failed!", 13, 10, 0

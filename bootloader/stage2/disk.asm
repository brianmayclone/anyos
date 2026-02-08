; =============================================================================
; disk.asm - Load kernel from disk using unreal mode
; =============================================================================

; Enter unreal mode: load a 32-bit GDT, enter/exit protected mode
; to load 32-bit segment limits, then return to real mode with big segments
enter_unreal_mode:
    pusha

    cli
    lgdt [unreal_gdt_desc]

    ; Enter protected mode briefly
    mov eax, cr0
    or al, 1
    mov cr0, eax

    ; Load 32-bit data segment
    mov bx, 0x08            ; First GDT data entry
    mov ds, bx
    mov es, bx

    ; Return to real mode
    and al, 0xFE
    mov cr0, eax

    ; Restore real-mode segments (but keep 32-bit limits!)
    xor ax, ax
    mov ds, ax
    mov es, ax

    sti
    popa
    ret

; Unreal mode GDT - just needs a flat 4GB data segment
ALIGN 8
unreal_gdt:
    dq 0                    ; Null descriptor
    ; 32-bit data segment, base=0, limit=4GB
    dw 0xFFFF               ; Limit low
    dw 0x0000               ; Base low
    db 0x00                 ; Base mid
    db 10010010b            ; Access: present, ring 0, data, read/write
    db 11001111b            ; Flags: 4K gran, 32-bit, Limit high=0xF
    db 0x00                 ; Base high
unreal_gdt_end:

unreal_gdt_desc:
    dw unreal_gdt_end - unreal_gdt - 1
    dd unreal_gdt

; =============================================================================
; load_kernel - Load kernel sectors from disk to KERNEL_LOAD_PHYS (0x100000)
; =============================================================================
load_kernel:
    pusha

    movzx ecx, word [kernel_sectors]    ; Total sectors to read
    mov eax, [kernel_start_lba]         ; Starting LBA
    mov edi, KERNEL_LOAD_PHYS           ; Destination in high memory

    ; Sectors per chunk
    mov ebx, 64                         ; Read 64 sectors at a time (32 KiB)

.read_loop:
    test ecx, ecx
    jz .done

    ; Determine how many sectors to read this iteration
    cmp ecx, ebx
    jae .full_chunk
    mov ebx, ecx           ; Last partial chunk

.full_chunk:
    ; Set up DAP for this chunk
    mov word  [kernel_load_dap + 2], bx ; Sector count
    mov dword [kernel_load_dap + 8], eax ; LBA

    ; Read sectors to temp buffer
    push eax
    push ecx

    mov si, kernel_load_dap
    mov dl, [boot_drive]
    mov ah, 0x42
    int 0x13
    jc .disk_error

    pop ecx
    pop eax

    ; Copy from temp buffer (0x10000) to high memory (EDI) using 32-bit addressing
    push ecx
    push eax

    ; Calculate byte count
    movzx ecx, bx
    shl ecx, 9             ; * 512 = bytes

    mov esi, TEMP_BUFFER    ; Source
    ; EDI is already set as destination

    ; Use 32-bit copy in unreal mode
    a32 rep movsb

    pop eax
    pop ecx

    ; Advance: LBA += sectors read, remaining -= sectors read
    add eax, ebx
    sub ecx, ebx
    ; EDI already advanced by movsb

    mov ebx, 64            ; Reset chunk size
    jmp .read_loop

.disk_error:
    mov si, msg_disk_err
    call print_string_16
    cli
    hlt

.done:
    popa
    ret

; DAP for kernel loading (non-local label since it's data)
ALIGN 4
kernel_load_dap:
    db 0x10                 ; DAP size
    db 0                    ; Reserved
    dw 0                    ; Sector count (patched per chunk)
    dw 0x0000               ; Buffer offset = 0
    dw TEMP_BUFFER_SEG      ; Buffer segment (0x1000 -> physical 0x10000)
    dd 0                    ; LBA low (patched per chunk)
    dd 0                    ; LBA high

msg_disk_err: db "FATAL: Kernel disk read failed!", 13, 10, 0

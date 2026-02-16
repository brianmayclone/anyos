; =============================================================================
; disk.asm - Load kernel from disk using unreal mode
; =============================================================================
; Supports three paths (tried in order):
;   1. INT 13h AH=48h detects sector size → AH=42h with correct LBAs
;      - HDD/AHCI: 512-byte sectors, reads 64 sectors at a time (32 KiB)
;      - CD-ROM:   2048-byte sectors, reads 16 CD sectors at a time (32 KiB)
;   2. INT 13h AH=42h test read (if AH=48h unavailable) → HDD path
;   3. ATAPI PACKET commands — direct PIO for legacy IDE CD-ROM drives
;      (last resort, only if BIOS doesn't support INT 13h for CD-ROM)
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
; kernel_start_lba and kernel_sectors are in 512-byte sector units (patched by
; mkimage.py). This routine detects the actual drive sector size and adjusts
; LBAs accordingly, so it works on both HDD (512-byte) and CD-ROM (2048-byte).
; =============================================================================
load_kernel:
    pusha

    ; =====================================================================
    ; Step 1: Detect sector size via INT 13h AH=48h (EDD Get Drive Params)
    ; This is the universal way to distinguish HDD vs CD-ROM on any BIOS.
    ; Works on real hardware (AMI, Award, Phoenix, Insyde, etc.) and QEMU.
    ; =====================================================================
    mov word [drive_params_buf], 0x1E   ; Minimum buffer size (30 bytes)
    mov ah, 0x48
    mov dl, [boot_drive]
    mov si, drive_params_buf
    int 0x13
    jc .no_edd                          ; AH=48h not supported → try test read

    ; Check bytes_per_sector at offset 0x18
    mov ax, [drive_params_buf + 0x18]
    cmp ax, 1024
    ja .cd_int13_path                   ; > 1024 → CD-ROM (2048-byte sectors)
    test ax, ax
    jz .no_edd                          ; 0 = field not filled → try test read
    jmp .hdd_int13_path                 ; ≤ 1024 → HDD (512-byte sectors)

.no_edd:
    ; AH=48h not supported (very old BIOS) — try INT 13h AH=42h test read
    ; If this succeeds, assume HDD. If it fails, fall back to ATAPI PIO.
    mov eax, [kernel_start_lba]
    mov word  [kernel_load_dap + 2], 1  ; Read 1 sector
    mov dword [kernel_load_dap + 8], eax
    push eax
    mov si, kernel_load_dap
    mov dl, [boot_drive]
    mov ah, 0x42
    int 0x13
    pop eax
    jc .try_atapi                       ; INT 13h unavailable → ATAPI PIO
    ; Fall through to HDD path

    ; =====================================================================
    ; HDD/AHCI INT 13h path — 512-byte sectors, 64 at a time (32 KiB)
    ; =====================================================================
.hdd_int13_path:
    movzx ecx, word [kernel_sectors]    ; Total 512-byte sectors
    mov eax, [kernel_start_lba]         ; Starting LBA (512-byte)
    mov edi, KERNEL_LOAD_PHYS           ; Destination in high memory
    mov ebx, 64                         ; 64 × 512 = 32 KiB per chunk

.hdd_read_loop:
    test ecx, ecx
    jz .done

    cmp ecx, ebx
    jae .hdd_full_chunk
    mov ebx, ecx                        ; Last partial chunk

.hdd_full_chunk:
    mov word  [kernel_load_dap + 2], bx ; Sector count
    mov dword [kernel_load_dap + 8], eax ; LBA

    push eax
    push ecx
    mov si, kernel_load_dap
    mov dl, [boot_drive]
    mov ah, 0x42
    int 0x13
    jc .disk_error
    pop ecx
    pop eax

    ; Copy from TEMP_BUFFER to high memory via unreal mode 32-bit addressing
    push ecx
    push eax
    movzx ecx, bx
    shl ecx, 9                          ; × 512 = byte count
    mov esi, TEMP_BUFFER
    a32 rep movsb                        ; EDI advances automatically
    pop eax
    pop ecx

    add eax, ebx
    sub ecx, ebx
    mov ebx, 64                          ; Reset chunk size
    jmp .hdd_read_loop

    ; =====================================================================
    ; CD-ROM INT 13h path — 2048-byte sectors, 16 at a time (32 KiB)
    ; Uses BIOS INT 13h which abstracts the hardware (IDE/AHCI/USB/etc.)
    ; so this works on ANY x86 PC, not just legacy IDE controllers.
    ; =====================================================================
.cd_int13_path:
    movzx ecx, word [kernel_sectors]    ; Total 512-byte sectors
    mov eax, [kernel_start_lba]         ; Starting LBA (512-byte)
    mov edi, KERNEL_LOAD_PHYS

    ; Convert 512-byte LBA → CD sector LBA
    shr eax, 2                           ; LBA ÷ 4
    ; Convert 512-byte count → CD sector count (round up)
    add ecx, 3
    shr ecx, 2                           ; (count + 3) ÷ 4

    mov ebx, 16                          ; 16 × 2048 = 32 KiB per chunk

.cd_read_loop:
    test ecx, ecx
    jz .done

    cmp ecx, ebx
    jae .cd_full_chunk
    mov ebx, ecx                         ; Last partial chunk

.cd_full_chunk:
    mov word  [kernel_load_dap + 2], bx  ; CD sector count
    mov dword [kernel_load_dap + 8], eax ; CD sector LBA

    push eax
    push ecx
    mov si, kernel_load_dap
    mov dl, [boot_drive]
    mov ah, 0x42
    int 0x13
    jc .disk_error
    pop ecx
    pop eax

    ; Copy (chunk × 2048) bytes from TEMP_BUFFER to high memory
    push ecx
    push eax
    movzx ecx, bx
    shl ecx, 11                          ; × 2048 = byte count
    mov esi, TEMP_BUFFER
    a32 rep movsb
    pop eax
    pop ecx

    add eax, ebx
    sub ecx, ebx
    mov ebx, 16                           ; Reset chunk size
    jmp .cd_read_loop

    ; =====================================================================
    ; ATAPI PIO path — direct PACKET commands (legacy IDE CD-ROM fallback)
    ; Only reached if BIOS has no EDD support AND no INT 13h for CD-ROM.
    ; This covers very old BIOSes with legacy IDE controllers.
    ; =====================================================================
.try_atapi:
    ; Try secondary master (most common for QEMU -cdrom)
    mov word [atapi_base], 0x170
    mov byte [atapi_slave], 0
    call atapi_probe
    test al, al
    jnz .atapi_found

    ; Try secondary slave
    mov word [atapi_base], 0x170
    mov byte [atapi_slave], 1
    call atapi_probe
    test al, al
    jnz .atapi_found

    ; Try primary slave
    mov word [atapi_base], 0x1F0
    mov byte [atapi_slave], 1
    call atapi_probe
    test al, al
    jnz .atapi_found

    ; Try primary master
    mov word [atapi_base], 0x1F0
    mov byte [atapi_slave], 0
    call atapi_probe
    test al, al
    jnz .atapi_found

    jmp .disk_error                      ; No readable drive found

.atapi_found:
    ; Reload kernel parameters (may have been clobbered by probing)
    movzx ecx, word [kernel_sectors]
    mov eax, [kernel_start_lba]
    mov edi, KERNEL_LOAD_PHYS

    ; Convert 512-byte LBA → CD sector LBA
    shr eax, 2
    add ecx, 3
    shr ecx, 2

.atapi_read_loop:
    test ecx, ecx
    jz .done

    ; Read one CD sector (2048 bytes) to temp buffer
    push eax
    push ecx
    call atapi_read_sector               ; EAX = CD LBA → TEMP_BUFFER
    pop ecx
    pop eax
    jc .disk_error

    ; Copy 2048 bytes from temp buffer to high memory via unreal mode
    push ecx
    push eax
    mov ecx, 2048
    mov esi, TEMP_BUFFER
    a32 rep movsb
    pop eax
    pop ecx

    inc eax
    dec ecx
    jmp .atapi_read_loop

.disk_error:
    mov si, msg_disk_err
    call print_string_16
    cli
    hlt

.done:
    popa
    ret

; =============================================================================
; atapi_probe - Try to read CD sector 16 (PVD) and verify "CD001" signature
; Input:  [atapi_base] = I/O base port, [atapi_slave] = 0 master, 1 slave
; Output: AL = 1 if ISO 9660 CD-ROM found, 0 if not
; =============================================================================
atapi_probe:
    push ebx
    push edi

    ; Try reading PVD (CD sector 16)
    mov eax, 16
    call atapi_read_sector
    jc .probe_fail

    ; Verify ISO 9660 magic "CD001" at offset 1 of the PVD
    ; PVD: byte 0 = type (1), bytes 1-5 = "CD001"
    ; Must use 32-bit register for TEMP_BUFFER (0x10000 > 16-bit range)
    mov edi, TEMP_BUFFER
    cmp byte [edi + 1], 'C'
    jne .probe_fail
    cmp byte [edi + 2], 'D'
    jne .probe_fail
    cmp byte [edi + 3], '0'
    jne .probe_fail
    cmp byte [edi + 4], '0'
    jne .probe_fail
    cmp byte [edi + 5], '1'
    jne .probe_fail

    mov al, 1              ; Found ISO 9660 CD-ROM
    pop edi
    pop ebx
    ret

.probe_fail:
    xor al, al             ; Not found
    pop edi
    pop ebx
    ret

; =============================================================================
; atapi_read_sector - Read one 2048-byte CD sector using ATAPI PACKET command
; Input:  EAX = CD sector LBA
;         [atapi_base] = I/O base, [atapi_slave] = drive select
; Output: 2048 bytes at TEMP_BUFFER (physical 0x10000)
;         CF clear on success, CF set on error
; =============================================================================
atapi_read_sector:
    push eax
    push ebx
    push ecx
    push edx
    push edi

    mov ebx, eax           ; Save LBA in EBX

    ; Select device
    mov dx, [atapi_base]
    add dx, 6              ; Device/Head register
    mov al, [atapi_slave]
    test al, al
    jz .sel_master
    mov al, 0xB0           ; Slave (bit 4 = 1)
    jmp .sel_done
.sel_master:
    mov al, 0xA0           ; Master (bit 4 = 0)
.sel_done:
    out dx, al

    ; 400ns delay (read status 4 times)
    mov dx, [atapi_base]
    add dx, 7              ; Status register
    in al, dx
    in al, dx
    in al, dx
    in al, dx

    ; Quick check: floating bus (0xFF) = no device present
    in al, dx
    cmp al, 0xFF
    je .atapi_err

    ; Wait for device ready (BSY=0) with timeout
    mov ecx, 200000
.wait_ready:
    in al, dx
    test al, 0x80          ; BSY bit
    jz .ready
    dec ecx
    jnz .wait_ready
    jmp .atapi_err          ; Timeout → no ATAPI device
.ready:

    ; Set features = 0 (PIO mode)
    mov dx, [atapi_base]
    inc dx                 ; Features register (base + 1)
    xor al, al
    out dx, al

    ; Set byte count = 2048 (max transfer per DRQ)
    mov dx, [atapi_base]
    add dx, 4              ; Byte Count Low (base + 4)
    mov al, 0x00           ; 2048 & 0xFF = 0
    out dx, al
    inc dx                 ; Byte Count High (base + 5)
    mov al, 0x08           ; 2048 >> 8 = 8
    out dx, al

    ; Send PACKET command
    mov dx, [atapi_base]
    add dx, 7              ; Command register
    mov al, 0xA0           ; PACKET command
    out dx, al

    ; Wait for DRQ (device ready to receive CDB)
    ; Status register = base + 7
    mov ecx, 200000
.wait_drq_cdb:
    in al, dx
    test al, 0x80          ; BSY — keep spinning while BSY=1
    jnz .wait_drq_cdb_next
    test al, 0x01          ; ERR
    jnz .atapi_err
    test al, 0x08          ; DRQ
    jnz .send_cdb
.wait_drq_cdb_next:
    dec ecx
    jnz .wait_drq_cdb
    jmp .atapi_err          ; Timeout

.send_cdb:
    ; Build READ(10) CDB (12 bytes, sent as 6 × 16-bit words)
    ;
    ;   Byte 0: 0x28 (READ(10) opcode)
    ;   Byte 1: 0x00
    ;   Bytes 2-5: LBA (big-endian u32)
    ;   Byte 6: 0x00
    ;   Bytes 7-8: Transfer length = 1 sector (big-endian u16)
    ;   Bytes 9-11: 0x00
    ;
    ; x86 "out dx, ax" sends AL first, AH second (little-endian wire order).
    ; So for each word: AL = even CDB byte, AH = odd CDB byte.

    mov dx, [atapi_base]   ; Data port (base + 0)

    ; Word 0: CDB[0]=0x28, CDB[1]=0x00
    mov ax, 0x0028
    out dx, ax

    ; Word 1: CDB[2]=LBA[31:24], CDB[3]=LBA[23:16]
    mov eax, ebx           ; EAX = LBA (little-endian)
    bswap eax              ; EAX = LBA (big-endian byte order)
    push eax
    out dx, ax             ; AL=LBA[31:24], AH=LBA[23:16]

    ; Word 2: CDB[4]=LBA[15:8], CDB[5]=LBA[7:0]
    pop eax
    shr eax, 16
    out dx, ax             ; AL=LBA[15:8], AH=LBA[7:0]

    ; Word 3: CDB[6]=0x00, CDB[7]=0x00 (transfer length high)
    xor ax, ax
    out dx, ax

    ; Word 4: CDB[8]=0x01 (transfer length low), CDB[9]=0x00
    mov ax, 0x0001
    out dx, ax

    ; Word 5: CDB[10]=0x00, CDB[11]=0x00
    xor ax, ax
    out dx, ax

    ; Wait for data DRQ
    mov dx, [atapi_base]
    add dx, 7              ; Status register
    mov ecx, 1000000       ; Generous timeout for CD seek + read
.wait_drq_data:
    in al, dx
    test al, 0x80          ; BSY — keep spinning
    jnz .wait_drq_data_next
    test al, 0x01          ; ERR
    jnz .atapi_err
    test al, 0x08          ; DRQ
    jnz .read_data
.wait_drq_data_next:
    dec ecx
    jnz .wait_drq_data
    jmp .atapi_err          ; Timeout

.read_data:
    ; Read 2048 bytes (1024 words) from data port to TEMP_BUFFER
    mov dx, [atapi_base]   ; Data port
    mov edi, TEMP_BUFFER   ; Destination (32-bit address, unreal mode)
    mov ecx, 1024          ; Word count
.read_word:
    in ax, dx
    mov [edi], ax
    add edi, 2
    dec ecx
    jnz .read_word

    ; Wait for BSY to clear (command complete)
    mov dx, [atapi_base]
    add dx, 7
    mov ecx, 200000
.wait_complete:
    in al, dx
    test al, 0x80          ; BSY
    jz .check_err
    dec ecx
    jnz .wait_complete
    jmp .atapi_err

.check_err:
    test al, 0x01          ; ERR bit
    jnz .atapi_err

    clc                     ; Success
    jmp .atapi_ret

.atapi_err:
    stc                     ; Error

.atapi_ret:
    pop edi
    pop edx
    pop ecx
    pop ebx
    pop eax
    ret

; =============================================================================
; Data
; =============================================================================

; DAP for kernel loading (INT 13h path)
ALIGN 4
kernel_load_dap:
    db 0x10                 ; DAP size
    db 0                    ; Reserved
    dw 0                    ; Sector count (patched per chunk)
    dw 0x0000               ; Buffer offset = 0
    dw TEMP_BUFFER_SEG      ; Buffer segment (0x1000 -> physical 0x10000)
    dd 0                    ; LBA low (patched per chunk)
    dd 0                    ; LBA high

; INT 13h AH=48h result buffer (EDD Get Drive Parameters)
; Minimum 30 bytes (0x1E). Bytes_per_sector at offset 0x18.
ALIGN 4
drive_params_buf:
    dw 0x1E                 ; Buffer size (input)
    times 28 db 0           ; Result filled by BIOS (30 bytes total)

; ATAPI state
atapi_base:  dw 0x170      ; I/O base port for ATAPI channel
atapi_slave: db 0           ; 0 = master, 1 = slave

msg_disk_err: db "FATAL: Kernel disk read failed!", 13, 10, 0

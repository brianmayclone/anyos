; =============================================================================
; exfat.asm - Minimal read-only exFAT driver for Stage 2 bootloader
; =============================================================================
; Provides filesystem-based file loading in 16-bit real mode + unreal mode.
; Two-level path traversal only (root -> subdir -> file).
; ASCII filenames only (<=15 chars). Read-only.
;
; All memory accesses to buffers above 0xFFFF use 32-bit registers (ESI/EDI)
; with address-size override (a32) or explicit 32-bit addressing, since we
; are in unreal mode with 4GB segment limits.
; =============================================================================

; Data variables
exfat_fat_offset:       dd 0    ; FAT sector offset from volume start
exfat_heap_offset:      dd 0    ; Cluster heap sector offset from volume start
exfat_root_cluster:     dd 0    ; First cluster of root directory
exfat_spc_shift:        db 0    ; SectorsPerClusterShift (cluster = 2^n sectors)
exfat_bps_shift:        db 0    ; BytesPerSectorShift (must be 9 = 512 bytes)
exfat_volume_start:     dd 0    ; Absolute sector of exFAT volume on disk
exfat_cur_dir_cluster:  dd 0    ; Current directory's first cluster

; Directory entry buffer at 0x18000 (within 64KB TEMP area 0x10000-0x1FFFF)
; FAT sector reads go to TEMP_BUFFER (0x10000) -- no overlap with 0x18000.
EXFAT_DIR_BUF           equ 0x18000

; =============================================================================
; exfat_init - Initialize exFAT driver from boot sector
; Input:  EAX = volume start sector (fs_start)
; Output: CF set on error (bad signature or bad sector size), CF clear on success
; =============================================================================
exfat_init:
    pusha
    mov [exfat_volume_start], eax

    ; Read boot sector (sector 0 of volume) into TEMP_BUFFER
    mov edi, TEMP_BUFFER
    mov ecx, 1
    call load_sectors_32

    ; Verify "EXFAT   " signature at offset 3
    cmp dword [TEMP_BUFFER + 3], 'EXFA'
    jne .init_fail
    cmp dword [TEMP_BUFFER + 7], 'T   '
    jne .init_fail

    ; Validate BytesPerSectorShift == 9
    cmp byte [TEMP_BUFFER + 108], 9
    jne .init_fail

    ; Parse boot sector fields
    mov eax, [TEMP_BUFFER + 80]        ; FatOffset
    mov [exfat_fat_offset], eax
    mov eax, [TEMP_BUFFER + 88]        ; ClusterHeapOffset
    mov [exfat_heap_offset], eax
    mov eax, [TEMP_BUFFER + 96]        ; FirstClusterOfRootDirectory
    mov [exfat_root_cluster], eax
    mov [exfat_cur_dir_cluster], eax
    mov al, [TEMP_BUFFER + 108]        ; BytesPerSectorShift
    mov [exfat_bps_shift], al
    mov al, [TEMP_BUFFER + 109]        ; SectorsPerClusterShift
    mov [exfat_spc_shift], al

    popa
    clc
    ret

.init_fail:
    popa
    stc
    ret

; =============================================================================
; exfat_cluster_to_lba - Convert cluster number to absolute LBA
; Input:  EAX = cluster number (2-based)
; Output: EAX = absolute disk sector LBA
; Clobbers: ECX
; =============================================================================
exfat_cluster_to_lba:
    sub eax, 2
    movzx ecx, byte [exfat_spc_shift]
    shl eax, cl
    add eax, [exfat_heap_offset]
    add eax, [exfat_volume_start]
    ret

; =============================================================================
; exfat_spc - Get sectors per cluster
; Output: ECX = sectors per cluster
; Clobbers: EBX
; =============================================================================
exfat_spc:
    movzx ecx, byte [exfat_spc_shift]
    mov ebx, 1
    shl ebx, cl
    mov ecx, ebx
    ret

; =============================================================================
; exfat_next_cluster - Get next cluster in FAT chain
; Input:  EAX = current cluster number
; Output: EAX = next cluster, CF set if end-of-chain (>= 0xFFFFFFF8)
; Preserves: EDI, ESI, EBP
; =============================================================================
exfat_next_cluster:
    push ebx
    push ecx
    push edi

    mov ebx, eax                        ; Save cluster number
    shl eax, 2                          ; x 4 = byte offset in FAT
    mov ecx, eax
    shr eax, 9                          ; / 512 = sector offset in FAT
    and ecx, 0x1FF                      ; byte offset within sector

    add eax, [exfat_fat_offset]
    add eax, [exfat_volume_start]

    ; Read FAT sector to TEMP_BUFFER (0x10000)
    push ecx
    mov edi, TEMP_BUFFER
    mov ecx, 1
    call load_sectors_32
    pop ecx

    ; Read 32-bit FAT entry
    add ecx, TEMP_BUFFER
    mov eax, [ecx]

    cmp eax, 0xFFFFFFF8
    jae .fat_end

    pop edi
    pop ecx
    pop ebx
    clc
    ret

.fat_end:
    pop edi
    pop ecx
    pop ebx
    stc
    ret

; =============================================================================
; exfat_reset_to_root - Reset current directory to root
; =============================================================================
exfat_reset_to_root:
    mov eax, [exfat_root_cluster]
    mov [exfat_cur_dir_cluster], eax
    ret

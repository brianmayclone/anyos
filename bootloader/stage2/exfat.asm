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

; =============================================================================
; exfat_match_name - Compare FileName entry UTF-16LE chars against ASCII name
; Input:  ESI = pointer to FileName entry UTF-16LE chars (at entry offset +2)
;         EBX = pointer to null-terminated ASCII search name
;         CL  = NameLength from Stream Extension entry (max 15 for single entry)
; Output: CF clear if match, CF set if no match
; Clobbers: AL, DL
; =============================================================================
exfat_match_name:
    push esi
    push ebx
    push cx
    movzx cx, cl

.mn_loop:
    test cl, cl
    jz .mn_check_end

    a32 mov al, [esi]              ; Low byte of UTF-16 char
    a32 mov ah, [esi + 1]          ; High byte
    test ah, ah
    jnz .mn_fail                   ; Non-ASCII -> no match

    ; Upcase entry char
    cmp al, 'a'
    jb .mn_no_up1
    cmp al, 'z'
    ja .mn_no_up1
    sub al, 0x20
.mn_no_up1:
    ; Upcase search char
    a32 mov dl, [ebx]
    test dl, dl
    jz .mn_fail                    ; Search name shorter
    cmp dl, 'a'
    jb .mn_no_up2
    cmp dl, 'z'
    ja .mn_no_up2
    sub dl, 0x20
.mn_no_up2:
    cmp al, dl
    jne .mn_fail

    add esi, 2
    inc ebx
    dec cl
    jmp .mn_loop

.mn_check_end:
    a32 cmp byte [ebx], 0         ; Search name must also end here
    jne .mn_fail
    pop cx
    pop ebx
    pop esi
    clc
    ret

.mn_fail:
    pop cx
    pop ebx
    pop esi
    stc
    ret

; =============================================================================
; exfat_find_entry - Search current directory for a named entry
; Input:  EBX = pointer to null-terminated ASCII filename
; Output: CF clear if found:
;           EAX = FirstCluster, EDX = DataLength (low 32 bits)
;           CH  = GeneralSecondaryFlags, CL = FileAttributes low byte
;         CF set if not found
; Clobbers: ESI, EDI, EBP
; =============================================================================

; Local storage
_fe_search_name:    dd 0
_fe_cur_cluster:    dd 0
_fe_stream_cluster: dd 0
_fe_stream_size:    dd 0

exfat_find_entry:
    mov [_fe_search_name], ebx
    mov eax, [exfat_cur_dir_cluster]

.fe_cluster_loop:
    mov [_fe_cur_cluster], eax

    ; Read directory cluster to EXFAT_DIR_BUF (0x18000)
    push eax
    call exfat_cluster_to_lba      ; EAX = absolute LBA
    mov edi, EXFAT_DIR_BUF
    call exfat_spc                 ; ECX = sectors per cluster
    call load_sectors_32
    pop eax

    ; Calculate cluster byte size
    call exfat_spc                 ; ECX = sectors per cluster
    shl ecx, 9                    ; x 512 = bytes

    ; Walk 32-byte directory entries
    mov esi, EXFAT_DIR_BUF
    mov ebp, ecx
    add ebp, esi                   ; End pointer

.fe_entry_loop:
    cmp esi, ebp
    jae .fe_next_cluster

    a32 mov al, [esi]              ; EntryType
    test al, al
    jz .fe_not_found               ; 0x00 = end of directory

    cmp al, 0x85                   ; File entry?
    jne .fe_skip

    ; File entry -- save FileAttributes (offset 4)
    a32 mov cl, [esi + 4]

    ; Next: Stream Extension (0xC0)
    add esi, 32
    cmp esi, ebp
    jae .fe_next_cluster
    a32 cmp byte [esi], 0xC0
    jne .fe_entry_loop

    ; Stream Extension -- save fields
    a32 mov ch, [esi + 1]              ; GeneralSecondaryFlags
    a32 mov dl, [esi + 3]              ; NameLength
    mov eax, [esi + 20]               ; FirstCluster
    mov [_fe_stream_cluster], eax
    mov eax, [esi + 8]                ; DataLength low 32 bits
    mov [_fe_stream_size], eax
    push dx                            ; Save DL=NameLength

    ; Next: FileName (0xC1)
    add esi, 32
    cmp esi, ebp
    jae .fe_fn_overflow
    a32 cmp byte [esi], 0xC1
    jne .fe_skip_fn_fail

    ; Compare name
    pop dx
    push esi
    add esi, 2                         ; UTF-16 chars start at offset 2
    mov ebx, [_fe_search_name]
    mov cl, dl                         ; NameLength for comparison
    call exfat_match_name
    pop esi
    jc .fe_no_match

    ; Match! Re-read CL/CH from the entry set (File entry is at ESI - 64)
    push esi
    sub esi, 64
    a32 mov cl, [esi + 4]             ; FileAttributes from File entry
    a32 mov ch, [esi + 32 + 1]       ; GeneralSecondaryFlags from Stream Extension
    pop esi
    mov eax, [_fe_stream_cluster]
    mov edx, [_fe_stream_size]
    clc
    ret

.fe_no_match:
    add esi, 32                        ; Skip past FileName entry
    jmp .fe_entry_loop

.fe_skip_fn_fail:
    pop dx
    jmp .fe_entry_loop

.fe_fn_overflow:
    pop dx
    jmp .fe_next_cluster

.fe_skip:
    add esi, 32
    jmp .fe_entry_loop

.fe_next_cluster:
    mov eax, [_fe_cur_cluster]
    call exfat_next_cluster
    jc .fe_not_found
    jmp .fe_cluster_loop

.fe_not_found:
    stc
    ret

; =============================================================================
; exfat_open_dir - Open a subdirectory by name
; Input:  EBX = pointer to null-terminated ASCII directory name
; Output: CF clear if found (cur_dir_cluster updated), CF set if not found
; =============================================================================
exfat_open_dir:
    call exfat_find_entry
    jc .od_fail
    test cl, 0x10                  ; Check directory attribute
    jz .od_fail
    mov [exfat_cur_dir_cluster], eax
    clc
    ret
.od_fail:
    stc
    ret

; =============================================================================
; exfat_read_file - Read a file from current directory to destination
; Input:  EBX = pointer to null-terminated ASCII filename
;         EDI = destination address (32-bit, unreal mode)
; Output: EAX = file size in bytes, CF clear if loaded
;         CF set if not found (EAX = 0)
; Note:   EDI advances past loaded data.
; =============================================================================

_rf_file_size:     dd 0
_rf_first_cluster: dd 0
_rf_flags:         db 0
_rf_dest:          dd 0

exfat_read_file:
    push ebx
    push ecx
    push edx
    ; EDI intentionally NOT saved -- must advance

    mov [_rf_dest], edi

    call exfat_find_entry
    jc .rf_not_found

    ; EAX = FirstCluster, EDX = DataLength, CH = GeneralSecondaryFlags
    mov [_rf_file_size], edx
    mov [_rf_first_cluster], eax
    mov [_rf_flags], ch

    mov edi, [_rf_dest]

    ; Calculate total sector count
    mov ecx, edx               ; DataLength
    add ecx, 511
    shr ecx, 9                 ; Ceiling divide by 512

    ; Check NoFatChain flag (bit 1 of GeneralSecondaryFlags)
    test byte [_rf_flags], 0x02
    jz .rf_chain

    ; Contiguous: single read from FirstCluster
    mov eax, [_rf_first_cluster]
    call exfat_cluster_to_lba
    call load_sectors_32       ; EDI advances
    jmp .rf_done

.rf_chain:
    ; Follow FAT chain cluster by cluster
    mov eax, [_rf_first_cluster]

.rf_chain_loop:
    test ecx, ecx
    jz .rf_done

    ; Read one cluster
    push eax
    push ecx
    call exfat_cluster_to_lba  ; EAX = LBA
    call exfat_spc             ; ECX = sectors per cluster
    ; Don't read more sectors than remaining
    cmp ecx, [esp]             ; Compare with remaining sector count on stack
    jbe .rf_chunk_ok
    mov ecx, [esp]             ; Limit to remaining
.rf_chunk_ok:
    push ecx                   ; Save sectors actually read this iteration
    call load_sectors_32       ; EDI advances
    pop ebx                    ; EBX = sectors read
    pop ecx                    ; ECX = original remaining sector count
    pop eax                    ; EAX = original cluster number

    sub ecx, ebx
    jbe .rf_done

    ; Next cluster via FAT
    call exfat_next_cluster
    jc .rf_done
    jmp .rf_chain_loop

.rf_done:
    mov eax, [_rf_file_size]
    pop edx
    pop ecx
    pop ebx
    clc
    ret

.rf_not_found:
    xor eax, eax
    pop edx
    pop ecx
    pop ebx
    stc
    ret

; =============================================================================
; int19h.asm — INT 19h: Bootstrap loader
;
; Boot priority:
;   1. El Torito CD boot — probes the slave drive first (ISO/CD).
;      If no slave is present, falls back to probing master.
;   2. MBR hard-disk boot (reads ATA LBA 0 from master, checks 0xAA55).
;
; ISO ↔ ATA sector translation:
;   ISO uses 2048-byte logical sectors; the ATA device uses 512-byte sectors.
;   ATA LBA = ISO sector × 4.
;
; After a successful El Torito boot the following variables are filled and
; can be queried by INT 13h AH=4Bh:
;   el_torito_drive     — virtual drive number handed to the boot image in DL
;   el_torito_emul      — Boot Media Type byte from the catalog entry
;   el_torito_load_seg  — segment the boot image was loaded into
;   el_torito_rba       — Load RBA from the catalog (ISO 2048-byte sector units)
;   el_torito_count     — virtual sector count that was loaded
; =============================================================================

; ── El Torito runtime data (queried by INT 13h AH=4Bh) ─────────────────────
el_torito_present:  db 0       ; 1 after successful El Torito detection
el_torito_drive:    db 0xE0    ; Virtual drive number returned in DL (0xE0=CD)
el_torito_emul:     db 0       ; Boot Media Type (0=no-emul, 1-3=floppy, 4=HD)
el_torito_load_seg: dw 0x07C0  ; Load segment (from catalog or default 0x07C0)
el_torito_rba:      dd 0       ; Boot image Load RBA (ISO 2048-byte sectors)
el_torito_count:    dw 1       ; Sector count (512-byte virtual sectors loaded)
el_torito_handoff:  db 0       ; Set to 1 right before jumping into boot image

; ── Work buffer ─────────────────────────────────────────────────────────────
; ide_identify_buf is only needed during POST (ide_detect). After POST it is
; safe to reuse those 512 bytes as a scratch buffer for El Torito parsing.
el_torito_buf       equ ide_identify_buf

; ── INT 19h entry ───────────────────────────────────────────────────────────
int19h_handler:
    sti

    ; Need at least one drive to boot.
    cmp byte [cs:ide_master_present], 0
    jne .have_drive
    cmp byte [cs:ide_slave_present], 0
    jne .have_drive
    jmp .no_boot
.have_drive:

    ; ── Step 1: Try El Torito ──────────────────────────────────────────────
    call detect_el_torito
    jc .try_hd             ; CF=1 → not an El Torito disk

    ; El Torito signature verified — load and jump.
    call boot_el_torito
    ; boot_el_torito only returns on failure.
    jmp .no_boot

    ; ── Step 2: Fall back to MBR/HDD boot ─────────────────────────────────
.try_hd:
    cmp byte [cs:ide_master_present], 0
    je .no_boot

    mov si, str_booting_hd
    call bios_print

    ; Select master drive for MBR read.
    mov byte [cs:ide_drive_sel], 0xE0  ; Master

    ; Read LBA 0 (MBR) to 0x0000:0x7C00.
    mov eax, 0
    mov ecx, 1
    push es
    push word 0x0000
    pop es
    mov di, 0x7C00
    call ide_read_sectors
    pop es
    jc .no_boot

    ; Verify MBR signature.
    cmp word [0x7C00 + 510], 0xAA55
    jne .no_boot

    ; Valid MBR — launch it.
    mov dl, 0x80            ; First HDD
    jmp 0x0000:0x7C00

.no_boot:
    mov si, str_no_boot
    call bios_print
.halt:
    cli
    hlt
    jmp .halt


; =============================================================================
; detect_el_torito
;
; Checks whether the attached IDE disk is an El Torito CD image.
; Probes slave first (if present), then master as fallback.
; Reads ISO sector 17 (ATA LBA 68) which, for a standards-compliant ISO,
; contains the Boot Record Volume Descriptor.  On success it reads the Boot
; Catalog and fills all el_torito_* variables.
;
; Returns: CF=0 on success, CF=1 if this is not an El Torito disk.
; Clobbers: el_torito_buf (= ide_identify_buf)
; =============================================================================
detect_el_torito:
    push eax
    push ecx
    push edi
    push es

    ; Decide which drive to probe for El Torito.
    ; Prefer slave (CD/ISO), fall back to master.
    cmp byte [cs:ide_slave_present], 1
    je .et_use_slave
    cmp byte [cs:ide_master_present], 0
    je .fail
    mov byte [cs:ide_drive_sel], 0xE0  ; Master
    jmp .et_read_brvd
.et_use_slave:
    mov byte [cs:ide_drive_sel], 0xF0  ; Slave

.et_read_brvd:
    ; ── Read ISO sector 17 = ATA LBA 68 ──────────────────────────────────
    ; The Boot Record Volume Descriptor occupies bytes 0–2047 of ISO sector 17.
    ; We only need the first 512 bytes (fits in one ATA sector).
    mov eax, 68             ; ATA LBA 68 = ISO sector 17 (17 × 4 = 68)
    mov ecx, 1
    push ds
    pop es
    mov di, el_torito_buf
    call ide_read_sectors
    jc .fail

    ; ── Validate Volume Descriptor type = 0 (Boot Record) ─────────────────
    cmp byte [el_torito_buf + 0], 0x00
    jne .fail

    ; ── Validate Standard Identifier "CD001" at bytes 1–5 ─────────────────
    cmp byte [el_torito_buf + 1], 'C'
    jne .fail
    cmp byte [el_torito_buf + 2], 'D'
    jne .fail
    cmp byte [el_torito_buf + 3], '0'
    jne .fail
    cmp byte [el_torito_buf + 4], '0'
    jne .fail
    cmp byte [el_torito_buf + 5], '1'
    jne .fail

    ; ── Validate Boot System Identifier = "EL TORITO SPECIFICATION" ───────
    ; Bytes 7–38.  We check the first 8 characters ("EL TORIT") for speed.
    cmp dword [el_torito_buf + 7],  'EL T'
    jne .fail
    cmp dword [el_torito_buf + 11], 'ORIT'
    jne .fail

    ; ── Read Boot Catalog ─────────────────────────────────────────────────
    ; Boot Catalog location in ISO sector units is at bytes 71–74 (DWORD LE).
    ; Convert to ATA LBA by multiplying by 4.
    mov eax, [el_torito_buf + 71]   ; ISO sector of Boot Catalog
    shl eax, 2                       ; × 4 → ATA LBA
    mov ecx, 1
    push ds
    pop es
    mov di, el_torito_buf
    call ide_read_sectors
    jc .fail

    ; ── Validate Validation Entry ─────────────────────────────────────────
    ; Byte 0 must be 0x01 (Header ID for Validation Entry).
    cmp byte [el_torito_buf + 0], 0x01
    jne .fail
    ; Bytes 30–31 must be 0x55, 0xAA (key).
    cmp byte [el_torito_buf + 30], 0x55
    jne .fail
    cmp byte [el_torito_buf + 31], 0xAA
    jne .fail

    ; ── Read Initial/Default Entry (bytes 32–63 of the catalog) ──────────
    ; Byte 32: Boot Indicator — 0x88 = bootable.
    cmp byte [el_torito_buf + 32], 0x88
    jne .fail

    ; Byte 33: Boot Media Type.
    mov al, [el_torito_buf + 33]
    mov [cs:el_torito_emul], al

    ; Bytes 34–35: Load Segment (0 → default 0x07C0).
    mov ax, [el_torito_buf + 34]
    test ax, ax
    jnz .store_seg
    mov ax, 0x07C0
.store_seg:
    mov [cs:el_torito_load_seg], ax

    ; Bytes 38–39: Sector Count (512-byte virtual sectors to transfer).
    ; A count of 0 is treated as 1.
    mov ax, [el_torito_buf + 38]
    test ax, ax
    jnz .store_count
    mov ax, 1
.store_count:
    mov [cs:el_torito_count], ax

    ; Bytes 40–43: Load RBA (2048-byte CD sector of the boot image).
    mov eax, [el_torito_buf + 40]
    mov [cs:el_torito_rba], eax

    ; ── Mark detection as successful ──────────────────────────────────────
    mov byte [cs:el_torito_present], 1

    ; Set el_torito_drive based on emulation type.
    ; All BIOS variables live in the ROM segment (cs:) to avoid conflicts
    ; with bootloader data in low RAM.
    mov al, [cs:el_torito_emul]
    cmp al, 0x04
    je .drive_hd
    cmp al, 0x00
    je .drive_cd
    ; Floppy emulation (1–3).
    mov byte [cs:el_torito_drive], 0x00
    jmp .drive_done
.drive_hd:
    mov byte [cs:el_torito_drive], 0x80
    jmp .drive_done
.drive_cd:
    mov byte [cs:el_torito_drive], 0xE0
.drive_done:

    pop es
    pop edi
    pop ecx
    pop eax
    clc
    ret

.fail:
    pop es
    pop edi
    pop ecx
    pop eax
    stc
    ret


; =============================================================================
; boot_el_torito
;
; Loads and executes the El Torito boot image.
; Assumes detect_el_torito succeeded and el_torito_* variables are filled.
; The ide_drive_sel variable is already set to the correct drive from
; detect_el_torito.
;
; The boot image is read from ATA LBA (el_torito_rba × 4) into
; (el_torito_load_seg):0x0000.
;
; DL is set to the appropriate drive number before jumping:
;   No emulation (0x00) → DL = 0xE0   (virtual CD-ROM)
;   HD  emulation (0x04) → DL = 0x80   (first HD)
;   Floppy emulation     → DL = 0x00   (first floppy)
;
; DS:SI is set to 0x0000:0x0000 (most bootloaders don't require a specific
; value; those that query INT 13h AH=4Bh will call it themselves).
;
; Returns: only on error (CF=1).
; =============================================================================
boot_el_torito:
    mov si, str_booting_cd
    call bios_print

    ; Print El Torito parameters for diagnostics.
    mov si, str_et_rba
    call bios_print
    mov eax, [cs:el_torito_rba]
    call bios_print_hex32
    mov si, str_et_cnt
    call bios_print
    mov ax, [cs:el_torito_count]
    call bios_print_dec16
    mov si, str_et_seg
    call bios_print
    mov ax, [cs:el_torito_load_seg]
    call bios_print_hex16
    mov si, str_et_emul
    call bios_print
    movzx ax, byte [cs:el_torito_emul]
    call bios_print_hex8
    mov si, str_et_load
    call bios_print

    ; Convert Load RBA (ISO 2048-byte sector units) to ATA LBA.
    mov eax, [cs:el_torito_rba]
    shl eax, 2              ; × 4 → ATA LBA

    ; Number of 512-byte sectors to load.
    ; Many ISOs set the El Torito sector count to just 4 (2 KB), expecting the
    ; bootloader bootstrap to reload the rest.  ISOLINUX keeps the self-load
    ; code in the upper portion of the image, so if only 4 sectors are loaded
    ; that code is missing.  To work around this, enforce a minimum of 64
    ; sectors (32 KB) which covers a full ISOLINUX/SYSLINUX image.
    movzx ecx, word [cs:el_torito_count]
    cmp ecx, 64
    jae .count_ok
    mov ecx, 64
.count_ok:

    ; ide_drive_sel is already set by detect_el_torito.
    ; Set ES:DI to (load_seg):0x0000 for the read.
    push es
    mov bx, [cs:el_torito_load_seg]
    mov es, bx
    mov di, 0x0000
    call ide_read_sectors
    pop es
    jc .fail

    mov si, str_et_loaded
    call bios_print

    ; ── Choose DL based on emulation type ─────────────────────────────────
    mov al, [cs:el_torito_emul]
    cmp al, 0x04
    je .hd_emul
    cmp al, 0x00
    je .no_emul
    ; Floppy emulation (media type 1, 2, or 3).
    mov dl, 0x00
    jmp .jump

.hd_emul:
    mov dl, 0x80
    jmp .jump

.no_emul:
    mov dl, 0xE0

.jump:
    ; DS:SI = 0 (boot image will call INT 13h AH=4Bh if it needs the packet).
    xor ax, ax
    mov ds, ax
    xor si, si

    ; Mark that we are handing off to CD boot image.
    mov byte [cs:el_torito_handoff], 1

    ; Far-jump to (load_seg):0x0000 via a far return.
    ; Stack layout for retf: [SP+0]=offset(IP), [SP+2]=segment(CS).
    mov ax, [cs:el_torito_load_seg]
    push ax             ; CS
    push word 0x0000    ; IP
    retf                ; CS:IP ← load_seg:0x0000

.fail:
    stc
    ret

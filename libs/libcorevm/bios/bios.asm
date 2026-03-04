; =============================================================================
; CoreVM BIOS — Custom BIOS for libcorevm hypervisor
; =============================================================================
; Assembled as a flat 64KB binary, loaded at physical address 0xF0000.
; All code uses segment 0xF000 with offsets 0x0000-0xFFFF.
; The CPU reset vector at 0xFFFF0 jumps to the POST entry point.
; =============================================================================

[BITS 16]
[ORG 0]

; ---------------------------------------------------------------------------
; Include modules in layout order.
; Data and tables come first, then routines, then POST entry, then reset vector.
; ---------------------------------------------------------------------------

%include "bda.asm"
%include "strings.asm"
%include "e820.asm"
%include "pic_pit.asm"
%include "serial.asm"
%include "video_init.asm"
%include "pci.asm"
%include "ide.asm"
%include "int10h.asm"
%include "int13h.asm"
%include "int15h.asm"
%include "int16h.asm"
%include "int19h.asm"
%include "int1ah.asm"
%include "int_misc.asm"
%include "selftest.asm"
%include "boot.asm"
%include "ivt.asm"
%include "post.asm"

; ---------------------------------------------------------------------------
; Pad to 0xFFF0 (64KB - 16 bytes), then place the reset vector.
; ---------------------------------------------------------------------------
times (0xFFF0 - ($ - $$)) db 0xFF

reset_vector:
    jmp 0xF000:post_entry       ; Far jump to POST (5 bytes)
    db 0                        ; Padding
    db '03/03/26'               ; BIOS date string (8 bytes at 0xFFF5)
    db 0xFF                     ; Model byte
    db 0                        ; Checksum placeholder

; Ensure we are exactly 64KB.
times (0x10000 - ($ - $$)) db 0xFF

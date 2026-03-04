; =============================================================================
; strings.asm — BIOS text strings
; =============================================================================

str_banner:         db 'CoreVM BIOS v1.0-trace2', 13, 10, 0
str_memory:         db 'Memory: ', 0
str_mb:             db ' MB', 13, 10, 0
str_kb:             db ' KB', 13, 10, 0
str_pci_scan:       db 'PCI: ', 0
str_pci_device:     db ' device(s)', 13, 10, 0
str_ide_master:     db 'IDE master: ', 0
str_ide_none:       db 'not present', 13, 10, 0
str_ide_found:      db 'present', 13, 10, 0
str_booting_hd:     db 'Booting from hard disk...', 13, 10, 0
str_booting_cd:     db 'Booting from CD-ROM...', 13, 10, 0
str_no_boot:        db 'No bootable device found!', 13, 10, 0
str_crlf:           db 13, 10, 0
str_et_rba:         db 'ET rba=', 0
str_et_cnt:         db ' cnt=', 0
str_et_seg:         db ' seg=', 0
str_et_emul:        db ' emul=', 0
str_et_load:        db 13, 10, 'Loading boot image...', 13, 10, 0
str_et_loaded:      db 'Boot image loaded. Jumping.', 13, 10, 0
str_cd_reentry:     db '[diag] returned to POST after CD handoff', 13, 10, 0
str_cd_reentry_ah:  db ' last INT13 AH=', 0
str_cd_reentry_dl:  db ' DL=', 0
str_cd_reentry_st:  db ' ST=', 0
str_i13_diag:       db '[diag] INT13 cnt=', 0

; Self-test strings.
str_st_header:      db '=== BIOS Self-Test ===', 13, 10, 0
str_st_ok:          db ' OK', 0
str_st_fail:        db ' FAIL', 0
str_st_summary:     db '=== Self-Test: ', 0
str_st_slash:       db '/', 0
str_st_passed:      db ' passed ===', 13, 10, 0
str_st_warn:        db '!!! WARNING: ', 0
str_st_failed:      db ' test(s) FAILED !!!', 13, 10, 0
str_st_ram:         db '  RAM read/write .......', 0
str_st_biosvar:     db '  BIOS var persist .....', 0
str_st_biosvar_hint:db '  (ROM not writable!)', 13, 10, 0
str_st_int15_88:    db '  INT15 AH=88 extmem ..', 0
str_st_e820:        db '  INT15 E820 map .......', 0
str_st_a20:         db '  A20 gate status ......', 0
str_st_a20_wire:    db '  A20 line wire test ...', 0
str_st_rtc:         db '  INT1A RTC time .......', 0
str_st_pci_bios:    db '  INT1A PCI BIOS .......', 0
str_st_pci_enum:    db '  PCI enumeration ......', 0
str_st_ide:         db '  IDE detection ........', 0
str_st_int13_rd:    db '  INT13 disk read ......', 0
str_st_timer:       db '  Timer tick (PIT) .....', 0
str_st_ivt:         db '  IVT integrity ........', 0
str_st_vga:         db '  VGA memory ...........', 0
str_st_cpuid:       db '  CPUID ................', 0
str_st_pmode:       db '  Protected mode .......', 0
str_st_extmem:      db '  Extended mem (>1MB) ..', 0
str_st_lparen:      db ' (', 0
str_st_rparen:      db ')', 13, 10, 0
str_st_kb:          db ' KB)', 13, 10, 0
str_st_entries:     db ' entries)', 13, 10, 0
str_st_devs:        db ' devs)', 13, 10, 0
str_st_colon:       db ':', 0
str_st_dot:         db '.', 0
str_st_pci_ver:     db 'v', 0
str_st_comma_lv:    db ', lv=', 0
str_st_mbr_sig:     db 'MBR', 0
str_st_skipped:     db 'skip', 0
str_st_ah_eq:       db 'AH=', 0
str_st_yes:         db 'yes', 0
str_st_no:          db 'no', 0

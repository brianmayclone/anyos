; =============================================================================
; strings.asm — BIOS text strings
; =============================================================================

str_banner:         db 'CoreVM BIOS v1.0', 13, 10, 0
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

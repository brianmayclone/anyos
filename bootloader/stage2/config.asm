; =============================================================================
; config.asm - Boot configuration parser (stub)
; =============================================================================

cfg_timeout:    dw 3
cfg_default:    db 0
cfg_count:      db 1
cfg_entries:    times 128*8 db 0

parse_config:
    ret

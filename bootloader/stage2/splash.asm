; =============================================================================
; splash.asm - Boot splash screen (stub)
; =============================================================================

show_splash:
    ret

wait_for_input:
    xor al, al
    ret

draw_gradient:
    ret

draw_logo:
    ret

logo_w:      dd 0
logo_h:      dd 0
logo_x:      dd 0
logo_y:      dd 0
logo_repeat: db 0

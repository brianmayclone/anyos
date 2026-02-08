; =============================================================================
; syscall_entry.asm - System call entry point (int 0x80)
; =============================================================================
; Convention:
;   EAX = syscall number
;   EBX = arg1, ECX = arg2, EDX = arg3, ESI = arg4, EDI = arg5
;   Return value in EAX

[BITS 32]

extern syscall_dispatch

global syscall_entry
syscall_entry:
    ; Save all registers
    pushad
    push ds
    push es
    push fs
    push gs

    ; Load kernel data segment
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    ; Push pointer to registers on stack as argument
    push esp
    call syscall_dispatch
    add esp, 4

    ; Store return value (EAX) back into the saved EAX on stack
    ; The saved EAX is at offset: gs(4) + fs(4) + es(4) + ds(4) + pushad_edi(4)
    ;   + pushad_esi(4) + pushad_ebp(4) + pushad_esp(4) + pushad_ebx(4)
    ;   + pushad_edx(4) + pushad_ecx(4) + pushad_eax(4) = offset 44 from esp
    ; pushad order: eax, ecx, edx, ebx, esp, ebp, esi, edi
    ; So eax is at [esp + 16 + 7*4] = [esp + 16 + 28] = [esp + 44]
    ; Wait: seg registers: gs, fs, es, ds = 16 bytes
    ; pushad: edi, esi, ebp, esp, ebx, edx, ecx, eax
    ; eax is last pushed = highest address = esp + 16 + 28 = esp + 44
    mov [esp + 44], eax

    ; Restore segments and registers
    pop gs
    pop fs
    pop es
    pop ds
    popad

    iretd

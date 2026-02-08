; =============================================================================
; hello.bin - First user-mode program for .anyOS
; =============================================================================
; Flat binary, runs in Ring 3
; Uses int 0x80 for system calls:
;   EAX = syscall number
;   EBX = arg1, ECX = arg2, EDX = arg3
;
; Syscall numbers:
;   1 = exit(status)
;   2 = write(fd, buf, len)
;   6 = getpid()
; =============================================================================

bits 32
org 0x08000000      ; Must match PROGRAM_LOAD_ADDR in loader.rs

_start:
    ; --- Print "Hello from .anyOS user mode!" ---
    mov eax, 2          ; SYS_WRITE
    mov ebx, 1          ; fd = stdout
    mov ecx, msg_hello  ; buffer pointer
    mov edx, msg_hello_len ; length
    int 0x80

    ; --- Get our PID ---
    mov eax, 6          ; SYS_GETPID
    int 0x80
    ; PID is now in EAX

    ; --- Print "PID = " ---
    push eax            ; save PID
    mov eax, 2          ; SYS_WRITE
    mov ebx, 1          ; fd = stdout
    mov ecx, msg_pid    ; "PID = "
    mov edx, msg_pid_len
    int 0x80

    ; --- Convert PID to decimal and print ---
    pop eax             ; restore PID
    call print_u32

    ; --- Print newline ---
    mov eax, 2
    mov ebx, 1
    mov ecx, msg_nl
    mov edx, 1
    int 0x80

    ; --- Print goodbye message ---
    mov eax, 2
    mov ebx, 1
    mov ecx, msg_bye
    mov edx, msg_bye_len
    int 0x80

    ; --- Exit with code 0 ---
    mov eax, 1          ; SYS_EXIT
    mov ebx, 0          ; exit code
    int 0x80

    ; Should never reach here
    jmp $

; print_u32: Print the unsigned 32-bit integer in EAX as decimal to stdout
print_u32:
    push ebp
    mov ebp, esp
    sub esp, 12         ; buffer for digits (max 10 digits for u32)
    lea edi, [ebp - 12]
    mov ecx, 0          ; digit count

    ; Handle zero case
    test eax, eax
    jnz .convert
    mov byte [edi], '0'
    mov ecx, 1
    jmp .print

.convert:
    ; Convert digits (least significant first)
    push esi
    lea esi, [ebp - 12]
    mov ecx, 0
.digit_loop:
    xor edx, edx
    mov ebx, 10
    div ebx             ; EAX = quotient, EDX = remainder
    add dl, '0'
    mov [esi + ecx], dl
    inc ecx
    test eax, eax
    jnz .digit_loop
    pop esi

    ; Reverse digits in place
    lea edi, [ebp - 12]
    mov esi, ecx
    dec esi             ; esi = last index
    xor ebx, ebx        ; ebx = first index
.reverse:
    cmp ebx, esi
    jge .print
    mov al, [edi + ebx]
    mov ah, [edi + esi]
    mov [edi + ebx], ah
    mov [edi + esi], al
    inc ebx
    dec esi
    jmp .reverse

.print:
    ; Write the digits
    mov edx, ecx        ; length
    lea ecx, [ebp - 12] ; buffer
    mov eax, 2           ; SYS_WRITE
    mov ebx, 1           ; fd = stdout
    int 0x80

    mov esp, ebp
    pop ebp
    ret

; Data section (part of the flat binary)
msg_hello:    db "Hello from .anyOS user mode!", 0x0A
msg_hello_len equ $ - msg_hello

msg_pid:      db "PID = "
msg_pid_len   equ $ - msg_pid

msg_bye:      db "User program exiting cleanly.", 0x0A
msg_bye_len   equ $ - msg_bye

msg_nl:       db 0x0A

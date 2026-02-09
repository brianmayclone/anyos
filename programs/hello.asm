; =============================================================================
; hello.bin - First user-mode program for .anyOS (x86-64)
; =============================================================================
; Flat binary, runs in Ring 3 (64-bit long mode)
; Uses int 0x80 for system calls:
;   RAX = syscall number
;   RBX = arg1, RCX = arg2, RDX = arg3
;
; Syscall numbers:
;   1 = exit(status)
;   2 = write(fd, buf, len)
;   6 = getpid()
; =============================================================================

bits 64
org 0x08000000      ; Must match PROGRAM_LOAD_ADDR in loader.rs

_start:
    ; --- Print "Hello from .anyOS user mode!" ---
    mov rax, 2          ; SYS_WRITE
    mov rbx, 1          ; fd = stdout
    lea rcx, [rel msg_hello]  ; buffer pointer (RIP-relative)
    mov rdx, msg_hello_len    ; length
    int 0x80

    ; --- Get our PID ---
    mov rax, 6          ; SYS_GETPID
    int 0x80
    ; PID is now in RAX

    ; --- Print "PID = " ---
    push rax            ; save PID
    mov rax, 2          ; SYS_WRITE
    mov rbx, 1          ; fd = stdout
    lea rcx, [rel msg_pid]
    mov rdx, msg_pid_len
    int 0x80

    ; --- Convert PID to decimal and print ---
    pop rax             ; restore PID
    call print_u64

    ; --- Print newline ---
    mov rax, 2
    mov rbx, 1
    lea rcx, [rel msg_nl]
    mov rdx, 1
    int 0x80

    ; --- Print goodbye message ---
    mov rax, 2
    mov rbx, 1
    lea rcx, [rel msg_bye]
    mov rdx, msg_bye_len
    int 0x80

    ; --- Exit with code 0 ---
    mov rax, 1          ; SYS_EXIT
    mov rbx, 0          ; exit code
    int 0x80

    ; Should never reach here
    jmp $

; print_u64: Print the unsigned 64-bit integer in RAX as decimal to stdout
print_u64:
    push rbp
    mov rbp, rsp
    sub rsp, 24         ; buffer for digits (max 20 digits for u64)
    lea rdi, [rbp - 24]
    xor rcx, rcx        ; digit count

    ; Handle zero case
    test rax, rax
    jnz .convert
    mov byte [rdi], '0'
    mov rcx, 1
    jmp .print

.convert:
    ; Convert digits (least significant first)
    push rsi
    lea rsi, [rbp - 24]
    xor rcx, rcx
.digit_loop:
    xor rdx, rdx
    mov rbx, 10
    div rbx             ; RAX = quotient, RDX = remainder
    add dl, '0'
    mov [rsi + rcx], dl
    inc rcx
    test rax, rax
    jnz .digit_loop
    pop rsi

    ; Reverse digits in place
    lea rdi, [rbp - 24]
    mov rsi, rcx
    dec rsi             ; rsi = last index
    xor rbx, rbx        ; rbx = first index
.reverse:
    cmp rbx, rsi
    jge .print
    mov al, [rdi + rbx]
    mov ah, [rdi + rsi]
    mov [rdi + rbx], ah
    mov [rdi + rsi], al
    inc rbx
    dec rsi
    jmp .reverse

.print:
    ; Write the digits
    mov rdx, rcx        ; length
    lea rcx, [rbp - 24] ; buffer
    mov rax, 2           ; SYS_WRITE
    mov rbx, 1           ; fd = stdout
    int 0x80

    mov rsp, rbp
    pop rbp
    ret

; Data section (part of the flat binary)
msg_hello:    db "Hello from .anyOS user mode! (64-bit)", 0x0A
msg_hello_len equ $ - msg_hello

msg_pid:      db "PID = "
msg_pid_len   equ $ - msg_pid

msg_bye:      db "User program exiting cleanly.", 0x0A
msg_bye_len   equ $ - msg_bye

msg_nl:       db 0x0A

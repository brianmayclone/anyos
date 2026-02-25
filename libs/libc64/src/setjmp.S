/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 setjmp/longjmp.
 *
 * jmp_buf layout (8 registers × 8 bytes = 64 bytes):
 *   [0]  rbx
 *   [8]  rbp
 *   [16] r12
 *   [24] r13
 *   [32] r14
 *   [40] r15
 *   [48] rsp  (caller's stack pointer, past return address)
 *   [56] rip  (return address)
 */

.section .text

/* int setjmp(jmp_buf env) — rdi = env pointer */
.global setjmp
setjmp:
    movq %rbx,  0(%rdi)
    movq %rbp,  8(%rdi)
    movq %r12, 16(%rdi)
    movq %r13, 24(%rdi)
    movq %r14, 32(%rdi)
    movq %r15, 40(%rdi)
    leaq 8(%rsp), %rax       /* caller's rsp (past return address) */
    movq %rax, 48(%rdi)
    movq (%rsp), %rax        /* return address */
    movq %rax, 56(%rdi)
    xorl %eax, %eax          /* return 0 */
    ret

/* void longjmp(jmp_buf env, int val) — rdi = env, esi = val */
.global longjmp
longjmp:
    movl %esi, %eax
    testl %eax, %eax
    jnz 1f
    incl %eax                 /* if val == 0, return 1 */
1:
    movq  0(%rdi), %rbx
    movq  8(%rdi), %rbp
    movq 16(%rdi), %r12
    movq 24(%rdi), %r13
    movq 32(%rdi), %r14
    movq 40(%rdi), %r15
    movq 48(%rdi), %rsp
    jmpq *56(%rdi)

/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 SYSCALL wrapper for anyOS.
 *
 * C prototype: long _syscall(long num, long a1, long a2, long a3, long a4, long a5)
 *
 * System V AMD64 ABI (C caller convention):
 *   rdi = num, rsi = a1, rdx = a2, rcx = a3, r8 = a4, r9 = a5
 *
 * anyOS kernel SYSCALL convention:
 *   rax = num, rbx = a1, r10 = a2, rdx = a3, rsi = a4, rdi = a5
 *   Return value in rax. Clobbers: rcx, r11 (by SYSCALL instruction).
 *
 * Note: LLVM reserves rbx on x86_64, so we must save/restore it.
 */

.section .text
.global _syscall
_syscall:
    pushq %rbx
    /* Map C ABI registers → anyOS kernel registers */
    movq %rdi, %rax       /* rax = syscall number */
    movq %rsi, %rbx       /* rbx = arg1 */
    movq %rdx, %r10       /* r10 = arg2 */
    movq %rcx, %rdx       /* rdx = arg3 */
    movq %r8,  %rsi       /* rsi = arg4 */
    movq %r9,  %rdi       /* rdi = arg5 */
    syscall
    popq %rbx
    ret

/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

.section .text

/* int _syscall(int num, int a1, int a2, int a3, int a4) */
.global _syscall
_syscall:
    pushl %ebx
    pushl %esi
    movl 12(%esp), %eax   /* num */
    movl 16(%esp), %ebx   /* a1 */
    movl 20(%esp), %ecx   /* a2 */
    movl 24(%esp), %edx   /* a3 */
    movl 28(%esp), %esi   /* a4 */
    int $0x80
    popl %esi
    popl %ebx
    ret

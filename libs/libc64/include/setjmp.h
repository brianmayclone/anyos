/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SETJMP_H
#define _SETJMP_H

/* x86_64: save rbx, rbp, r12, r13, r14, r15, rsp, rip = 8 regs = 64 bytes */
typedef unsigned long jmp_buf[8];

int setjmp(jmp_buf env);
void longjmp(jmp_buf env, int val) __attribute__((noreturn));

#endif

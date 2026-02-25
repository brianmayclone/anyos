/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SYS_UCONTEXT_H
#define _SYS_UCONTEXT_H

#include <signal.h>

typedef struct {
    unsigned long rax, rbx, rcx, rdx;
    unsigned long rsi, rdi, rbp, rsp;
    unsigned long r8, r9, r10, r11;
    unsigned long r12, r13, r14, r15;
    unsigned long rip, rflags;
} mcontext_t;

typedef struct ucontext {
    struct ucontext *uc_link;
    sigset_t         uc_sigmask;
    mcontext_t       uc_mcontext;
} ucontext_t;

#endif

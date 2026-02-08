/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#ifndef _SYS_UCONTEXT_H
#define _SYS_UCONTEXT_H

/* Minimal stub for TCC compilation */
typedef struct {
    unsigned int eip;
    unsigned int esp;
    unsigned int ebp;
} mcontext_t;

typedef struct ucontext {
    mcontext_t uc_mcontext;
} ucontext_t;

#endif

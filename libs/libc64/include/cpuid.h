/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * Minimal cpuid.h for freestanding x86_64.
 * Returns 0 (no features) — libraries fall back to software implementations.
 */

#ifndef _CPUID_H
#define _CPUID_H

static inline int __get_cpuid(
    unsigned int __level,
    unsigned int *__eax,
    unsigned int *__ebx,
    unsigned int *__ecx,
    unsigned int *__edx)
{
    *__eax = 0;
    *__ebx = 0;
    *__ecx = 0;
    *__edx = 0;
    return 0;
}

#endif

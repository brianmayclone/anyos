/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — minimal freestanding C library for x86_64 anyOS user programs.
 */

#ifndef _STDDEF_H
#define _STDDEF_H

typedef __SIZE_TYPE__    size_t;
typedef __PTRDIFF_TYPE__ ptrdiff_t;

/* In C++, wchar_t is a built-in keyword */
#ifndef __cplusplus
typedef __WCHAR_TYPE__   wchar_t;
#endif

#define NULL ((void *)0)
#define offsetof(type, member) __builtin_offsetof(type, member)

#endif

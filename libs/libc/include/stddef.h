/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#ifndef _STDDEF_H
#define _STDDEF_H

typedef unsigned int size_t;
typedef int ssize_t;
typedef int ptrdiff_t;
typedef unsigned short wchar_t;

#define NULL ((void *)0)
#define offsetof(type, member) ((size_t)&((type *)0)->member)

#endif

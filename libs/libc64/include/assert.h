/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _ASSERT_H
#define _ASSERT_H

#include <stdio.h>
#include <stdlib.h>

#ifdef NDEBUG
#define assert(expr) ((void)0)
#else
#define assert(expr) \
    ((expr) ? (void)0 : \
     (fprintf(stderr, "Assertion failed: %s, file %s, line %d\n", \
              #expr, __FILE__, __LINE__), abort()))
#endif

#endif

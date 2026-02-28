/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _FNMATCH_H
#define _FNMATCH_H

#define FNM_NOMATCH   1
#define FNM_PATHNAME  (1 << 0)
#define FNM_NOESCAPE  (1 << 1)
#define FNM_PERIOD    (1 << 2)

#ifdef __cplusplus
extern "C" {
#endif

int fnmatch(const char *pattern, const char *string, int flags);

#ifdef __cplusplus
}
#endif

#endif

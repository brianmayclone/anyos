/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _UTIME_H
#define _UTIME_H

#include <sys/types.h>

struct utimbuf {
    time_t actime;
    time_t modtime;
};

#ifdef __cplusplus
extern "C" {
#endif

int utime(const char *filename, const struct utimbuf *times);

#ifdef __cplusplus
}
#endif

#endif

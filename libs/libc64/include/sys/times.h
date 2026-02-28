/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SYS_TIMES_H
#define _SYS_TIMES_H

typedef long clock_t;

struct tms {
    clock_t tms_utime;
    clock_t tms_stime;
    clock_t tms_cutime;
    clock_t tms_cstime;
};

#ifdef __cplusplus
extern "C" {
#endif

clock_t times(struct tms *buf);

#ifdef __cplusplus
}
#endif

#endif

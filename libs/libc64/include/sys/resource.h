/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SYS_RESOURCE_H
#define _SYS_RESOURCE_H

#define RLIMIT_STACK  3
#define RLIMIT_NOFILE 7
#define RLIM_INFINITY (~0UL)

typedef unsigned long rlim_t;

struct rlimit {
    rlim_t rlim_cur;
    rlim_t rlim_max;
};

#ifdef __cplusplus
extern "C" {
#endif

int getrlimit(int resource, struct rlimit *rlim);
int setrlimit(int resource, const struct rlimit *rlim);

#ifdef __cplusplus
}
#endif

#endif

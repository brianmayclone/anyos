/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SYS_RESOURCE_H
#define _SYS_RESOURCE_H

#define RLIMIT_NOFILE 7
#define RLIM_INFINITY (~0UL)

struct rlimit {
    unsigned long rlim_cur;
    unsigned long rlim_max;
};

int getrlimit(int resource, struct rlimit *rlim);
int setrlimit(int resource, const struct rlimit *rlim);

#endif

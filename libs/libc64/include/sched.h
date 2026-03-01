/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SCHED_H
#define _SCHED_H

/* Scheduling policies (stubs for compatibility) */
#define SCHED_OTHER  0
#define SCHED_FIFO   1
#define SCHED_RR     2

struct sched_param {
    int sched_priority;
};

#ifdef __cplusplus
extern "C" {
#endif

int sched_yield(void);

#ifdef __cplusplus
}
#endif

#endif

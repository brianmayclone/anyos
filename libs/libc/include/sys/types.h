/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SYS_TYPES_H
#define _SYS_TYPES_H

#include <stddef.h>
#include <stdint.h>

typedef int pid_t;
typedef int ssize_t;
typedef unsigned int mode_t;
typedef unsigned int uid_t;
typedef unsigned int gid_t;
typedef unsigned int dev_t;
typedef unsigned int ino_t;
typedef unsigned int nlink_t;
typedef int off_t;
typedef unsigned int blksize_t;
typedef unsigned int blkcnt_t;

#ifndef _TIME_T_DEFINED
#define _TIME_T_DEFINED
typedef unsigned int time_t;
#endif

#endif

/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SYS_PARAM_H
#define _SYS_PARAM_H

#include <limits.h>

#ifndef PATH_MAX
#define PATH_MAX 256
#endif

#ifndef MAXPATHLEN
#define MAXPATHLEN PATH_MAX
#endif

#ifndef PAGE_SIZE
#define PAGE_SIZE 4096
#endif

#ifndef HZ
#define HZ 100
#endif

#ifndef NOFILE
#define NOFILE 64
#endif

#ifndef NBBY
#define NBBY 8
#endif

#ifndef howmany
#define howmany(x, y) (((x) + ((y) - 1)) / (y))
#endif

#ifndef MIN
#define MIN(a,b) (((a)<(b))?(a):(b))
#endif

#ifndef MAX
#define MAX(a,b) (((a)>(b))?(a):(b))
#endif

#endif

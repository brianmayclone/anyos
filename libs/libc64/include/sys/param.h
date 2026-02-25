/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SYS_PARAM_H
#define _SYS_PARAM_H

#define MAXPATHLEN 4096
#define PATH_MAX   4096
#define MAXHOSTNAMELEN 256

#ifndef MIN
#define MIN(a, b) ((a) < (b) ? (a) : (b))
#endif
#ifndef MAX
#define MAX(a, b) ((a) > (b) ? (a) : (b))
#endif

#endif

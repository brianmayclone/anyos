/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SYS_UN_H
#define _SYS_UN_H

#include <sys/socket.h>

#define AF_UNIX  1
#define AF_LOCAL AF_UNIX

#define UNIX_PATH_MAX 108

struct sockaddr_un {
    sa_family_t sun_family;
    char        sun_path[UNIX_PATH_MAX];
};

#endif /* _SYS_UN_H */

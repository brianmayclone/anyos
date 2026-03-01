/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _GRP_H
#define _GRP_H

#include <stddef.h>

typedef unsigned int gid_t;

struct group {
    char   *gr_name;    /* Group name */
    char   *gr_passwd;  /* Group password (unused) */
    gid_t   gr_gid;     /* Group ID */
    char  **gr_mem;     /* Group members */
};

#ifdef __cplusplus
extern "C" {
#endif

struct group *getgrgid(gid_t gid);
struct group *getgrnam(const char *name);
void setgrent(void);
void endgrent(void);
struct group *getgrent(void);

#ifdef __cplusplus
}
#endif

#endif

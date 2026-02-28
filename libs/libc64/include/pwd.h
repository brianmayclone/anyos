/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _PWD_H
#define _PWD_H

#include <sys/types.h>

struct passwd {
    char *pw_name;
    char *pw_dir;
    char *pw_shell;
    uid_t pw_uid;
    gid_t pw_gid;
};

#ifdef __cplusplus
extern "C" {
#endif

struct passwd *getpwuid(uid_t uid);
struct passwd *getpwnam(const char *name);
int getpwuid_r(uid_t uid, struct passwd *pwd, char *buf, size_t buflen, struct passwd **result);

#ifdef __cplusplus
}
#endif

#endif

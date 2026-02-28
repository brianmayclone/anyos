/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SPAWN_H
#define _SPAWN_H

#include <sys/types.h>

typedef int posix_spawn_file_actions_t;
typedef int posix_spawnattr_t;

#ifdef __cplusplus
extern "C" {
#endif

int posix_spawn(pid_t *pid, const char *path,
    const posix_spawn_file_actions_t *file_actions,
    const posix_spawnattr_t *attrp,
    char *const argv[], char *const envp[]);
int posix_spawnp(pid_t *pid, const char *file,
    const posix_spawn_file_actions_t *file_actions,
    const posix_spawnattr_t *attrp,
    char *const argv[], char *const envp[]);
int posix_spawn_file_actions_init(posix_spawn_file_actions_t *fa);
int posix_spawn_file_actions_destroy(posix_spawn_file_actions_t *fa);
int posix_spawn_file_actions_addopen(posix_spawn_file_actions_t *fa,
    int fildes, const char *path, int oflag, unsigned int mode);
int posix_spawn_file_actions_addclose(posix_spawn_file_actions_t *fa, int fildes);
int posix_spawn_file_actions_adddup2(posix_spawn_file_actions_t *fa, int fildes, int newfildes);
int posix_spawn_file_actions_addchdir_np(posix_spawn_file_actions_t *fa, const char *path);
int posix_spawnattr_setflags(posix_spawnattr_t *attr, short flags);
int posix_spawnattr_init(posix_spawnattr_t *attr);
int posix_spawnattr_destroy(posix_spawnattr_t *attr);

#ifdef __cplusplus
}
#endif

#endif

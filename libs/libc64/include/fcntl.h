/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _FCNTL_H
#define _FCNTL_H

#define O_RDONLY 0
#define O_WRONLY 1
#define O_RDWR 2
#define O_CREAT 0x04
#define O_TRUNC 0x08
#define O_APPEND 0x02
#define O_NONBLOCK 0x800
#define O_EXCL 0x10
#define O_CLOEXEC 0x80000
#define O_DIRECTORY 0x10000

#define F_DUPFD 0
#define F_GETFD 1
#define F_SETFD 2
#define F_GETFL 3
#define F_SETFL 4
#define F_DUPFD_CLOEXEC 1030

#define FD_CLOEXEC 1

#define AT_FDCWD (-100)
#define AT_SYMLINK_NOFOLLOW 0x100
#define AT_REMOVEDIR 0x200
#define AT_EACCESS 0x200

int open(const char *path, int flags, ...);
int fcntl(int fd, int cmd, ...);

#endif

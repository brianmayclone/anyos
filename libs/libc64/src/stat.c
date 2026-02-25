/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 stat/fstat/mkdir.
 */

#include <sys/stat.h>
#include <errno.h>
#include <string.h>

extern long _syscall(long num, long a1, long a2, long a3, long a4, long a5);

#define SYS_STAT   24
#define SYS_FSTAT  106
#define SYS_MKDIR  90

int stat(const char *path, struct stat *buf) {
    /* Kernel writes 7 × u32 (4 bytes each); use unsigned int to match. */
    unsigned int info[7]; /* type, size, flags, uid, gid, mode, mtime */
    long ret = _syscall(SYS_STAT, (long)path, (long)info, 0, 0, 0);
    if (ret < 0) { errno = (int)-ret; return -1; }
    if (buf) {
        memset(buf, 0, sizeof(*buf));
        unsigned int mode = info[5];
        if (info[0] == 1)
            buf->st_mode = S_IFDIR | (mode ? (mode & 0777) : 0755);
        else if (info[0] == 2)
            buf->st_mode = S_IFCHR | 0666;
        else
            buf->st_mode = S_IFREG | (mode ? (mode & 0777) : 0644);
        buf->st_size = (off_t)info[1];
        buf->st_nlink = 1;
        buf->st_uid = (uid_t)info[3];
        buf->st_gid = (gid_t)info[4];
        buf->st_mtime = (time_t)info[6];
        buf->st_atime = (time_t)info[6];
        buf->st_ctime = (time_t)info[6];
    }
    return 0;
}

int fstat(int fd, struct stat *buf) {
    /* Kernel writes 4 × u32 (4 bytes each); use unsigned int to match. */
    unsigned int info[4]; /* type, size, position, mtime */
    long ret = _syscall(SYS_FSTAT, fd, (long)info, 0, 0, 0);
    if (ret < 0) { errno = (int)-ret; return -1; }
    if (buf) {
        memset(buf, 0, sizeof(*buf));
        if (info[0] == 0) buf->st_mode = S_IFREG | 0644;
        else if (info[0] == 1) buf->st_mode = S_IFDIR | 0755;
        else buf->st_mode = S_IFCHR | 0666;
        buf->st_size = (off_t)info[1];
        buf->st_nlink = 1;
        buf->st_mtime = (time_t)info[3];
        buf->st_atime = (time_t)info[3];
        buf->st_ctime = (time_t)info[3];
    }
    return 0;
}

int lstat(const char *path, struct stat *buf) {
    /* anyOS has no symlinks, lstat == stat */
    return stat(path, buf);
}

int mkdir(const char *path, unsigned int mode) {
    (void)mode;
    long ret = _syscall(SYS_MKDIR, (long)path, 0, 0, 0, 0);
    if (ret < 0) { errno = (int)-ret; return -1; }
    return 0;
}

/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#include <sys/stat.h>
#include <errno.h>
#include <string.h>

extern int _syscall(int num, int a1, int a2, int a3, int a4);

#define SYS_STAT   24
#define SYS_FSTAT  106
#define SYS_MKDIR  90

int stat(const char *path, struct stat *buf) {
    unsigned int info[7]; /* type, size, flags, uid, gid, mode, mtime */
    int ret = _syscall(SYS_STAT, (int)path, (int)info, 0, 0);
    if (ret < 0) { errno = -ret; return -1; }
    if (buf) {
        memset(buf, 0, sizeof(*buf));
        unsigned int mode = info[5];
        if (info[0] == 1)
            buf->st_mode = S_IFDIR | (mode ? (mode & 0777) : 0755);
        else if (info[0] == 2)
            buf->st_mode = S_IFCHR | 0666;
        else
            buf->st_mode = S_IFREG | (mode ? (mode & 0777) : 0644);
        buf->st_size = info[1];
        buf->st_nlink = 1;
        buf->st_uid = info[3];
        buf->st_gid = info[4];
        buf->st_mtime = info[6];
        buf->st_atime = info[6];
        buf->st_ctime = info[6];
    }
    return 0;
}

int fstat(int fd, struct stat *buf) {
    unsigned int info[4]; /* type, size, position, mtime */
    int ret = _syscall(SYS_FSTAT, fd, (int)info, 0, 0);
    if (ret < 0) { errno = -ret; return -1; }
    if (buf) {
        memset(buf, 0, sizeof(*buf));
        if (info[0] == 0) buf->st_mode = S_IFREG | 0644;
        else if (info[0] == 1) buf->st_mode = S_IFDIR | 0755;
        else buf->st_mode = S_IFCHR | 0666;
        buf->st_size = info[1];
        buf->st_nlink = 1;
        buf->st_mtime = info[3];
        buf->st_atime = info[3];
        buf->st_ctime = info[3];
    }
    return 0;
}

int mkdir(const char *path, unsigned int mode) {
    (void)mode;
    int ret = _syscall(SYS_MKDIR, (int)path, 0, 0, 0);
    if (ret < 0) { errno = -ret; return -1; }
    return 0;
}

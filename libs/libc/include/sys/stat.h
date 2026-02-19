/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#ifndef _SYS_STAT_H
#define _SYS_STAT_H

#include <stddef.h>

struct stat {
    unsigned int st_mode;
    unsigned int st_size;
    unsigned int st_ino;
    unsigned int st_dev;
    unsigned int st_rdev;
    unsigned int st_nlink;
    unsigned int st_uid;
    unsigned int st_gid;
    unsigned int st_atime;
    unsigned int st_mtime;
    unsigned int st_ctime;
};

#define S_IFMT   0170000
#define S_IFREG  0100000
#define S_IFDIR  0040000
#define S_IFCHR  0020000
#define S_IFLNK  0120000
#define S_IFIFO  0010000
#define S_IFBLK  0060000
#define S_IFSOCK 0140000

/* Windows-style underscore variants (for libgit2 compat) */
#define _S_IFMT   S_IFMT
#define _S_IFREG  S_IFREG
#define _S_IFDIR  S_IFDIR
#define _S_IFLNK  S_IFLNK
#define _S_IFIFO  S_IFIFO

#define S_ISREG(m)  (((m) & S_IFMT) == S_IFREG)
#define S_ISDIR(m)  (((m) & S_IFMT) == S_IFDIR)
#define S_ISCHR(m)  (((m) & S_IFMT) == S_IFCHR)
#define S_ISLNK(m)  (((m) & S_IFMT) == S_IFLNK)
#define S_ISFIFO(m) (((m) & S_IFMT) == S_IFIFO)
#define S_ISBLK(m)  (((m) & S_IFMT) == S_IFBLK)
#define S_ISSOCK(m) (((m) & S_IFMT) == S_IFSOCK)

/* Special permission bits */
#define S_ISUID  0004000
#define S_ISGID  0002000
#define S_ISVTX  0001000

/* Permission bits */
#define S_IRWXU  0700
#define S_IRUSR  0400
#define S_IWUSR  0200
#define S_IXUSR  0100
#define S_IRWXG  0070
#define S_IRGRP  0040
#define S_IWGRP  0020
#define S_IXGRP  0010
#define S_IRWXO  0007
#define S_IROTH  0004
#define S_IWOTH  0002
#define S_IXOTH  0001

int stat(const char *path, struct stat *buf);
int fstat(int fd, struct stat *buf);
int fstatat(int dirfd, const char *pathname, struct stat *statbuf, int flags);
int mkdir(const char *path, unsigned int mode);

#endif

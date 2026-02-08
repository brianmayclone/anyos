#ifndef _SYS_STAT_H
#define _SYS_STAT_H

#include <stddef.h>

struct stat {
    unsigned int st_mode;
    unsigned int st_size;
    unsigned int st_ino;
    unsigned int st_dev;
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

#define S_ISREG(m)  (((m) & S_IFMT) == S_IFREG)
#define S_ISDIR(m)  (((m) & S_IFMT) == S_IFDIR)
#define S_ISCHR(m)  (((m) & S_IFMT) == S_IFCHR)

int stat(const char *path, struct stat *buf);
int fstat(int fd, struct stat *buf);
int mkdir(const char *path, unsigned int mode);

#endif

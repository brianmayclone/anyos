/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _DIRENT_H
#define _DIRENT_H

#include <sys/types.h>

#define DT_UNKNOWN 0
#define DT_FIFO    1
#define DT_CHR     2
#define DT_DIR     4
#define DT_BLK     6
#define DT_REG     8
#define DT_LNK     10
#define DT_SOCK    12
#define DT_WHT     14

struct dirent {
    ino_t d_ino;
    unsigned char d_type;
    char d_name[256];
};

typedef struct {
    int __fd;
    void *__data;
} DIR;

#ifdef __cplusplus
extern "C" {
#endif

DIR *opendir(const char *name);
struct dirent *readdir(DIR *dirp);
int closedir(DIR *dirp);
void rewinddir(DIR *dirp);
int alphasort(const struct dirent **a, const struct dirent **b);
int scandir(const char *dirp, struct dirent ***namelist,
            int (*filter)(const struct dirent *),
            int (*compar)(const struct dirent **, const struct dirent **));
int dirfd(DIR *dirp);

#ifdef __cplusplus
}
#endif

#endif

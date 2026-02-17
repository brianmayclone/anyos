/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * Stub implementations for POSIX functions not yet fully implemented.
 * These allow code to compile and link; most return error codes at runtime.
 *
 * NOTE: zlib functions are NOT stubbed here — link with real libz.a.
 * NOTE: signal/raise are NOT here — they live in signal.c.
 */

#include <stddef.h>
#include <errno.h>
#include <string.h>

/* ── getopt ── */
#include <getopt.h>

char *optarg = NULL;
int optind = 1, opterr = 1, optopt = '?';

int getopt(int argc, char * const argv[], const char *optstring) {
    (void)argc; (void)argv; (void)optstring;
    return -1;
}

int getopt_long(int argc, char * const argv[], const char *optstring,
                const struct option *longopts, int *longindex) {
    (void)argc; (void)argv; (void)optstring;
    (void)longopts; (void)longindex;
    return -1;
}

/* ── dirent — real implementations using SYS_READDIR ── */
#include <dirent.h>
#include <stdlib.h>

extern int _syscall(int num, int a1, int a2, int a3, int a4);
#define SYS_READDIR 23

/* Kernel readdir entry: 64 bytes each
 * [type:u8, name_len:u8, pad:u16, size:u32, name:56bytes] */
#define KDIR_ENTRY_SIZE 64
#define KDIR_MAX_ENTRIES 128
#define KDIR_BUF_SIZE (KDIR_ENTRY_SIZE * KDIR_MAX_ENTRIES)

typedef struct {
    char path[256];
    unsigned char buf[KDIR_BUF_SIZE];
    int count;
    int pos;
} DIR_INTERNAL;

static struct dirent _de;  /* returned by readdir */

DIR *opendir(const char *name) {
    if (!name) { errno = EINVAL; return NULL; }
    DIR_INTERNAL *d = (DIR_INTERNAL *)malloc(sizeof(DIR_INTERNAL));
    if (!d) { errno = ENOMEM; return NULL; }
    /* Copy path */
    size_t len = strlen(name);
    if (len >= sizeof(d->path)) { free(d); errno = ENAMETOOLONG; return NULL; }
    memcpy(d->path, name, len + 1);
    /* Fetch all entries from kernel */
    int n = _syscall(SYS_READDIR, (int)d->path, (int)d->buf, KDIR_BUF_SIZE, 0);
    if (n == -1 || n == (int)0xFFFFFFFF) { free(d); errno = ENOENT; return NULL; }
    d->count = n;
    d->pos = 0;
    return (DIR *)d;
}

struct dirent *readdir(DIR *dirp) {
    if (!dirp) return NULL;
    DIR_INTERNAL *d = (DIR_INTERNAL *)dirp;
    if (d->pos >= d->count) return NULL;
    unsigned char *e = d->buf + d->pos * KDIR_ENTRY_SIZE;
    unsigned char type = e[0];
    unsigned char name_len = e[1];
    /* unsigned int size = *(unsigned int *)(e + 4); */
    if (name_len > 55) name_len = 55;
    _de.d_ino = d->pos + 1;
    _de.d_type = (type == 1) ? DT_DIR : DT_REG;
    memcpy(_de.d_name, e + 8, name_len);
    _de.d_name[name_len] = '\0';
    d->pos++;
    return &_de;
}

int closedir(DIR *dirp) {
    if (dirp) free(dirp);
    return 0;
}

void rewinddir(DIR *dirp) {
    if (dirp) ((DIR_INTERNAL *)dirp)->pos = 0;
}

int alphasort(const struct dirent **a, const struct dirent **b) {
    return strcmp((*a)->d_name, (*b)->d_name);
}

int scandir(const char *dirp, struct dirent ***namelist,
            int (*filter)(const struct dirent *),
            int (*compar)(const struct dirent **, const struct dirent **)) {
    DIR *d = opendir(dirp);
    if (!d) return -1;

    struct dirent **list = NULL;
    int count = 0, cap = 0;
    struct dirent *entry;

    while ((entry = readdir(d)) != NULL) {
        if (filter && !filter(entry)) continue;
        if (count >= cap) {
            cap = cap ? cap * 2 : 16;
            struct dirent **tmp = (struct dirent **)realloc(list, cap * sizeof(struct dirent *));
            if (!tmp) { goto fail; }
            list = tmp;
        }
        list[count] = (struct dirent *)malloc(sizeof(struct dirent));
        if (!list[count]) { goto fail; }
        memcpy(list[count], entry, sizeof(struct dirent));
        count++;
    }
    closedir(d);

    if (compar && count > 1) {
        /* Simple insertion sort */
        for (int i = 1; i < count; i++) {
            struct dirent *tmp = list[i];
            int j = i;
            while (j > 0 && compar((const struct dirent **)&list[j-1],
                                    (const struct dirent **)&tmp) > 0) {
                list[j] = list[j-1];
                j--;
            }
            list[j] = tmp;
        }
    }

    *namelist = list;
    return count;

fail:
    for (int i = 0; i < count; i++) free(list[i]);
    free(list);
    closedir(d);
    errno = ENOMEM;
    return -1;
}

/* ── locale ── */
#include <locale.h>

static struct lconv _default_lconv = {
    ".", "", "", "", "", "", "", "", "", "",
    127, 127, 127, 127, 127, 127, 127, 127
};

char *setlocale(int category, const char *locale) {
    (void)category; (void)locale;
    return "C";
}

struct lconv *localeconv(void) {
    return &_default_lconv;
}

/* ── iconv ── */
#include <iconv.h>

iconv_t iconv_open(const char *tocode, const char *fromcode) {
    (void)tocode; (void)fromcode;
    errno = EINVAL;
    return (iconv_t)-1;
}

size_t iconv(iconv_t cd, char **inbuf, size_t *inbytesleft,
             char **outbuf, size_t *outbytesleft) {
    (void)cd; (void)inbuf; (void)inbytesleft;
    (void)outbuf; (void)outbytesleft;
    errno = EINVAL;
    return (size_t)-1;
}

int iconv_close(iconv_t cd) {
    (void)cd;
    return 0;
}

/* ── regex ── */
#include <regex.h>

int regcomp(regex_t *preg, const char *regex, int cflags) {
    (void)preg; (void)regex; (void)cflags;
    return REG_ESPACE;
}

int regexec(const regex_t *preg, const char *string, size_t nmatch,
            regmatch_t pmatch[], int eflags) {
    (void)preg; (void)string; (void)nmatch; (void)pmatch; (void)eflags;
    return REG_NOMATCH;
}

void regfree(regex_t *preg) {
    (void)preg;
}

size_t regerror(int errcode, const regex_t *preg, char *errbuf, size_t errbuf_size) {
    (void)errcode; (void)preg;
    if (errbuf && errbuf_size > 0) {
        errbuf[0] = '\0';
    }
    return 0;
}

/* ── sys/utsname ── */
#include <sys/utsname.h>

int uname(struct utsname *buf) {
    if (!buf) { errno = EFAULT; return -1; }
    strcpy(buf->sysname, "anyOS");
    strcpy(buf->nodename, "anyos");
    strcpy(buf->release, "1.0");
    strcpy(buf->version, "1.0");
    strcpy(buf->machine, "i686");
    return 0;
}

/* ── ctype: isascii ── */
int isascii(int c) {
    return (c >= 0 && c <= 127);
}

/* ── stdlib: atexit ── */
typedef void (*atexit_func)(void);
static atexit_func _atexit_funcs[32];
static int _atexit_count = 0;

int atexit(void (*function)(void)) {
    if (_atexit_count >= 32) return -1;
    _atexit_funcs[_atexit_count++] = function;
    return 0;
}

int setenv(const char *name, const char *value, int overwrite) {
    (void)name; (void)value; (void)overwrite;
    return 0;
}

int unsetenv(const char *name) {
    (void)name;
    return 0;
}

int mkstemp(char *tmpl) {
    (void)tmpl;
    errno = ENOSYS;
    return -1;
}

char *mktemp(char *tmpl) {
    (void)tmpl;
    return tmpl;
}

/* ── realpath ── */
char *realpath(const char *path, char *resolved_path) {
    if (!path) { errno = EINVAL; return NULL; }
    if (!resolved_path) {
        static char _rp_buf[256];
        resolved_path = _rp_buf;
    }
    size_t len = strlen(path);
    if (len >= 256) { errno = ENAMETOOLONG; return NULL; }
    memcpy(resolved_path, path, len + 1);
    return resolved_path;
}

/* ── mktime / difftime ── */
#include <time.h>

time_t mktime(struct tm *tm) {
    if (!tm) return (time_t)-1;
    int y = tm->tm_year + 1900;
    int m = tm->tm_mon + 1;
    int d = tm->tm_mday;
    if (m <= 2) { y--; m += 12; }
    int days = 365 * y + y/4 - y/100 + y/400 + (153*(m-3)+2)/5 + d - 719469;
    return (time_t)(days * 86400 + tm->tm_hour * 3600 + tm->tm_min * 60 + tm->tm_sec);
}

double difftime(time_t time1, time_t time0) {
    return (double)((int)time1 - (int)time0);
}

int nanosleep(const struct timespec *req, struct timespec *rem) {
    (void)req; (void)rem;
    return 0;
}

/* ── stdio: setbuf / setlinebuf ── */
#include <stdio.h>

void setbuf(FILE *stream, char *buf) {
    (void)stream; (void)buf;
}

void setlinebuf(FILE *stream) {
    (void)stream;
}

/* ── POSIX filesystem stubs ── */
#include <sys/stat.h>
#include <dirent.h>
#include <unistd.h>

int dirfd(DIR *dirp) {
    if (!dirp) { errno = EINVAL; return -1; }
    return dirp->__fd;
}

int fstatat(int dirfd, const char *pathname, struct stat *statbuf, int flags) {
    (void)dirfd; (void)flags;
    /* Fall back to regular stat — anyOS has no openat/fstatat */
    return stat(pathname, statbuf);
}

int unlinkat(int dirfd, const char *pathname, int flags) {
    (void)dirfd; (void)flags;
    return unlink(pathname);
}

int rmdir(const char *pathname) {
    (void)pathname;
    errno = ENOSYS;
    return -1;
}

/* mkdir lives in stat.c with real SYS_MKDIR implementation */

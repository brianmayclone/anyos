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
#include <fcntl.h>
#include <unistd.h>
#include <signal.h>

/* ── getopt (full GNU-compatible implementation) ── */
#include <getopt.h>
#include <stdio.h>

char *optarg = NULL;
int optind = 1, opterr = 1, optopt = '?';
static int _optpos = 0; /* position within clustered short options */

int getopt(int argc, char * const argv[], const char *optstring) {
    if (optind >= argc || !argv[optind]) return -1;

    const char *arg = argv[optind];

    /* Reset position if we've moved to a new argument */
    if (_optpos == 0) {
        if (arg[0] != '-' || arg[1] == '\0') return -1; /* not an option */
        if (arg[1] == '-' && arg[2] == '\0') { optind++; return -1; } /* "--" */
    }

    /* Current option character */
    int pos = _optpos ? _optpos : 1;
    int c = arg[pos];
    if (c == '\0') { /* end of this arg, advance */
        optind++;
        _optpos = 0;
        return getopt(argc, argv, optstring);
    }

    /* Leading ':' suppresses error messages */
    int quiet = (optstring[0] == ':');
    const char *os = optstring;
    if (*os == ':' || *os == '+' || *os == '-') os++;

    /* Find in optstring */
    const char *match = NULL;
    for (const char *p = os; *p; p++) {
        if (*p == c) { match = p; break; }
    }

    if (!match) {
        optopt = c;
        if (opterr && !quiet) fprintf(stderr, "%s: invalid option -- '%c'\n", argv[0], c);
        if (arg[pos + 1]) _optpos = pos + 1; else { optind++; _optpos = 0; }
        return '?';
    }

    if (match[1] == ':') {
        /* Option requires argument */
        if (arg[pos + 1]) {
            /* Argument is rest of this argv entry */
            optarg = (char *)&arg[pos + 1];
            optind++;
            _optpos = 0;
        } else if (match[2] == ':') {
            /* Optional argument (::) — no arg if not adjacent */
            optarg = NULL;
            optind++;
            _optpos = 0;
        } else if (optind + 1 < argc) {
            /* Argument is next argv entry */
            optarg = argv[optind + 1];
            optind += 2;
            _optpos = 0;
        } else {
            optopt = c;
            optind++;
            _optpos = 0;
            if (opterr && !quiet)
                fprintf(stderr, "%s: option requires an argument -- '%c'\n", argv[0], c);
            return quiet ? ':' : '?';
        }
    } else {
        /* No argument */
        optarg = NULL;
        if (arg[pos + 1]) _optpos = pos + 1;
        else { optind++; _optpos = 0; }
    }

    return c;
}

int getopt_long(int argc, char * const argv[], const char *optstring,
                const struct option *longopts, int *longindex) {
    if (optind >= argc || !argv[optind]) return -1;

    const char *arg = argv[optind];

    /* Check for long option (--foo) */
    if (arg[0] == '-' && arg[1] == '-' && arg[2] != '\0' && _optpos == 0) {
        const char *name = arg + 2;
        /* Find '=' separator */
        const char *eq = NULL;
        int namelen = 0;
        for (const char *p = name; *p; p++) {
            if (*p == '=') { eq = p; break; }
            namelen++;
        }
        if (!eq) namelen = (int)strlen(name);

        /* Search longopts */
        int match_idx = -1;
        int match_count = 0;
        for (int i = 0; longopts && longopts[i].name; i++) {
            if (strncmp(longopts[i].name, name, namelen) == 0) {
                if ((int)strlen(longopts[i].name) == namelen) {
                    /* Exact match */
                    match_idx = i;
                    match_count = 1;
                    break;
                }
                match_idx = i;
                match_count++;
            }
        }

        if (match_count == 0) {
            if (opterr) fprintf(stderr, "%s: unrecognized option '--%.*s'\n", argv[0], namelen, name);
            optind++;
            return '?';
        }
        if (match_count > 1) {
            if (opterr) fprintf(stderr, "%s: option '--%.*s' is ambiguous\n", argv[0], namelen, name);
            optind++;
            return '?';
        }

        if (longindex) *longindex = match_idx;
        const struct option *o = &longopts[match_idx];

        if (o->has_arg == no_argument) {
            if (eq) {
                if (opterr)
                    fprintf(stderr, "%s: option '--%s' doesn't allow an argument\n", argv[0], o->name);
                optind++;
                return '?';
            }
            optarg = NULL;
        } else if (o->has_arg == required_argument) {
            if (eq) {
                optarg = (char *)(eq + 1);
            } else if (optind + 1 < argc) {
                optarg = argv[optind + 1];
                optind++;
            } else {
                if (opterr)
                    fprintf(stderr, "%s: option '--%s' requires an argument\n", argv[0], o->name);
                optind++;
                return (optstring[0] == ':') ? ':' : '?';
            }
        } else { /* optional_argument */
            optarg = eq ? (char *)(eq + 1) : NULL;
        }

        optind++;
        if (o->flag) { *o->flag = o->val; return 0; }
        return o->val;
    }

    /* Fall back to short option parsing */
    return getopt(argc, argv, optstring);
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
    if (n < 0) { free(d); errno = -n; return NULL; }
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

#define SYS_SETENV 182

int setenv(const char *name, const char *value, int overwrite) {
    if (!name || !*name || strchr(name, '=')) { errno = EINVAL; return -1; }
    if (!overwrite) {
        /* Check if already set — SYS_GETENV returns u32::MAX if not found */
        char tmp[4];
        int r = _syscall(183, (int)name, (int)tmp, sizeof(tmp), 0);
        if (r != -1 && r != (int)0xFFFFFFFF) return 0; /* already set, don't overwrite */
    }
    /* Build "NAME=VALUE" string for SYS_SETENV */
    size_t nlen = strlen(name);
    size_t vlen = value ? strlen(value) : 0;
    char buf[512];
    if (nlen + 1 + vlen >= sizeof(buf)) { errno = ENOMEM; return -1; }
    memcpy(buf, name, nlen);
    buf[nlen] = '=';
    if (value) memcpy(buf + nlen + 1, value, vlen);
    buf[nlen + 1 + vlen] = '\0';
    _syscall(SYS_SETENV, (int)buf, 0, 0, 0);
    return 0;
}

int unsetenv(const char *name) {
    if (!name || !*name || strchr(name, '=')) { errno = EINVAL; return -1; }
    /* SYS_SETENV with empty value effectively clears it */
    char buf[256];
    size_t nlen = strlen(name);
    if (nlen + 2 >= sizeof(buf)) return -1;
    memcpy(buf, name, nlen);
    buf[nlen] = '=';
    buf[nlen + 1] = '\0';
    _syscall(SYS_SETENV, (int)buf, 0, 0, 0);
    return 0;
}

int mkstemp(char *tmpl) {
    if (!tmpl) { errno = EINVAL; return -1; }
    size_t len = strlen(tmpl);
    if (len < 6) { errno = EINVAL; return -1; }
    char *suffix = tmpl + len - 6;
    /* Verify template ends with XXXXXX */
    for (int i = 0; i < 6; i++) {
        if (suffix[i] != 'X') { errno = EINVAL; return -1; }
    }
    static unsigned int _mkstemp_counter = 0;
    for (int tries = 0; tries < 100; tries++) {
        unsigned int v = (unsigned int)rand() ^ (++_mkstemp_counter * 7919);
        for (int i = 0; i < 6; i++) {
            int r = (v >> (i * 5)) % 36;
            suffix[i] = (char)(r < 10 ? '0' + r : 'a' + r - 10);
        }
        int fd = open(tmpl, 0x201 /* O_CREAT | O_RDWR */, 0);
        if (fd >= 0) return fd;
    }
    errno = EEXIST;
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
    if (!req) { errno = EINVAL; return -1; }
    /* Convert timespec to milliseconds for SYS_SLEEP (8) */
    unsigned int ms = (unsigned int)(req->tv_sec * 1000 + req->tv_nsec / 1000000);
    if (ms > 0) _syscall(8 /*SYS_SLEEP*/, (int)ms, 0, 0, 0);
    if (rem) { rem->tv_sec = 0; rem->tv_nsec = 0; }
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
    if (!pathname) { errno = EINVAL; return -1; }
    int r = _syscall(91 /*SYS_UNLINK*/, (int)pathname, 0, 0, 0);
    if (r < 0) { errno = -r; return -1; }
    return 0;
}

/* mkdir lives in stat.c with real SYS_MKDIR implementation */

/* ── posix_spawn ── */
#include <spawn.h>

#define SYS_SPAWN_STUBS 27

int posix_spawn(pid_t *pid, const char *path,
    const posix_spawn_file_actions_t *file_actions,
    const posix_spawnattr_t *attrp,
    char *const argv[], char *const envp[]) {
    (void)file_actions; (void)attrp; (void)envp;
    if (!path) { errno = EINVAL; return EINVAL; }
    /* Build space-separated args string from argv[] */
    char args[1024];
    int pos = 0;
    if (argv) {
        for (int i = 0; argv[i]; i++) {
            if (i > 0 && pos < 1022) args[pos++] = ' ';
            for (const char *s = argv[i]; *s && pos < 1022; s++)
                args[pos++] = *s;
        }
    }
    args[pos] = '\0';
    int tid = _syscall(SYS_SPAWN_STUBS, (int)path, 0, (int)args, 0);
    if (tid < 0) { errno = ENOENT; return ENOENT; }
    if (pid) *pid = (pid_t)tid;
    return 0;
}

int posix_spawnp(pid_t *pid, const char *file,
    const posix_spawn_file_actions_t *file_actions,
    const posix_spawnattr_t *attrp,
    char *const argv[], char *const envp[]) {
    /* Try /bin/<file> if not an absolute path */
    if (file && file[0] != '/') {
        char path[256];
        int len = 0;
        const char *prefix = "/bin/";
        for (const char *p = prefix; *p; p++) path[len++] = *p;
        for (const char *p = file; *p && len < 254; p++) path[len++] = *p;
        path[len] = '\0';
        return posix_spawn(pid, path, file_actions, attrp, argv, envp);
    }
    return posix_spawn(pid, file, file_actions, attrp, argv, envp);
}

int posix_spawn_file_actions_init(posix_spawn_file_actions_t *fa) { if (fa) *fa = 0; return 0; }
int posix_spawn_file_actions_destroy(posix_spawn_file_actions_t *fa) { (void)fa; return 0; }
int posix_spawnattr_init(posix_spawnattr_t *attr) { if (attr) *attr = 0; return 0; }
int posix_spawnattr_destroy(posix_spawnattr_t *attr) { (void)attr; return 0; }

/* ── POSIX stubs for libgit2 and other ports ── */

int fsync(int fd) { (void)fd; return 0; }
int fdatasync(int fd) { (void)fd; return 0; }
int chmod(const char *path, unsigned int mode) {
    if (!path) { errno = EINVAL; return -1; }
    int r = _syscall(224 /*SYS_CHMOD*/, (int)path, (int)mode, 0, 0);
    if (r < 0) { errno = -r; return -1; }
    return 0;
}
int fchmod(int fd, unsigned int mode) { (void)fd; (void)mode; return 0; }

extern int _syscall(int num, int a1, int a2, int a3, int a4);

int lstat(const char *path, struct stat *buf) {
    /* anyOS has no symlinks, lstat == stat */
    return stat(path, buf);
}

unsigned int getuid(void) { return 0; }
unsigned int getgid(void) { return 0; }
unsigned int umask(unsigned int mask) { (void)mask; return 022; }

int link(const char *oldpath, const char *newpath) {
    (void)oldpath; (void)newpath;
    errno = ENOSYS;
    return -1;
}

int symlink(const char *target, const char *linkpath) {
    if (!target || !linkpath) { errno = EINVAL; return -1; }
    int r = _syscall(96 /*SYS_SYMLINK*/, (int)target, (int)linkpath, 0, 0);
    if (r < 0) { errno = -r; return -1; }
    return 0;
}

int readlink(const char *path, char *buf, size_t bufsiz) {
    if (!path || !buf) { errno = EINVAL; return -1; }
    int r = _syscall(97 /*SYS_READLINK*/, (int)path, (int)buf, (int)bufsiz, 0);
    if (r < 0) { errno = -r; return -1; }
    return r;
}

int chown(const char *path, unsigned int owner, unsigned int group) {
    (void)path; (void)owner; (void)group;
    return 0;
}

long sysconf(int name) {
    if (name == 30) return 4096; /* _SC_PAGESIZE */
    return -1;
}

int getpid(void) { return _syscall(6, 0, 0, 0, 0); }   /* SYS_GETPID = 6 */
int getppid(void) { return _syscall(247, 0, 0, 0, 0); } /* SYS_GETPPID = 247 */
int getpgid(int pid) { (void)pid; return getpid(); }
int setpgid(int pid, int pgid) { (void)pid; (void)pgid; return 0; }
int setpgrp(void) { return 0; }
int getpgrp(void) { return getpid(); }
unsigned int geteuid(void) { return 0; }
unsigned int getegid(void) { return 0; }
int getsid(int pid) { (void)pid; return getpid(); }
int setsid(void) { return getpid(); }
unsigned int alarm(unsigned int seconds) { (void)seconds; return 0; }

int execve(const char *path, char *const argv[], char *const envp[]) {
    (void)envp; /* anyOS exec ignores envp for now */
    return execv(path, argv);
}

/* Resource limits — stubs */
struct rlimit { unsigned long rlim_cur; unsigned long rlim_max; };
int getrlimit(int resource, struct rlimit *rlim) {
    (void)resource;
    if (rlim) { rlim->rlim_cur = ~0UL; rlim->rlim_max = ~0UL; }
    return 0;
}
int setrlimit(int resource, const struct rlimit *rlim) {
    (void)resource; (void)rlim;
    return 0;
}

/* Terminal control — stubs (no job control) */
int tcgetpgrp(int fd) { (void)fd; return getpid(); }
int tcsetpgrp(int fd, int pgrp) { (void)fd; (void)pgrp; return 0; }

struct termios;
int tcgetattr(int fd, struct termios *t) { (void)fd; (void)t; return -1; }
int tcsetattr(int fd, int act, const struct termios *t) { (void)fd; (void)act; (void)t; return -1; }
unsigned int cfgetispeed(const struct termios *t) { (void)t; return 0; }
unsigned int cfgetospeed(const struct termios *t) { (void)t; return 0; }

/* wait() — calls waitpid(-1, status, 0) */
int wait(int *status) { return waitpid(-1, status, 0); }

/* utimes stub */
#include <sys/time.h>
int utimes(const char *filename, const struct timeval times[2]) {
    (void)filename; (void)times;
    return 0; /* no-op */
}

/* strnlen */
size_t strnlen(const char *s, size_t maxlen) {
    size_t len = 0;
    while (len < maxlen && s[len]) len++;
    return len;
}

/* pwd.h stubs */
#include <pwd.h>

struct passwd *getpwuid(uid_t uid) {
    static struct passwd pw = { "user", "/home/user", "/bin/sh", 0, 0 };
    (void)uid;
    return &pw;
}

struct passwd *getpwnam(const char *name) {
    (void)name;
    static struct passwd pw = { "user", "/home/user", "/bin/sh", 0, 0 };
    return &pw;
}

int getpwuid_r(uid_t uid, struct passwd *pwd, char *buf, size_t buflen, struct passwd **result) {
    (void)uid; (void)buf; (void)buflen;
    if (pwd) {
        pwd->pw_name = "user";
        pwd->pw_dir = "/home/user";
        pwd->pw_shell = "/bin/sh";
        pwd->pw_uid = 0;
        pwd->pw_gid = 0;
    }
    if (result) *result = pwd;
    return 0;
}

/* gmtime_r / localtime_r */
#include <sys/time.h>
struct tm *gmtime_r(const time_t *timer, struct tm *result) {
    struct tm *t = gmtime(timer);
    if (t && result) *result = *t;
    return result;
}

struct tm *localtime_r(const time_t *timer, struct tm *result) {
    struct tm *t = localtime(timer);
    if (t && result) *result = *t;
    return result;
}

/* wait3 — wrapper around waitpid (ignores rusage) */
int wait3(int *status, int options, void *rusage) {
    (void)rusage;
    return waitpid(-1, status, options);
}

/* times — stub (no process accounting) */
long times(void *buf) {
    if (buf) memset(buf, 0, 16); /* sizeof(struct tms) = 4 * sizeof(clock_t) */
    return 0;
}

/* ioctl is in unistd.c */

/* mempcpy — like memcpy but returns dest + n */
void *mempcpy(void *dest, const void *src, size_t n) {
    memcpy(dest, src, n);
    return (char *)dest + n;
}

/* stpcpy — like strcpy but returns pointer to '\0' */
char *stpcpy(char *dest, const char *src) {
    while ((*dest = *src)) { dest++; src++; }
    return dest;
}

/* strchrnul is in string.c */

/* strsignal — return signal name string */
static char _signame_buf[16];
char *strsignal(int sig) {
    if (sig >= 0 && sig < 32) {
        /* Simple numeric representation */
        char *p = _signame_buf;
        *p++ = 'S'; *p++ = 'i'; *p++ = 'g'; *p++ = ' ';
        if (sig >= 10) *p++ = '0' + sig / 10;
        *p++ = '0' + sig % 10;
        *p = '\0';
        return _signame_buf;
    }
    return "Unknown signal";
}

/* isblank */
int isblank(int c) {
    return c == ' ' || c == '\t';
}

/* killpg — send signal to process group */
int killpg(int pgrp, int sig) {
    return kill(-pgrp, sig);
}

/* faccessat — stub, falls back to access() */
int faccessat(int dirfd, const char *pathname, int mode, int flags) {
    (void)dirfd; (void)flags;
    return access(pathname, mode);
}

/* vfork — anyOS has no vfork, just use fork */
int vfork(void) {
    return fork();
}

/* stpncpy — like strncpy but returns pointer to end */
char *stpncpy(char *dest, const char *src, size_t n) {
    size_t i;
    for (i = 0; i < n && src[i]; i++)
        dest[i] = src[i];
    char *ret = dest + i;
    for (; i < n; i++)
        dest[i] = '\0';
    return ret;
}

/* strtoimax / strtoumax — needed by dash printf/arith */
/* intmax_t is long long (64-bit) on i686 — must return 64-bit value */
long long strtoimax(const char *nptr, char **endptr, int base) {
    return (long long)strtol(nptr, endptr, base);
}

unsigned long long strtoumax(const char *nptr, char **endptr, int base) {
    return (unsigned long long)strtoul(nptr, endptr, base);
}

/* environ — populated from kernel env store at startup */
#define SYS_LISTENV 184
#define MAX_ENV_ENTRIES 64
#define ENV_BUF_SIZE   4096

static char  _env_buf[ENV_BUF_SIZE];
static char *_env_ptrs[MAX_ENV_ENTRIES + 1]; /* +1 for NULL terminator */
char **environ = _env_ptrs; /* initially empty (all-zero → first entry is NULL) */

/* Called from crt0 before main() to populate environ from kernel store.
 * Format of SYS_LISTENV: "KEY=VALUE\0KEY2=VALUE2\0..." */
void __init_environ(void) {
    int total = _syscall(SYS_LISTENV, (int)_env_buf, ENV_BUF_SIZE - 1, 0, 0);
    if (total <= 0) return;
    if (total >= ENV_BUF_SIZE) total = ENV_BUF_SIZE - 1;
    _env_buf[total] = '\0';

    int idx = 0;
    int i = 0;
    while (i < total && idx < MAX_ENV_ENTRIES) {
        if (_env_buf[i] == '\0') { i++; continue; }
        _env_ptrs[idx++] = &_env_buf[i];
        /* Advance past this entry */
        while (i < total && _env_buf[i] != '\0') i++;
        i++; /* skip the null terminator */
    }
    _env_ptrs[idx] = (char *)0;
}

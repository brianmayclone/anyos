/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 POSIX stubs and utility functions.
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

/* Weak symbols: libiberty (GCC) provides its own getopt/fnmatch.
   Marking ours weak lets libiberty's versions win at link time while
   keeping these available for programs that don't link libiberty. */
__attribute__((weak)) char *optarg = NULL;
__attribute__((weak)) int optind = 1;
__attribute__((weak)) int opterr = 1;
__attribute__((weak)) int optopt = '?';
static int _optpos = 0;

__attribute__((weak))
int getopt(int argc, char * const argv[], const char *optstring) {
    if (optind >= argc || !argv[optind]) return -1;
    const char *arg = argv[optind];

    if (_optpos == 0) {
        if (arg[0] != '-' || arg[1] == '\0') return -1;
        if (arg[1] == '-' && arg[2] == '\0') { optind++; return -1; }
    }

    int pos = _optpos ? _optpos : 1;
    int c = arg[pos];
    if (c == '\0') {
        optind++;
        _optpos = 0;
        return getopt(argc, argv, optstring);
    }

    int quiet = (optstring[0] == ':');
    const char *os = optstring;
    if (*os == ':' || *os == '+' || *os == '-') os++;

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
        if (arg[pos + 1]) {
            optarg = (char *)&arg[pos + 1];
            optind++;
            _optpos = 0;
        } else if (match[2] == ':') {
            optarg = NULL;
            optind++;
            _optpos = 0;
        } else if (optind + 1 < argc) {
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
        optarg = NULL;
        if (arg[pos + 1]) _optpos = pos + 1;
        else { optind++; _optpos = 0; }
    }
    return c;
}

__attribute__((weak))
int getopt_long(int argc, char * const argv[], const char *optstring,
                const struct option *longopts, int *longindex) {
    if (optind >= argc || !argv[optind]) return -1;
    const char *arg = argv[optind];

    if (arg[0] == '-' && arg[1] == '-' && arg[2] != '\0' && _optpos == 0) {
        const char *name = arg + 2;
        const char *eq = NULL;
        int namelen = 0;
        for (const char *p = name; *p; p++) {
            if (*p == '=') { eq = p; break; }
            namelen++;
        }
        if (!eq) namelen = (int)strlen(name);

        int match_idx = -1, match_count = 0;
        for (int i = 0; longopts && longopts[i].name; i++) {
            if (strncmp(longopts[i].name, name, namelen) == 0) {
                if ((int)strlen(longopts[i].name) == namelen) {
                    match_idx = i; match_count = 1; break;
                }
                match_idx = i; match_count++;
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
                if (opterr) fprintf(stderr, "%s: option '--%s' doesn't allow an argument\n", argv[0], o->name);
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
                if (opterr) fprintf(stderr, "%s: option '--%s' requires an argument\n", argv[0], o->name);
                optind++;
                return (optstring[0] == ':') ? ':' : '?';
            }
        } else {
            optarg = eq ? (char *)(eq + 1) : NULL;
        }
        optind++;
        if (o->flag) { *o->flag = o->val; return 0; }
        return o->val;
    }
    return getopt(argc, argv, optstring);
}

/* ── dirent — real implementations using SYS_READDIR ── */
#include <dirent.h>
#include <stdlib.h>

#include <sys/syscall.h>

extern long _syscall(long num, long a1, long a2, long a3, long a4, long a5);

#define KDIR_ENTRY_SIZE 64
#define KDIR_MAX_ENTRIES 128
#define KDIR_BUF_SIZE (KDIR_ENTRY_SIZE * KDIR_MAX_ENTRIES)

typedef struct {
    char path[256];
    unsigned char buf[KDIR_BUF_SIZE];
    int count;
    int pos;
} DIR_INTERNAL;

static struct dirent _de;

DIR *opendir(const char *name) {
    if (!name) { errno = EINVAL; return NULL; }
    DIR_INTERNAL *d = (DIR_INTERNAL *)malloc(sizeof(DIR_INTERNAL));
    if (!d) { errno = ENOMEM; return NULL; }
    size_t len = strlen(name);
    if (len >= sizeof(d->path)) { free(d); errno = ENAMETOOLONG; return NULL; }
    memcpy(d->path, name, len + 1);
    long n = _syscall(SYS_READDIR, (long)d->path, (long)d->buf, KDIR_BUF_SIZE, 0, 0);
    if (n < 0) { free(d); errno = (int)-n; return NULL; }
    d->count = (int)n;
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
    if (errbuf && errbuf_size > 0) errbuf[0] = '\0';
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
    strcpy(buf->machine, "x86_64");
    return 0;
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
    if (!name || !*name || strchr(name, '=')) { errno = EINVAL; return -1; }
    if (!overwrite) {
        char tmp[4];
        long r = _syscall(183, (long)name, (long)tmp, sizeof(tmp), 0, 0);
        if (r != -1L && r != (long)0xFFFFFFFF) return 0;
    }
    size_t nlen = strlen(name);
    size_t vlen = value ? strlen(value) : 0;
    char buf[512];
    if (nlen + 1 + vlen >= sizeof(buf)) { errno = ENOMEM; return -1; }
    memcpy(buf, name, nlen);
    buf[nlen] = '=';
    if (value) memcpy(buf + nlen + 1, value, vlen);
    buf[nlen + 1 + vlen] = '\0';
    _syscall(SYS_SETENV, (long)buf, 0, 0, 0, 0);
    return 0;
}

int unsetenv(const char *name) {
    if (!name || !*name || strchr(name, '=')) { errno = EINVAL; return -1; }
    char buf[256];
    size_t nlen = strlen(name);
    if (nlen + 2 >= sizeof(buf)) return -1;
    memcpy(buf, name, nlen);
    buf[nlen] = '=';
    buf[nlen + 1] = '\0';
    _syscall(SYS_SETENV, (long)buf, 0, 0, 0, 0);
    return 0;
}

int mkstemp(char *tmpl) {
    if (!tmpl) { errno = EINVAL; return -1; }
    size_t len = strlen(tmpl);
    if (len < 6) { errno = EINVAL; return -1; }
    char *suffix = tmpl + len - 6;
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

/* ── nanosleep ── */
#include <time.h>

int nanosleep(const struct timespec *req, struct timespec *rem) {
    if (!req) { errno = EINVAL; return -1; }
    unsigned long ms = (unsigned long)(req->tv_sec * 1000 + req->tv_nsec / 1000000);
    if (ms > 0) _syscall(8 /*SYS_SLEEP*/, (long)ms, 0, 0, 0, 0);
    if (rem) { rem->tv_sec = 0; rem->tv_nsec = 0; }
    return 0;
}

/* ── stdio: setbuf / setlinebuf ── */

void setbuf(FILE *stream, char *buf) {
    (void)stream; (void)buf;
}

void setlinebuf(FILE *stream) {
    (void)stream;
}

/* ── POSIX filesystem stubs ── */
#include <sys/stat.h>
#include <dirent.h>

int dirfd(DIR *dirp) {
    if (!dirp) { errno = EINVAL; return -1; }
    return dirp->__fd;
}

int fstatat(int dirfd, const char *pathname, struct stat *statbuf, int flags) {
    (void)dirfd; (void)flags;
    return stat(pathname, statbuf);
}

int unlinkat(int dirfd, const char *pathname, int flags) {
    (void)dirfd; (void)flags;
    return unlink(pathname);
}

int rmdir(const char *pathname) {
    if (!pathname) { errno = EINVAL; return -1; }
    long r = _syscall(91 /*SYS_UNLINK*/, (long)pathname, 0, 0, 0, 0);
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}

/* ── posix_spawn ── */
#include <spawn.h>

int posix_spawn(pid_t *pid, const char *path,
    const posix_spawn_file_actions_t *file_actions,
    const posix_spawnattr_t *attrp,
    char *const argv[], char *const envp[]) {
    (void)file_actions; (void)attrp; (void)envp;
    if (!path) { errno = EINVAL; return EINVAL; }
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
    long tid = _syscall(SYS_SPAWN, (long)path, 0, (long)args, 0, 0);
    if (tid < 0) { errno = ENOENT; return ENOENT; }
    if (pid) *pid = (pid_t)tid;
    return 0;
}

int posix_spawnp(pid_t *pid, const char *file,
    const posix_spawn_file_actions_t *file_actions,
    const posix_spawnattr_t *attrp,
    char *const argv[], char *const envp[]) {
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

/* ── POSIX stubs ── */

int fsync(int fd) { (void)fd; return 0; }
int fdatasync(int fd) { (void)fd; return 0; }

int chmod(const char *path, unsigned int mode) {
    if (!path) { errno = EINVAL; return -1; }
    long r = _syscall(224 /*SYS_CHMOD*/, (long)path, (long)mode, 0, 0, 0);
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}

int fchmod(int fd, unsigned int mode) { (void)fd; (void)mode; return 0; }

unsigned int getuid(void)  { return (unsigned int)_syscall(221, 0, 0, 0, 0, 0); }
unsigned int getgid(void)  { return (unsigned int)_syscall(222, 0, 0, 0, 0, 0); }
unsigned int umask(unsigned int mask) { (void)mask; return 022; }

int link(const char *oldpath, const char *newpath) {
    (void)oldpath; (void)newpath;
    errno = ENOSYS;
    return -1;
}

int symlink(const char *target, const char *linkpath) {
    if (!target || !linkpath) { errno = EINVAL; return -1; }
    long r = _syscall(96 /*SYS_SYMLINK*/, (long)target, (long)linkpath, 0, 0, 0);
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}

ssize_t readlink(const char *path, char *buf, size_t bufsiz) {
    if (!path || !buf) { errno = EINVAL; return -1; }
    long r = _syscall(97 /*SYS_READLINK*/, (long)path, (long)buf, (long)bufsiz, 0, 0);
    if (r < 0) { errno = (int)-r; return -1; }
    return (ssize_t)r;
}

int chown(const char *path, unsigned int owner, unsigned int group) {
    if (!path) { errno = EINVAL; return -1; }
    long r = _syscall(225 /*SYS_CHOWN*/, (long)path, (long)owner, (long)group, 0, 0);
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}

long sysconf(int name) {
    switch (name) {
        case 2:  return 256;    /* _SC_OPEN_MAX */
        case 3:  return 100;    /* _SC_CLK_TCK */
        case 11: return 4096;   /* _SC_PAGE_SIZE (alias) */
        case 28: return 4;      /* _SC_NPROCESSORS_CONF */
        case 29: return 4;      /* _SC_NPROCESSORS_ONLN */
        case 30: return 4096;   /* _SC_PAGESIZE */
        case 84: return 256;    /* _SC_LINE_MAX */
        default: return -1;
    }
}

int getpid(void) { return (int)_syscall(6, 0, 0, 0, 0, 0); }
int getppid(void) { return (int)_syscall(247, 0, 0, 0, 0, 0); }
int getpgid(int pid) { (void)pid; return getpid(); }
int setpgid(int pid, int pgid) { (void)pid; (void)pgid; return 0; }
int setpgrp(void) { return 0; }
int getpgrp(void) { return getpid(); }
unsigned int geteuid(void) { return (unsigned int)_syscall(221, 0, 0, 0, 0, 0); }
unsigned int getegid(void) { return (unsigned int)_syscall(222, 0, 0, 0, 0, 0); }
int getsid(int pid) { (void)pid; return getpid(); }
int setsid(void) { return getpid(); }
unsigned int alarm(unsigned int seconds) { (void)seconds; return 0; }

int execve(const char *path, char *const argv[], char *const envp[]) {
    (void)envp;
    return execv(path, argv);
}

/* Resource limits — stubs */
#include <sys/resource.h>
int getrlimit(int resource, struct rlimit *rlim) {
    (void)resource;
    if (rlim) { rlim->rlim_cur = ~0UL; rlim->rlim_max = ~0UL; }
    return 0;
}
int setrlimit(int resource, const struct rlimit *rlim) {
    (void)resource; (void)rlim;
    return 0;
}

/* Terminal control — stubs */
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
    return 0;
}

/* pwd.h stubs */
#include <pwd.h>

/* Shared static storage for getpwuid() / getpwnam(). */
static char _pw_name_buf[64];
static char _pw_dir_buf[128];
static struct passwd _pw_entry;

/* Populate _pw_entry for the given uid using kernel syscalls. */
static struct passwd *_pw_fill(uid_t uid) {
    /* Look up username by UID (SYS_GETUSERNAME = 232). */
    _pw_name_buf[0] = '\0';
    _syscall(232, (long)(unsigned int)uid, (long)_pw_name_buf,
             (long)sizeof(_pw_name_buf), 0, 0);
    if (_pw_name_buf[0] == '\0')
        snprintf(_pw_name_buf, sizeof(_pw_name_buf), "user%u", (unsigned int)uid);

    /* Home directory: /root for root, /home/<name> for others. */
    if (uid == 0)
        snprintf(_pw_dir_buf, sizeof(_pw_dir_buf), "/root");
    else
        snprintf(_pw_dir_buf, sizeof(_pw_dir_buf), "/home/%s", _pw_name_buf);

    _pw_entry.pw_name  = _pw_name_buf;
    _pw_entry.pw_dir   = _pw_dir_buf;
    _pw_entry.pw_shell = "/bin/sh";
    _pw_entry.pw_uid   = uid;
    /* Use caller's GID when looking up the current user, else fall back to uid. */
    _pw_entry.pw_gid   = ((unsigned int)_syscall(221, 0, 0, 0, 0, 0) == (unsigned int)uid)
                         ? (gid_t)_syscall(222, 0, 0, 0, 0, 0)
                         : (gid_t)uid;
    return &_pw_entry;
}

struct passwd *getpwuid(uid_t uid) {
    return _pw_fill(uid);
}

struct passwd *getpwnam(const char *name) {
    /* Look up by current user's name; fall back to current user for all queries. */
    (void)name;
    return _pw_fill((uid_t)_syscall(221, 0, 0, 0, 0, 0));
}

int getpwuid_r(uid_t uid, struct passwd *pwd, char *buf, size_t buflen,
               struct passwd **result) {
    if (!pwd || !buf || buflen < 128) {
        if (result) *result = NULL;
        return ERANGE;
    }
    /* Layout: [0..63] = name, [64..127+] = home dir */
    char *name = buf;
    name[0] = '\0';
    _syscall(232, (long)(unsigned int)uid, (long)name, 63, 0, 0);
    if (name[0] == '\0')
        snprintf(name, 64, "user%u", (unsigned int)uid);

    char *dir = buf + 64;
    if (uid == 0)
        snprintf(dir, buflen - 64, "/root");
    else
        snprintf(dir, buflen - 64, "/home/%s", name);

    pwd->pw_name  = name;
    pwd->pw_dir   = dir;
    pwd->pw_shell = "/bin/sh";
    pwd->pw_uid   = uid;
    pwd->pw_gid   = ((unsigned int)_syscall(221, 0, 0, 0, 0, 0) == (unsigned int)uid)
                    ? (gid_t)_syscall(222, 0, 0, 0, 0, 0)
                    : (gid_t)uid;
    if (result) *result = pwd;
    return 0;
}

/* wait3 — wrapper around waitpid */
int wait3(int *status, int options, void *rusage) {
    (void)rusage;
    return waitpid(-1, status, options);
}

/* times — stub */
long times(void *buf) {
    if (buf) memset(buf, 0, 32); /* sizeof(struct tms) = 4 * sizeof(clock_t) */
    return 0;
}

/* strtoimax / strtoumax */
long long strtoimax(const char *nptr, char **endptr, int base) {
    return (long long)strtol(nptr, endptr, base);
}

unsigned long long strtoumax(const char *nptr, char **endptr, int base) {
    return (unsigned long long)strtoul(nptr, endptr, base);
}

/* environ — populated from kernel env store at startup */
#define MAX_ENV_ENTRIES 64
#define ENV_BUF_SIZE   4096

static char  _env_buf[ENV_BUF_SIZE];
static char *_env_ptrs[MAX_ENV_ENTRIES + 1];
char **environ = _env_ptrs;

void __init_environ(void) {
    long total = _syscall(SYS_LISTENV, (long)_env_buf, ENV_BUF_SIZE - 1, 0, 0, 0);
    if (total <= 0) return;
    if (total >= ENV_BUF_SIZE) total = ENV_BUF_SIZE - 1;
    _env_buf[total] = '\0';

    int idx = 0;
    long i = 0;
    while (i < total && idx < MAX_ENV_ENTRIES) {
        if (_env_buf[i] == '\0') { i++; continue; }
        _env_ptrs[idx++] = &_env_buf[i];
        while (i < total && _env_buf[i] != '\0') i++;
        i++;
    }
    _env_ptrs[idx] = (char *)0;
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

/* ── grp.h — group database (minimal) ── */
#include <grp.h>

static char  _gr_name_buf[64];
static char *_gr_mem_empty[] = { NULL };
static struct group _gr_entry;

struct group *getgrgid(gid_t gid) {
    snprintf(_gr_name_buf, sizeof(_gr_name_buf), "group%u", (unsigned int)gid);
    _gr_entry.gr_name   = _gr_name_buf;
    _gr_entry.gr_passwd = "";
    _gr_entry.gr_gid    = gid;
    _gr_entry.gr_mem    = _gr_mem_empty;
    return &_gr_entry;
}

struct group *getgrnam(const char *name) {
    if (!name) return NULL;
    size_t len = strlen(name);
    if (len >= sizeof(_gr_name_buf)) len = sizeof(_gr_name_buf) - 1;
    memcpy(_gr_name_buf, name, len);
    _gr_name_buf[len] = '\0';
    _gr_entry.gr_name   = _gr_name_buf;
    _gr_entry.gr_passwd = "";
    _gr_entry.gr_gid    = (gid_t)_syscall(222, 0, 0, 0, 0, 0);
    _gr_entry.gr_mem    = _gr_mem_empty;
    return &_gr_entry;
}

void setgrent(void) {}
void endgrent(void) {}
struct group *getgrent(void) { return NULL; }

/* ── readv / writev — scatter/gather I/O ── */
struct iovec {
    void  *iov_base;
    size_t iov_len;
};

ssize_t readv(int fd, const struct iovec *iov, int iovcnt) {
    ssize_t total = 0;
    for (int i = 0; i < iovcnt; i++) {
        ssize_t r = read(fd, iov[i].iov_base, iov[i].iov_len);
        if (r < 0) return total > 0 ? total : r;
        total += r;
        if ((size_t)r < iov[i].iov_len) break;
    }
    return total;
}

ssize_t writev(int fd, const struct iovec *iov, int iovcnt) {
    ssize_t total = 0;
    for (int i = 0; i < iovcnt; i++) {
        ssize_t r = write(fd, iov[i].iov_base, iov[i].iov_len);
        if (r < 0) return total > 0 ? total : r;
        total += r;
        if ((size_t)r < iov[i].iov_len) break;
    }
    return total;
}

/* ── mkdtemp — create a uniquely-named temporary directory ── */
#include <sys/stat.h>
#include <stdlib.h>

char *mkdtemp(char *tmpl) {
    if (!tmpl) { errno = EINVAL; return NULL; }
    size_t len = strlen(tmpl);
    if (len < 6) { errno = EINVAL; return NULL; }
    char *suffix = tmpl + len - 6;
    for (int i = 0; i < 6; i++) {
        if (suffix[i] != 'X') { errno = EINVAL; return NULL; }
    }
    static unsigned int _mkdtemp_counter = 0;
    for (int tries = 0; tries < 100; tries++) {
        unsigned int v = (unsigned int)rand() ^ (++_mkdtemp_counter * 6271);
        for (int i = 0; i < 6; i++) {
            int r = (v >> (i * 5)) % 36;
            suffix[i] = (char)(r < 10 ? '0' + r : 'a' + r - 10);
        }
        if (mkdir(tmpl, 0700) == 0) return tmpl;
        if (errno != EEXIST) return NULL;
    }
    errno = EEXIST;
    return NULL;
}

/* ── tmpnam — generate a unique temporary filename ── */
char *tmpnam(char *s) {
    static char _tmpnam_buf[L_tmpnam + 1];
    static unsigned int _tmpnam_counter = 0;
    char *buf = s ? s : _tmpnam_buf;
    unsigned int v = (unsigned int)rand() ^ (++_tmpnam_counter * 5381);
    snprintf(buf, L_tmpnam, "/tmp/t%06x", v & 0xFFFFFF);
    return buf;
}

/* ── fnmatch — shell-style filename pattern matching ── */
__attribute__((weak))
int fnmatch(const char *pattern, const char *string, int flags) {
    (void)flags;
    const char *p = pattern, *s = string;
    const char *star_p = NULL, *star_s = NULL;
    while (*s) {
        if (*p == '*') {
            star_p = ++p;
            star_s = s;
            continue;
        }
        if (*p == '?' || *p == *s) {
            p++;
            s++;
            continue;
        }
        if (star_p) {
            p = star_p;
            s = ++star_s;
            continue;
        }
        return 1; /* FNM_NOMATCH */
    }
    while (*p == '*') p++;
    return *p ? 1 : 0;
}

/* ── pathconf — get configurable pathname limits ── */
long pathconf(const char *path, int name) {
    (void)path;
    switch (name) {
        case 1: return 255;     /* _PC_NAME_MAX */
        case 2: return 4096;    /* _PC_PATH_MAX */
        case 5: return 1;       /* _PC_LINK_MAX */
        case 6: return 512;     /* _PC_PIPE_BUF */
        default: return -1;
    }
}

long fpathconf(int fd, int name) {
    (void)fd;
    return pathconf(NULL, name);
}

/* ── putenv ── */
int putenv(char *string) {
    if (!string) { errno = EINVAL; return -1; }
    char *eq = strchr(string, '=');
    if (!eq) { errno = EINVAL; return -1; }
    /* Temporarily NUL-terminate to get the name */
    *eq = '\0';
    int r = setenv(string, eq + 1, 1);
    *eq = '=';
    return r;
}

/* ── clearenv ── */
int clearenv(void) { return 0; }

/* ── confstr ── */
size_t confstr(int name, char *buf, size_t len) {
    (void)name;
    if (buf && len > 0) buf[0] = '\0';
    return 0;
}

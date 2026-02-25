/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 POSIX system call wrappers.
 */

#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <string.h>

extern long _syscall(long num, long a1, long a2, long a3, long a4, long a5);

#define SYS_EXIT    1
#define SYS_WRITE   2
#define SYS_READ    3
#define SYS_OPEN    4
#define SYS_CLOSE   5
#define SYS_SLEEP   8
#define SYS_SBRK    9
#define SYS_FORK    10
#define SYS_EXEC    11
#define SYS_WAITPID 12
#define SYS_KILL    13
#define SYS_GETCWD  25
#define SYS_CHDIR   26
#define SYS_UNLINK  91
#define SYS_LSEEK   105
#define SYS_FSTAT   106
#define SYS_FTRUNCATE 107
#define SYS_ISATTY  108
#define SYS_PIPE2   240
#define SYS_DUP     241
#define SYS_DUP2    242
#define SYS_FCNTL   243

/* Socket fd base — socket fds start at 128 */
#define SOCKET_FD_BASE 128

extern ssize_t recv(int sockfd, void *buf, size_t len, int flags);
extern ssize_t send(int sockfd, const void *buf, size_t len, int flags);
extern int __socket_close(int sockfd);

ssize_t read(int fd, void *buf, size_t count) {
    if (fd >= SOCKET_FD_BASE) return recv(fd, buf, count, 0);
    long ret = _syscall(SYS_READ, fd, (long)buf, (long)count, 0, 0);
    if (ret < 0) { errno = (int)-ret; return -1; }
    if (fd == 0 && ret == 0 && count > 0) {
        for (;;) {
            _syscall(SYS_SLEEP, 10, 0, 0, 0, 0);
            ret = _syscall(SYS_READ, 0, (long)buf, (long)count, 0, 0);
            if (ret < 0) { errno = (int)-ret; return -1; }
            if (ret > 0) return (ssize_t)ret;
        }
    }
    return (ssize_t)ret;
}

ssize_t write(int fd, const void *buf, size_t count) {
    if (fd >= SOCKET_FD_BASE) return send(fd, buf, count, 0);
    long ret = _syscall(SYS_WRITE, fd, (long)buf, (long)count, 0, 0);
    if (ret < 0) { errno = (int)-ret; return -1; }
    return (ssize_t)ret;
}

int open(const char *path, int flags, ...) {
    int anyos_flags = 0;
    if (flags & O_WRONLY) anyos_flags |= 1;
    if (flags & O_RDWR)  anyos_flags |= 1;
    if (flags & O_APPEND) anyos_flags |= 2;
    if (flags & O_CREAT)  anyos_flags |= 4;
    if (flags & O_TRUNC)  anyos_flags |= 8;
    long ret = _syscall(SYS_OPEN, (long)path, anyos_flags, 0, 0, 0);
    if (ret < 0) { errno = (int)-ret; return -1; }
    return (int)ret;
}

int close(int fd) {
    if (fd >= SOCKET_FD_BASE) return __socket_close(fd);
    long ret = _syscall(SYS_CLOSE, fd, 0, 0, 0, 0);
    if (ret < 0) { errno = (int)-ret; return -1; }
    return 0;
}

off_t lseek(int fd, off_t offset, int whence) {
    long ret = _syscall(SYS_LSEEK, fd, offset, whence, 0, 0);
    if (ret < 0) { errno = (int)-ret; return -1; }
    return (off_t)ret;
}

int isatty(int fd) {
    return (int)_syscall(SYS_ISATTY, fd, 0, 0, 0, 0);
}

char *getcwd(char *buf, size_t size) {
    long ret = _syscall(SYS_GETCWD, (long)buf, (long)size, 0, 0, 0);
    if (ret < 0) { errno = (int)-ret; return NULL; }
    return buf;
}

int chdir(const char *path) {
    long ret = _syscall(SYS_CHDIR, (long)path, 0, 0, 0, 0);
    if (ret < 0) { errno = (int)-ret; return -1; }
    return 0;
}

void _exit(int status) {
    _syscall(SYS_EXIT, status, 0, 0, 0, 0);
    __builtin_unreachable();
}

void *sbrk(long increment) {
    long ret = _syscall(SYS_SBRK, increment, 0, 0, 0, 0);
    if (ret == -1) { errno = ENOMEM; return (void *)-1; }
    return (void *)ret;
}

int unlink(const char *path) {
    long ret = _syscall(SYS_UNLINK, (long)path, 0, 0, 0, 0);
    if (ret < 0) { errno = (int)-ret; return -1; }
    return 0;
}

int access(const char *path, int mode) {
    (void)mode;
    int fd = open(path, O_RDONLY);
    if (fd < 0) return -1;
    close(fd);
    return 0;
}

pid_t fork(void) {
    long r = _syscall(SYS_FORK, 0, 0, 0, 0, 0);
    if (r == (long)0xFFFFFFFF || r < 0) { errno = EAGAIN; return -1; }
    return (pid_t)r;
}

pid_t waitpid(pid_t pid, int *status, int options) {
    unsigned int child_tid = 0;
    long r = _syscall(SYS_WAITPID, pid, (long)&child_tid, options, 0, 0);
    if (r == (long)0xFFFFFFFF || r < 0) { errno = ECHILD; return -1; }
    if (status) *status = (int)r;
    if (pid == -1 && child_tid != 0) return (pid_t)child_tid;
    return pid;
}

static int _build_args(char *const argv[], char *buf, int bufsize) {
    int pos = 0;
    for (int i = 0; argv[i] != (void*)0; i++) {
        int len = 0;
        while (argv[i][len]) len++;
        if (pos + len + 1 >= bufsize) break;
        if (pos > 0) buf[pos++] = ' ';
        for (int j = 0; j < len; j++) buf[pos++] = argv[i][j];
    }
    buf[pos] = '\0';
    return pos;
}

int execv(const char *path, char *const argv[]) {
    char args[512];
    _build_args(argv, args, sizeof(args));
    _syscall(SYS_EXEC, (long)path, (long)args, 0, 0, 0);
    errno = ENOENT;
    return -1;
}

int execvp(const char *file, char *const argv[]) {
    if (execv(file, argv) == 0) return 0;
    if (file[0] != '/') {
        char path[256];
        int pos = 0;
        const char *prefix = "/bin/";
        while (*prefix) path[pos++] = *prefix++;
        int i = 0;
        while (file[i] && pos < 255) path[pos++] = file[i++];
        path[pos] = '\0';
        return execv(path, argv);
    }
    errno = ENOENT;
    return -1;
}

int ftruncate(int fd, off_t length) {
    long r = _syscall(SYS_FTRUNCATE, fd, (long)length, 0, 0, 0);
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}

ssize_t pread(int fd, void *buf, size_t count, off_t offset) {
    off_t saved = lseek(fd, 0, SEEK_CUR);
    if (saved < 0) return -1;
    if (lseek(fd, offset, SEEK_SET) < 0) return -1;
    ssize_t n = read(fd, buf, count);
    lseek(fd, saved, SEEK_SET);
    return n;
}

ssize_t pwrite(int fd, const void *buf, size_t count, off_t offset) {
    off_t saved = lseek(fd, 0, SEEK_CUR);
    if (saved < 0) return -1;
    if (lseek(fd, offset, SEEK_SET) < 0) return -1;
    ssize_t n = write(fd, buf, count);
    lseek(fd, saved, SEEK_SET);
    return n;
}

int dup(int oldfd) {
    long r = _syscall(SYS_DUP, oldfd, 0, 0, 0, 0);
    if (r < 0 || r == (long)0xFFFFFFFF) { errno = EBADF; return -1; }
    return (int)r;
}

int dup2(int oldfd, int newfd) {
    long r = _syscall(SYS_DUP2, oldfd, newfd, 0, 0, 0);
    if (r < 0 || r == (long)0xFFFFFFFF) { errno = EBADF; return -1; }
    return (int)r;
}

int gethostname(char *name, size_t len) {
    const char *hostname = "anyos";
    if (len < 6) { errno = ENAMETOOLONG; return -1; }
    memcpy(name, hostname, 6);
    return 0;
}

int ioctl(int fd, unsigned long request, ...) {
    (void)fd; (void)request;
    errno = ENOSYS;
    return -1;
}

int fcntl(int fd, int cmd, ...) {
    int arg = 0;
    if (cmd == 0 || cmd == 1030 || cmd == 2 || cmd == 4) {
        __builtin_va_list ap;
        __builtin_va_start(ap, cmd);
        arg = __builtin_va_arg(ap, int);
        __builtin_va_end(ap);
    }
    long r = _syscall(SYS_FCNTL, fd, cmd, arg, 0, 0);
    if (r < 0 || r == (long)0xFFFFFFFF) { errno = EBADF; return -1; }
    return (int)r;
}

int pipe(int pipefd[2]) {
    long r = _syscall(SYS_PIPE2, (long)pipefd, 0, 0, 0, 0);
    if (r < 0 || r == (long)0xFFFFFFFF) { errno = EMFILE; return -1; }
    return 0;
}

unsigned int sleep(unsigned int seconds) {
    _syscall(SYS_SLEEP, (long)(seconds * 1000), 0, 0, 0, 0);
    return 0;
}

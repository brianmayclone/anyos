/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#include <unistd.h>
#include <fcntl.h>
#include <errno.h>

extern int _syscall(int num, int a1, int a2, int a3, int a4);

#define SYS_EXIT    1
#define SYS_WRITE   2
#define SYS_READ    3
#define SYS_OPEN    4
#define SYS_CLOSE   5
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
#define SYS_ISATTY  108

/* Socket fd base — socket fds start at 128 */
#define SOCKET_FD_BASE 128

/* Defined in socket.c — handles socket fd I/O */
extern ssize_t recv(int sockfd, void *buf, size_t len, int flags);
extern ssize_t send(int sockfd, const void *buf, size_t len, int flags);

ssize_t read(int fd, void *buf, size_t count) {
    if (fd >= SOCKET_FD_BASE) {
        return recv(fd, buf, count, 0);
    }
    int ret = _syscall(SYS_READ, fd, (int)buf, (int)count, 0);
    if (ret < 0) { errno = -ret; return -1; }
    return ret;
}

ssize_t write(int fd, const void *buf, size_t count) {
    if (fd >= SOCKET_FD_BASE) {
        return send(fd, buf, count, 0);
    }
    int ret = _syscall(SYS_WRITE, fd, (int)buf, (int)count, 0);
    if (ret < 0) { errno = -ret; return -1; }
    return ret;
}

int open(const char *path, int flags, ...) {
    /* Map POSIX flags to anyOS flags */
    int anyos_flags = 0;
    if (flags & O_WRONLY) anyos_flags |= 1;  /* O_WRITE */
    if (flags & O_RDWR)  anyos_flags |= 1;   /* O_WRITE */
    if (flags & O_APPEND) anyos_flags |= 2;  /* O_APPEND */
    if (flags & O_CREAT)  anyos_flags |= 4;  /* O_CREATE */
    if (flags & O_TRUNC)  anyos_flags |= 8;  /* O_TRUNC */

    int ret = _syscall(SYS_OPEN, (int)path, anyos_flags, 0, 0);
    if (ret < 0) { errno = -ret; return -1; }
    return ret;
}

/* Defined in socket.c — handles socket fd cleanup */
extern int __socket_close(int sockfd);

int close(int fd) {
    /* Route socket fds to socket layer */
    if (fd >= SOCKET_FD_BASE) {
        return __socket_close(fd);
    }
    int ret = _syscall(SYS_CLOSE, fd, 0, 0, 0);
    if (ret < 0) { errno = -ret; return -1; }
    return ret;
}

int lseek(int fd, int offset, int whence) {
    int ret = _syscall(SYS_LSEEK, fd, offset, whence, 0);
    if (ret < 0) { errno = -ret; return -1; }
    return ret;
}

int isatty(int fd) {
    return _syscall(SYS_ISATTY, fd, 0, 0, 0);
}

char *getcwd(char *buf, size_t size) {
    int ret = _syscall(SYS_GETCWD, (int)buf, (int)size, 0, 0);
    if (ret < 0) { errno = -ret; return NULL; }
    return buf;
}

int chdir(const char *path) {
    int ret = _syscall(SYS_CHDIR, (int)path, 0, 0, 0);
    if (ret < 0) { errno = -ret; return -1; }
    return 0;
}

void _exit(int status) {
    _syscall(SYS_EXIT, status, 0, 0, 0);
    __builtin_unreachable();
}

void *sbrk(int increment) {
    int ret = _syscall(SYS_SBRK, increment, 0, 0, 0);
    if (ret == -1) { errno = ENOMEM; return (void *)-1; }
    return (void *)ret;
}

int unlink(const char *path) {
    int ret = _syscall(SYS_UNLINK, (int)path, 0, 0, 0);
    if (ret < 0) { errno = -ret; return -1; }
    return 0;
}

int access(const char *path, int mode) {
    (void)mode;
    /* Check if file exists by trying to open it */
    int fd = open(path, O_RDONLY);
    if (fd < 0) return -1; /* errno already set by open() */
    close(fd);
    return 0;
}

pid_t fork(void) {
    int r = _syscall(SYS_FORK, 0, 0, 0, 0);
    if (r == (int)0xFFFFFFFF) {
        errno = EAGAIN;
        return -1;
    }
    return (pid_t)r;
}

pid_t waitpid(pid_t pid, int *status, int options) {
    (void)options;
    int r = _syscall(SYS_WAITPID, pid, 0, 0, 0);
    if (r == (int)0xFFFFFFFF) {
        errno = ECHILD;
        return -1;
    }
    if (status) *status = r;
    return pid;
}

/* Build a single space-separated args string from argv[] for SYS_EXEC. */
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
    int r = _syscall(SYS_EXEC, (int)path, (int)args, 0, 0);
    /* exec only returns on error */
    (void)r;
    errno = ENOENT;
    return -1;
}

int execvp(const char *file, char *const argv[]) {
    /* Try exact path first */
    if (execv(file, argv) == 0) return 0;  /* never reached on success */

    /* If not absolute, try /bin/<file> */
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

#define SYS_FTRUNCATE_UNISTD 107

int ftruncate(int fd, unsigned int length) {
    int r = _syscall(SYS_FTRUNCATE_UNISTD, fd, (int)length, 0, 0);
    if (r < 0) { errno = -r; return -1; }
    return 0;
}

ssize_t pread(int fd, void *buf, size_t count, long offset) {
    int saved = lseek(fd, 0, SEEK_CUR);
    if (saved < 0) return -1;
    if (lseek(fd, offset, SEEK_SET) < 0) return -1;
    ssize_t n = read(fd, buf, count);
    lseek(fd, saved, SEEK_SET);
    return n;
}

ssize_t pwrite(int fd, const void *buf, size_t count, long offset) {
    int saved = lseek(fd, 0, SEEK_CUR);
    if (saved < 0) return -1;
    if (lseek(fd, offset, SEEK_SET) < 0) return -1;
    ssize_t n = write(fd, buf, count);
    lseek(fd, saved, SEEK_SET);
    return n;
}

int dup(int oldfd) {
    (void)oldfd;
    errno = ENOSYS;
    return -1;
}

int dup2(int oldfd, int newfd) {
    (void)oldfd; (void)newfd;
    errno = ENOSYS;
    return -1;
}

int gethostname(char *name, size_t len) {
    const char *hostname = "anyos";
    size_t hlen = 5;
    if (len < hlen + 1) { errno = ENAMETOOLONG; return -1; }
    for (size_t i = 0; i <= hlen; i++) name[i] = hostname[i];
    return 0;
}

int ioctl(int fd, unsigned long request, ...) {
    (void)fd; (void)request;
    errno = ENOSYS;
    return -1;
}

int fcntl(int fd, int cmd, ...) {
    (void)fd;
    if (cmd == 1 /* F_GETFD */) return 0; /* fd exists */
    if (cmd == 2 /* F_SETFD */) return 0; /* pretend success */
    if (cmd == 3 /* F_GETFL */) return 0; /* return current flags = 0 */
    if (cmd == 4 /* F_SETFL */) return 0; /* pretend success */
    errno = ENOSYS;
    return -1;
}

int pipe(int pipefd[2]) {
    /* anyOS has named pipes, not POSIX anonymous pipes.
       Stub: curl uses this only for stderr redirect. */
    (void)pipefd;
    errno = ENOSYS;
    return -1;
}

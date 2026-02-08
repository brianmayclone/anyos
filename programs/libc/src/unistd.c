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
#define SYS_GETCWD  25
#define SYS_UNLINK  91
#define SYS_LSEEK   105
#define SYS_FSTAT   106
#define SYS_ISATTY  108

ssize_t read(int fd, void *buf, size_t count) {
    int ret = _syscall(SYS_READ, fd, (int)buf, (int)count, 0);
    if (ret == -1) { errno = EIO; return -1; }
    return ret;
}

ssize_t write(int fd, const void *buf, size_t count) {
    int ret = _syscall(SYS_WRITE, fd, (int)buf, (int)count, 0);
    if (ret == -1) { errno = EIO; return -1; }
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
    if (ret == -1) { errno = ENOENT; return -1; }
    return ret;
}

int close(int fd) {
    int ret = _syscall(SYS_CLOSE, fd, 0, 0, 0);
    if (ret == -1) { errno = EBADF; return -1; }
    return ret;
}

int lseek(int fd, int offset, int whence) {
    int ret = _syscall(SYS_LSEEK, fd, offset, whence, 0);
    if (ret == -1) { errno = EINVAL; return -1; }
    return ret;
}

int isatty(int fd) {
    return _syscall(SYS_ISATTY, fd, 0, 0, 0);
}

char *getcwd(char *buf, size_t size) {
    int ret = _syscall(SYS_GETCWD, (int)buf, (int)size, 0, 0);
    if (ret == -1) { errno = ERANGE; return NULL; }
    return buf;
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
    if (ret == -1) { errno = ENOENT; return -1; }
    return 0;
}

int access(const char *path, int mode) {
    (void)mode;
    /* Check if file exists by trying to open it */
    int fd = open(path, O_RDONLY);
    if (fd < 0) { errno = ENOENT; return -1; }
    close(fd);
    return 0;
}

int execvp(const char *file, char *const argv[]) {
    (void)file; (void)argv;
    errno = ENOSYS;
    return -1;
}

int ftruncate(int fd, unsigned int length) {
    (void)fd; (void)length;
    errno = ENOSYS;
    return -1;
}

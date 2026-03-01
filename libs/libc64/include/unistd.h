/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _UNISTD_H
#define _UNISTD_H

#include <stddef.h>
#include <sys/types.h>

#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

#define STDIN_FILENO 0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2

#ifdef __cplusplus
extern "C" {
#endif

ssize_t read(int fd, void *buf, size_t count);
ssize_t write(int fd, const void *buf, size_t count);
int close(int fd);
off_t lseek(int fd, off_t offset, int whence);
int isatty(int fd);
char *getcwd(char *buf, size_t size);
int chdir(const char *path);
void _exit(int status);
void *sbrk(long increment);
int unlink(const char *path);
int access(const char *path, int mode);
pid_t fork(void);
pid_t waitpid(pid_t pid, int *status, int options);
int execv(const char *path, char *const argv[]);
int execvp(const char *file, char *const argv[]);
int ftruncate(int fd, off_t length);
ssize_t pread(int fd, void *buf, size_t count, off_t offset);
ssize_t pwrite(int fd, const void *buf, size_t count, off_t offset);
int dup(int oldfd);
int dup2(int oldfd, int newfd);
int pipe(int pipefd[2]);
int gethostname(char *name, size_t len);
char *realpath(const char *path, char *resolved_path);
int rmdir(const char *pathname);
int unlinkat(int dirfd, const char *pathname, int flags);
int fsync(int fd);
int fdatasync(int fd);
int chmod(const char *path, unsigned int mode);
int fchmod(int fd, unsigned int mode);
struct stat;
int lstat(const char *path, struct stat *buf);
unsigned int getuid(void);
unsigned int getgid(void);
unsigned int umask(unsigned int mask);
int link(const char *oldpath, const char *newpath);
int symlink(const char *target, const char *linkpath);
ssize_t readlink(const char *path, char *buf, size_t bufsiz);
int chown(const char *path, unsigned int owner, unsigned int group);
int fchown(int fd, unsigned int owner, unsigned int group);
int lchown(const char *path, unsigned int owner, unsigned int group);
unsigned int sleep(unsigned int seconds);
long sysconf(int name);
pid_t getpid(void);
pid_t getppid(void);
pid_t getpgid(pid_t pid);
int setpgid(pid_t pid, pid_t pgid);
pid_t setpgrp(void);
pid_t getpgrp(void);
unsigned int geteuid(void);
unsigned int getegid(void);
pid_t getsid(pid_t pid);
pid_t setsid(void);
int execve(const char *path, char *const argv[], char *const envp[]);
unsigned int alarm(unsigned int seconds);
pid_t vfork(void);
int faccessat(int dirfd, const char *pathname, int mode, int flags);

long pathconf(const char *path, int name);
long fpathconf(int fd, int name);
size_t confstr(int name, char *buf, size_t len);

#define _SC_CLK_TCK       2
#define _SC_OPEN_MAX      4
#define _SC_PAGESIZE      30
#define _SC_PAGE_SIZE     _SC_PAGESIZE
#define _SC_GETPW_R_SIZE_MAX 70
#define _SC_NPROCESSORS_CONF 28
#define _SC_NPROCESSORS_ONLN 29
#define _SC_LINE_MAX      84
#define _SC_PHYS_PAGES    85

#define _PC_NAME_MAX 1
#define _PC_PATH_MAX 2
#define _PC_LINK_MAX 5
#define _PC_PIPE_BUF 6

#define F_OK 0
#define R_OK 4
#define W_OK 2
#define X_OK 1

int getpagesize(void);
int usleep(unsigned int usec);

#ifdef __cplusplus
}
#endif

#endif

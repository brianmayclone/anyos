/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#ifndef _UNISTD_H
#define _UNISTD_H

#include <stddef.h>

#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

#define STDIN_FILENO 0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2

ssize_t read(int fd, void *buf, size_t count);
ssize_t write(int fd, const void *buf, size_t count);
int close(int fd);
int lseek(int fd, int offset, int whence);
int isatty(int fd);
char *getcwd(char *buf, size_t size);
int chdir(const char *path);
void _exit(int status);
void *sbrk(int increment);
int unlink(const char *path);
int access(const char *path, int mode);
int execvp(const char *file, char *const argv[]);
int ftruncate(int fd, unsigned int length);
ssize_t pread(int fd, void *buf, size_t count, long offset);
ssize_t pwrite(int fd, const void *buf, size_t count, long offset);
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
int readlink(const char *path, char *buf, size_t bufsiz);
int chown(const char *path, unsigned int owner, unsigned int group);
unsigned int sleep(unsigned int seconds);
long sysconf(int name);
int getpid(void);
int getppid(void);
int getpgid(int pid);
unsigned int geteuid(void);
unsigned int getegid(void);
int getsid(int pid);

#define _SC_PAGESIZE 30
#define _SC_PAGE_SIZE _SC_PAGESIZE
#define _SC_GETPW_R_SIZE_MAX 70

#define F_OK 0
#define R_OK 4
#define W_OK 2
#define X_OK 1

typedef int pid_t;

#endif

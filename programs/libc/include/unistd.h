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
void _exit(int status);
void *sbrk(int increment);
int unlink(const char *path);
int access(const char *path, int mode);
int execvp(const char *file, char *const argv[]);
int ftruncate(int fd, unsigned int length);

#define F_OK 0
#define R_OK 4
#define W_OK 2
#define X_OK 1

typedef int pid_t;

#endif

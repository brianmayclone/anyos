#ifndef _SYS_WAIT_H
#define _SYS_WAIT_H

#include <sys/types.h>

/* Wait status macros — anyOS passes raw exit code (no encoding) */
#define WIFEXITED(s)    1
#define WEXITSTATUS(s)  (s)
#define WIFSIGNALED(s)  0
#define WTERMSIG(s)     0
#define WIFSTOPPED(s)   0
#define WSTOPSIG(s)     0
#define WCOREDUMP(s)    0

/* waitpid options */
#define WNOHANG   1
#define WUNTRACED 2

pid_t waitpid(pid_t pid, int *status, int options);
pid_t wait(int *status);
pid_t wait3(int *status, int options, void *rusage);

#endif

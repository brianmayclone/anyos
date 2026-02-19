/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#ifndef _SIGNAL_H
#define _SIGNAL_H

/* POSIX signal numbers (Linux i386 ABI) */
#define SIGHUP    1
#define SIGINT    2
#define SIGQUIT   3
#define SIGILL    4
#define SIGTRAP   5
#define SIGABRT   6
#define SIGBUS    7
#define SIGFPE    8
#define SIGKILL   9
#define SIGUSR1   10
#define SIGSEGV   11
#define SIGUSR2   12
#define SIGPIPE   13
#define SIGALRM   14
#define SIGTERM   15
#define SIGCHLD   17
#define SIGCONT   18
#define SIGSTOP   19
#define SIGTSTP   20
#define SIGTTIN   21
#define SIGTTOU   22

/* Signal handler type */
typedef void (*sighandler_t)(int);
#define SIG_DFL ((sighandler_t)0)
#define SIG_IGN ((sighandler_t)1)
#define SIG_ERR ((sighandler_t)-1)

/* sigprocmask how values */
#define SIG_BLOCK   0
#define SIG_UNBLOCK 1
#define SIG_SETMASK 2

/* Signal set type (bitmask) */
typedef unsigned int sigset_t;

/* Signal functions */
sighandler_t signal(int signum, sighandler_t handler);
int raise(int sig);
int kill(int pid, int sig);
int sigprocmask(int how, const sigset_t *set, sigset_t *oldset);

/* Signal set manipulation macros */
#define sigemptyset(s)    (*(s) = 0, 0)
#define sigfillset(s)     (*(s) = ~0U, 0)
#define sigaddset(s, n)   (*(s) |= (1U << (n)), 0)
#define sigdelset(s, n)   (*(s) &= ~(1U << (n)), 0)
#define sigismember(s, n) ((*(s) >> (n)) & 1)

#endif

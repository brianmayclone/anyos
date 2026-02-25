/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SIGNAL_H
#define _SIGNAL_H

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

#define NSIG      32

typedef volatile int sig_atomic_t;

typedef void (*sighandler_t)(int);
#define SIG_DFL ((sighandler_t)0)
#define SIG_IGN ((sighandler_t)1)
#define SIG_ERR ((sighandler_t)-1)

#define SIG_BLOCK   0
#define SIG_UNBLOCK 1
#define SIG_SETMASK 2

typedef unsigned long sigset_t;

struct sigaction {
    union {
        sighandler_t sa_handler;
        void (*sa_sigaction)(int, void *, void *);
    };
    sigset_t sa_mask;
    int      sa_flags;
};

#define SA_RESTART  0x10000000
#define SA_NODEFER  0x40000000
#define SA_RESETHAND 0x80000000

sighandler_t signal(int signum, sighandler_t handler);
int raise(int sig);
int kill(int pid, int sig);
int sigprocmask(int how, const sigset_t *set, sigset_t *oldset);
int sigaction(int signum, const struct sigaction *act, struct sigaction *oldact);
int sigsuspend(const sigset_t *mask);
int sigpending(sigset_t *set);
int siginterrupt(int sig, int flag);

#define sigemptyset(s)    (*(s) = 0, 0)
#define sigfillset(s)     (*(s) = ~0UL, 0)
#define sigaddset(s, n)   (*(s) |= (1UL << (n)), 0)
#define sigdelset(s, n)   (*(s) &= ~(1UL << (n)), 0)
#define sigismember(s, n) ((*(s) >> (n)) & 1)

#endif

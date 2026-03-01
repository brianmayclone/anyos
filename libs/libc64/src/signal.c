/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 signal handling.
 */

#include <signal.h>
#include <stdlib.h>
#include <unistd.h>

#include <sys/syscall.h>

extern long _syscall(long num, long a1, long a2, long a3, long a4, long a5);

sighandler_t signal(int signum, sighandler_t handler) {
    if (signum < 0 || signum >= 32) return SIG_ERR;
    long old = _syscall(SYS_SIGACTION, signum, (long)handler, 0, 0, 0);
    if (old == -1L) return SIG_ERR;
    return (sighandler_t)old;
}

int raise(int sig) {
    return kill(getpid(), sig);
}

int kill(int pid, int sig) {
    long ret = _syscall(SYS_KILL, pid, sig, 0, 0, 0);
    if (ret == -1L) return -1;
    return 0;
}

int sigprocmask(int how, const sigset_t *set, sigset_t *oldset) {
    unsigned long new_set = set ? *set : 0;
    long old = _syscall(SYS_SIGPROCMASK, how, (long)new_set, 0, 0, 0);
    if (oldset) *oldset = (sigset_t)old;
    return 0;
}

int sigaction(int signum, const struct sigaction *act, struct sigaction *oldact) {
    if (signum < 0 || signum >= 32) return -1;
    if (act) {
        long h = (long)act->sa_handler;
        long old = _syscall(SYS_SIGACTION, signum, h, 0, 0, 0);
        if (oldact) {
            oldact->sa_handler = (sighandler_t)old;
            oldact->sa_mask = 0;
            oldact->sa_flags = 0;
        }
    } else if (oldact) {
        /* Query only: set to SIG_DFL, get old, restore old */
        long old = _syscall(SYS_SIGACTION, signum, 0 /* SIG_DFL */, 0, 0, 0);
        _syscall(SYS_SIGACTION, signum, old, 0, 0, 0); /* restore */
        oldact->sa_handler = (sighandler_t)old;
        oldact->sa_mask = 0;
        oldact->sa_flags = 0;
    }
    return 0;
}

int sigsuspend(const sigset_t *mask) {
    (void)mask;
    return -1;
}

int sigpending(sigset_t *set) {
    if (set) *set = 0;
    return 0;
}

int siginterrupt(int sig, int flag) {
    (void)sig; (void)flag;
    return 0;
}

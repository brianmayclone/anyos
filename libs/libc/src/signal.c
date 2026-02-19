/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#include <signal.h>
#include <stdlib.h>
#include <unistd.h>

/* Syscall numbers — must match kernel */
#define SYS_KILL        13
#define SYS_SIGACTION   244
#define SYS_SIGPROCMASK 245

/* Implemented in syscall.S */
extern int _syscall(int num, int a1, int a2, int a3, int a4);

sighandler_t signal(int signum, sighandler_t handler) {
    if (signum < 0 || signum >= 32) return SIG_ERR;
    unsigned int old = _syscall(SYS_SIGACTION, signum,
                                (int)(unsigned int)(unsigned long)handler, 0, 0);
    if (old == 0xFFFFFFFF) return SIG_ERR;
    return (sighandler_t)(unsigned long)old;
}

int raise(int sig) {
    return kill(getpid(), sig);
}

int kill(int pid, int sig) {
    unsigned int ret = _syscall(SYS_KILL, pid, sig, 0, 0);
    if (ret == 0xFFFFFFFF) return -1;
    return 0;
}

int sigprocmask(int how, const sigset_t *set, sigset_t *oldset) {
    unsigned int new_set = set ? *set : 0;
    unsigned int old = _syscall(SYS_SIGPROCMASK, how, (int)new_set, 0, 0);
    if (oldset) *oldset = old;
    return 0;
}

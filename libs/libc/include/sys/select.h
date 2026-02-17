/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * POSIX sys/select.h - select() and fd_set
 */

#ifndef _SYS_SELECT_H
#define _SYS_SELECT_H

#include <sys/types.h>
#include <sys/time.h>
#include <time.h>
#include <signal.h>

/* Maximum number of file descriptors in an fd_set */
#ifndef FD_SETSIZE
#define FD_SETSIZE 256
#endif

/* Number of bits per unsigned long */
#define __NFDBITS (8 * (int)sizeof(unsigned long))

/* How many unsigned longs we need */
#define __FD_SET_LONGS  (FD_SETSIZE / __NFDBITS)

typedef struct {
    unsigned long fds_bits[__FD_SET_LONGS];
} fd_set;

#define FD_ZERO(set) \
    do { \
        unsigned int __i; \
        for (__i = 0; __i < __FD_SET_LONGS; __i++) \
            ((fd_set *)(set))->fds_bits[__i] = 0; \
    } while (0)

#define FD_SET(fd, set) \
    ((fd_set *)(set))->fds_bits[(fd) / __NFDBITS] |= (1UL << ((fd) % __NFDBITS))

#define FD_CLR(fd, set) \
    ((fd_set *)(set))->fds_bits[(fd) / __NFDBITS] &= ~(1UL << ((fd) % __NFDBITS))

#define FD_ISSET(fd, set) \
    (((fd_set *)(set))->fds_bits[(fd) / __NFDBITS] & (1UL << ((fd) % __NFDBITS)))

int select(int nfds, fd_set *readfds, fd_set *writefds,
           fd_set *exceptfds, struct timeval *timeout);

int pselect(int nfds, fd_set *readfds, fd_set *writefds,
            fd_set *exceptfds, const struct timespec *timeout,
            const void *sigmask);

#endif /* _SYS_SELECT_H */

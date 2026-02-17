/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * POSIX poll.h - poll() interface
 */

#ifndef _POLL_H
#define _POLL_H

/* Event types for poll() */
#define POLLIN      0x0001  /* Data available to read */
#define POLLPRI     0x0002  /* Urgent data */
#define POLLOUT     0x0004  /* Writing possible */
#define POLLERR     0x0008  /* Error condition */
#define POLLHUP     0x0010  /* Hung up */
#define POLLNVAL    0x0020  /* Invalid fd */
#define POLLRDNORM  0x0040  /* Normal data available */
#define POLLWRNORM  POLLOUT

typedef unsigned int nfds_t;

struct pollfd {
    int   fd;       /* File descriptor */
    short events;   /* Requested events */
    short revents;  /* Returned events */
};

int poll(struct pollfd *fds, nfds_t nfds, int timeout);

#endif /* _POLL_H */

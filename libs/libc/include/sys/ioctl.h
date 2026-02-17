/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _SYS_IOCTL_H
#define _SYS_IOCTL_H

/* ioctl request codes */
#define FIONBIO     0x5421  /* Set/clear non-blocking I/O */
#define FIONREAD    0x541B  /* Get number of bytes available */
#define SIOCGIFADDR 0x8915  /* Get interface address */

int ioctl(int fd, unsigned long request, ...);

#endif /* _SYS_IOCTL_H */

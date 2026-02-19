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

/* Terminal ioctl */
#define TIOCGPGRP   0x540F
#define TIOCSPGRP   0x5410
#define TIOCGWINSZ  0x5413
#define TIOCSWINSZ  0x5414
#define TIOCSCTTY   0x540E

struct winsize {
    unsigned short ws_row;
    unsigned short ws_col;
    unsigned short ws_xpixel;
    unsigned short ws_ypixel;
};

int ioctl(int fd, unsigned long request, ...);

#endif /* _SYS_IOCTL_H */

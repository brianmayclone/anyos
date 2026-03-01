/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _TERMIOS_H
#define _TERMIOS_H

#define NCCS 32

typedef unsigned int tcflag_t;
typedef unsigned char cc_t;
typedef unsigned int speed_t;

struct termios {
    tcflag_t c_iflag;
    tcflag_t c_oflag;
    tcflag_t c_cflag;
    tcflag_t c_lflag;
    cc_t     c_cc[NCCS];
    speed_t  c_ispeed;
    speed_t  c_ospeed;
};

/* c_iflag bits */
#define IGNBRK  0x001
#define BRKINT  0x002
#define IGNPAR  0x004
#define INPCK   0x010
#define ISTRIP  0x020
#define INLCR   0x040
#define IGNCR   0x080
#define ICRNL   0x100
#define IXON    0x400
#define IXOFF   0x1000

/* c_oflag bits */
#define OPOST   0x001

/* c_cflag bits */
#define CSIZE   0x030
#define CS8     0x030
#define CSTOPB  0x040
#define CREAD   0x080
#define PARENB  0x100
#define HUPCL   0x400
#define CLOCAL  0x800

/* c_lflag bits */
#define ISIG    0x001
#define ICANON  0x002
#define ECHO    0x008
#define ECHOE   0x010
#define ECHOK   0x020
#define ECHONL  0x040
#define NOFLSH  0x080
#define IEXTEN  0x8000

/* c_cc indices */
#define VINTR   0
#define VQUIT   1
#define VERASE  2
#define VKILL   3
#define VEOF    4
#define VTIME   5
#define VMIN    6

/* Baud rates */
#define B0      0
#define B9600   9600
#define B19200  19200
#define B38400  38400
#define B57600  57600
#define B115200 115200

/* tcsetattr optional_actions */
#define TCSANOW   0
#define TCSADRAIN 1
#define TCSAFLUSH 2

#ifdef __cplusplus
extern "C" {
#endif

int tcgetattr(int fd, struct termios *termios_p);
int tcsetattr(int fd, int optional_actions, const struct termios *termios_p);
speed_t cfgetispeed(const struct termios *termios_p);
speed_t cfgetospeed(const struct termios *termios_p);

#ifdef __cplusplus
}
#endif

#endif

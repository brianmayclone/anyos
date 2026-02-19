#ifndef _TERMIOS_H
#define _TERMIOS_H

#include <sys/types.h>

typedef unsigned int tcflag_t;
typedef unsigned char cc_t;
typedef unsigned int speed_t;

#define NCCS 32

struct termios {
    tcflag_t c_iflag;
    tcflag_t c_oflag;
    tcflag_t c_cflag;
    tcflag_t c_lflag;
    cc_t     c_cc[NCCS];
};

/* c_lflag bits */
#define ECHO    0x0008
#define ECHOE   0x0010
#define ECHOK   0x0020
#define ECHONL  0x0040
#define ICANON  0x0002
#define ISIG    0x0001
#define TOSTOP  0x0100

/* c_iflag bits */
#define IGNBRK  0x0001
#define BRKINT  0x0002
#define ICRNL   0x0100
#define IXON    0x0400
#define IXOFF   0x1000

/* c_oflag bits */
#define OPOST   0x0001
#define ONLCR   0x0004

/* c_cflag bits */
#define CSIZE   0x0030
#define CS8     0x0030
#define CREAD   0x0080
#define HUPCL   0x0400

/* tcsetattr optional actions */
#define TCSANOW   0
#define TCSADRAIN 1
#define TCSAFLUSH 2

/* Control characters */
#define VEOF    4
#define VEOL    11
#define VERASE  2
#define VINTR   0
#define VKILL   3
#define VMIN    6
#define VQUIT   1
#define VSTART  8
#define VSTOP   9
#define VSUSP   10
#define VTIME   5

int tcgetattr(int fd, struct termios *termios_p);
int tcsetattr(int fd, int optional_actions, const struct termios *termios_p);
pid_t tcgetpgrp(int fd);
int tcsetpgrp(int fd, pid_t pgrp);
speed_t cfgetispeed(const struct termios *termios_p);
speed_t cfgetospeed(const struct termios *termios_p);

#endif

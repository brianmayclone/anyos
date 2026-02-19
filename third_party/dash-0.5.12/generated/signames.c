/* signames.c -- Manually generated for anyOS (Linux i386 signal ABI). */

#include <signal.h>

/* A translation list so we can be polite to our users. */
/* Signal numbers match libs/libc/include/signal.h (Linux i386 ABI). */
const char *const signal_names[NSIG + 1] = {
    "EXIT",     /*  0 */
    "HUP",      /*  1 - SIGHUP */
    "INT",      /*  2 - SIGINT */
    "QUIT",     /*  3 - SIGQUIT */
    "ILL",      /*  4 - SIGILL */
    "TRAP",     /*  5 - SIGTRAP */
    "ABRT",     /*  6 - SIGABRT */
    "BUS",      /*  7 - SIGBUS */
    "FPE",      /*  8 - SIGFPE */
    "KILL",     /*  9 - SIGKILL */
    "USR1",     /* 10 - SIGUSR1 */
    "SEGV",     /* 11 - SIGSEGV */
    "USR2",     /* 12 - SIGUSR2 */
    "PIPE",     /* 13 - SIGPIPE */
    "ALRM",     /* 14 - SIGALRM */
    "TERM",     /* 15 - SIGTERM */
    "16",       /* 16 - unused */
    "CHLD",     /* 17 - SIGCHLD */
    "CONT",     /* 18 - SIGCONT */
    "STOP",     /* 19 - SIGSTOP */
    "TSTP",     /* 20 - SIGTSTP */
    "TTIN",     /* 21 - SIGTTIN */
    "TTOU",     /* 22 - SIGTTOU */
    "23",       /* 23 */
    "24",       /* 24 */
    "25",       /* 25 */
    "26",       /* 26 */
    "27",       /* 27 */
    "28",       /* 28 */
    "29",       /* 29 */
    "30",       /* 30 */
    "31",       /* 31 */
    (char *)0x0
};

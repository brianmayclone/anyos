/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 C runtime startup.
 * Called from crt0.S _start. Parses args from kernel and calls main(argc, argv).
 */

#include <sys/syscall.h>

extern long _syscall(long num, long a1, long a2, long a3, long a4, long a5);
extern int main(int argc, char **argv);
extern void exit(int status);
extern void __init_environ(void);

/* .init_array constructors — linker script provides these symbols */
typedef void (*init_func)(void);
extern init_func __init_array_start[];
extern init_func __init_array_end[];
#define MAX_ARGS    64
#define ARG_BUF_SIZE 1024

void __libc_start_main(void) {
    static char arg_buf[ARG_BUF_SIZE];
    static char *argv[MAX_ARGS + 1];
    int argc = 0;

    /* Get the raw args string from the kernel */
    long len = _syscall(SYS_GETARGS, (long)arg_buf, ARG_BUF_SIZE - 1, 0, 0, 0);
    if (len < 0) len = 0;
    arg_buf[len] = '\0';

    /* Parse space-separated arguments */
    char *p = arg_buf;
    while (*p == ' ') p++;

    while (*p != '\0' && argc < MAX_ARGS) {
        argv[argc++] = p;
        while (*p != '\0' && *p != ' ') p++;
        if (*p == ' ') {
            *p++ = '\0';
            while (*p == ' ') p++;
        }
    }

    argv[argc] = (char *)0;

    /* Populate environ from kernel env store */
    __init_environ();

    /* Run .init_array constructors */
    for (init_func *fn = __init_array_start; fn < __init_array_end; fn++) {
        if (*fn) (*fn)();
    }

    int ret = main(argc, argv);
    exit(ret);
}

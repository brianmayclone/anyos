/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

/* __libc_start_main — Parse args from kernel and call main(argc, argv). */

extern int _syscall(int num, int a1, int a2, int a3, int a4);
extern int main(int argc, char **argv);
extern void exit(int status);
extern void __init_environ(void);

/* .init_array constructors — linker script provides these symbols */
typedef void (*init_func)(void);
extern init_func __init_array_start[];
extern init_func __init_array_end[];

#define SYS_GETARGS 28
#define MAX_ARGS    64
#define ARG_BUF_SIZE 1024

void __libc_start_main(void) {
    static char arg_buf[ARG_BUF_SIZE];
    static char *argv[MAX_ARGS + 1];  /* +1 for NULL terminator */
    int argc = 0;

    /* Get the raw args string from the kernel */
    int len = _syscall(SYS_GETARGS, (int)arg_buf, ARG_BUF_SIZE - 1, 0, 0);
    if (len < 0) len = 0;
    arg_buf[len] = '\0';

    /* Parse space-separated arguments.
     * The kernel provides: "program_path arg1 arg2 ..."
     * argv[0] = program path, argv[1..] = arguments
     */
    char *p = arg_buf;

    /* Skip leading spaces */
    while (*p == ' ') p++;

    while (*p != '\0' && argc < MAX_ARGS) {
        argv[argc++] = p;
        /* Find end of argument */
        while (*p != '\0' && *p != ' ') p++;
        if (*p == ' ') {
            *p++ = '\0';
            /* Skip consecutive spaces */
            while (*p == ' ') p++;
        }
    }

    argv[argc] = (char *)0;  /* NULL-terminate argv */

    /* Populate environ from kernel env store (SYS_LISTENV) */
    __init_environ();

    /* Run .init_array constructors (e.g. __attribute__((constructor))) */
    for (init_func *fn = __init_array_start; fn < __init_array_end; fn++) {
        if (*fn) (*fn)();
    }

    int ret = main(argc, argv);
    exit(ret);
}

/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

.section .text
.global _start
_start:
    /* Clear frame pointer for backtraces */
    xorl %ebp, %ebp
    /* __libc_start_main parses args and calls main(argc, argv) */
    call __libc_start_main
    /* Should not reach here (__libc_start_main calls exit) */
    hlt

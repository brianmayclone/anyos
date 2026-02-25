/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 C runtime entry point.
 * Called by kernel via iret to Ring 3. Calls __libc_start_main
 * which parses args and invokes main(argc, argv).
 */

.section .text
.global _start
_start:
    /* Clear frame pointer for backtraces */
    xorq %rbp, %rbp
    /* __libc_start_main parses args and calls main(argc, argv) */
    call __libc_start_main
    /* Should not reach here (__libc_start_main calls exit) */
    hlt

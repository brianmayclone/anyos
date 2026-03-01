/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 crtn.S (constructor/destructor epilogue stubs).
 * Closes the .init/.fini sections opened by crti.S.
 */

.section .init
    popq %rbp
    ret

.section .fini
    popq %rbp
    ret

/* Mark as not needing executable stack. */
.section .note.GNU-stack,"",@progbits

/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 crti.S (constructor/destructor prologue stubs).
 * Required by some toolchains; provides .init/.fini section headers.
 */

.section .init
.global _init
_init:
    pushq %rbp
    movq %rsp, %rbp

.section .fini
.global _fini
_fini:
    pushq %rbp
    movq %rsp, %rbp

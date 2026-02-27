/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * unwind_registers.S — x86_64 register save/restore helpers for libunwind.
 *
 * Provides:
 *   _Unwind_RaiseException   — Public ABI entry: saves caller registers,
 *                               then calls _Unwind_RaiseException_impl()
 *   _unwind_restore_and_jump — Restores registers from cursor and jumps
 *                               to the landing pad (does not return)
 *
 * unwind_cursor_t register layout (matches unwind.c):
 *
 *   Offset   DWARF#   Register
 *   ------   ------   --------
 *     0       0       RAX
 *     8       1       RDX
 *    16       2       RCX
 *    24       3       RBX
 *    32       4       RSI
 *    40       5       RDI
 *    48       6       RBP
 *    56       7       RSP
 *    64       8       R8
 *    72       9       R9
 *    80      10       R10
 *    88      11       R11
 *    96      12       R12
 *   104      13       R13
 *   112      14       R14
 *   120      15       R15
 *   128      16       RIP (RA)
 *
 * Each entry is 8 bytes (uint64_t), indexed by DWARF register number.
 * Total register area: 17 * 8 = 136 bytes.
 */

.section .text

/* ================================================================== */
/*  _Unwind_RaiseException — public ABI trampoline                     */
/* ================================================================== */
/*
 * _Unwind_Reason_Code _Unwind_RaiseException(
 *     struct _Unwind_Exception *exception_object);  // rdi
 *
 * This is the public entry point called by __cxa_throw (or equivalent).
 * We must capture the caller's register state before any prologue code
 * modifies callee-saved registers, then delegate to the C implementation.
 *
 * Strategy:
 *   1. Allocate an unwind_cursor_t on the stack (17 * 8 = 136 bytes,
 *      rounded to 144 for 16-byte alignment).
 *   2. Save all registers into it.
 *   3. Call _Unwind_RaiseException_impl(exception_object, &cursor).
 *   4. If it returns (no handler found), return its result to our caller.
 *
 * We save ALL 17 registers so the cursor accurately represents the
 * caller's state.  The caller-saved registers (RAX, RCX, RDX, RSI, RDI,
 * R8-R11) may have been modified by the throw-site prologue, but the
 * personality routine only needs the callee-saved ones (RBX, RBP,
 * R12-R15, RSP) plus RIP (return address) to be correct.
 */
.global _Unwind_RaiseException
.type _Unwind_RaiseException, @function
_Unwind_RaiseException:
    .cfi_startproc

    /* Save exception_object (rdi) across our setup */
    pushq   %rbp
    .cfi_def_cfa_offset 16
    .cfi_offset %rbp, -16
    movq    %rsp, %rbp
    .cfi_def_cfa_register %rbp

    /*
     * Allocate 144 bytes for unwind_cursor_t on the stack.
     * 136 bytes for 17 registers + 8 bytes padding for alignment.
     * The cursor starts at %rsp after this sub.
     */
    subq    $144, %rsp

    /* rdi still holds exception_object — save it temporarily */
    pushq   %rdi

    /* Pointer to cursor in rsi (second arg to C function) */
    leaq    8(%rsp), %rsi       /* rsi = &cursor (skip the pushed rdi) */

    /* Save all registers into the cursor.
     * We want the CALLER's values, not ours.  For callee-saved regs
     * they are still intact.  For RSP/RIP we compute from our frame.
     */
    movq    %rax,    0(%rsi)    /* [0]  RAX (DWARF 0) */
    movq    %rdx,    8(%rsi)    /* [1]  RDX (DWARF 1) */
    movq    %rcx,   16(%rsi)    /* [2]  RCX (DWARF 2) */
    movq    %rbx,   24(%rsi)    /* [3]  RBX (DWARF 3) */
    /* RSI/RDI: these are ours, but that's fine — personality ignores them */
    movq    %rsi,   32(%rsi)    /* [4]  RSI (DWARF 4) — approximate */
    movq    %rdi,   40(%rsi)    /* [5]  RDI (DWARF 5) — the exception_object ptr */
    movq    0(%rbp), %rax       /* caller's RBP (saved by our pushq %rbp) */
    movq    %rax,   48(%rsi)    /* [6]  RBP (DWARF 6) — caller's value */
    leaq    16(%rbp), %rax      /* caller's RSP: rbp + 16 (past saved rbp + ret addr) */
    movq    %rax,   56(%rsi)    /* [7]  RSP (DWARF 7) */
    movq    %r8,    64(%rsi)    /* [8]  R8  (DWARF 8) */
    movq    %r9,    72(%rsi)    /* [9]  R9  (DWARF 9) */
    movq    %r10,   80(%rsi)    /* [10] R10 (DWARF 10) */
    movq    %r11,   88(%rsi)    /* [11] R11 (DWARF 11) */
    movq    %r12,   96(%rsi)    /* [12] R12 (DWARF 12) */
    movq    %r13,  104(%rsi)    /* [13] R13 (DWARF 13) */
    movq    %r14,  112(%rsi)    /* [14] R14 (DWARF 14) */
    movq    %r15,  120(%rsi)    /* [15] R15 (DWARF 15) */
    movq    8(%rbp), %rax       /* return address = caller's RIP */
    movq    %rax,  128(%rsi)    /* [16] RIP/RA (DWARF 16) */

    /* Restore exception_object into rdi (first arg) */
    popq    %rdi
    /* rsi already points to cursor (second arg) */

    /* Call the C implementation */
    call    _Unwind_RaiseException_impl

    /*
     * If we reach here, _Unwind_RaiseException_impl returned an error
     * (typically _URC_END_OF_STACK).  Return it to our caller.
     * eax already holds the return value.
     */
    movq    %rbp, %rsp
    popq    %rbp
    .cfi_def_cfa %rsp, 8
    ret

    .cfi_endproc
    .size _Unwind_RaiseException, . - _Unwind_RaiseException


/* ================================================================== */
/*  _unwind_restore_and_jump — land at a catch handler / landing pad   */
/* ================================================================== */
/*
 * void _unwind_restore_and_jump(void *cursor_ptr);  // rdi = &cursor
 *
 * Restores all callee-saved registers from the cursor, sets RSP, and
 * jumps to cursor->regs[16] (RIP).  This transfers control to the
 * landing pad selected by the personality routine.
 *
 * The personality routine has already called _Unwind_SetGR to place
 * the exception pointer in RAX (DWARF reg 0) and the selector in
 * RDX (DWARF reg 1), and _Unwind_SetIP to set the landing pad address.
 *
 * This function does NOT return.
 */
.global _unwind_restore_and_jump
.type _unwind_restore_and_jump, @function
_unwind_restore_and_jump:
    .cfi_startproc

    /* rdi = pointer to unwind_cursor_t */

    /* Load the target RIP into a scratch register */
    movq    128(%rdi), %rcx     /* RIP/RA (DWARF 16) — landing pad addr */

    /* Restore callee-saved registers */
    movq     24(%rdi), %rbx     /* RBX (DWARF 3) */
    movq     48(%rdi), %rbp     /* RBP (DWARF 6) */
    movq     96(%rdi), %r12     /* R12 (DWARF 12) */
    movq    104(%rdi), %r13     /* R13 (DWARF 13) */
    movq    112(%rdi), %r14     /* R14 (DWARF 14) */
    movq    120(%rdi), %r15     /* R15 (DWARF 15) */

    /*
     * Restore RAX and RDX — the landing pad expects:
     *   RAX = pointer to _Unwind_Exception (set by personality via SetGR)
     *   RDX = type selector index (set by personality via SetGR)
     */
    movq      0(%rdi), %rax     /* RAX (DWARF 0) */
    movq      8(%rdi), %rdx     /* RDX (DWARF 1) */

    /* Restore RSP — must be done last before the jump */
    movq     56(%rdi), %rsp     /* RSP (DWARF 7) */

    /*
     * Jump to the landing pad.  We use jmp rather than ret because:
     *   1. There is no return address on the stack (RSP points to the
     *      handler frame, not a call frame).
     *   2. The landing pad is not our caller — it is a different function
     *      or a different location within a function.
     */
    jmpq    *%rcx

    .cfi_endproc
    .size _unwind_restore_and_jump, . - _unwind_restore_and_jump

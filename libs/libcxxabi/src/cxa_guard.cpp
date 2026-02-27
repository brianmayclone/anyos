/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * cxa_guard.cpp — Thread-safe static local variable initialization guards.
 *
 * The Itanium C++ ABI specifies that static local variables must be
 * initialized exactly once, even with concurrent threads.  The compiler
 * emits calls to __cxa_guard_acquire/release/abort around the initializer.
 *
 * Guard variable layout (64-bit):
 *   byte 0: initialization state (0=uninit, 1=in-progress, 2=done)
 *   bytes 1-7: unused padding
 */

#include <stdint.h>
#include <cxxabi.h>

extern "C" {

/// Acquire the guard.
/// Returns 1 if the caller should run the initializer, 0 if already done.
int __cxa_guard_acquire(__cxa_guard_type *guard) {
    volatile uint8_t *state = reinterpret_cast<volatile uint8_t *>(guard);

    /* Fast path: already initialized. */
    if (__atomic_load_n(state, __ATOMIC_ACQUIRE) == 2)
        return 0;

    /* Try to claim the guard. */
    uint8_t expected = 0;
    if (__atomic_compare_exchange_n(state, &expected, 1,
                                    /*weak=*/false,
                                    __ATOMIC_ACQ_REL,
                                    __ATOMIC_ACQUIRE)) {
        return 1; /* Caller must initialize. */
    }

    /* Another thread is initializing — spin until done. */
    while (__atomic_load_n(state, __ATOMIC_ACQUIRE) != 2) {
        /* Yield to scheduler. */
        extern long _syscall(long, long, long, long, long, long);
        _syscall(7 /*SYS_YIELD*/, 0, 0, 0, 0, 0);
    }
    return 0;
}

/// Release the guard after successful initialization.
void __cxa_guard_release(__cxa_guard_type *guard) {
    volatile uint8_t *state = reinterpret_cast<volatile uint8_t *>(guard);
    __atomic_store_n(state, (uint8_t)2, __ATOMIC_RELEASE);
}

/// Abort the guard (initialization threw an exception).
void __cxa_guard_abort(__cxa_guard_type *guard) {
    volatile uint8_t *state = reinterpret_cast<volatile uint8_t *>(guard);
    __atomic_store_n(state, (uint8_t)0, __ATOMIC_RELEASE);
}

} // extern "C"

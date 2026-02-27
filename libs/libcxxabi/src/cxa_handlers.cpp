/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * cxa_handlers.cpp — Termination, pure virtual, and atexit handlers.
 */

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <cxxabi.h>

extern "C" {

/* ── std::terminate / std::set_terminate ──────────────────────────────── */

typedef void (*terminate_handler)(void);
static terminate_handler _terminate_handler = nullptr;

terminate_handler __cxa_set_terminate(terminate_handler handler) {
    terminate_handler old = _terminate_handler;
    _terminate_handler = handler;
    return old;
}

void __cxa_terminate(void) {
    if (_terminate_handler) {
        _terminate_handler();
    }
    fprintf(stderr, "std::terminate() called\n");
    abort();
}

/* ── Pure virtual / deleted virtual call handlers ────────────────────── */

void __cxa_pure_virtual(void) {
    fprintf(stderr, "Pure virtual function called!\n");
    abort();
}

void __cxa_deleted_virtual(void) {
    fprintf(stderr, "Deleted virtual function called!\n");
    abort();
}

/* ── __cxa_atexit — destructor registration for static objects ───────── */

#define MAX_ATEXIT_FUNCS 128

struct atexit_entry {
    void (*func)(void *);
    void *arg;
    void *dso;
};

static struct atexit_entry _atexit_table[MAX_ATEXIT_FUNCS];
static int _atexit_count = 0;

int __cxa_atexit(void (*func)(void *), void *arg, void *dso) {
    if (_atexit_count >= MAX_ATEXIT_FUNCS) return -1;
    _atexit_table[_atexit_count].func = func;
    _atexit_table[_atexit_count].arg = arg;
    _atexit_table[_atexit_count].dso = dso;
    _atexit_count++;
    return 0;
}

void __cxa_finalize(void *dso) {
    /* Run registered destructors in reverse order. */
    for (int i = _atexit_count - 1; i >= 0; i--) {
        if (dso == nullptr || _atexit_table[i].dso == dso) {
            if (_atexit_table[i].func) {
                _atexit_table[i].func(_atexit_table[i].arg);
                _atexit_table[i].func = nullptr;
            }
        }
    }
}

} // extern "C"

/* ── std::terminate / std::set_terminate (C++ wrappers) ──────────────── */

namespace std {

void terminate() noexcept {
    __cxa_terminate();
    __builtin_unreachable();
}

terminate_handler set_terminate(terminate_handler f) noexcept {
    return __cxa_set_terminate(f);
}

} // namespace std

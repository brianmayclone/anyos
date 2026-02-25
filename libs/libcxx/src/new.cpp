/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * operator new/delete — routes to libc64 malloc/free.
 * No exceptions (-fno-exceptions): regular new calls abort on failure.
 */

#include <new>
#include <cstdlib>

/* Regular new — abort on failure (no exceptions) */
void* operator new(std::size_t size) {
    if (size == 0) size = 1;
    void* p = std::malloc(size);
    if (!p) std::abort();
    return p;
}

void* operator new[](std::size_t size) {
    if (size == 0) size = 1;
    void* p = std::malloc(size);
    if (!p) std::abort();
    return p;
}

/* Regular delete */
void operator delete(void* ptr) noexcept {
    std::free(ptr);
}

void operator delete[](void* ptr) noexcept {
    std::free(ptr);
}

void operator delete(void* ptr, std::size_t) noexcept {
    std::free(ptr);
}

void operator delete[](void* ptr, std::size_t) noexcept {
    std::free(ptr);
}

/* Nothrow new — returns nullptr on failure */
void* operator new(std::size_t size, const std::nothrow_t&) noexcept {
    if (size == 0) size = 1;
    return std::malloc(size);
}

void* operator new[](std::size_t size, const std::nothrow_t&) noexcept {
    if (size == 0) size = 1;
    return std::malloc(size);
}

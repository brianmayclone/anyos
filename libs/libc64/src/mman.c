/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 mmap/munmap shims.
 */

#include <sys/mman.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>

/* mmap shim: MAP_ANONYMOUS -> calloc, file mapping -> malloc+read */
void *mmap(void *addr, size_t length, int prot, int flags, int fd, long offset) {
    (void)addr; (void)prot;
    if (length == 0) return MAP_FAILED;
    void *buf = malloc(length);
    if (!buf) return MAP_FAILED;
    if (flags & MAP_ANONYMOUS) {
        memset(buf, 0, length);
    } else if (fd >= 0) {
        lseek(fd, (off_t)offset, 0 /* SEEK_SET */);
        size_t total = 0;
        while (total < length) {
            size_t chunk = (length - total > 32768) ? 32768 : (length - total);
            ssize_t n = read(fd, (char *)buf + total, chunk);
            if (n <= 0) break;
            total += (size_t)n;
        }
        if (total < length) memset((char *)buf + total, 0, length - total);
    }
    return buf;
}

int munmap(void *addr, size_t length) {
    (void)length;
    if (addr && addr != MAP_FAILED) free(addr);
    return 0;
}

int mprotect(void *addr, size_t length, int prot) {
    (void)addr; (void)length; (void)prot;
    return 0;
}

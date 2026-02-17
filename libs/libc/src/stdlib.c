/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* Arena-based malloc: requests memory from sbrk in large chunks, suballocates
 * locally. This avoids a syscall for every malloc — critical for complex C
 * programs like NetSurf that do tens of thousands of small allocations. */

typedef struct block_header {
    size_t size;        /* payload size (not including header) */
    int free;
    struct block_header *next;
} block_header_t;

#define HEADER_SIZE (sizeof(block_header_t))
#define ALIGN(x) (((x) + 7) & ~7)
#define ARENA_CHUNK 65536  /* request 64 KiB from sbrk at a time */

static block_header_t *free_list = NULL;

/* Arena: current position and remaining bytes in the current sbrk chunk */
static char *arena_ptr = NULL;
static size_t arena_remaining = 0;

/* Allocate raw memory from the arena, calling sbrk only when needed */
static void *arena_alloc(size_t total) {
    if (total > arena_remaining) {
        /* Request a new chunk from sbrk — at least ARENA_CHUNK or the
         * requested size, whichever is larger */
        size_t chunk = total > ARENA_CHUNK ? total : ARENA_CHUNK;
        void *p = sbrk(chunk);
        if (p == (void *)-1) return NULL;
        arena_ptr = (char *)p;
        arena_remaining = chunk;
    }
    void *result = arena_ptr;
    arena_ptr += total;
    arena_remaining -= total;
    return result;
}

void *malloc(size_t size) {
    if (size == 0) return NULL;
    size = ALIGN(size);

    /* Search free list for a reusable block */
    block_header_t *prev = NULL;
    block_header_t *curr = free_list;
    while (curr) {
        if (curr->free && curr->size >= size) {
            /* Split if the block is significantly larger */
            if (curr->size >= size + HEADER_SIZE + 16) {
                block_header_t *split = (block_header_t *)((char *)curr + HEADER_SIZE + size);
                split->size = curr->size - size - HEADER_SIZE;
                split->free = 1;
                split->next = curr->next;
                curr->size = size;
                curr->next = split;
            }
            curr->free = 0;
            return (void *)((char *)curr + HEADER_SIZE);
        }
        prev = curr;
        curr = curr->next;
    }

    /* Allocate from arena (batched sbrk) */
    size_t total = HEADER_SIZE + size;
    void *p = arena_alloc(total);
    if (!p) return NULL;

    block_header_t *blk = (block_header_t *)p;
    blk->size = size;
    blk->free = 0;
    blk->next = NULL;

    if (prev) prev->next = blk;
    else free_list = blk;

    return (void *)((char *)blk + HEADER_SIZE);
}

void *calloc(size_t nmemb, size_t size) {
    size_t total = nmemb * size;
    void *p = malloc(total);
    if (p) memset(p, 0, total);
    return p;
}

void *realloc(void *ptr, size_t size) {
    if (!ptr) return malloc(size);
    if (size == 0) { free(ptr); return NULL; }

    block_header_t *blk = (block_header_t *)((char *)ptr - HEADER_SIZE);
    size = ALIGN(size);
    if (blk->size >= size) return ptr;

    void *new_ptr = malloc(size);
    if (!new_ptr) return NULL;
    memcpy(new_ptr, ptr, blk->size);
    free(ptr);
    return new_ptr;
}

void free(void *ptr) {
    if (!ptr) return;
    block_header_t *blk = (block_header_t *)((char *)ptr - HEADER_SIZE);
    blk->free = 1;
}

void exit(int status) {
    _exit(status);
    __builtin_unreachable();
}

void abort(void) {
    _exit(134);
    __builtin_unreachable();
}

int atoi(const char *nptr) {
    return (int)strtol(nptr, NULL, 10);
}

long atol(const char *nptr) {
    return strtol(nptr, NULL, 10);
}

long strtol(const char *nptr, char **endptr, int base) {
    const char *s = nptr;
    long result = 0;
    int neg = 0;

    while (*s == ' ' || *s == '\t' || *s == '\n') s++;
    if (*s == '-') { neg = 1; s++; }
    else if (*s == '+') s++;

    if (base == 0) {
        if (*s == '0') {
            s++;
            if (*s == 'x' || *s == 'X') { base = 16; s++; }
            else base = 8;
        } else base = 10;
    } else if (base == 16 && *s == '0' && (*(s+1) == 'x' || *(s+1) == 'X')) {
        s += 2;
    }

    while (*s) {
        int digit;
        if (*s >= '0' && *s <= '9') digit = *s - '0';
        else if (*s >= 'a' && *s <= 'f') digit = *s - 'a' + 10;
        else if (*s >= 'A' && *s <= 'F') digit = *s - 'A' + 10;
        else break;
        if (digit >= base) break;
        result = result * base + digit;
        s++;
    }

    if (endptr) *endptr = (char *)s;
    return neg ? -result : result;
}

unsigned long strtoul(const char *nptr, char **endptr, int base) {
    return (unsigned long)strtol(nptr, endptr, base);
}

int abs(int j) { return j < 0 ? -j : j; }
long labs(long j) { return j < 0 ? -j : j; }

extern int _syscall(int num, int a1, int a2, int a3, int a4);
#define SYS_GETENV 183

char *getenv(const char *name) {
    if (!name || !*name) return NULL;
    static char _env_buf[1024];
    int r = _syscall(SYS_GETENV, (int)name, (int)_env_buf, sizeof(_env_buf) - 1, 0);
    if (r < 0) return NULL;
    if (r < (int)sizeof(_env_buf)) _env_buf[r] = '\0';
    else _env_buf[sizeof(_env_buf) - 1] = '\0';
    return _env_buf;
}

static unsigned int _rand_seed = 1;

int rand(void) {
    _rand_seed = _rand_seed * 1103515245 + 12345;
    return (int)((_rand_seed >> 16) & 0x7FFFFFFF);
}

void srand(unsigned int seed) {
    _rand_seed = seed;
}

long long strtoll(const char *nptr, char **endptr, int base) {
    return (long long)strtol(nptr, endptr, base);
}

unsigned long long strtoull(const char *nptr, char **endptr, int base) {
    return (unsigned long long)strtoul(nptr, endptr, base);
}

void qsort(void *base, size_t nmemb, size_t size, int (*compar)(const void *, const void *)) {
    /* Simple insertion sort for small sets */
    char *b = base;
    char tmp[256]; /* max element size */
    if (size > sizeof(tmp)) return;
    for (size_t i = 1; i < nmemb; i++) {
        memcpy(tmp, b + i * size, size);
        size_t j = i;
        while (j > 0 && compar(b + (j - 1) * size, tmp) > 0) {
            memcpy(b + j * size, b + (j - 1) * size, size);
            j--;
        }
        memcpy(b + j * size, tmp, size);
    }
}

void *bsearch(const void *key, const void *base, size_t nmemb, size_t size,
              int (*compar)(const void *, const void *)) {
    const char *b = base;
    size_t lo = 0, hi = nmemb;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        int cmp = compar(key, b + mid * size);
        if (cmp < 0) hi = mid;
        else if (cmp > 0) lo = mid + 1;
        else return (void *)(b + mid * size);
    }
    return NULL;
}

double atof(const char *nptr) {
    return strtod(nptr, NULL);
}

#define SYS_SPAWN_STDLIB   27
#define SYS_WAITPID_STDLIB 12

int system(const char *command) {
    if (!command) return 1; /* POSIX: non-zero means shell available */
    /* Find the executable — first word of command */
    char path[256];
    const char *p = command;
    while (*p == ' ') p++;
    int i = 0;
    while (*p && *p != ' ' && i < 254) path[i++] = *p++;
    path[i] = '\0';
    /* Skip spaces to find args */
    while (*p == ' ') p++;
    /* Build full args: "progname args..." */
    char args[512];
    int alen = 0;
    /* Copy program basename as argv[0] */
    const char *base = path;
    for (const char *s = path; *s; s++) if (*s == '/') base = s + 1;
    for (const char *s = base; *s && alen < 510; s++) args[alen++] = *s;
    if (*p) {
        args[alen++] = ' ';
        while (*p && alen < 510) args[alen++] = *p++;
    }
    args[alen] = '\0';
    int tid = _syscall(SYS_SPAWN_STDLIB, (int)path, 0, (int)args, 0);
    if (tid < 0) return -1;
    int status = _syscall(SYS_WAITPID_STDLIB, tid, 0, 0, 0);
    return status;
}

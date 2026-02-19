/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#ifndef _INTTYPES_H
#define _INTTYPES_H

#include <stdint.h>

/* printf format macros */
#define PRId8  "d"
#define PRId16 "d"
#define PRId32 "d"
#define PRId64 "lld"

#define PRIu8  "u"
#define PRIu16 "u"
#define PRIu32 "u"
#define PRIu64 "llu"

#define PRIx8  "x"
#define PRIx16 "x"
#define PRIx32 "x"
#define PRIx64 "llx"

#define PRIX8  "X"
#define PRIX16 "X"
#define PRIX32 "X"
#define PRIX64 "llX"

#define PRIo8  "o"
#define PRIo16 "o"
#define PRIo32 "o"
#define PRIo64 "llo"

#define PRIdMAX "ld"
#define PRIuMAX "lu"

/* scanf format macros */
#define SCNd8  "hhd"
#define SCNd16 "hd"
#define SCNd32 "d"
#define SCNd64 "lld"

#define SCNu8  "hhu"
#define SCNu16 "hu"
#define SCNu32 "u"
#define SCNu64 "llu"

#define SCNx8  "hhx"
#define SCNx16 "hx"
#define SCNx32 "x"
#define SCNx64 "llx"

#define SCNo8  "hho"
#define SCNo16 "ho"
#define SCNo32 "o"
#define SCNo64 "llo"

typedef struct {
    long long quot;
    long long rem;
} imaxdiv_t;

intmax_t strtoimax(const char *nptr, char **endptr, int base);
uintmax_t strtoumax(const char *nptr, char **endptr, int base);

#endif

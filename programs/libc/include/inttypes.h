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

/* scanf format macros */
#define SCNd8  "hhd"
#define SCNd16 "hd"
#define SCNd32 "d"
#define SCNd64 "lld"

#define SCNu8  "hhu"
#define SCNu16 "hu"
#define SCNu32 "u"
#define SCNu64 "llu"

typedef struct {
    long long quot;
    long long rem;
} imaxdiv_t;

intmax_t strtoimax(const char *nptr, char **endptr, int base);
uintmax_t strtoumax(const char *nptr, char **endptr, int base);

#endif

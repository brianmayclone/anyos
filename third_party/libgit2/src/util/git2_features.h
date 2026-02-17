/* git2_features.h -- anyOS bare-metal configuration (no CMake) */
#ifndef INCLUDE_features_h__
#define INCLUDE_features_h__

/* 32-bit architecture */
#define GIT_ARCH_32 1

/* No threading (single-threaded bare metal) */
/* #undef GIT_THREADS */

/* No networking */
/* #undef GIT_SSH */
/* #undef GIT_HTTPS */
/* #undef GIT_WINHTTP */
/* #undef GIT_OPENSSL */
/* #undef GIT_MBEDTLS */
/* #undef GIT_SCHANNEL */
/* #undef GIT_SECURE_TRANSPORT */
/* #undef GIT_NTLM */
/* #undef GIT_GSSAPI */

/* HTTP parser: builtin (stubs without networking) */
#define GIT_HTTPPARSER_BUILTIN 1

/* SHA1: collision-detecting (pure C, no deps) */
#define GIT_SHA1_COLLISIONDETECT 1

/* SHA256: builtin RFC 6234 (pure C, no deps) */
#define GIT_SHA256_BUILTIN 1

/* Regex: builtin (bundled PCRE) */
#define GIT_REGEX_BUILTIN 1

/* No nanosecond timestamps */
/* #undef GIT_USE_NSEC */
/* #undef GIT_USE_FUTIMENS */

/* No iconv */
/* #undef GIT_USE_ICONV */

/* No special qsort */
/* #undef GIT_QSORT_BSD */
/* #undef GIT_QSORT_GNU */
/* #undef GIT_QSORT_C11 */

/* No special rand */
/* #undef GIT_RAND_GETENTROPY */
/* #undef GIT_RAND_GETLOADAVG */

/* Use select()-based poll shim */
/* #undef GIT_IO_POLL */
#define GIT_IO_SELECT 1

#endif

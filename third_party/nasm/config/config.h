/* config/config.h - anyOS cross-compilation configuration */
/* Generated for i686-elf-gcc targeting anyOS */

#ifndef NASM_CONFIG_CONFIG_H
#define NASM_CONFIG_CONFIG_H

/* Include the unconfig defaults */
#include "config/unconfig.h"

/* C compiler features (GCC) */
#define HAVE_STDC_INLINE 1
#define HAVE_TYPEOF 1
#define HAVE___BUILTIN_EXPECT 1
#define HAVE___BUILTIN_CHOOSE_EXPR 1
#define HAVE___BUILTIN_CONSTANT_P 1

/* GCC function attributes */
#define HAVE_FUNC_ATTRIBUTE_NORETURN 1
#define HAVE_FUNC_ATTRIBUTE_COLD 1
#define HAVE_FUNC_ATTRIBUTE_UNUSED 1
#define HAVE_FUNC_ATTRIBUTE_PURE 1
#define HAVE_FUNC_ATTRIBUTE_CONST 1
#define HAVE_FUNC_ATTRIBUTE_MALLOC 1
#define HAVE_FUNC_ATTRIBUTE_SENTINEL 1
#define HAVE_FUNC_ATTRIBUTE_RETURNS_NONNULL 1
#define HAVE_FUNC_ATTRIBUTE1_ALLOC_SIZE 1
#define HAVE_FUNC_ATTRIBUTE2_ALLOC_SIZE 1
#define HAVE_FUNC_ATTRIBUTE3_FORMAT 1

/* Standard headers available in anyOS libc */
#define HAVE_INTTYPES_H 1
#define HAVE_STDBOOL_H 1
/* #undef HAVE_STDNORETURN_H -- anyOS libc doesn't have it */
#define HAVE_FCNTL_H 1
#define HAVE_UNISTD_H 1
#define HAVE_SYS_TYPES_H 1
#define HAVE_SYS_STAT_H 1
#define HAVE_STRINGS_H 1
#define HAVE_ENDIAN_H 1

/* Standard functions available */
#define HAVE_SNPRINTF 1
#define HAVE_VSNPRINTF 1
#define HAVE_STRCASECMP 1
#define HAVE_DECL_STRCASECMP 1
#define HAVE_STRNCASECMP 1
#define HAVE_DECL_STRNCASECMP 1
#define HAVE_DECL_STRNLEN 0
#define HAVE_ACCESS 1
#define HAVE_STRUCT_STAT 1
#define HAVE_STAT 1
#define HAVE_FSTAT 1
#define HAVE_FILENO 1
#define HAVE_DECL_FILENO 1
#define HAVE_FTRUNCATE 1

/* Functions we DON'T have - let NASM use its own stdlib/ versions */
/* #undef HAVE_STRLCPY */
/* #undef HAVE_STRNLEN */
/* #undef HAVE_STRCHRNUL - we have it but NASM has its own strrchrnul */
#define HAVE_DECL_STRLCPY 0
#define HAVE_DECL_STRCHRNUL 0

/* We DON'T have mmap (libc shim is malloc-based, not real mmap) */
/* #undef HAVE_MMAP */
/* #undef HAVE_SYS_MMAN_H */

/* We DON'T have these */
/* #undef HAVE_GETRLIMIT */
/* #undef HAVE_PATHCONF */
/* #undef HAVE_FSEEKO */
/* #undef HAVE_FTELLO */
/* #undef HAVE_FACCESSAT */
/* #undef HAVE_REALPATH */
/* #undef HAVE_CANONICALIZE_FILE_NAME */

/* Endianness - x86 is little-endian */
#define WORDS_LITTLEENDIAN 1

#endif /* NASM_CONFIG_CONFIG_H */

/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * Stub zlib.h — link with real libz.a for actual zlib support.
 */

#ifndef _ZLIB_H
#define _ZLIB_H

#include <stddef.h>

#define Z_OK            0
#define Z_STREAM_END    1
#define Z_NEED_DICT     2
#define Z_ERRNO        (-1)
#define Z_STREAM_ERROR (-2)
#define Z_DATA_ERROR   (-3)
#define Z_MEM_ERROR    (-4)
#define Z_BUF_ERROR    (-5)

#define Z_NO_FLUSH      0
#define Z_SYNC_FLUSH    2
#define Z_FINISH        4

#define Z_DEFLATED      8
#define Z_DEFAULT_COMPRESSION (-1)

typedef void *(*alloc_func)(void *opaque, unsigned int items, unsigned int size);
typedef void  (*free_func)(void *opaque, void *address);

typedef struct z_stream_s {
    const unsigned char *next_in;
    unsigned int avail_in;
    unsigned long total_in;
    unsigned char *next_out;
    unsigned int avail_out;
    unsigned long total_out;
    const char *msg;
    void *state;
    alloc_func zalloc;
    free_func zfree;
    void *opaque;
    int data_type;
    unsigned long adler;
    unsigned long reserved;
} z_stream;

typedef z_stream *z_streamp;
typedef unsigned long uLong;
typedef unsigned int uInt;
typedef unsigned char Byte;
typedef Byte *Bytef;

#ifdef __cplusplus
extern "C" {
#endif

const char *zlibVersion(void);

#ifdef __cplusplus
}
#endif

#endif

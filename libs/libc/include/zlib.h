/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * Minimal zlib.h stub for anyOS
 * Provides type/macro definitions so code compiles.
 * inflate/deflate functions return errors at runtime.
 */

#ifndef _ZLIB_H
#define _ZLIB_H

#include <stddef.h>

typedef unsigned char Bytef;
typedef unsigned long uLong;
typedef unsigned int uInt;

typedef struct z_stream_s {
    const Bytef *next_in;
    uInt avail_in;
    uLong total_in;
    Bytef *next_out;
    uInt avail_out;
    uLong total_out;
    const char *msg;
    void *state;
    void *(*zalloc)(void *, uInt, uInt);
    void (*zfree)(void *, void *);
    void *opaque;
    int data_type;
    uLong adler;
    uLong reserved;
} z_stream;

typedef z_stream *z_streamp;
typedef struct gzFile_s *gzFile;
typedef unsigned long uLongf;

#define Z_OK            0
#define Z_STREAM_END    1
#define Z_NEED_DICT     2
#define Z_ERRNO        (-1)
#define Z_STREAM_ERROR (-2)
#define Z_DATA_ERROR   (-3)
#define Z_MEM_ERROR    (-4)
#define Z_BUF_ERROR    (-5)
#define Z_VERSION_ERROR (-6)

#define Z_NO_FLUSH      0
#define Z_SYNC_FLUSH    2
#define Z_FINISH        4

#define Z_DEFLATED      8
#define MAX_WBITS       15

#define Z_NULL          0

int inflateInit2_(z_streamp strm, int windowBits,
                  const char *version, int stream_size);
#define inflateInit2(strm, windowBits) \
    inflateInit2_((strm), (windowBits), "1.0.0", (int)sizeof(z_stream))

int inflate(z_streamp strm, int flush);
int inflateEnd(z_streamp strm);

int deflateInit2_(z_streamp strm, int level, int method, int windowBits,
                  int memLevel, int strategy, const char *version, int stream_size);
int deflate(z_streamp strm, int flush);
int deflateEnd(z_streamp strm);

uLong crc32(uLong crc, const Bytef *buf, uInt len);

/* gzip file functions (stubs) */
gzFile gzopen(const char *path, const char *mode);
int gzread(gzFile file, void *buf, unsigned len);
int gzwrite(gzFile file, const void *buf, unsigned len);
char *gzgets(gzFile file, char *buf, int len);
int gzclose(gzFile file);
int gzeof(gzFile file);
const char *gzerror(gzFile file, int *errnum);

int uncompress(Bytef *dest, uLongf *destLen, const Bytef *source, uLong sourceLen);
int compress(Bytef *dest, uLongf *destLen, const Bytef *source, uLong sourceLen);

#endif /* _ZLIB_H */

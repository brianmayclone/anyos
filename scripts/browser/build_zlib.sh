#!/bin/bash
# Build zlib for anyOS (i686 freestanding cross-compile)
# Uses anyOS libc headers for full zlib support including gz* functions
#
# Output: third_party/zlib/libz.a

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ZLIB_SRC="$PROJECT_ROOT/third_party/zlib"
LIBC_INCLUDE="$PROJECT_ROOT/libs/libc/include"
BUILD_DIR="$ZLIB_SRC/obj"
OUTPUT="$ZLIB_SRC/libz.a"

CC=i686-elf-gcc
AR=i686-elf-ar

CORE_SRCS="adler32.c compress.c crc32.c deflate.c infback.c inffast.c
    inflate.c inftrees.c trees.c uncompr.c zutil.c"

GZ_SRCS="gzclose.c gzlib.c gzread.c gzwrite.c"

ALL_SRCS="$CORE_SRCS $GZ_SRCS"

CFLAGS="-O2 -ffreestanding -nostdlib -fno-builtin -m32 -w"
CFLAGS="$CFLAGS -DHAVE_MEMCPY -DHAVE_STDARG_H=1 -DHAVE_UNISTD_H -DHAVE_VSNPRINTF"
CFLAGS="$CFLAGS -I$ZLIB_SRC -I$LIBC_INCLUDE"

rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

echo "=== Building zlib for anyOS (i686) ==="

OBJ_FILES=""
for src in $ALL_SRCS; do
    obj="$BUILD_DIR/${src%.c}.o"
    echo "  CC  $src"
    $CC $CFLAGS -c "$ZLIB_SRC/$src" -o "$obj"
    OBJ_FILES="$OBJ_FILES $obj"
done

echo "  AR  libz.a"
$AR rcs "$OUTPUT" $OBJ_FILES

echo "=== Done: $OUTPUT ($(du -h "$OUTPUT" | cut -f1)) ==="

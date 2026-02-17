#!/bin/bash
# Build BearSSL for anyOS (i686 freestanding cross-compile)
#
# Output: third_party/bearssl/build/libbearssl.a
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BEARSSL_DIR="$ROOT/third_party/bearssl"
CC="i686-elf-gcc"
AR="i686-elf-ar"
OBJDIR="$BEARSSL_DIR/build/obj"
OUTPUT="$BEARSSL_DIR/build/libbearssl.a"

CFLAGS="-O2 -ffreestanding -nostdlib -fno-builtin -m32 -w -I$BEARSSL_DIR/inc -I$BEARSSL_DIR/src -I$ROOT/libs/libc/include"

mkdir -p "$OBJDIR"

echo "=== Building BearSSL for anyOS (i686) ==="

# Find all .c files in src/
find "$BEARSSL_DIR/src" -name '*.c' | while read src; do
    name=$(basename "$src" .c)
    obj="$OBJDIR/${name}.o"
    $CC $CFLAGS -c "$src" -o "$obj"
done

echo "  AR  libbearssl.a"
$AR rcs "$OUTPUT" "$OBJDIR"/*.o

echo "=== Done: $OUTPUT ($(du -h "$OUTPUT" | cut -f1)) ==="

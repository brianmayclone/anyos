#!/bin/bash
# Build BearSSL for anyOS (x86_64 freestanding cross-compile using clang)
#
# Output: third_party/bearssl/build_x64/libbearssl_x64.a
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BEARSSL_DIR="$ROOT/third_party/bearssl"
CC="clang"
AR="llvm-ar"
OBJDIR="$BEARSSL_DIR/build_x64/obj"
OUTPUT="$BEARSSL_DIR/build_x64/libbearssl_x64.a"

FREESTANDING_INC="$BEARSSL_DIR/freestanding_x64"
CFLAGS="--target=x86_64-unknown-none-elf -ffreestanding -nostdlib -fno-builtin -nostdinc -O2 -w -I$FREESTANDING_INC -I$BEARSSL_DIR/inc -I$BEARSSL_DIR/src"

if [ -f "$OUTPUT" ]; then
    echo "=== BearSSL x64 already built: $OUTPUT ==="
    exit 0
fi

mkdir -p "$OBJDIR"

echo "=== Building BearSSL for anyOS (x86_64) ==="

# Find all .c files in src/
count=0
find "$BEARSSL_DIR/src" -name '*.c' | while read src; do
    name=$(basename "$src" .c)
    obj="$OBJDIR/${name}.o"
    $CC $CFLAGS -c "$src" -o "$obj" 2>&1
    count=$((count + 1))
done

echo "  AR  libbearssl_x64.a"
$AR rcs "$OUTPUT" "$OBJDIR"/*.o

echo "=== Done: $OUTPUT ($(du -h "$OUTPUT" | cut -f1)) ==="

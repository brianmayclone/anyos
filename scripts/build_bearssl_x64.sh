#!/bin/bash
# Build BearSSL for anyOS (x86_64 freestanding cross-compile using clang)
#
# Uses libs/libc64/include for standard headers.
# Output: third_party/bearssl/build_x64/libbearssl_x64.a
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BEARSSL_DIR="$ROOT/third_party/bearssl"
LIBC64_INC="$ROOT/libs/libc64/include"
CC="clang"
AR="/opt/homebrew/Cellar/llvm@20/20.1.8/bin/llvm-ar"
OBJDIR="$BEARSSL_DIR/build_x64/obj"
OUTPUT="$BEARSSL_DIR/build_x64/libbearssl_x64.a"

# Disable HW intrinsics (AES-NI, SSE2, PCLMUL) — software fallbacks used instead.
# Enable BR_64 (64-bit registers) and BR_LE_UNALIGNED (x86 tolerates unaligned).
# Disable RDRAND, /dev/urandom, time — not available in freestanding.
CFLAGS="--target=x86_64-unknown-none-elf -ffreestanding -nostdlib -fno-builtin -nostdinc -O2 -w \
  -I$LIBC64_INC -I$BEARSSL_DIR/inc -I$BEARSSL_DIR/src \
  -DBR_AES_X86NI=0 -DBR_SSE2=0 -DBR_RDRAND=0 \
  -DBR_64=1 -DBR_LE_UNALIGNED=1 \
  -DBR_USE_URANDOM=0 -DBR_USE_UNIX_TIME=0 -DBR_USE_GETENTROPY=0"

if [ -f "$OUTPUT" ]; then
    echo "=== BearSSL x64 already built: $OUTPUT ==="
    exit 0
fi

mkdir -p "$OBJDIR"

echo "=== Building BearSSL for anyOS (x86_64) ==="

# Also compile libc64 stubs
for src in "$ROOT"/libs/libc64/src/*.c; do
    name=$(basename "$src" .c)
    obj="$OBJDIR/libc64_${name}.o"
    $CC $CFLAGS -c "$src" -o "$obj"
done

# Compile all BearSSL .c files
find "$BEARSSL_DIR/src" -name '*.c' | while read src; do
    name=$(basename "$src" .c)
    obj="$OBJDIR/${name}.o"
    $CC $CFLAGS -c "$src" -o "$obj"
done

echo "  AR  libbearssl_x64.a"
$AR rcs "$OUTPUT" "$OBJDIR"/*.o

echo "=== Done: $OUTPUT ($(du -h "$OUTPUT" | cut -f1)) ==="

#!/bin/bash
# Build mini git CLI for anyOS (links against libgit2 + BearSSL)
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
GIT_DIR="$PROJECT_DIR/bin/git"
LG2_DIR="$PROJECT_DIR/third_party/libgit2"
BEARSSL_DIR="$PROJECT_DIR/third_party/bearssl"
LIBC_DIR="$PROJECT_DIR/libs/libc"
OUTPUT="$GIT_DIR/git.elf"

CC=i686-elf-gcc

CFLAGS="-m32 -O2 -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector -fcommon -std=c99 -w"
CFLAGS="$CFLAGS -I$LG2_DIR/include"
CFLAGS="$CFLAGS -I$BEARSSL_DIR/inc"
CFLAGS="$CFLAGS -I$LIBC_DIR/include"

$CC $CFLAGS -c "$GIT_DIR/src/main.c" -o "$GIT_DIR/main.o"
$CC $CFLAGS -c "$GIT_DIR/src/bearssl_stream.c" -o "$GIT_DIR/bearssl_stream.o"

$CC -nostdlib -static -m32 \
    -T "$LIBC_DIR/link.ld" \
    -o "$OUTPUT" \
    "$LIBC_DIR/obj/crt0.o" \
    "$GIT_DIR/main.o" \
    "$GIT_DIR/bearssl_stream.o" \
    "$LG2_DIR/libgit2.a" \
    "$BEARSSL_DIR/build/libbearssl.a" \
    "$LIBC_DIR/libc.a" \
    -lgcc

echo "=== git: $(ls -la "$OUTPUT" | awk '{print $5}') bytes ==="

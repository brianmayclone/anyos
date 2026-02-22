#!/bin/bash
# Build SSH library for anyOS (i686 freestanding cross-compile)
#
# Output: third_party/ssh/build/libssh.a
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SSH_DIR="$ROOT/third_party/ssh"
BEARSSL_DIR="$ROOT/third_party/bearssl"
LIBC_DIR="$ROOT/libs/libc"
CC="i686-elf-gcc"
AR="i686-elf-ar"
OBJDIR="$SSH_DIR/build"
OUTPUT="$SSH_DIR/build/libssh.a"

CFLAGS="-O2 -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector -m32 -std=c99 -w"
CFLAGS="$CFLAGS -I$SSH_DIR/include -I$BEARSSL_DIR/inc -I$LIBC_DIR/include"

mkdir -p "$OBJDIR"

echo "=== Building SSH library for anyOS (i686) ==="

$CC $CFLAGS -c "$SSH_DIR/src/ssh.c" -o "$OBJDIR/ssh.o"

echo "  AR  libssh.a"
$AR rcs "$OUTPUT" "$OBJDIR/ssh.o"

echo "=== Done: $OUTPUT ==="

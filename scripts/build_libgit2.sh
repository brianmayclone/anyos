#!/bin/bash
# Build libgit2 as a static library for anyOS (cross-compiled with i686-elf-gcc)
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
LG2_DIR="$PROJECT_DIR/third_party/libgit2"
LIBC_DIR="$PROJECT_DIR/libs/libc"
OBJ_DIR="$LG2_DIR/obj"
OUTPUT="$LG2_DIR/libgit2.a"

CC=i686-elf-gcc
AR=i686-elf-ar

CFLAGS="-m32 -O2 -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector -fcommon -std=c99 -w"

# Include paths
CFLAGS="$CFLAGS -I$LG2_DIR/include"
CFLAGS="$CFLAGS -I$LG2_DIR/src/libgit2"
CFLAGS="$CFLAGS -I$LG2_DIR/src/util"
CFLAGS="$CFLAGS -I$LG2_DIR/deps/xdiff"
CFLAGS="$CFLAGS -I$LG2_DIR/deps/zlib"
CFLAGS="$CFLAGS -I$LG2_DIR/deps/pcre"
CFLAGS="$CFLAGS -I$LG2_DIR/deps/llhttp"
CFLAGS="$CFLAGS -I$LG2_DIR/src/util/hash"
CFLAGS="$CFLAGS -I$LG2_DIR/src/util/hash/sha1dc"
CFLAGS="$CFLAGS -I$LG2_DIR/src/util/hash/rfc6234"
CFLAGS="$CFLAGS -I$LIBC_DIR/include"

# Defines
CFLAGS="$CFLAGS -DHAVE_STDINT_H -DHAVE_LIMITS_H"
CFLAGS="$CFLAGS -DPCRE_STATIC -DHAVE_CONFIG_H"
CFLAGS="$CFLAGS -DNO_READDIR_R"

mkdir -p "$OBJ_DIR"

# Collect source files
SRCS=""

# Core libgit2
for f in "$LG2_DIR"/src/libgit2/*.c; do
    SRCS="$SRCS $f"
done

# Transports - local + smart (HTTP/HTTPS) + credential
SRCS="$SRCS $LG2_DIR/src/libgit2/transports/local.c"
SRCS="$SRCS $LG2_DIR/src/libgit2/transports/credential.c"
SRCS="$SRCS $LG2_DIR/src/libgit2/transports/credential_helpers.c"
SRCS="$SRCS $LG2_DIR/src/libgit2/transports/smart.c"
SRCS="$SRCS $LG2_DIR/src/libgit2/transports/smart_pkt.c"
SRCS="$SRCS $LG2_DIR/src/libgit2/transports/smart_protocol.c"
SRCS="$SRCS $LG2_DIR/src/libgit2/transports/http.c"
SRCS="$SRCS $LG2_DIR/src/libgit2/transports/httpclient.c"
SRCS="$SRCS $LG2_DIR/src/libgit2/transports/httpparser.c"
SRCS="$SRCS $LG2_DIR/src/libgit2/transports/auth.c"
SRCS="$SRCS $LG2_DIR/src/libgit2/transports/git.c"

# Streams - socket + registry + tls (BearSSL registered at runtime in git CLI)
SRCS="$SRCS $LG2_DIR/src/libgit2/streams/socket.c"
SRCS="$SRCS $LG2_DIR/src/libgit2/streams/registry.c"
SRCS="$SRCS $LG2_DIR/src/libgit2/streams/tls.c"

# Utility layer
for f in "$LG2_DIR"/src/util/*.c; do
    SRCS="$SRCS $f"
done

# Utility - allocators (stdalloc only)
SRCS="$SRCS $LG2_DIR/src/util/allocators/stdalloc.c"

# Utility - hash implementations
SRCS="$SRCS $LG2_DIR/src/util/hash/collisiondetect.c"
SRCS="$SRCS $LG2_DIR/src/util/hash/sha1dc/sha1.c"
SRCS="$SRCS $LG2_DIR/src/util/hash/sha1dc/ubc_check.c"
SRCS="$SRCS $LG2_DIR/src/util/hash/builtin.c"
SRCS="$SRCS $LG2_DIR/src/util/hash/rfc6234/sha224-256.c"

# Utility - unix
SRCS="$SRCS $LG2_DIR/src/util/unix/map.c"
SRCS="$SRCS $LG2_DIR/src/util/unix/realpath.c"

# Deps - zlib
for f in "$LG2_DIR"/deps/zlib/*.c; do
    SRCS="$SRCS $f"
done

# Deps - xdiff
for f in "$LG2_DIR"/deps/xdiff/*.c; do
    SRCS="$SRCS $f"
done

# Deps - pcre (bundled regex)
for f in "$LG2_DIR"/deps/pcre/*.c; do
    SRCS="$SRCS $f"
done

# Deps - llhttp (for GIT_HTTPPARSER_BUILTIN)
for f in "$LG2_DIR"/deps/llhttp/*.c; do
    SRCS="$SRCS $f"
done

# anyOS-specific stubs (networking symbols)
SRCS="$SRCS $LG2_DIR/anyos_stubs.c"

OBJS=""
ERRORS=0
for src in $SRCS; do
    name=$(basename "$src" .c)
    dir=$(dirname "$src" | sed "s|$LG2_DIR/||; s|/|_|g")
    obj="$OBJ_DIR/${dir}_${name}.o"
    shortname="${src#$LG2_DIR/}"
    if $CC $CFLAGS -c "$src" -o "$obj" 2>/dev/null; then
        OBJS="$OBJS $obj"
    else
        # Retry with output for debugging
        if ! $CC $CFLAGS -c "$src" -o "$obj" 2>&1; then
            echo "  FAILED: $shortname" >&2
            ERRORS=$((ERRORS + 1))
        fi
    fi
done

if [ $ERRORS -gt 0 ]; then
    echo "=== libgit2: $ERRORS files failed ===" >&2
fi

$AR rcs "$OUTPUT" $OBJS
echo "=== libgit2: $(echo $OBJS | wc -w | tr -d ' ') objects, $(ls -la "$OUTPUT" | awk '{print $5}') bytes ==="

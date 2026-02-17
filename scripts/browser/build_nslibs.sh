#!/bin/bash
# Build all NetSurf component libraries for anyOS (i686-elf cross-compile)
# Uses the NetSurf buildsystem (make-based)
#
# Output: Each library's .a in its own build-*-release-lib-static/ directory
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
NS="$ROOT/third_party/netsurf"
CC="i686-elf-gcc"
AR="i686-elf-ar"

# Build identifier used by NetSurf buildsystem
BUILD_ID="$(uname -m)-apple-$(uname -s | tr '[:upper:]' '[:lower:]')$(uname -r)-i686-elf"

# Common make flags for all component libraries
MAKE_FLAGS=(
    "HOST_CC=gcc"
    "CC=$CC"
    "AR=$AR"
    "TARGET=i686-elf"
    "BUILD=${BUILD_ID}-release-lib-static"
    "PREFIX=/dev/null"
    "NSSHARED=$NS/buildsystem"
    "WARNFLAGS=-w"
    "OPTCFLAGS=-O2 -ffreestanding -nostdlib -fno-builtin -m32"
    "OPTLDFLAGS=-nostdlib -m32"
    "COMPONENT_TYPE=lib-static"
)

build_lib() {
    local name="$1"
    local component="$2"
    local extra_flags=("${@:3}")
    local dir="$NS/$name"
    local builddir="$dir/build-${BUILD_ID}-release-lib-static"

    if [ -f "$builddir/$component.a" ]; then
        echo "  SKIP  $name ($component.a already exists)"
        return 0
    fi

    echo "  BUILD $name"
    make -C "$dir" \
        "${MAKE_FLAGS[@]}" \
        "COMPONENT=$component" \
        "${extra_flags[@]}" \
        -j4 2>&1 | tail -1
}

echo "=== Building NetSurf component libraries ==="

# Order matters — dependencies first
LIBC_INC="-isystem $ROOT/libs/libc/include"

build_lib libparserutils parserutils \
    "CFLAGS=$LIBC_INC"

build_lib libwapcaplet wapcaplet \
    "CFLAGS=$LIBC_INC"

build_lib libhubbub hubbub \
    "CFLAGS=$LIBC_INC -I$NS/libparserutils/include -DNDEBUG"

build_lib libdom dom \
    "CFLAGS=$LIBC_INC -I$NS/libparserutils/include -I$NS/libwapcaplet/include -I$NS/libhubbub/include"

build_lib libcss css \
    "CFLAGS=$LIBC_INC -I$NS/libparserutils/include -I$NS/libwapcaplet/include"

build_lib libnsutils nsutils \
    "CFLAGS=$LIBC_INC"

build_lib libnslog nslog \
    "CFLAGS=$LIBC_INC"

build_lib libnspsl nspsl \
    "CFLAGS=$LIBC_INC"

build_lib libnsgif nsgif \
    "CFLAGS=$LIBC_INC"

build_lib libnsbmp nsbmp \
    "CFLAGS=$LIBC_INC"

build_lib libnsfb nsfb \
    "CFLAGS=$LIBC_INC -I$NS/libnsutils/include"

echo "=== All component libraries done ==="

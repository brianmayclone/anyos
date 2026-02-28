#!/usr/bin/env bash
# =============================================================================
# build_ladybird.sh — Build Ladybird browser components for anyOS
# =============================================================================
# Usage:
#   ./scripts/build_ladybird.sh [--clean] [--deps] [--ak] [--all]
#
# Phases:
#   --deps    Download and build third-party dependencies (fast_float, simdutf, fmt)
#   --ak      Build AK library (libak.a)
#   --all     Build everything (default)
#   --clean   Remove build artifacts
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Directories
LADYBIRD_DIR="$ROOT_DIR/third_party/ladybird"
AK_DIR="$LADYBIRD_DIR/AK"
DEPS_DIR="$LADYBIRD_DIR/deps"
BUILD_DIR="$ROOT_DIR/build/ladybird"
LIBC64_DIR="$ROOT_DIR/libs/libc64"
LIBCXX_DIR="$ROOT_DIR/libs/libcxx"
LIBUNWIND_DIR="$ROOT_DIR/libs/libunwind"
LIBCXXABI_DIR="$ROOT_DIR/libs/libcxxabi"

# Compiler settings
CXX="${CXX:-clang++}"
CC="${CC:-clang}"
AR="${AR:-ar}"
RANLIB="${RANLIB:-ranlib}"

# Cross-compilation flags
TARGET="x86_64-unknown-none-elf"
COMMON_FLAGS=(
    --target=$TARGET
    -ffreestanding
    -nostdlib
    -O2
    -w
    -isystem "$LIBCXX_DIR/include"
    -isystem "$LIBC64_DIR/include"
    -I"$LIBUNWIND_DIR/include"
    -I"$LIBCXXABI_DIR/include"
)

CXX_FLAGS=(
    "${COMMON_FLAGS[@]}"
    -std=c++23
    -fexceptions
    -frtti
    -include compare
    -include cstdio
    -I"$LADYBIRD_DIR"
    -I"$DEPS_DIR/include"
    -I"$BUILD_DIR/generated"
    -DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64
    -DFMT_HEADER_ONLY=1
    -DFMT_USE_LOCALE=0
)

C_FLAGS=(
    "${COMMON_FLAGS[@]}"
    -std=c11
    -fno-builtin
)

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${CYAN}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[ OK ]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
err()   { echo -e "${RED}[ERR ]${NC} $*"; }

# =============================================================================
# Phase 0: Download third-party dependencies
# =============================================================================
download_deps() {
    info "Downloading third-party dependencies..."
    mkdir -p "$DEPS_DIR/include" "$DEPS_DIR/src"

    # fast_float (header-only)
    if [ ! -f "$DEPS_DIR/include/fast_float/fast_float.h" ]; then
        info "Downloading fast_float 8.1.0..."
        mkdir -p "$DEPS_DIR/include/fast_float"
        curl -sL "https://github.com/fastfloat/fast_float/releases/download/v8.1.0/fast_float.h" \
            -o "$DEPS_DIR/include/fast_float/fast_float.h"
        ok "fast_float downloaded"
    else
        ok "fast_float already present"
    fi

    # simdutf
    if [ ! -f "$DEPS_DIR/include/simdutf.h" ]; then
        info "Downloading simdutf 7.4.0..."
        local SIMDUTF_VER="7.4.0"
        curl -sL "https://github.com/simdutf/simdutf/releases/download/v${SIMDUTF_VER}/singleheader.zip" \
            -o "/tmp/simdutf.zip"
        (cd /tmp && unzip -o simdutf.zip simdutf.h simdutf.cpp 2>/dev/null)
        cp /tmp/simdutf.h "$DEPS_DIR/include/"
        cp /tmp/simdutf.cpp "$DEPS_DIR/src/"
        rm -f /tmp/simdutf.zip /tmp/simdutf.h /tmp/simdutf.cpp
        ok "simdutf downloaded"
    else
        ok "simdutf already present"
    fi

    # fmt (header-only mode)
    if [ ! -f "$DEPS_DIR/include/fmt/format.h" ]; then
        info "Downloading fmt 12.1.0..."
        local FMT_VER="12.1.0"
        curl -sL "https://github.com/fmtlib/fmt/archive/refs/tags/${FMT_VER}.tar.gz" \
            -o "/tmp/fmt.tar.gz"
        (cd /tmp && tar xzf fmt.tar.gz)
        mkdir -p "$DEPS_DIR/include/fmt"
        cp /tmp/fmt-${FMT_VER}/include/fmt/*.h "$DEPS_DIR/include/fmt/"
        # fmt source files for compiled mode
        mkdir -p "$DEPS_DIR/src/fmt"
        cp /tmp/fmt-${FMT_VER}/src/format.cc "$DEPS_DIR/src/fmt/"
        cp /tmp/fmt-${FMT_VER}/src/os.cc "$DEPS_DIR/src/fmt/" 2>/dev/null || true
        rm -rf /tmp/fmt.tar.gz /tmp/fmt-${FMT_VER}
        ok "fmt downloaded"
    else
        ok "fmt already present"
    fi
}

# =============================================================================
# Build third-party dependency libraries
# =============================================================================
build_deps() {
    info "Building third-party dependencies..."
    mkdir -p "$BUILD_DIR/deps"

    # Build simdutf
    if [ ! -f "$DEPS_DIR/libsimdutf.a" ] || [ "$DEPS_DIR/src/simdutf.cpp" -nt "$DEPS_DIR/libsimdutf.a" ]; then
        info "Compiling simdutf (scalar fallback only)..."
        $CXX "${CXX_FLAGS[@]}" \
            -DSIMDUTF_NO_THREADS \
            -DSIMDUTF_IMPLEMENTATION_ICELAKE=0 \
            -DSIMDUTF_IMPLEMENTATION_HASWELL=0 \
            -DSIMDUTF_IMPLEMENTATION_WESTMERE=0 \
            -DSIMDUTF_IMPLEMENTATION_ARM64=0 \
            -DSIMDUTF_IMPLEMENTATION_FALLBACK=1 \
            -I"$DEPS_DIR/include" \
            -c "$DEPS_DIR/src/simdutf.cpp" \
            -o "$BUILD_DIR/deps/simdutf.o"
        $AR rcs "$DEPS_DIR/libsimdutf.a" "$BUILD_DIR/deps/simdutf.o"
        ok "libsimdutf.a built"
    else
        ok "libsimdutf.a up to date"
    fi

    # fmt: used in header-only mode (FMT_HEADER_ONLY=1), no compilation needed
    ok "fmt: header-only mode (FMT_HEADER_ONLY=1)"
}

# =============================================================================
# Generate AK headers that are normally produced by CMake
# =============================================================================
generate_ak_headers() {
    info "Generating AK build headers..."
    mkdir -p "$BUILD_DIR/generated/AK"

    # Backtrace.h — no cpptrace, no backtrace on anyOS
    cat > "$BUILD_DIR/generated/AK/Backtrace.h" << 'EOF'
/* Generated by build_ladybird.sh for anyOS — no backtrace support */
#pragma once
/* Backtrace_FOUND is NOT defined */
EOF

    # Debug.h — all debug flags default to 0
    cat > "$BUILD_DIR/generated/AK/Debug.h" << 'EOF'
/* Generated by build_ladybird.sh for anyOS — all debug flags off */
#pragma once
#define UTF8_DEBUG 0
#define RESOURCE_DEBUG 0
#define SPAM_DEBUG 0
#define JSON_DEBUG 0
#define FORMAT_DEBUG 0
#define REGEX_DEBUG 0
#define URL_DEBUG 0
#define HTML_PARSER_DEBUG 0
#define CSS_PARSER_DEBUG 0
#define TOKENIZER_DEBUG 0
#define LAYOUT_DEBUG 0
#define PAINTING_DEBUG 0
#define JS_BYTECODE_DEBUG 0
#define GC_DEBUG 0
#define HEAP_DEBUG 0
#define CRYPTO_DEBUG 0
#define TLS_DEBUG 0
#define HTTP_DEBUG 0
#define REQUEST_DEBUG 0
#define UNICODE_DEBUG 0
#define IMAGE_DECODER_DEBUG 0
#define PNG_DEBUG 0
#define JPEG_DEBUG 0
#define WOFF_DEBUG 0
#define FONT_DEBUG 0
#define EVENT_DEBUG 0
#define SOCKET_DEBUG 0
#define DNS_DEBUG 0
#define FILE_DEBUG 0
#define TIMER_DEBUG 0
#define PROMISE_DEBUG 0
#define GENERATE_DEBUG 0
#define SYNTAX_HIGHLIGHTING_DEBUG 0
#define LIBWEB_CSS_DEBUG 0
#define LIBWEB_CSS_ANIMATION_DEBUG 0
EOF

    ok "Generated AK/Backtrace.h and AK/Debug.h"
}

# =============================================================================
# Build AK library
# =============================================================================
build_ak() {
    info "Building AK library..."
    mkdir -p "$BUILD_DIR/ak"

    # AK source files (from CMakeLists.txt, non-Windows)
    local AK_SOURCES=(
        Assertions.cpp
        Base64.cpp
        ByteString.cpp
        ByteStringImpl.cpp
        CircularBuffer.cpp
        ConstrainedStream.cpp
        CountingStream.cpp
        Error.cpp
        FlyString.cpp
        Format.cpp
        GenericLexer.cpp
        Hex.cpp
        JsonArray.cpp
        JsonObject.cpp
        JsonParser.cpp
        JsonValue.cpp
        LexicalPath.cpp
        MemoryStream.cpp
        NumberFormat.cpp
        OptionParser.cpp
        Random.cpp
        StackInfo.cpp
        Stream.cpp
        String.cpp
        StringBase.cpp
        StringBuilder.cpp
        StringConversions.cpp
        StringUtils.cpp
        StringView.cpp
        Time.cpp
        Utf16FlyString.cpp
        Utf16String.cpp
        Utf16StringData.cpp
        Utf16View.cpp
        Utf32View.cpp
        Utf8View.cpp
        kmalloc.cpp
    )

    local OBJECTS=()
    local FAILED=0

    for src in "${AK_SOURCES[@]}"; do
        local obj="$BUILD_DIR/ak/${src%.cpp}.o"
        local src_path="$AK_DIR/$src"

        if [ ! -f "$src_path" ]; then
            warn "Source file not found: $src_path"
            continue
        fi

        # Only rebuild if source is newer than object
        if [ -f "$obj" ] && [ "$obj" -nt "$src_path" ]; then
            OBJECTS+=("$obj")
            continue
        fi

        echo -n "  Compiling $src... "
        if $CXX "${CXX_FLAGS[@]}" \
            -c "$src_path" \
            -o "$obj" 2>"$BUILD_DIR/ak/${src%.cpp}.err"; then
            echo -e "${GREEN}OK${NC}"
            OBJECTS+=("$obj")
        else
            echo -e "${RED}FAILED${NC}"
            cat "$BUILD_DIR/ak/${src%.cpp}.err" | head -20
            FAILED=$((FAILED + 1))
        fi
    done

    if [ $FAILED -gt 0 ]; then
        err "$FAILED files failed to compile"
        return 1
    fi

    # Archive
    info "Archiving libak.a (${#OBJECTS[@]} objects)..."
    $AR rcs "$LADYBIRD_DIR/libak.a" "${OBJECTS[@]}"
    ok "libak.a built successfully (${#OBJECTS[@]} objects)"
}

# =============================================================================
# Clean
# =============================================================================
do_clean() {
    info "Cleaning build artifacts..."
    rm -rf "$BUILD_DIR"
    rm -f "$LADYBIRD_DIR/libak.a"
    rm -f "$DEPS_DIR/libsimdutf.a" "$DEPS_DIR/libfmt.a"
    ok "Clean complete"
}

# =============================================================================
# Main
# =============================================================================
BUILD_DEPS=0
BUILD_AK=0
CLEAN=0

if [ $# -eq 0 ]; then
    BUILD_DEPS=1
    BUILD_AK=1
fi

for arg in "$@"; do
    case "$arg" in
        --deps)  BUILD_DEPS=1 ;;
        --ak)    BUILD_AK=1 ;;
        --all)   BUILD_DEPS=1; BUILD_AK=1 ;;
        --clean) CLEAN=1 ;;
        *)       err "Unknown option: $arg"; exit 1 ;;
    esac
done

if [ $CLEAN -eq 1 ]; then
    do_clean
    exit 0
fi

echo "============================================"
echo "  Ladybird for anyOS — Build System"
echo "============================================"

if [ $BUILD_DEPS -eq 1 ]; then
    download_deps
    build_deps
fi

if [ $BUILD_AK -eq 1 ]; then
    generate_ak_headers
    build_ak
fi

echo ""
ok "Build complete!"

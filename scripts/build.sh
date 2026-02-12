#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build anyOS
# Usage: ./build.sh [--clean] 

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${SCRIPT_DIR}/.."
BUILD_DIR="${PROJECT_DIR}/build"

CLEAN=0

for arg in "$@"; do
    case "$arg" in
        --clean)
            CLEAN=1
            ;;
        *)
            echo "Usage: $0 [--clean]"
            echo ""
            echo "  --clean    Force full rebuild of all components"
            exit 1
            ;;
    esac
done

# Ensure build directory exists
if [ ! -f "${BUILD_DIR}/build.ninja" ]; then
    echo "Configuring build..."
    cmake -B "$BUILD_DIR" -G Ninja "$PROJECT_DIR"
fi

# Force full rebuild if --clean
if [ "$CLEAN" -eq 1 ]; then
    echo "Cleaning build..."
    "${SCRIPT_DIR}/clean.sh"
    # Re-configure CMake after clean (build.ninja was deleted)
    echo "Configuring build..."
    cmake -B "$BUILD_DIR" -G Ninja "$PROJECT_DIR"
fi

# Suppress Rust warnings and notes — only show errors
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-Awarnings"

# Build
echo "Building anyOS..."
ninja -C "$BUILD_DIR"
BUILD_RC=$?

if [ $BUILD_RC -ne 0 ]; then
    echo "Build failed!"
    exit $BUILD_RC
fi

echo "Build successful."

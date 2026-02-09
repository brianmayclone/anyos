#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build anyOS
# Usage: ./build.sh [--clean] [--run [--vmware | --std]]

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${SCRIPT_DIR}/.."
BUILD_DIR="${PROJECT_DIR}/build"

CLEAN=0
RUN=0
VGA_FLAG=""

for arg in "$@"; do
    case "$arg" in
        --clean)
            CLEAN=1
            ;;
        --run)
            RUN=1
            ;;
        --vmware)
            VGA_FLAG="--vmware"
            ;;
        --std)
            VGA_FLAG="--std"
            ;;
        *)
            echo "Usage: $0 [--clean] [--run [--vmware | --std]]"
            echo ""
            echo "  --clean    Force full kernel rebuild"
            echo "  --run      Launch QEMU after build"
            echo "  --vmware   Use VMware SVGA II (2D accel, HW cursor)"
            echo "  --std      Use Bochs VGA (double-buffering) [default]"
            exit 1
            ;;
    esac
done

# Ensure build directory exists
if [ ! -f "${BUILD_DIR}/build.ninja" ]; then
    echo "Configuring build..."
    cmake -B "$BUILD_DIR" -G Ninja "$PROJECT_DIR"
fi

# Force kernel rebuild if --clean
if [ "$CLEAN" -eq 1 ]; then
    echo "Cleaning kernel build artifacts..."
    rm -rf "${BUILD_DIR}/kernel/x86_64-anyos/debug/anyos_kernel.elf" \
           "${BUILD_DIR}/kernel/x86_64-anyos/debug/.fingerprint/anyos_kernel-"* \
           "${BUILD_DIR}/kernel/x86_64-anyos/debug/incremental/anyos_kernel-"* \
           2>/dev/null
fi

# Build
echo "Building anyOS..."
ninja -C "$BUILD_DIR"
BUILD_RC=$?

if [ $BUILD_RC -ne 0 ]; then
    echo "Build failed!"
    exit $BUILD_RC
fi

echo "Build successful."

# Run if requested
if [ "$RUN" -eq 1 ]; then
    exec "${SCRIPT_DIR}/run.sh" $VGA_FLAG
fi

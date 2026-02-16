#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build anyOS
# Usage: ./build.sh [--clean] [--uefi] [--iso] [--all]

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${SCRIPT_DIR}/.."
BUILD_DIR="${PROJECT_DIR}/build"

CLEAN=0
BUILD_UEFI=0
BUILD_ISO=0
BUILD_ALL=0

for arg in "$@"; do
    case "$arg" in
        --clean)
            CLEAN=1
            ;;
        --uefi)
            BUILD_UEFI=1
            ;;
        --iso)
            BUILD_ISO=1
            ;;
        --all)
            BUILD_ALL=1
            ;;
        *)
            echo "Usage: $0 [--clean] [--uefi] [--iso] [--all]"
            echo ""
            echo "  --clean    Force full rebuild of all components"
            echo "  --uefi     Build UEFI disk image (in addition to BIOS)"
            echo "  --iso      Build bootable ISO 9660 image (El Torito, CD-ROM)"
            echo "  --all      Build BIOS, UEFI, and ISO images"
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

# Build BIOS image (default target)
echo "Building anyOS (BIOS)..."
ninja -C "$BUILD_DIR"
BUILD_RC=$?

if [ $BUILD_RC -ne 0 ]; then
    echo "BIOS build failed!"
    exit $BUILD_RC
fi

echo "BIOS build successful."

# Build UEFI image if requested
if [ "$BUILD_UEFI" -eq 1 ] || [ "$BUILD_ALL" -eq 1 ]; then
    echo "Building anyOS (UEFI)..."
    ninja -C "$BUILD_DIR" uefi-image
    UEFI_RC=$?

    if [ $UEFI_RC -ne 0 ]; then
        echo "UEFI build failed!"
        exit $UEFI_RC
    fi

    echo "UEFI build successful."
fi

# Build ISO image if requested
if [ "$BUILD_ISO" -eq 1 ] || [ "$BUILD_ALL" -eq 1 ]; then
    echo "Building anyOS (ISO 9660, El Torito)..."
    ninja -C "$BUILD_DIR" iso
    ISO_RC=$?

    if [ $ISO_RC -ne 0 ]; then
        echo "ISO build failed!"
        exit $ISO_RC
    fi

    echo "ISO build successful: ${BUILD_DIR}/anyos.iso"
fi

echo "Build complete."

#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Clean anyOS build artifacts
# Usage: ./clean.sh [--all]
#
#   (no args)  Remove all build artifacts (kernel, stdlib, DLLs, shared libs,
#              user/system programs, libc, TCC, buildsystem tools, sysroot, image).
#              Preserves CMake cache — just run: ninja -C build
#   --all      Remove entire build directory (requires re-running CMake + ninja)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${SCRIPT_DIR}/.."
BUILD_DIR="${PROJECT_DIR}/build"

if [ ! -d "$BUILD_DIR" ]; then
    echo "Nothing to clean (no build directory)"
    exit 0
fi

if [ "$1" = "--all" ]; then
    echo "Removing entire build directory..."
    rm -rf "$BUILD_DIR"
    # Also clean libc build artifacts in source tree
    make -C "${PROJECT_DIR}/libs/libc" clean 2>/dev/null
    echo "Done. Run: cmake -B build -G Ninja && ninja -C build"
    exit 0
fi

echo "Cleaning build artifacts..."

# Kernel
echo "  Kernel..."
rm -rf "${BUILD_DIR}/kernel" 2>/dev/null

# Stdlib (Cargo build artifacts are inside program dirs, but also shared via target)
echo "  Stdlib..."
# Stdlib is built as part of each program's Cargo build, no separate dir

# DLLs (uisys, libcompositor, libimage, librender)
echo "  DLLs..."
rm -rf "${BUILD_DIR}/dll" 2>/dev/null

# Shared libraries (.so — libanyui, libfont, libdb)
echo "  Shared libraries..."
rm -rf "${BUILD_DIR}/shlib" 2>/dev/null

# User programs (in build/programs/<name>/)
echo "  User programs..."
rm -rf "${BUILD_DIR}/programs" 2>/dev/null

# Buildsystem tools (anyelf, anyld, mkimage, mkappbundle)
echo "  Buildsystem tools..."
rm -rf "${BUILD_DIR}/buildsystem" 2>/dev/null

# Libc (built in source tree via Makefile)
echo "  Libc..."
make -C "${PROJECT_DIR}/libs/libc" clean 2>/dev/null

# TCC object
echo "  TCC..."
rm -f "${BUILD_DIR}/tcc.o" 2>/dev/null
rm -f "${BUILD_DIR}/tcc.elf" 2>/dev/null

# Sysroot (regenerated from build outputs)
echo "  Sysroot..."
rm -rf "${BUILD_DIR}/sysroot" 2>/dev/null

# Disk images
echo "  Disk images..."
rm -f "${BUILD_DIR}/anyos.img" 2>/dev/null
rm -f "${BUILD_DIR}/anyos-uefi.img" 2>/dev/null
rm -f "${BUILD_DIR}/anyos.iso" 2>/dev/null

# Flat binaries
echo "  Flat binaries..."
rm -f "${BUILD_DIR}/anyos_kernel.bin" 2>/dev/null

echo "Done. Run: ninja -C build"

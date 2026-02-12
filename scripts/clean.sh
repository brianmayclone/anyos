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
#   (no args)  Remove all Cargo/program build artifacts (forces full rebuild
#              of kernel, DLLs, user programs, system programs, libc, TCC)
#   --all      Remove entire build directory (requires re-running CMake)

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
    make -C "${PROJECT_DIR}/programs/libc" clean 2>/dev/null
    echo "Done. Run ./scripts/build.sh to rebuild from scratch."
    exit 0
fi

echo "Cleaning build artifacts..."

# Kernel
echo "  Kernel..."
rm -rf "${BUILD_DIR}/kernel/x86_64-anyos" 2>/dev/null

# DLLs (e.g. uisys)
echo "  DLLs..."
rm -rf "${BUILD_DIR}/dll" 2>/dev/null

# User programs (in build/programs/<name>/)
echo "  User programs..."
for dir in "${BUILD_DIR}/programs"/*/; do
    [ -d "$dir" ] && rm -rf "${dir}x86_64-anyos-user" 2>/dev/null
    [ -d "$dir" ] && rm -rf "${dir}debug" 2>/dev/null
done

# System programs (in build/programs/compositor/ etc.)
echo "  System programs..."
for dir in "${BUILD_DIR}/programs"/*/; do
    [ -d "$dir" ] && rm -rf "${dir}x86_64-anyos-user" 2>/dev/null
    [ -d "$dir" ] && rm -rf "${dir}debug" 2>/dev/null
    # Nested dirs (e.g. programs/compositor/dock/)
    for subdir in "${dir}"*/; do
        [ -d "$subdir" ] && rm -rf "${subdir}x86_64-anyos-user" 2>/dev/null
        [ -d "$subdir" ] && rm -rf "${subdir}debug" 2>/dev/null
    done
done

# Libc (built in source tree)
echo "  Libc..."
make -C "${PROJECT_DIR}/programs/libc" clean 2>/dev/null

# TCC object
echo "  TCC..."
rm -f "${BUILD_DIR}/tcc.o" 2>/dev/null
rm -f "${BUILD_DIR}/tcc.elf" 2>/dev/null

# Sysroot (regenerated from build outputs)
echo "  Sysroot..."
rm -rf "${BUILD_DIR}/sysroot" 2>/dev/null

# Disk image
echo "  Disk image..."
rm -f "${BUILD_DIR}/anyos.img" 2>/dev/null

# Flat binaries
echo "  Flat binaries..."
rm -f "${BUILD_DIR}/anyos_kernel.bin" 2>/dev/null

echo "Done. Run ./scripts/build.sh to rebuild."

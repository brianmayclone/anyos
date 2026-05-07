#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build anyOS
# Usage: ./build.sh [--clean] [--reset] [--uefi] [--iso] [--all] [--debug] [--no-cross]
#                   [--iminor] [--imajor] [--nover] [--arm64] [--image-size SIZE]

BUILD_START=$(date +%s)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${SCRIPT_DIR}/.."
BUILD_ROOT="${PROJECT_DIR}/build"

CLEAN=0
RESET=0
BUILD_UEFI=0
BUILD_ISO=0
BUILD_ALL=0
DEBUG_VERBOSE=0
DEBUG_SURF=0
NO_CROSS=0
VER_MODE="patch"
ANYOS_ARCH="x86_64"
IMAGE_SIZE_MIB="512"
# Empty unless --system-fs / --system-fs-size is explicitly given.
# Forwarded to CMake as -DANYOS_SYSTEM_FS / -DANYOS_SYSTEM_FS_SIZE_MIB.
SYSTEM_FS=""
SYSTEM_FS_SIZE_MIB=""
CMAKE_PASSTHROUGH=()

parse_image_size_mib() {
    local raw="$1"
    local num unit unit_lc

    if [[ ! "$raw" =~ ^([0-9]+)([mMgG]([iI]?[bB])?)?$ ]]; then
        return 1
    fi

    num=$((10#${BASH_REMATCH[1]}))
    unit="${BASH_REMATCH[2]}"
    unit_lc="${unit,,}"
    if [ "$num" -le 0 ]; then
        return 1
    fi

    case "$unit_lc" in
        ""|m|mb|mib)
            echo "$num"
            ;;
        g|gb|gib)
            echo $((num * 1024))
            ;;
        *)
            return 1
            ;;
    esac
}

i=0
argv=("$@")
while [ $i -lt ${#argv[@]} ]; do
    arg="${argv[$i]}"
    case "$arg" in
        --clean)
            CLEAN=1
            ;;
        --reset)
            RESET=1
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
        --debug)
            DEBUG_VERBOSE=1
            ;;
        --debug-surf)
            DEBUG_SURF=1
            ;;
        --no-cross)
            NO_CROSS=1
            ;;
        --iminor)
            VER_MODE="minor"
            ;;
        --imajor)
            VER_MODE="major"
            ;;
        --nover)
            VER_MODE="none"
            ;;
        --arm64)
            ANYOS_ARCH="arm64"
            ;;
        --image-size)
            i=$((i + 1))
            next="${argv[$i]:-}"
            if IMAGE_SIZE_PARSED=$(parse_image_size_mib "$next"); then
                IMAGE_SIZE_MIB="$IMAGE_SIZE_PARSED"
            else
                echo "Error: --image-size expects a positive size like 512M, 1024M, 5G, or 10G; got '$next'"
                exit 1
            fi
            ;;
        --image-size=*)
            val="${arg#--image-size=}"
            if IMAGE_SIZE_PARSED=$(parse_image_size_mib "$val"); then
                IMAGE_SIZE_MIB="$IMAGE_SIZE_PARSED"
            else
                echo "Error: --image-size expects a positive size like 512M, 1024M, 5G, or 10G; got '$val'"
                exit 1
            fi
            ;;
        --system-fs)
            i=$((i + 1))
            next="${argv[$i]:-}"
            case "$next" in
                exfat|corefs)
                    SYSTEM_FS="$next"
                    ;;
                *)
                    echo "Error: --system-fs expects 'exfat' or 'corefs', got '$next'"
                    exit 1
                    ;;
            esac
            ;;
        --system-fs=*)
            val="${arg#--system-fs=}"
            case "$val" in
                exfat|corefs)
                    SYSTEM_FS="$val"
                    ;;
                *)
                    echo "Error: --system-fs expects 'exfat' or 'corefs', got '$val'"
                    exit 1
                    ;;
            esac
            ;;
        --system-fs-size)
            i=$((i + 1))
            next="${argv[$i]:-}"
            if [[ "$next" =~ ^[0-9]+$ ]] && [ "$next" -gt 0 ]; then
                SYSTEM_FS_SIZE_MIB="$next"
            else
                echo "Error: --system-fs-size expects a positive integer (MiB), got '$next'"
                exit 1
            fi
            ;;
        --system-fs-size=*)
            val="${arg#--system-fs-size=}"
            if [[ "$val" =~ ^[0-9]+$ ]] && [ "$val" -gt 0 ]; then
                SYSTEM_FS_SIZE_MIB="$val"
            else
                echo "Error: --system-fs-size expects a positive integer (MiB), got '$val'"
                exit 1
            fi
            ;;
        -D*)
            # Transparent CMake pass-through — allow any -D<VAR>=<VAL>
            # flag to be forwarded to cmake without build.sh having to
            # know about it.  Example: ./build.sh -DANYOS_SYSTEM_FS=corefs
            CMAKE_PASSTHROUGH+=("$arg")
            ;;
        *)
            echo "Usage: $0 [--clean] [--reset] [--uefi] [--iso] [--all] [--debug] [--debug-surf] [--no-cross]"
            echo "       [--iminor] [--imajor] [--nover] [--arm64] [--image-size SIZE]"
            echo "       [--system-fs {exfat|corefs}] [--system-fs-size <MiB>]"
            echo "       [-D<VAR>=<VAL> ...]"
            echo ""
            echo "  --clean       Force full rebuild of all components"
            echo "  --reset       Force fresh disk image (destroy runtime data)"
            echo "  --uefi        Build UEFI disk image (in addition to BIOS)"
            echo "  --iso         Build bootable ISO 9660 image (El Torito, CD-ROM)"
            echo "  --all         Build BIOS, UEFI, and ISO images"
            echo "  --debug       Enable verbose kernel debug prints"
            echo "  --debug-surf  Enable Surf browser debug logging (HTML/CSS/JS pipeline)"
            echo "  --no-cross    Disable cross-compilation (skip libc, TCC, games, curl)"
            echo "  --iminor      Increment minor version (reset patch to 0)"
            echo "  --imajor      Increment major version (reset minor and patch to 0)"
            echo "  --nover       Skip version increment"
            echo "  --arm64       Build for AArch64 (ARM64) instead of x86_64"
            echo "  --image-size  Disk image size: bare/M/MiB = MiB, G/GiB = GiB (default 512M)"
            echo "  --system-fs   System filesystem: exfat (default) or corefs"
            echo "                (adds CoreFS partition at the end of the disk image)"
            echo "  --system-fs-size  Size of the CoreFS partition in MiB (default 128)"
            echo "  -D<VAR>=<VAL>    Pass additional CMake variables through"
            exit 1
            ;;
    esac
    i=$((i + 1))
done

if [ "$ANYOS_ARCH" = "arm64" ]; then
    BUILD_DIR="${BUILD_ROOT}/arm64"
else
    BUILD_DIR="${BUILD_ROOT}"
fi

# ── Version management ──────────────────────────────────────────────────
VERSION_FILE="${PROJECT_DIR}/VERSION"
if [ ! -f "$VERSION_FILE" ]; then
    echo "0.1.0" > "$VERSION_FILE"
fi

CURRENT_VERSION=$(tr -d '[:space:]' < "$VERSION_FILE")
IFS='.' read -r V_MAJOR V_MINOR V_PATCH <<< "$CURRENT_VERSION"

if [ "$VER_MODE" = "none" ]; then
    # --nover: keep current version
    export ANYOS_VERSION="${CURRENT_VERSION}"
else
    # Auto-detect: only increment if there are uncommitted git changes or
    # if explicitly requesting a minor/major bump.
    # For "patch" mode, skip the increment when there are no staged/unstaged changes.
    HAS_CHANGES=0
    if [ "$VER_MODE" = "patch" ]; then
        if git -C "$PROJECT_DIR" diff --quiet HEAD 2>/dev/null && \
           git -C "$PROJECT_DIR" diff --cached --quiet 2>/dev/null; then
            HAS_CHANGES=0
        else
            HAS_CHANGES=1
        fi
    else
        # minor/major always bump
        HAS_CHANGES=1
    fi

    if [ "$HAS_CHANGES" -eq 1 ]; then
        case "$VER_MODE" in
            patch)
                V_PATCH=$((V_PATCH + 1))
                ;;
            minor)
                V_MINOR=$((V_MINOR + 1))
                V_PATCH=0
                ;;
            major)
                V_MAJOR=$((V_MAJOR + 1))
                V_MINOR=0
                V_PATCH=0
                ;;
        esac
        export ANYOS_VERSION="${V_MAJOR}.${V_MINOR}.${V_PATCH}"
        echo "${ANYOS_VERSION}" > "$VERSION_FILE"
    else
        export ANYOS_VERSION="${CURRENT_VERSION}"
    fi
fi
echo "Version: ${ANYOS_VERSION}"

# CMake flags
CMAKE_EXTRA_FLAGS="-DANYOS_DEBUG_VERBOSE=$([ "$DEBUG_VERBOSE" -eq 1 ] && echo ON || echo OFF) -DANYOS_DEBUG_SURF=$([ "$DEBUG_SURF" -eq 1 ] && echo ON || echo OFF) -DANYOS_RESET=$([ "$RESET" -eq 1 ] && echo ON || echo OFF) -DANYOS_VERSION=${ANYOS_VERSION} -DANYOS_ARCH=${ANYOS_ARCH} -DANYOS_IMAGE_SIZE_MIB=${IMAGE_SIZE_MIB}"

# System-filesystem switches — only appended when the user opted in, so
# existing builds without the flag keep using whatever value is currently
# in the CMake cache (typically the default "exfat").
if [ -n "$SYSTEM_FS" ]; then
    CMAKE_EXTRA_FLAGS="$CMAKE_EXTRA_FLAGS -DANYOS_SYSTEM_FS=${SYSTEM_FS}"
fi
if [ -n "$SYSTEM_FS_SIZE_MIB" ]; then
    CMAKE_EXTRA_FLAGS="$CMAKE_EXTRA_FLAGS -DANYOS_SYSTEM_FS_SIZE_MIB=${SYSTEM_FS_SIZE_MIB}"
fi

# Generic -D<var>=<val> pass-through (collected above).
for def in "${CMAKE_PASSTHROUGH[@]}"; do
    CMAKE_EXTRA_FLAGS="$CMAKE_EXTRA_FLAGS $def"
done

# Ensure build directory exists
if [ ! -f "${BUILD_DIR}/build.ninja" ]; then
    echo "Configuring build..."
    cmake -B "$BUILD_DIR" -G Ninja $CMAKE_EXTRA_FLAGS "$PROJECT_DIR"
fi

# Force full rebuild if --clean
if [ "$CLEAN" -eq 1 ]; then
    echo "Cleaning build..."
    rm -rf "$BUILD_DIR"
    # Re-configure CMake after clean (entire build dir was removed)
    echo "Configuring build..."
    cmake -B "$BUILD_DIR" -G Ninja $CMAKE_EXTRA_FLAGS "$PROJECT_DIR"
fi

# Always re-run cmake to pick up flag changes (fast if nothing changed)
cmake -B "$BUILD_DIR" -G Ninja $CMAKE_EXTRA_FLAGS "$PROJECT_DIR" > /dev/null 2>&1

# Suppress Rust warnings and notes — only show errors
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-Awarnings"

if [ "$ANYOS_ARCH" = "arm64" ]; then
    # ── ARM64 build (kernel + disk image) ─────────────────────────────
    echo "Building anyOS (ARM64)..."

    # Build kernel
    echo "  Building ARM64 kernel..."
    ninja -C "$BUILD_DIR" kernel-arm64
    BUILD_RC=$?
    if [ $BUILD_RC -ne 0 ]; then
        echo "ARM64 kernel build failed!"
        exit $BUILD_RC
    fi

    # Build ARM64 disk image (mkimage --arm64 with sysroot)
    echo "  Building ARM64 disk image..."
    ninja -C "$BUILD_DIR" arm64-image
    BUILD_RC=$?
    if [ $BUILD_RC -ne 0 ]; then
        echo "ARM64 disk image build failed!"
        echo "(This is OK if userspace programs haven't been built yet)"
    fi

    echo "ARM64 build successful."
else
    # ── x86_64 build ────────────────────────────────────────────────────

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
fi

ELAPSED=$(( $(date +%s) - BUILD_START ))
printf "Build complete in %02d:%02d\n" $((ELAPSED / 60)) $((ELAPSED % 60))

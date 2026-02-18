#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Convert anyOS disk image to VMDK for VirtualBox
# Usage: ./scripts/convert_vmdk.sh [--out path/to/anyos.vmdk]

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${SCRIPT_DIR}/.."
BUILD_DIR="${PROJECT_DIR}/build"
IMG_PATH="${BUILD_DIR}/anyos.img"
VMDK_PATH="${BUILD_DIR}/anyos.vmdk"

for arg in "$@"; do
    case "$arg" in
        --out)
            shift; VMDK_PATH="$1"; shift ;;
        --out=*)
            VMDK_PATH="${arg#--out=}" ;;
        *)
            echo "Usage: $0 [--out <path/to/anyos.vmdk>]"
            exit 1 ;;
    esac
done

# Verify the raw image exists
if [ ! -f "$IMG_PATH" ]; then
    echo "ERROR: Disk image not found: $IMG_PATH"
    echo "Run ./scripts/build.sh first."
    exit 1
fi

# Locate VBoxManage
if ! command -v VBoxManage &>/dev/null; then
    echo "ERROR: VBoxManage not found. Install VirtualBox from https://www.virtualbox.org"
    exit 1
fi

# Remove existing VMDK so VBoxManage doesn't refuse to overwrite
rm -f "$VMDK_PATH"

echo "Converting $IMG_PATH -> $VMDK_PATH ..."
VBoxManage convertfromraw "$IMG_PATH" "$VMDK_PATH" --format VMDK

echo "Done: $VMDK_PATH"
echo ""
echo "To use in VirtualBox:"
echo "  1. Create a new VM (Type: Other, Version: Other/Unknown 64-bit)"
echo "  2. Under Storage, add the VMDK as an existing hard disk"
echo "  3. Set firmware to BIOS and boot from disk"

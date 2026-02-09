#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Run anyOS in QEMU
# Usage: ./run.sh [--vmware | --std] [--ahci]
#   --vmware   VMware SVGA II (2D acceleration, HW cursor)
#   --std      Bochs VGA / Standard VGA (double-buffering, no accel) [default]
#   --ahci     Use AHCI (SATA DMA) instead of legacy IDE for disk I/O

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
IMAGE="${SCRIPT_DIR}/../build/anyos.img"

if [ ! -f "$IMAGE" ]; then
    echo "Error: Disk image not found at $IMAGE"
    echo "Run: ./scripts/build.sh first"
    exit 1
fi

VGA="std"
VGA_LABEL="Bochs VGA (standard)"
DRIVE_FLAGS="-drive format=raw,file=\"$IMAGE\""
DRIVE_LABEL="IDE (PIO)"

for arg in "$@"; do
    case "$arg" in
        --vmware)
            VGA="vmware"
            VGA_LABEL="VMware SVGA II (accelerated)"
            ;;
        --std)
            VGA="std"
            VGA_LABEL="Bochs VGA (standard)"
            ;;
        --ahci)
            DRIVE_FLAGS="-drive id=hd0,if=none,format=raw,file=\"$IMAGE\" -device ich9-ahci,id=ahci -device ide-hd,drive=hd0,bus=ahci.0"
            DRIVE_LABEL="AHCI (DMA)"
            ;;
        *)
            echo "Usage: $0 [--vmware | --std] [--ahci]"
            exit 1
            ;;
    esac
done

echo "Starting anyOS with $VGA_LABEL (-vga $VGA), disk: $DRIVE_LABEL"

eval qemu-system-x86_64 \
    $DRIVE_FLAGS \
    -m 128M \
    -smp cpus=4 \
    -serial stdio \
    -vga "$VGA" \
    -netdev user,id=net0 -device e1000,netdev=net0 \
    -no-reboot \
    -no-shutdown

#!/bin/bash
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
IMAGE="${SCRIPT_DIR}/../build/anyos.img"

if [ ! -f "$IMAGE" ]; then
    echo "Error: Disk image not found at $IMAGE"
    echo "Run: cmake -B build -G Ninja && cmake --build build"
    exit 1
fi

qemu-system-i386 \
    -drive format=raw,file="$IMAGE" \
    -m 128M \
    -serial stdio \
    -vga std \
    -no-reboot \
    -no-shutdown

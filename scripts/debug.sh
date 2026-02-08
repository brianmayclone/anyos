#!/bin/bash
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
IMAGE="${SCRIPT_DIR}/../build/anyos.img"

if [ ! -f "$IMAGE" ]; then
    echo "Error: Disk image not found at $IMAGE"
    echo "Run: cmake -B build -G Ninja && cmake --build build"
    exit 1
fi

echo "Starting QEMU in debug mode..."
echo "Connect GDB with: gdb -ex 'target remote :1234' -ex 'symbol-file build/kernel/i686-anyos/debug/anyos_kernel'"

qemu-system-i386 \
    -drive format=raw,file="$IMAGE" \
    -m 128M \
    -serial stdio \
    -vga std \
    -s -S \
    -no-reboot \
    -no-shutdown

#!/bin/bash
# Run anyOS in QEMU
# Usage: ./run.sh [--vmware | --std]
#   --vmware   VMware SVGA II (2D acceleration, HW cursor)
#   --std      Bochs VGA / Standard VGA (double-buffering, no accel) [default]

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
IMAGE="${SCRIPT_DIR}/../build/anyos.img"

if [ ! -f "$IMAGE" ]; then
    echo "Error: Disk image not found at $IMAGE"
    echo "Run: ./scripts/build.sh first"
    exit 1
fi

VGA="std"
VGA_LABEL="Bochs VGA (standard)"

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
        *)
            echo "Usage: $0 [--vmware | --std]"
            exit 1
            ;;
    esac
done

echo "Starting anyOS with $VGA_LABEL (-vga $VGA)"

qemu-system-i386 \
    -drive format=raw,file="$IMAGE" \
    -m 128M \
    -smp cpus=4 \
    -serial stdio \
    -vga "$VGA" \
    -netdev user,id=net0 -device e1000,netdev=net0 \
    -no-reboot \
    -no-shutdown

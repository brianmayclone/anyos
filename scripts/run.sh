#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Run anyOS in QEMU
# Usage: ./run.sh [--vmware | --std | --virtio] [--ide] [--audio] [--usb] [--uefi]
#   --vmware   VMware SVGA II (2D acceleration, HW cursor)
#   --std      Bochs VGA / Standard VGA (double-buffering, no accel) [default]
#   --virtio   VirtIO GPU (modern transport, ARGB cursor)
#   --ide      Use legacy IDE (PIO) instead of AHCI (DMA) for disk I/O
#   --audio    Enable AC'97 audio device
#   --usb      Enable USB controller with keyboard + mouse devices
#   --uefi     Boot via UEFI (OVMF) instead of BIOS

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

UEFI_MODE=false
VGA="std"
VGA_LABEL="Bochs VGA (standard)"
IDE_MODE=false
AUDIO_FLAGS=""
AUDIO_LABEL=""
USB_FLAGS=""
USB_LABEL=""

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
        --virtio)
            VGA="virtio"
            VGA_LABEL="Virtio GPU (paravirtualized)"
            ;;
        --ide)
            IDE_MODE=true
            ;;
        --audio)
            AUDIO_FLAGS="-device AC97,audiodev=audio0 -audiodev coreaudio,id=audio0"
            AUDIO_LABEL=", audio: AC'97"
            ;;
        --usb)
            USB_FLAGS="-usb -device usb-kbd -device usb-mouse"
            USB_LABEL=", USB: keyboard + mouse"
            ;;
        --uefi)
            UEFI_MODE=true
            ;;
        *)
            echo "Usage: $0 [--vmware | --std | --virtio] [--ide] [--audio] [--usb] [--uefi]"
            exit 1
            ;;
    esac
done

if [ "$UEFI_MODE" = true ]; then
    IMAGE="${SCRIPT_DIR}/../build/anyos-uefi.img"
    OVMF_FW="/opt/homebrew/share/qemu/edk2-x86_64-code.fd"
    BIOS_FLAGS="-drive if=pflash,format=raw,readonly=on,file=$OVMF_FW"
    DRIVE_FLAGS="-drive format=raw,file=\"$IMAGE\""
    DRIVE_LABEL="UEFI (GPT)"

    if [ ! -f "$OVMF_FW" ]; then
        echo "Error: OVMF firmware not found at $OVMF_FW"
        echo "Install with: brew install qemu"
        exit 1
    fi
else
    IMAGE="${SCRIPT_DIR}/../build/anyos.img"
    BIOS_FLAGS=""
    if [ "$IDE_MODE" = true ]; then
        DRIVE_FLAGS="-drive format=raw,file=\"$IMAGE\""
        DRIVE_LABEL="IDE (PIO)"
    else
        DRIVE_FLAGS="-drive id=hd0,if=none,format=raw,file=\"$IMAGE\" -device ich9-ahci,id=ahci -device ide-hd,drive=hd0,bus=ahci.0"
        DRIVE_LABEL="AHCI (DMA)"
    fi
fi

if [ ! -f "$IMAGE" ]; then
    echo "Error: Disk image not found at $IMAGE"
    echo "Run: ./scripts/build.sh first"
    exit 1
fi

echo "Starting anyOS with $VGA_LABEL (-vga $VGA), disk: $DRIVE_LABEL$AUDIO_LABEL$USB_LABEL"

eval qemu-system-x86_64 \
    $BIOS_FLAGS \
    $DRIVE_FLAGS \
    -m 1024M \
    -smp cpus=4 \
    -serial stdio \
    -vga "$VGA" \
    -netdev user,id=net0 -device e1000,netdev=net0 \
    $AUDIO_FLAGS \
    $USB_FLAGS \
    -no-reboot \
    -no-shutdown
    

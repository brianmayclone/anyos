#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Run anyOS in QEMU (or VMware Workstation)
# Usage: ./run.sh [--vmware | --std | --virtio] [--res WxH] [--ide] [--cdrom] [--audio] [--usb] [--uefi] [--kvm] [--kbd LAYOUT] [--fwd HOST:GUEST ...] [--vmware-ws]
#   --vmware-ws Start VMware Workstation VM named 'anyos' and stream its COM1 serial output
#   --vmware   VMware SVGA II (2D acceleration, HW cursor)
#   --std      Bochs VGA / Standard VGA (double-buffering, no accel) [default]
#   --virtio   VirtIO GPU (modern transport, ARGB cursor)
#   --res WxH  Set initial GPU resolution (VirtIO only). Example: --res 1280x1024
#   --ide      Use legacy IDE (PIO) instead of AHCI (DMA) for disk I/O
#   --cdrom    Boot from ISO image (CD-ROM) instead of hard drive
#   --audio    Enable AC'97 audio device
#   --usb      Enable USB controller with keyboard + mouse devices
#   --uefi     Boot via UEFI (OVMF) instead of BIOS
#   --kvm      Enable hardware virtualization (KVM on Linux, HVF on macOS)
#   --kbd LAY  Set keyboard layout: us, de, ch, fr, pl (default: keep current)
#   --fwd H:G  Forward host port H to guest port G (TCP). Repeatable.
#              Example: --fwd 2222:22 --fwd 8080:8080

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

UEFI_MODE=false
CDROM_MODE=false
VGA="std"
VGA_LABEL="Bochs VGA (standard)"
IDE_MODE=false
AUDIO_FLAGS=""
AUDIO_LABEL=""
USB_FLAGS=""
USB_LABEL=""
KVM_FLAGS=""
KVM_LABEL=""
# Expose SSE3/SSSE3/SSE4.1/SSE4.2/POPCNT by default; cleared when --kvm uses -cpu host.
CPU_FLAGS="-cpu qemu64,+sse3,+ssse3,+sse4.1,+sse4.2,+popcnt"
FWD_RULES=""
EXPECT_FWD=false
RESOLUTION=""
EXPECT_RES=false
KBD_LAYOUT=""
EXPECT_KBD=false
VMWARE_WS=false
MIN_RES_W=1024
MIN_RES_H=768

for arg in "$@"; do
    if [ "$EXPECT_KBD" = true ]; then
        EXPECT_KBD=false
        case "$arg" in
            us|US)   KBD_LAYOUT=0 ;;
            de|DE)   KBD_LAYOUT=1 ;;
            ch|CH)   KBD_LAYOUT=2 ;;
            fr|FR)   KBD_LAYOUT=3 ;;
            pl|PL)   KBD_LAYOUT=4 ;;
            *)
                echo "Error: Unknown keyboard layout '$arg'. Available: us, de, ch, fr, pl"
                exit 1
                ;;
        esac
        continue
    fi

    if [ "$EXPECT_RES" = true ]; then
        EXPECT_RES=false
        # Validate format: WIDTHxHEIGHT (both numeric)
        case "$arg" in
            *x*)
                RES_W="${arg%%x*}"
                RES_H="${arg#*x}"
                if [ -n "$RES_W" ] && [ -n "$RES_H" ] && [ "$RES_W" -gt 0 ] 2>/dev/null && [ "$RES_H" -gt 0 ] 2>/dev/null; then
                    RESOLUTION="${RES_W}x${RES_H}"
                else
                    echo "Error: Invalid --res format '$arg'. Expected WIDTHxHEIGHT (e.g. 1280x1024)"
                    exit 1
                fi
                ;;
            *)
                echo "Error: Invalid --res format '$arg'. Expected WIDTHxHEIGHT (e.g. 1280x1024)"
                exit 1
                ;;
        esac
        continue
    fi

    if [ "$EXPECT_FWD" = true ]; then
        EXPECT_FWD=false
        # Validate format: HOST:GUEST (both numeric)
        case "$arg" in
            *:*)
                HOST_PORT="${arg%%:*}"
                GUEST_PORT="${arg#*:}"
                if [ -n "$HOST_PORT" ] && [ -n "$GUEST_PORT" ]; then
                    FWD_RULES="${FWD_RULES},hostfwd=tcp::${HOST_PORT}-:${GUEST_PORT}"
                else
                    echo "Error: Invalid --fwd format '$arg'. Expected HOST:GUEST (e.g. 2222:22)"
                    exit 1
                fi
                ;;
            *)
                echo "Error: Invalid --fwd format '$arg'. Expected HOST:GUEST (e.g. 2222:22)"
                exit 1
                ;;
        esac
        continue
    fi

    case "$arg" in
        --vmware-ws)
            VMWARE_WS=true
            ;;
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
            # VirtIO GPU has no VMware backdoor — add USB tablet for absolute mouse positioning
            if [ -z "$USB_FLAGS" ]; then
                USB_FLAGS="-usb -device usb-tablet"
            fi
            ;;
        --ide)
            IDE_MODE=true
            ;;
        --cdrom)
            CDROM_MODE=true
            ;;
        --audio)
            if [ "$(uname -s)" = "Darwin" ]; then
                AUDIO_FLAGS="-device AC97,audiodev=audio0 -audiodev coreaudio,id=audio0"
            else
                AUDIO_FLAGS="-device AC97,audiodev=audio0 -audiodev pa,id=audio0"
            fi
            AUDIO_LABEL=", audio: AC'97"
            ;;
        --usb)
            USB_FLAGS="-usb -device usb-kbd -device usb-mouse"
            USB_LABEL=", USB: keyboard + mouse"
            ;;
        --uefi)
            UEFI_MODE=true
            ;;
        --kvm)
            if [ "$(uname -s)" = "Darwin" ]; then
                # macOS: use Hypervisor.framework (HVF)
                if sysctl kern.hv_support 2>/dev/null | grep -q '1'; then
                    KVM_FLAGS="-accel hvf -cpu host"
                    CPU_FLAGS=""
                    KVM_LABEL=", HVF enabled"
                else
                    echo "Warning: HVF not available on this Mac (missing Hypervisor.framework support)"
                    echo "Continuing without hardware acceleration..."
                fi
            else
                # Linux: use KVM
                if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
                    KVM_FLAGS="-enable-kvm -cpu host"
                    CPU_FLAGS=""
                    KVM_LABEL=", KVM enabled"
                elif [ -e /dev/kvm ]; then
                    echo "Error: /dev/kvm exists but is not accessible."
                    echo "Fix permissions: sudo usermod -aG kvm $(whoami) && newgrp kvm"
                    exit 1
                else
                    echo "Error: /dev/kvm not found. KVM is not available."
                    echo "Enable with: sudo modprobe kvm && sudo modprobe kvm_intel  (or kvm_amd)"
                    echo "Install with: sudo apt-get install qemu-kvm"
                    exit 1
                fi
            fi
            ;;
        --res)
            EXPECT_RES=true
            ;;
        --kbd)
            EXPECT_KBD=true
            ;;
        --fwd)
            EXPECT_FWD=true
            ;;
        *)
            echo "Usage: $0 [--vmware | --std | --virtio] [--res WxH] [--ide] [--cdrom] [--audio] [--usb] [--uefi] [--kvm] [--kbd LAYOUT] [--fwd HOST:GUEST ...]"
            exit 1
            ;;
    esac
done

if [ "$EXPECT_RES" = true ]; then
    echo "Error: --res requires a WIDTHxHEIGHT argument (e.g. --res 1280x1024)"
    exit 1
fi

if [ "$EXPECT_FWD" = true ]; then
    echo "Error: --fwd requires a HOST:GUEST argument (e.g. --fwd 2222:22)"
    exit 1
fi

if [ "$EXPECT_KBD" = true ]; then
    echo "Error: --kbd requires a layout name (us, de, ch, fr, pl)"
    exit 1
fi

# ── VMware Workstation mode ─────────────────────────────────────────────────

if [ "$VMWARE_WS" = true ]; then
    BUILD_DIR="${SCRIPT_DIR}/../build"
    IMG_PATH="${BUILD_DIR}/anyos.img"
    VM_NAME="anyos"

    # ── Find VBoxManage (for raw → VMDK conversion) ─────────────────────
    VBOXMANAGE="$(command -v VBoxManage 2>/dev/null || true)"
    if [ -z "$VBOXMANAGE" ]; then
        echo "Error: VBoxManage not found (needed for VMDK conversion)"
        exit 1
    fi

    # ── Find vmrun ──────────────────────────────────────────────────────
    VMRUN="$(command -v vmrun 2>/dev/null || true)"
    if [ -z "$VMRUN" ]; then
        if [ "$(uname -s)" = "Darwin" ]; then
            for p in \
                "/Applications/VMware Fusion.app/Contents/Public/vmrun" \
                "/Applications/VMware Fusion.app/Contents/Library/vmrun"; do
                [ -x "$p" ] && VMRUN="$p" && break
            done
        fi
    fi
    if [ -z "$VMRUN" ]; then
        echo "Error: vmrun not found in PATH"
        exit 1
    fi

    # ── Locate .vmx file ───────────────────────────────────────────────
    VMX_PATH=""

    # 1. Environment variable override
    if [ -n "$ANYOS_VMX" ] && [ -f "$ANYOS_VMX" ]; then
        VMX_PATH="$ANYOS_VMX"
    fi

    # 2. Parse VMware inventory
    if [ -z "$VMX_PATH" ]; then
        if [ "$(uname -s)" = "Darwin" ]; then
            INV_FILE="$HOME/Library/Application Support/VMware Fusion/vmInventory"
        else
            INV_FILE="$HOME/.vmware/inventory.vmls"
        fi
        if [ -f "$INV_FILE" ]; then
            CANDIDATE=$(grep -oP '\.config\s*=\s*"\K[^"]*anyos[^"]*\.vmx' "$INV_FILE" 2>/dev/null | head -1)
            [ -n "$CANDIDATE" ] && [ -f "$CANDIDATE" ] && VMX_PATH="$CANDIDATE"
        fi
    fi

    # 3. Search default paths
    if [ -z "$VMX_PATH" ]; then
        for dir in \
            "$HOME/vmware/$VM_NAME" \
            "$HOME/Virtual Machines/$VM_NAME" \
            "$HOME/Documents/Virtual Machines/$VM_NAME" \
            "$HOME/Virtual Machines.localized/$VM_NAME.vmwarevm"; do
            if [ -f "$dir/$VM_NAME.vmx" ]; then
                VMX_PATH="$dir/$VM_NAME.vmx"
                break
            fi
        done
    fi

    if [ -z "$VMX_PATH" ]; then
        echo "Error: Could not find '$VM_NAME' VM. Set ANYOS_VMX to the .vmx path."
        exit 1
    fi

    VM_DIR="$(dirname "$VMX_PATH")"
    SERIAL_SOCK="/tmp/anyos-serial"

    echo "VMware VM: $VMX_PATH"

    # ── Check if VM is already running ─────────────────────────────────
    if "$VMRUN" list 2>/dev/null | grep -qiF "$VMX_PATH"; then
        echo "VM '$VM_NAME' is already running - skipping configuration."
    else
        if [ ! -f "$IMG_PATH" ]; then
            echo "Error: $IMG_PATH not found. Run ./scripts/build.sh first."
            exit 1
        fi

        # Find existing disk filename in .vmx
        DISK_FILE=$(grep -oP '(scsi|sata|ide|nvme)\d+:\d+\.fileName\s*=\s*"\K[^"]+\.vmdk' "$VMX_PATH" 2>/dev/null | head -1)
        [ -z "$DISK_FILE" ] && DISK_FILE="$VM_NAME.vmdk"

        case "$DISK_FILE" in
            /*) DISK_FULL="$DISK_FILE" ;;
            *)  DISK_FULL="$VM_DIR/$DISK_FILE" ;;
        esac

        echo "Refreshing disk image ..."

        # Remove old VMDK files and stale locks
        BASE_NAME="${DISK_FILE%.vmdk}"
        rm -f "$VM_DIR/$BASE_NAME"*.vmdk 2>/dev/null
        rm -rf "$VM_DIR"/*.lck 2>/dev/null

        echo "  Converting anyos.img -> $DISK_FILE ..."
        "$VBOXMANAGE" convertfromraw "$IMG_PATH" "$DISK_FULL" --format VMDK
        echo "[OK] Disk refreshed."

        # ── Configure serial port as pipe in .vmx ──────────────────────
        echo "Configuring COM1 -> pipe $SERIAL_SOCK"
        grep -v '^\s*serial0\.' "$VMX_PATH" > "${VMX_PATH}.tmp"
        cat >> "${VMX_PATH}.tmp" <<'SERIAL'
serial0.present = "TRUE"
serial0.fileType = "pipe"
serial0.yieldOnMsrRead = "TRUE"
serial0.startConnected = "TRUE"
SERIAL
        echo "serial0.fileName = \"$SERIAL_SOCK\"" >> "${VMX_PATH}.tmp"
        echo 'serial0.pipe.endPoint = "server"' >> "${VMX_PATH}.tmp"
        mv "${VMX_PATH}.tmp" "$VMX_PATH"

        # Remove stale socket
        rm -f "$SERIAL_SOCK" 2>/dev/null

        # ── Start the VM ───────────────────────────────────────────────
        echo "Starting VMware VM '$VM_NAME' ..."
        "$VMRUN" start "$VMX_PATH" gui
    fi

    # ── Connect to serial socket ───────────────────────────────────────
    if ! command -v socat &>/dev/null; then
        echo "Error: socat not found (needed to read VMware serial pipe)"
        echo "Install: sudo apt-get install socat  (or: brew install socat)"
        exit 1
    fi

    echo "Waiting for serial socket $SERIAL_SOCK ..."
    DEADLINE=$(($(date +%s) + 30))
    while [ ! -S "$SERIAL_SOCK" ] && [ "$(date +%s)" -lt "$DEADLINE" ]; do
        sleep 0.5
    done

    if [ ! -S "$SERIAL_SOCK" ]; then
        echo "Error: Serial socket $SERIAL_SOCK not found after 30 s."
        exit 1
    fi

    echo ""
    echo "============================================================"
    echo "  anyOS Serial Output  (Ctrl+C to disconnect)"
    echo "============================================================"
    echo ""

    socat UNIX-CONNECT:"$SERIAL_SOCK" STDOUT

    echo ""
    echo "============================================================"
    echo "  Serial session ended."
    echo "============================================================"
    exit 0
fi

# Validate --res is only used with --virtio
if [ -n "$RESOLUTION" ] && [ "$VGA" != "virtio" ]; then
    echo "Error: --res is only supported with --virtio (VirtIO GPU sets resolution via device properties)"
    echo "Bochs VGA and VMware SVGA set resolution from the guest OS."
    exit 1
fi

# VirtIO GPU: default to 1024x768 if no --res specified
if [ "$VGA" = "virtio" ] && [ -z "$RESOLUTION" ]; then
    RESOLUTION="${MIN_RES_W}x${MIN_RES_H}"
fi

# Enforce minimum resolution (1024x768)
if [ -n "$RESOLUTION" ]; then
    RES_W="${RESOLUTION%%x*}"
    RES_H="${RESOLUTION#*x}"
    if [ "$RES_W" -lt "$MIN_RES_W" ] || [ "$RES_H" -lt "$MIN_RES_H" ]; then
        echo "Error: Resolution ${RES_W}x${RES_H} is below minimum ${MIN_RES_W}x${MIN_RES_H}"
        exit 1
    fi
fi

if [ "$CDROM_MODE" = true ]; then
    IMAGE="${SCRIPT_DIR}/../build/anyos.iso"
    BIOS_FLAGS=""
    DRIVE_FLAGS="-cdrom \"$IMAGE\" -boot d"
    DRIVE_LABEL="CD-ROM (ISO 9660)"
elif [ "$UEFI_MODE" = true ]; then
    IMAGE="${SCRIPT_DIR}/../build/anyos-uefi.img"

    # Find OVMF firmware (platform-dependent paths)
    if [ "$(uname -s)" = "Darwin" ]; then
        OVMF_FW="/opt/homebrew/share/qemu/edk2-x86_64-code.fd"
        OVMF_HINT="Install with: brew install qemu"
    else
        # Common Linux locations
        for path in \
            /usr/share/OVMF/OVMF_CODE.fd \
            /usr/share/edk2/x64/OVMF_CODE.fd \
            /usr/share/qemu/OVMF.fd \
            /usr/share/edk2-ovmf/OVMF_CODE.fd; do
            if [ -f "$path" ]; then
                OVMF_FW="$path"
                break
            fi
        done
        OVMF_FW="${OVMF_FW:-/usr/share/OVMF/OVMF_CODE.fd}"
        OVMF_HINT="Install with: sudo apt-get install ovmf"
    fi

    BIOS_FLAGS="-drive if=pflash,format=raw,readonly=on,file=$OVMF_FW"
    DRIVE_FLAGS="-drive format=raw,file=\"$IMAGE\""
    DRIVE_LABEL="UEFI (GPT)"

    if [ ! -f "$OVMF_FW" ]; then
        echo "Error: OVMF firmware not found at $OVMF_FW"
        echo "$OVMF_HINT"
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
    echo "Error: Image not found at $IMAGE"
    if [ "$CDROM_MODE" = true ]; then
        echo "Run: cd build && ninja iso"
    else
        echo "Run: ./scripts/build.sh first"
    fi
    exit 1
fi

# Apply keyboard layout to disk image config if requested
KBD_LABEL=""
if [ -n "$KBD_LAYOUT" ]; then
    CONF_FILE="${SCRIPT_DIR}/../sysroot/System/etc/inputmon.conf"
    printf '[keyboard]\nlayout=%s\n' "$KBD_LAYOUT" > "$CONF_FILE"
    # Also update the build sysroot so mkimage picks it up
    BUILD_CONF="${SCRIPT_DIR}/../build/sysroot/System/etc/inputmon.conf"
    if [ -d "$(dirname "$BUILD_CONF")" ]; then
        cp "$CONF_FILE" "$BUILD_CONF"
    fi
    # Re-run mkimage to update the disk image with the new config
    ninja -C "${SCRIPT_DIR}/../build" 2>/dev/null
    LAYOUT_NAMES=("US" "DE" "CH" "FR" "PL")
    KBD_LABEL=", kbd: ${LAYOUT_NAMES[$KBD_LAYOUT]}"
fi

# VGA device flags: VirtIO always uses explicit -device with edid=on for reliable resolution
VGA_FLAGS="-vga $VGA"
RES_LABEL=""
if [ "$VGA" = "virtio" ]; then
    RES_W="${RESOLUTION%%x*}"
    RES_H="${RESOLUTION#*x}"
    VGA_FLAGS="-vga none -device virtio-vga,edid=on,xres=$RES_W,yres=$RES_H"
    VGA_LABEL="Virtio GPU (${RES_W}x${RES_H})"
    RES_LABEL=", res: ${RESOLUTION}"
fi

echo "Starting anyOS with $VGA_LABEL (-vga $VGA), disk: $DRIVE_LABEL$AUDIO_LABEL$USB_LABEL$KVM_LABEL$RES_LABEL$KBD_LABEL"

eval qemu-system-x86_64 \
    $CPU_FLAGS \
    $KVM_FLAGS \
    $BIOS_FLAGS \
    $DRIVE_FLAGS \
    -m 1024M \
    -smp cpus=4 \
    -serial stdio \
    $VGA_FLAGS \
    -netdev user,id=net0${FWD_RULES} -device e1000,netdev=net0 \
    $AUDIO_FLAGS \
    $USB_FLAGS \
    -no-reboot \
    -no-shutdown
    

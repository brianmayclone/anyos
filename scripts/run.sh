#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Run anyOS in QEMU (or VMware Workstation)
#
# Usage: ./run.sh [OPTIONS...]
#
# ── GPU / Display ─────────────────────────────────────────────────────────────
#   --std         Bochs VGA / Standard VGA (double-buffering, no accel) [default]
#   --vmware      VMware SVGA II (2D acceleration, HW cursor)
#   --virtio      VirtIO GPU (modern transport, ARGB cursor, 2D only)
#   --virgl       VirtIO GPU + virglrenderer (3D acceleration via host OpenGL)
#   --res WxH     Set display resolution. Min: 1024x768.
#                   VirtIO/virgl: sets GPU EDID resolution (guest sees this natively).
#                   std/vmware: sets QEMU GTK window size.
#                   Example: --res 1280x1024, --res 1920x1080
#
# ── Input Devices ─────────────────────────────────────────────────────────────
#   (none)        PS/2 keyboard + PS/2 mouse + vmmouse backdoor [default].
#                   vmmouse provides absolute mouse positioning via VMware backdoor.
#                   Works with all GPU modes — no USB needed.
#   --usb         USB keyboard + USB mouse (boot protocol, relative positioning).
#                   Uses UHCI controller + interrupt transfers.
#   --tablet      USB tablet (absolute mouse positioning, no keyboard).
#                   PS/2 keyboard still handles keyboard input.
#                   Good for HiDPI displays or when window resizing causes cursor offset.
#   --kbd LAYOUT  Set keyboard layout in anyOS: us, de, ch, fr, pl
#                   Writes /System/etc/inputmon.conf and rebuilds the disk image.
#
# ── Storage ───────────────────────────────────────────────────────────────────
#   --ide         Use legacy IDE (PIO) instead of AHCI (DMA) for disk I/O
#   --cdrom       Boot from ISO image (CD-ROM) instead of hard drive
#   --tempdisk    Attach a fresh 2 GB empty hard disk (recreated each start).
#                   Useful for testing ISO installations: --cdrom --tempdisk
#   --disk PATH[:SIZE]
#                   Attach a persistent hard disk at PATH (preserved across runs).
#                   Created with SIZE (e.g. 4G, 512M) if the file does not yet
#                   exist. Default SIZE is 2G. Repeatable to attach multiple
#                   disks. Example: --disk data.img:4G --disk /tmp/extra.img
#   --uefi        Boot via UEFI (OVMF firmware) instead of BIOS
#   --system-fs {exfat|corefs}
#                   Choose the image layout for the system partition and
#                   (re)configure CMake accordingly.  `exfat` (default) is the
#                   classic single-partition layout; `corefs` appends a CoreFS
#                   partition (MBR type 0xCF, default 128 MiB) at the end of
#                   the disk image.  Equivalent to:
#                   `cmake .. -DANYOS_SYSTEM_FS=corefs`
#   --system-fs-size N
#                   Override the size (in MiB) of the CoreFS system partition.
#                   Only meaningful together with `--system-fs corefs`.
#                   Equivalent to: `cmake .. -DANYOS_SYSTEM_FS_SIZE_MIB=N`
#
# ── CPU / Acceleration ────────────────────────────────────────────────────────
#   --kvm         Enable hardware virtualization (KVM on Linux, HVF on macOS).
#                   Significant speedup. Requires /dev/kvm (Linux) or HV framework (macOS).
#
# ── Audio ─────────────────────────────────────────────────────────────────────
#   --audio       Enable AC'97 audio device (PulseAudio on Linux, CoreAudio on macOS)
#
# ── Network ───────────────────────────────────────────────────────────────────
#   --fwd H:G     Forward host TCP port H to guest port G. Repeatable.
#                   Example: --fwd 2222:22 --fwd 8080:80
#   --bridge [IF] Use TAP/bridge networking instead of NAT (default: br0).
#                   Requires: qemu-bridge-helper + /etc/qemu/bridge.conf
#                   Example: --bridge virbr0
#   --wifi        Add a second NIC as Wi-Fi adapter (virtio-net, NAT, appears as wlan0)
#
# ── Clipboard / SPICE ─────────────────────────────────────────────────────
#   --spice       Enable SPICE server on port 5930 + virtio-serial for vdagent.
#                   Keeps the regular GTK display window for input (mouse +
#                   keyboard); SPICE is exposed only for clipboard sync and an
#                   optional external viewer. Connect display with:
#                     remote-viewer spice://localhost:5930
#                   Install viewer: sudo apt-get install virt-viewer
#                   Recommended for development — input works through GTK.
#   --spice-app   Like --spice, but uses QEMU's built-in `-display spice-app`
#                   viewer as the only display. Adds virtio-mouse-pci and
#                   virtio-keyboard-pci because spice-app routes input via
#                   the SPICE protocol (no PS/2 / vmmouse delivery), so the
#                   guest needs the virtio-input drivers to receive events.
#                   Use this for headless-style runs where SPICE is the
#                   primary interaction channel.
#   --clipboard   Enable clipboard sync only (no SPICE display).
#                   Adds virtio-serial + chardev for vdagent, but keeps the normal
#                   GTK/SDL display. Requires QEMU 6.1+ with -chardev qemu-vdagent.
#
# ── VMware Workstation ────────────────────────────────────────────────────────
#   --vmware-ws   Start VMware Workstation VM named 'anyos' and stream COM1 serial.
#                   Converts anyos.img to VMDK, configures serial pipe, starts VM.
#                   Requires: VBoxManage (for VMDK conversion) + vmrun.
#
# ── ARM64 ─────────────────────────────────────────────────────────────────────
#   --arm64       Run ARM64 kernel on QEMU aarch64 virt machine (serial-only).
#                   Uses: virtio-gpu, virtio-keyboard, virtio-mouse, GICv3.
#
# ── Examples ──────────────────────────────────────────────────────────────────
#   ./run.sh                                    # Bochs VGA, PS/2 input, NAT
#   ./run.sh --kvm                              # Same + KVM acceleration
#   ./run.sh --virtio --res 1920x1080 --kvm     # VirtIO GPU, PS/2 input, KVM
#   ./run.sh --virgl --kvm --audio              # 3D accelerated + audio
#   ./run.sh --kvm --tablet --kbd ch            # KVM, USB tablet, Swiss layout
#   ./run.sh --kvm --usb --kbd de              # KVM, USB keyboard+mouse, German layout
#   ./run.sh --kvm --fwd 2222:22 --fwd 80:80   # KVM + port forwarding
#   ./run.sh --vmware --kvm --bridge virbr0     # VMware SVGA, KVM, bridged network
#   ./run.sh --kvm --virtio --spice             # GTK display + SPICE for clipboard
#   ./run.sh --kvm --virtio --spice-app         # SPICE-only (built-in viewer + virtio-input)
#   ./run.sh --persist-power                    # Keep QEMU open after guest power events

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
BRIDGE_IFACE=""
EXPECT_BRIDGE=false
WIFI_FLAGS=""
WIFI_LABEL=""
TEMPDISK=false
DISK_SPECS=()
EXPECT_DISK=false
VMWARE_WS=false
ARM64_MODE=false
HEADLESS=false
SNAPSHOT=false
SPICE_MODE=false
SPICE_APP_MODE=false
CLIPBOARD_MODE=false
PERSIST_POWER=false
# Empty unless --system-fs / --system-fs-size is explicitly given.
SYSTEM_FS_OPT=""
SYSTEM_FS_SIZE_OPT=""
EXPECT_SYSTEM_FS=false
EXPECT_SYSTEM_FS_SIZE=false
RAM_MB=4096
EXPECT_RAM=false
MIN_RES_W=1024
MIN_RES_H=768

# ── WSL: prefer Windows QEMU if installed ───────────────────────────────────
QEMU_BIN="qemu-system-x86_64"

# Escape QEMU_BIN for use in eval (handles spaces in Windows paths)
QEMU_BIN_ESC=$(printf '%q' "$QEMU_BIN")

for arg in "$@"; do
    if [ "$EXPECT_BRIDGE" = true ]; then
        EXPECT_BRIDGE=false
        BRIDGE_IFACE="$arg"
        continue
    fi

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

    if [ "$EXPECT_DISK" = true ]; then
        EXPECT_DISK=false
        DISK_SPECS+=("$arg")
        continue
    fi

    if [ "$EXPECT_SYSTEM_FS" = true ]; then
        EXPECT_SYSTEM_FS=false
        case "$arg" in
            exfat|corefs)
                SYSTEM_FS_OPT="$arg"
                ;;
            *)
                echo "Error: --system-fs expects 'exfat' or 'corefs', got '$arg'"
                exit 1
                ;;
        esac
        continue
    fi

    if [ "$EXPECT_SYSTEM_FS_SIZE" = true ]; then
        EXPECT_SYSTEM_FS_SIZE=false
        if [[ "$arg" =~ ^[0-9]+$ ]] && [ "$arg" -gt 0 ]; then
            SYSTEM_FS_SIZE_OPT="$arg"
        else
            echo "Error: --system-fs-size expects a positive integer (MiB), got '$arg'"
            exit 1
        fi
        continue
    fi

    if [ "$EXPECT_RAM" = true ]; then
        EXPECT_RAM=false
        if [[ "$arg" =~ ^([0-9]+)([MmGg]?)$ ]]; then
            num="${BASH_REMATCH[1]}"
            unit="${BASH_REMATCH[2]}"
            case "$unit" in
                G|g) RAM_MB=$((num * 1024)) ;;
                *)   RAM_MB="$num" ;;
            esac
        else
            echo "Error: --ram expects e.g. 8192, 8192M, or 8G, got '$arg'"
            exit 1
        fi
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
            VGA_LABEL="Virtio GPU (paravirtualized, 2D)"
            ;;
        --virgl)
            VGA="virgl"
            VGA_LABEL="Virtio GPU + virgl (3D accelerated)"
            # virgl requires native Linux QEMU with OpenGL context — force WSL to use Linux QEMU
            if [[ "$QEMU_BIN" == *.exe ]]; then
                QEMU_BIN="qemu-system-x86_64"
                QEMU_BIN_ESC=$(printf '%q' "$QEMU_BIN")
                if ! command -v "$QEMU_BIN" &>/dev/null; then
                    echo "Error: --virgl requires native Linux QEMU (not Windows .exe)"
                    echo "Install with: sudo apt-get install qemu-system-x86"
                    exit 1
                fi
            fi
            ;;
        --ide)
            IDE_MODE=true
            ;;
        --cdrom)
            CDROM_MODE=true
            ;;
        --tempdisk)
            TEMPDISK=true
            ;;
        --disk)
            EXPECT_DISK=true
            ;;
        --system-fs)
            EXPECT_SYSTEM_FS=true
            ;;
        --system-fs-size)
            EXPECT_SYSTEM_FS_SIZE=true
            ;;
        --ram|-m)
            EXPECT_RAM=true
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
        --tablet)
            USB_FLAGS="-usb -device usb-tablet"
            USB_LABEL=", USB: tablet (absolute mouse)"
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
        --bridge)
            EXPECT_BRIDGE=true
            BRIDGE_IFACE="br0"  # default; overridden if next arg is an iface name
            ;;
        --wifi)
            WIFI_FLAGS="-netdev user,id=wifi0 -device virtio-net-pci,netdev=wifi0,mac=52:54:00:12:34:57"
            WIFI_LABEL=", wifi: virtio-net (NAT)"
            ;;
        --spice)
            SPICE_MODE=true
            ;;
        --spice-app)
            SPICE_APP_MODE=true
            ;;
        --clipboard)
            CLIPBOARD_MODE=true
            ;;
        --arm64)
            ARM64_MODE=true
            ;;
        --headless)
            HEADLESS=true
            ;;
        --snapshot)
            SNAPSHOT=true
            ;;
        --persist-power)
            PERSIST_POWER=true
            ;;
        *)
            echo "Usage: $0 [--vmware | --std | --virtio | --virgl] [--res WxH] [--ide] [--cdrom] [--tempdisk] [--disk PATH[:SIZE] ...] [--audio] [--usb | --tablet] [--uefi] [--kvm] [--kbd LAYOUT] [--fwd HOST:GUEST ...] [--bridge [IFACE]] [--wifi] [--spice | --spice-app | --clipboard] [--arm64] [--headless] [--snapshot] [--persist-power]"
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

if [ "$EXPECT_DISK" = true ]; then
    echo "Error: --disk requires an argument (PATH[:SIZE], e.g. data.img:4G)"
    exit 1
fi

if [ "$EXPECT_SYSTEM_FS" = true ]; then
    echo "Error: --system-fs requires an argument (exfat | corefs)"
    exit 1
fi
if [ "$EXPECT_SYSTEM_FS_SIZE" = true ]; then
    echo "Error: --system-fs-size requires a positive integer (MiB)"
    exit 1
fi

# ── Propagate --system-fs / --system-fs-size to CMake ───────────────────────
#
# The two knobs are build-time CMake options, not QEMU runtime flags, so
# we reconfigure the build directory and trigger a rebuild before
# dispatching to QEMU.  A no-op when the cache already reflects the
# requested values (CMake exits quickly without rewriting files).
if [ -n "$SYSTEM_FS_OPT" ] || [ -n "$SYSTEM_FS_SIZE_OPT" ]; then
    RECONFIG_BUILD_DIR="${SCRIPT_DIR}/../build"
    if [ ! -d "$RECONFIG_BUILD_DIR" ]; then
        echo "Error: build directory '$RECONFIG_BUILD_DIR' not found."
        echo "       Run 'cmake .. -G Ninja' from build/ first."
        exit 1
    fi
    CMAKE_DEFS=()
    [ -n "$SYSTEM_FS_OPT" ] && CMAKE_DEFS+=("-DANYOS_SYSTEM_FS=${SYSTEM_FS_OPT}")
    [ -n "$SYSTEM_FS_SIZE_OPT" ] && CMAKE_DEFS+=("-DANYOS_SYSTEM_FS_SIZE_MIB=${SYSTEM_FS_SIZE_OPT}")
    echo ">>> reconfiguring build dir with ${CMAKE_DEFS[*]}"
    (cd "$RECONFIG_BUILD_DIR" && cmake "${CMAKE_DEFS[@]}" .)
    echo ">>> rebuilding image (ninja)"
    if ! ninja -C "$RECONFIG_BUILD_DIR"; then
        echo "Error: ninja build failed — cannot launch QEMU."
        exit 1
    fi
fi

# ── VMware Workstation mode ─────────────────────────────────────────────────

if [ "$VMWARE_WS" = true ]; then
    # In WSL: delegate to PowerShell script (uses Windows named pipes, not Unix sockets)
    if [[ "$QEMU_BIN" == *.exe ]]; then
        PS1_SCRIPT="$(to_win_path "${SCRIPT_DIR}/run.ps1")"
        # Remove Windows Zone.Identifier (UNC paths from WSL are flagged as "Internet")
        powershell.exe -NoProfile -Command "Unblock-File -Path '$PS1_SCRIPT'" 2>/dev/null
        exec powershell.exe -ExecutionPolicy Bypass -NoProfile -Command "& { . '$PS1_SCRIPT' -VMwareWS }"
    fi

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
        # If VBoxManage is a Windows binary (.exe), convert WSL paths to Windows paths
        if [[ "$VBOXMANAGE" == *.exe ]]; then
            "$VBOXMANAGE" convertfromraw "$(to_win_path "$IMG_PATH")" "$(to_win_path "$DISK_FULL")" --format VMDK
        else
            "$VBOXMANAGE" convertfromraw "$IMG_PATH" "$DISK_FULL" --format VMDK
        fi
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

# ============================================================
# ARM64 mode: launch QEMU aarch64 and exit
# ============================================================
if [ "$ARM64_MODE" = true ]; then
    ARM64_BUILD_DIR="${SCRIPT_DIR}/../build/arm64"
    KERNEL_ELF="${ARM64_BUILD_DIR}/kernel/aarch64-anyos/release/anyos_kernel.elf"
    ARM64_DISK="${ARM64_BUILD_DIR}/anyos-arm64.img"
    ARM64_NET_ARGS="-netdev user,id=net0 -device virtio-net-device,netdev=net0,mac=52:54:00:12:34:56"
    ARM64_WIFI_ARGS=""
    echo "Refreshing ARM64 artifacts..."
    if ! ninja -C "$ARM64_BUILD_DIR" kernel-arm64 arm64-image; then
        echo "Error: ARM64 build refresh failed."
        echo "Run: ./scripts/build.sh --arm64"
        exit 1
    fi
    if [ ! -f "$KERNEL_ELF" ]; then
        echo "Error: ARM64 kernel not found at $KERNEL_ELF"
        echo "Run: ./scripts/build.sh --arm64"
        exit 1
    fi
    echo "Starting anyOS (ARM64) on QEMU virt machine..."
    DISK_ARGS=""
    ARM64_TMP_DISK=""
    if [ -f "$ARM64_DISK" ]; then
        echo "  Disk image: $ARM64_DISK"
        if [ "$SNAPSHOT" = true ]; then
            # QEMU snapshot temp files are not writable in the sandbox; use a temp copy instead.
            ARM64_TMP_DISK="$(mktemp /tmp/anyos-arm64-XXXXXX.img)"
            cp -f "$ARM64_DISK" "$ARM64_TMP_DISK"
            trap 'rm -f "$ARM64_TMP_DISK"' EXIT
            DISK_ARGS="-drive if=none,format=raw,file=$ARM64_TMP_DISK,id=hd0,cache=unsafe -device virtio-blk-device,drive=hd0"
        else
            DISK_ARGS="-drive if=none,format=raw,file=$ARM64_DISK,id=hd0 -device virtio-blk-device,drive=hd0"
        fi
    else
        echo "  Warning: No ARM64 disk image found at $ARM64_DISK"
        echo "  Running kernel-only (no filesystem). Build with: ./scripts/build.sh --arm64"
    fi
    if [ "$WIFI" = true ]; then
        ARM64_WIFI_ARGS="-netdev user,id=wifi0 -device virtio-net-device,netdev=wifi0,mac=52:54:00:12:34:57"
    fi
    ARM64_DISPLAY_ARGS=""
    if [ "$HEADLESS" = true ]; then
        ARM64_DISPLAY_ARGS="-display none"
    fi
    # QEMU uses TMPDIR for snapshot temp files; force a writable location in sandbox.
    TMPDIR="/tmp" \
    qemu-system-aarch64 \
        -M virt,gic-version=3 -cpu cortex-a72 \
        -m 512M \
        -smp cpus=4 \
        -kernel "$KERNEL_ELF" \
        $ARM64_NET_ARGS \
        $ARM64_WIFI_ARGS \
        -device virtio-gpu-device \
        -device virtio-keyboard-device \
        -device virtio-mouse-device \
        -serial stdio \
        $ARM64_DISPLAY_ARGS \
        $DISK_ARGS \
        -no-reboot -no-shutdown
    exit 0
fi

# VirtIO GPU (2D only): auto-detect host monitor resolution if no --res specified.
# This makes the guest start at the same resolution as the host display,
# so the Monitor EDID data in anyOS matches the real screen.
# virgl is excluded — its GL display backend handles resolution differently.
if [ "$VGA" = "virtio" ] && [ -z "$RESOLUTION" ]; then
    HOST_RES=""
    # Try xrandr (X11 and XWayland)
    if command -v xrandr &>/dev/null; then
        HOST_RES=$(xrandr --current 2>/dev/null | grep -oP '\d+x\d+(?=\s+\d+\.\d+\*)')
        # Take primary monitor (first match)
        HOST_RES=$(echo "$HOST_RES" | head -1)
    fi
    # Try wlr-randr (native Wayland)
    if [ -z "$HOST_RES" ] && command -v wlr-randr &>/dev/null; then
        HOST_RES=$(wlr-randr 2>/dev/null | grep -oP '\d+x\d+(?= px)')
        HOST_RES=$(echo "$HOST_RES" | head -1)
    fi
    if [ -n "$HOST_RES" ]; then
        H_W="${HOST_RES%%x*}"
        H_H="${HOST_RES#*x}"
        # Cap at 1920x1080 for VM performance (large framebuffers are slow in VirtIO).
        # HiDPI monitors (4K/5K) would produce huge guest framebuffers with no benefit
        # since QEMU does not pass through HiDPI scaling.
        MAX_RES_W=1920
        MAX_RES_H=1080
        if [ "$H_W" -gt "$MAX_RES_W" ] || [ "$H_H" -gt "$MAX_RES_H" ]; then
            H_W=$MAX_RES_W
            H_H=$MAX_RES_H
        fi
        # Enforce minimum 1024x768
        if [ "$H_W" -ge "$MIN_RES_W" ] 2>/dev/null && [ "$H_H" -ge "$MIN_RES_H" ] 2>/dev/null; then
            RESOLUTION="${H_W}x${H_H}"
            echo "Auto-detected host monitor: ${HOST_RES} -> VM resolution: ${RESOLUTION}"
        fi
    fi
    # Fallback if detection failed
    if [ -z "$RESOLUTION" ]; then
        RESOLUTION="${MIN_RES_W}x${MIN_RES_H}"
    fi
fi

# std/vmware GPU: auto-detect host resolution for GTK window size if no --res.
if { [ "$VGA" = "std" ] || [ "$VGA" = "vmware" ]; } && [ -z "$RESOLUTION" ]; then
    HOST_RES=""
    if command -v xrandr &>/dev/null; then
        HOST_RES=$(xrandr --current 2>/dev/null | grep -oP '\d+x\d+(?=\s+\d+\.\d+\*)')
        HOST_RES=$(echo "$HOST_RES" | head -1)
    fi
    if [ -z "$HOST_RES" ] && command -v wlr-randr &>/dev/null; then
        HOST_RES=$(wlr-randr 2>/dev/null | grep -oP '\d+x\d+(?= px)')
        HOST_RES=$(echo "$HOST_RES" | head -1)
    fi
    if [ -n "$HOST_RES" ]; then
        H_W="${HOST_RES%%x*}"
        H_H="${HOST_RES#*x}"
        MAX_RES_W=1920
        MAX_RES_H=1080
        if [ "$H_W" -gt "$MAX_RES_W" ] || [ "$H_H" -gt "$MAX_RES_H" ]; then
            H_W=$MAX_RES_W
            H_H=$MAX_RES_H
        fi
        if [ "$H_W" -ge "$MIN_RES_W" ] 2>/dev/null && [ "$H_H" -ge "$MIN_RES_H" ] 2>/dev/null; then
            RESOLUTION="${H_W}x${H_H}"
            echo "Auto-detected host monitor: ${HOST_RES} -> VM window: ${RESOLUTION}"
        fi
    fi
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

# ── Temp disk for ISO install testing ─────────────────────────────────────────
TEMPDISK_FLAGS=""
TEMPDISK_LABEL=""
if [ "$TEMPDISK" = true ]; then
    TEMPDISK_FILE="${SCRIPT_DIR}/../build/tempdisk.img"
    echo "Creating 2048 MB temp disk: $TEMPDISK_FILE"
    rm -f "$TEMPDISK_FILE"
    qemu-img create -f raw "$TEMPDISK_FILE" 2048M >/dev/null 2>&1
    # Attach as SATA disk via dedicated AHCI controller (detected by anyOS AHCI driver)
    TEMPDISK_FLAGS="-device ahci,id=tempdisk-ahci -drive file=$TEMPDISK_FILE,format=raw,if=none,id=tempdrive -device ide-hd,drive=tempdrive,bus=tempdisk-ahci.0"
    TEMPDISK_LABEL=", tempdisk: 2048 MB"
fi

# ── Persistent extra disks (--disk PATH[:SIZE]) ───────────────────────────────
# Each --disk attaches its own AHCI controller so every disk appears as a
# separate SATA device. Files are created on first use and preserved across
# runs. SIZE defaults to 2G when omitted.
EXTRA_DISK_FLAGS=""
EXTRA_DISK_LABEL=""
disk_idx=0
for spec in "${DISK_SPECS[@]}"; do
    disk_idx=$((disk_idx + 1))
    # Split on the LAST ':' so Windows-style paths (C:\...) stay intact.
    if [[ "$spec" == *:* ]]; then
        disk_path="${spec%:*}"
        disk_size="${spec##*:}"
    else
        disk_path="$spec"
        disk_size="2G"
    fi
    # Resolve relative paths against the project's build/ directory.
    if [[ "$disk_path" != /* ]]; then
        disk_path="${SCRIPT_DIR}/../build/$disk_path"
    fi
    if [ ! -f "$disk_path" ]; then
        echo "Creating persistent disk $disk_idx: $disk_path ($disk_size)"
        mkdir -p "$(dirname "$disk_path")"
        if ! qemu-img create -f raw "$disk_path" "$disk_size" >/dev/null 2>&1; then
            echo "Error: could not create disk '$disk_path' (size '$disk_size')"
            exit 1
        fi
    else
        echo "Attaching persistent disk $disk_idx: $disk_path"
    fi
    EXTRA_DISK_FLAGS="$EXTRA_DISK_FLAGS -device ahci,id=extra${disk_idx}-ahci -drive file=${disk_path},format=raw,if=none,id=extra${disk_idx}drive -device ide-hd,drive=extra${disk_idx}drive,bus=extra${disk_idx}-ahci.0"
    EXTRA_DISK_LABEL="${EXTRA_DISK_LABEL}, disk${disk_idx}: ${disk_path}"
done

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
    DRIVE_FLAGS="-device ahci,id=ahci0 -drive id=disk0,file=\"$IMAGE\",format=raw,if=none -device ide-hd,drive=disk0,bus=ahci0.0"
    DRIVE_LABEL="UEFI (GPT, AHCI)"

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
    # Also update the build sysroots so mkimage picks it up.
    BUILD_CONF="${SCRIPT_DIR}/../build/sysroot/System/etc/inputmon.conf"
    if [ -d "$(dirname "$BUILD_CONF")" ]; then
        cp "$CONF_FILE" "$BUILD_CONF"
    fi
    ARM64_BUILD_CONF="${SCRIPT_DIR}/../build/arm64/sysroot/System/etc/inputmon.conf"
    if [ -d "$(dirname "$ARM64_BUILD_CONF")" ]; then
        cp "$CONF_FILE" "$ARM64_BUILD_CONF"
    fi
    # Re-run mkimage to update the disk image with the new config.
    if [ "$ARM64_MODE" = true ]; then
        ninja -C "${SCRIPT_DIR}/../build/arm64" arm64-image 2>/dev/null
    else
        ninja -C "${SCRIPT_DIR}/../build" 2>/dev/null
    fi
    LAYOUT_NAMES=("US" "DE" "CH" "FR" "PL")
    KBD_LABEL=", kbd: ${LAYOUT_NAMES[$KBD_LAYOUT]}"
fi

# VGA device flags
VGA_FLAGS="-vga $VGA"
RES_LABEL=""
if [ "$VGA" = "virgl" ]; then
    VGA_FLAGS="-vga none -device virtio-vga-gl"
    VGA_LABEL="Virtio GPU + virgl (3D accelerated)"
elif [ "$VGA" = "virtio" ]; then
    RES_W="${RESOLUTION%%x*}"
    RES_H="${RESOLUTION#*x}"
    VGA_FLAGS="-vga none -device virtio-vga,edid=on,xres=$RES_W,yres=$RES_H"
    VGA_LABEL="Virtio GPU (${RES_W}x${RES_H})"
    RES_LABEL=", res: ${RESOLUTION}"
elif [ -n "$RESOLUTION" ]; then
    RES_W="${RESOLUTION%%x*}"
    RES_H="${RESOLUTION#*x}"
    RES_LABEL=", res: ${RESOLUTION}"
fi

# Display backend.
# Do NOT use zoom-to-fit — it prevents the GTK window from resizing when
# the guest changes resolution via SET_SCANOUT. Without zoom-to-fit,
# QEMU auto-resizes the window to match the guest framebuffer dimensions.
if [ "$HEADLESS" = true ]; then
    DISPLAY_FLAGS="-display none -nographic"
elif [ "$(uname)" = "Darwin" ]; then
    DISPLAY_FLAGS="-display cocoa"
elif [ "$VGA" = "virgl" ]; then
    DISPLAY_FLAGS="-display gtk,gl=on"
else
    DISPLAY_FLAGS="-display gtk"
fi

# Default input: PS/2 keyboard + PS/2 mouse + vmmouse backdoor (absolute positioning).
# The VMware backdoor (vmport) is always present in QEMU's 'pc' machine type,
# regardless of GPU mode. Works with all VGA backends (std, vmware, virtio, virgl).
# --usb / --tablet override this with USB HID devices (interrupt transfers).

# Network: bridge or NAT (user)
NET_FLAGS=""
NET_LABEL=""
if [ -n "$BRIDGE_IFACE" ]; then
    # TAP/bridge networking: QEMU creates a tap device and attaches it to the bridge.
    # Requires /etc/qemu/bridge.conf with: allow <BRIDGE_IFACE>
    # And qemu-bridge-helper must be setuid: sudo chmod u+s /usr/lib/qemu/qemu-bridge-helper
    HELPER="$(command -v qemu-bridge-helper 2>/dev/null || echo /usr/lib/qemu/qemu-bridge-helper)"
    if [ ! -x "$HELPER" ]; then
        echo "Error: qemu-bridge-helper not found at $HELPER"
        echo "Install with: sudo apt-get install qemu-system-x86"
        exit 1
    fi
    BRIDGE_CONF="/etc/qemu/bridge.conf"
    if ! grep -qs "allow ${BRIDGE_IFACE}" "$BRIDGE_CONF" 2>/dev/null; then
        echo "Warning: Bridge '$BRIDGE_IFACE' not listed in $BRIDGE_CONF"
        echo "  Fix: echo 'allow ${BRIDGE_IFACE}' | sudo tee -a $BRIDGE_CONF"
        echo "  And: sudo chmod u+s $HELPER"
    fi
    NET_FLAGS="-netdev bridge,id=net0,br=${BRIDGE_IFACE},helper=${HELPER} -device virtio-net-pci,netdev=net0"
    NET_LABEL=", net: bridge (${BRIDGE_IFACE}, virtio)"
else
    NET_FLAGS="-netdev user,id=net0${FWD_RULES} -device virtio-net-pci,netdev=net0"
    NET_LABEL=", net: NAT (user, virtio)"
    [ -n "$FWD_RULES" ] && NET_LABEL="${NET_LABEL}${FWD_RULES//,hostfwd=tcp::/ fwd:}"
fi

# ── SPICE / Clipboard sync ───────────────────────────────────────────────────
SPICE_FLAGS=""
SPICE_LABEL=""
if [ "$SPICE_MODE" = true ]; then
    # SPICE server on port 5930 + virtio-serial for vdagent clipboard sync.
    # We deliberately do NOT pass `-display spice-app`: QEMU's built-in
    # spice-app viewer breaks PS/2 / vmmouse input forwarding, leaving the
    # guest cursor unresponsive at the login screen. Keep the regular GTK
    # display window for input, and let the user attach an external
    # `remote-viewer spice://localhost:5930` if they want the SPICE display.
    #
    # We deliberately do NOT add `-device virtio-mouse-pci -device virtio-
    # keyboard-pci` here: those introduce a second pair of input devices that
    # QEMU's GTK frontend never routes user keypresses to. Worse, on some
    # QEMU versions the mere presence of virtio-keyboard-pci pre-empts PS/2
    # keyboard handling so the guest stops receiving any keyboard events at
    # all. Both PS/2 keyboard + vmmouse work fine via the GTK window.
    # Re-introduce virtio-input devices only when running with
    # `-display spice-app` or when an external SPICE viewer is the sole
    # input source.
    SPICE_FLAGS="-spice port=5930,disable-ticketing=on,addr=127.0.0.1 -device virtio-serial -chardev spicevmc,id=vdagent,debug=0,name=vdagent -device virtserialport,chardev=vdagent,name=com.redhat.spice.0"
    SPICE_LABEL=", spice: port 5930 (clipboard, optional remote-viewer)"
    echo "SPICE server on port 5930 (GTK display stays active for input)."
    echo "  Optional external viewer: remote-viewer spice://localhost:5930"
elif [ "$SPICE_APP_MODE" = true ]; then
    # Built-in SPICE viewer (`-display spice-app`) is the only display.
    # spice-app does NOT forward input to the emulated PS/2 / vmmouse
    # devices, so we add virtio-mouse-pci + virtio-keyboard-pci. The guest
    # virtio-input driver translates these events into the existing
    # mouse/keyboard pipelines.
    SPICE_FLAGS="-spice port=5930,disable-ticketing=on,addr=127.0.0.1 -device virtio-serial -chardev spicevmc,id=vdagent,debug=0,name=vdagent -device virtserialport,chardev=vdagent,name=com.redhat.spice.0 -device virtio-mouse-pci -device virtio-keyboard-pci"
    DISPLAY_FLAGS="-display spice-app"
    SPICE_LABEL=", spice-app: port 5930 (built-in viewer, virtio-input)"
    echo "SPICE built-in viewer on port 5930 (-display spice-app)."
    echo "  Input is delivered via virtio-mouse-pci + virtio-keyboard-pci."
elif [ "$CLIPBOARD_MODE" = true ]; then
    # Clipboard sync only (GTK display stays). Uses QEMU's built-in qemu-vdagent chardev
    # which bridges the host clipboard into the virtio-serial channel without SPICE.
    SPICE_FLAGS="-device virtio-serial -chardev qemu-vdagent,id=vdagent,clipboard=on -device virtserialport,chardev=vdagent,name=com.redhat.spice.0"
    SPICE_LABEL=", clipboard: vdagent (host sync)"
fi

# ── Windows QEMU (WSL): convert paths and adjust flags ──────────────────────
if [[ "$QEMU_BIN" == *.exe ]]; then
    WIN_IMAGE="$(to_win_path "$IMAGE")"
    # Double backslashes so eval doesn't collapse \\ -> \ (UNC paths need \\wsl.localhost\...)
    WIN_IMAGE_EVAL="${WIN_IMAGE//\\/\\\\}"
    DRIVE_FLAGS="${DRIVE_FLAGS//$IMAGE/$WIN_IMAGE_EVAL}"
    if [ -n "$OVMF_FW" ]; then
        WIN_OVMF="$(to_win_path "$OVMF_FW")"
        WIN_OVMF_EVAL="${WIN_OVMF//\\/\\\\}"
        BIOS_FLAGS="${BIOS_FLAGS//$OVMF_FW/$WIN_OVMF_EVAL}"
    fi
    # KVM is Linux-only; Windows uses WHPX (Windows Hypervisor Platform)
    KVM_FLAGS="${KVM_FLAGS//-enable-kvm/-accel whpx}"
    KVM_LABEL="${KVM_LABEL//, KVM enabled/, WHPX enabled}"
    # PulseAudio is Linux-only; use DirectSound on Windows
    AUDIO_FLAGS="${AUDIO_FLAGS//-audiodev pa,/-audiodev dsound,}"
fi

# Unset Snap-injected GTK/GDK environment variables to prevent GLIBC conflicts.
# VSCode (snap) sets GTK_PATH, GTK_EXE_PREFIX etc. to its snap dirs; QEMU then
# loads GTK modules built against the snap's GLIBC, causing symbol lookup errors.
unset GTK_PATH GTK_EXE_PREFIX GTK_IM_MODULE_FILE
unset GDK_PIXBUF_MODULE_FILE GDK_PIXBUF_MODULEDIR
unset GIO_MODULE_DIR
if [ -n "$LD_LIBRARY_PATH" ]; then
    CLEAN_LD=$(echo "$LD_LIBRARY_PATH" | tr ':' '\n' | grep -v '/snap/' | tr '\n' ':' | sed 's/:$//')
    export LD_LIBRARY_PATH="$CLEAN_LD"
fi

# Force GTK to use X11 backend to avoid NVIDIA GBM initialization errors
# (NVIDIA proprietary driver fails GBM probe; GTK falls back anyway, but spams stderr)
export GDK_BACKEND=x11

# For virgl: force Mesa EGL instead of NVIDIA EGL.
# NVIDIA's EGL GBM backend (10_nvidia.json) fails for virgl off-screen rendering;
# Mesa (50_mesa.json) works correctly via renderD128.
if [ "$VGA" = "virgl" ]; then
    export __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json
fi

echo "Starting anyOS with $VGA_LABEL (-vga $VGA), disk: $DRIVE_LABEL$TEMPDISK_LABEL$EXTRA_DISK_LABEL$AUDIO_LABEL$USB_LABEL$KVM_LABEL$RES_LABEL$KBD_LABEL$NET_LABEL$WIFI_LABEL$SPICE_LABEL"

POWER_FLAGS=""
if [ "$PERSIST_POWER" = true ]; then
    POWER_FLAGS="-no-reboot -no-shutdown"
fi

QEMU_CMD="$QEMU_BIN_ESC \
    $CPU_FLAGS \
    $KVM_FLAGS \
    $BIOS_FLAGS \
    $DRIVE_FLAGS \
    $TEMPDISK_FLAGS \
    $EXTRA_DISK_FLAGS \
    -m ${RAM_MB}M \
    -smp cpus=4 \
    -serial stdio \
    $VGA_FLAGS \
    $DISPLAY_FLAGS \
    $NET_FLAGS \
    $WIFI_FLAGS \
    $AUDIO_FLAGS \
    $USB_FLAGS \
    $SPICE_FLAGS \
    $POWER_FLAGS"

eval "$QEMU_CMD"

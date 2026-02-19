#!/bin/bash
# =============================================================================
# VirtualBox GDB Debug Script for anyOS
# =============================================================================
# Usage:
#   ./debug_vbox.sh setup     — Enable GDB stub in VirtualBox (one-time)
#   ./debug_vbox.sh start     — Start VM and attach GDB
#   ./debug_vbox.sh attach    — Attach GDB to already-running VM
#   ./debug_vbox.sh regs      — Dump all CPU registers (VM must be paused/crashed)
#   ./debug_vbox.sh stop      — Power off the VM
#   ./debug_vbox.sh cleanup   — Remove GDB stub config
# =============================================================================

VM_NAME="anyos"
GDB_PORT="1234"
VBOX="/c/Program Files/Oracle/VirtualBox/VBoxManage.exe"
GDB="gdb"
KERNEL="build/kernel/x86_64-anyos/release/anyos_kernel.elf"

# GDB init commands file
GDB_INIT="$(dirname "$0")/debug_vbox_gdb_init.txt"

case "${1}" in

# ---------------------------------------------------------------------------
setup)
    echo "=== Enabling GDB stub on VM '$VM_NAME' (port $GDB_PORT) ==="
    "$VBOX" setextradata "$VM_NAME" "VBoxInternal/DBGF/GDBStub/Port" "$GDB_PORT"
    echo "Done. GDB stub will listen on localhost:$GDB_PORT when VM starts."
    echo "Run: ./debug_vbox.sh start"
    ;;

# ---------------------------------------------------------------------------
start)
    echo "=== Starting VM '$VM_NAME' ==="
    "$VBOX" startvm "$VM_NAME" --type gui &
    echo "Waiting for VM to boot (5s)..."
    sleep 5
    echo "=== Attaching GDB ==="
    exec "$GDB" -x "$GDB_INIT" "$KERNEL"
    ;;

# ---------------------------------------------------------------------------
attach)
    echo "=== Attaching GDB to running VM ==="
    exec "$GDB" -x "$GDB_INIT" "$KERNEL"
    ;;

# ---------------------------------------------------------------------------
regs)
    echo "=== CPU registers for all CPUs ==="
    "$VBOX" debugvm "$VM_NAME" info registers 2>&1
    ;;

# ---------------------------------------------------------------------------
stop)
    echo "=== Powering off VM ==="
    "$VBOX" controlvm "$VM_NAME" poweroff 2>&1
    ;;

# ---------------------------------------------------------------------------
cleanup)
    echo "=== Removing GDB stub config ==="
    "$VBOX" setextradata "$VM_NAME" "VBoxInternal/DBGF/GDBStub/Port" ""
    echo "Done."
    ;;

# ---------------------------------------------------------------------------
*)
    echo "Usage: $0 {setup|start|attach|regs|stop|cleanup}"
    echo ""
    echo "  setup   — Enable GDB stub (one-time, VM must be off)"
    echo "  start   — Start VM + attach GDB"
    echo "  attach  — Attach GDB to running VM"
    echo "  regs    — Dump registers (quick, no GDB needed)"
    echo "  stop    — Power off VM"
    echo "  cleanup — Remove GDB stub"
    exit 1
    ;;
esac

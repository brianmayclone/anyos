#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# SPDX-License-Identifier: MIT
#
# Fast headless KUnit runner (UEFI).
#
# Builds a minimal kernel-only UEFI image with the `kunit` Cargo feature and
# boots it in QEMU headless, keying the verdict off the isa-debug-exit code:
#
#   QEMU exit 33  -> all KUnit suites passed   (script exits 0)
#   QEMU exit 35  -> one or more tests failed  (script exits 1)
#   QEMU exit 124 -> timeout / hang            (script exits 1)
#
# No display, no userspace build, no log scraping. This is the regression gate
# for the kernel-hardening work (see docs/kernel-hardening/ROADMAP.md).
#
# Usage:
#   scripts/run-kunit.sh [-v|--verbose]
# Env:
#   KUNIT_TIMEOUT   QEMU timeout in seconds (default 120)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BUILD_DIR="${PROJECT_DIR}/build"
TIMEOUT="${KUNIT_TIMEOUT:-120}"

VERBOSE=0
case "${1:-}" in
    -v|--verbose) VERBOSE=1 ;;
    "" ) ;;
    * ) echo "usage: $0 [-v|--verbose]" >&2; exit 2 ;;
esac

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BOLD='\033[1m'; NC='\033[0m'

# ── dependencies ─────────────────────────────────────────────────────────────
command -v qemu-system-x86_64 >/dev/null || { echo -e "${RED}qemu-system-x86_64 not found${NC}"; exit 2; }
command -v cmake >/dev/null || { echo -e "${RED}cmake not found${NC}"; exit 2; }
command -v ninja >/dev/null || { echo -e "${RED}ninja not found${NC}"; exit 2; }

# OVMF firmware autodetect (BIOS is intentionally unsupported).
OVMF=""
for c in /usr/share/ovmf/OVMF.fd /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/qemu/OVMF.fd /usr/share/edk2/x64/OVMF_CODE.4m.fd; do
    [ -f "$c" ] && OVMF="$c" && break
done
[ -z "$OVMF" ] && { echo -e "${RED}OVMF UEFI firmware not found — install the 'ovmf' package${NC}"; exit 2; }

version="$(tr -d '[:space:]' < "${PROJECT_DIR}/VERSION" 2>/dev/null || echo 0.0.0)"

echo -e "${BOLD}[run-kunit] configure (ANYOS_KUNIT=ON)…${NC}"
cmake -B "$BUILD_DIR" -G Ninja \
    -DANYOS_KUNIT=ON \
    -DANYOS_DEBUG_VERBOSE=OFF \
    -DANYOS_VERSION="$version" \
    "$PROJECT_DIR" >/dev/null

echo -e "${BOLD}[run-kunit] build minimal kunit image…${NC}"
ninja -C "$BUILD_DIR" kunit-image

IMG="${BUILD_DIR}/anyos-kunit.img"
[ -f "$IMG" ] || { echo -e "${RED}kunit image not built: $IMG${NC}"; exit 1; }

LOG="$(mktemp /tmp/anyos-kunit-XXXXXX.log)"
trap 'rm -f "$LOG"' EXIT
echo -e "${BOLD}[run-kunit] boot headless (UEFI/OVMF, timeout ${TIMEOUT}s)…${NC}"

set +e
timeout "$TIMEOUT" qemu-system-x86_64 \
    -machine q35 \
    -cpu qemu64,+sse3,+ssse3,+sse4.1,+sse4.2,+popcnt \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF" \
    -device ich9-ahci,id=ahci0 \
    -drive id=disk0,file="$IMG",format=raw,if=none \
    -device ide-hd,drive=disk0,bus=ahci0.0 \
    -m 1024M -smp 4 \
    -display none -serial file:"$LOG" \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
    -no-reboot >/dev/null 2>&1
QEXIT=$?
set -e

if [ "$VERBOSE" -eq 1 ]; then
    echo -e "${BOLD}--- serial ---${NC}"; cat "$LOG"; echo -e "${BOLD}--- end serial ---${NC}"
fi

grep -E 'KUnit (unit|integration):|KUNIT-DONE' "$LOG" 2>/dev/null | sed 's/^[[:space:]]*//' || true
FAILS="$(grep -E '\[FAIL\]' "$LOG" 2>/dev/null | head -40 || true)"
[ -n "$FAILS" ] && { echo -e "${RED}${FAILS}${NC}"; }

echo ""
case "$QEXIT" in
    33) echo -e "${GREEN}${BOLD}[run-kunit] ALL PASS${NC} (qemu rc=33)"; exit 0 ;;
    35) echo -e "${RED}${BOLD}[run-kunit] TEST FAILURES${NC} (qemu rc=35)"; exit 1 ;;
    124) echo -e "${YELLOW}${BOLD}[run-kunit] TIMEOUT/HANG${NC} after ${TIMEOUT}s — last serial lines:"; tail -15 "$LOG" | sed 's/^/  /'; exit 1 ;;
    *) echo -e "${RED}${BOLD}[run-kunit] QEMU error/unknown exit (rc=$QEXIT)${NC} — last serial lines:"; tail -15 "$LOG" | sed 's/^/  /'; exit 1 ;;
esac

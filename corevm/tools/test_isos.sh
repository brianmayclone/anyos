#!/bin/bash
#
# test_isos.sh — Test CoreVM with TinyCore, Ventoy and Memtest86+ ISOs
#
# Usage:
#   ./corevm/tools/test_isos.sh                  # Run all tests
#   ./corevm/tools/test_isos.sh tinycore          # Only TinyCore
#   ./corevm/tools/test_isos.sh ventoy            # Only Ventoy
#   ./corevm/tools/test_isos.sh memtest           # Only Memtest86+
#   ./corevm/tools/test_isos.sh --build           # Build vmctl first, then run all
#   ./corevm/tools/test_isos.sh --build tinycore  # Build, then run TinyCore only
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
COREVM_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT_DIR="$(dirname "$COREVM_DIR")"

VMCTL="$COREVM_DIR/vmctl/target/x86_64-unknown-linux-gnu/release/corevm-vmctl"
ISO_DIR="$COREVM_DIR/test-isos"
LOG_DIR="/tmp/corevm-test-$(date +%Y%m%d-%H%M%S)"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${CYAN}[INFO]${NC} $*"; }
pass()  { echo -e "${GREEN}[PASS]${NC} $*"; }
fail()  { echo -e "${RED}[FAIL]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }

# ── Parse arguments ──

DO_BUILD=0
TESTS=()

for arg in "$@"; do
    case "$arg" in
        --build|-b) DO_BUILD=1 ;;
        tinycore|ventoy|memtest) TESTS+=("$arg") ;;
        --help|-h)
            echo "Usage: $0 [--build] [tinycore|ventoy|memtest] ..."
            echo ""
            echo "Options:"
            echo "  --build, -b    Build vmctl before testing"
            echo "  tinycore       Test TinyCore Linux ISO"
            echo "  ventoy         Test Ventoy Live-CD ISO"
            echo "  memtest        Test Memtest86+ ISO"
            echo ""
            echo "If no test is specified, all tests are run."
            exit 0
            ;;
        *) echo "Unknown argument: $arg"; exit 1 ;;
    esac
done

# Default: run all tests
if [ ${#TESTS[@]} -eq 0 ]; then
    TESTS=(tinycore ventoy memtest)
fi

# ── Pre-flight checks ──

if [ ! -e /dev/kvm ]; then
    fail "KVM not available (/dev/kvm missing)"
    exit 1
fi

if [ "$DO_BUILD" -eq 1 ]; then
    info "Building vmctl..."
    cd "$COREVM_DIR/vmctl"
    cargo +stable build --release 2>&1
    cd "$PROJECT_DIR"
    info "Build complete."
fi

if [ ! -x "$VMCTL" ]; then
    fail "vmctl not found at $VMCTL"
    echo "  Run with --build or: cd corevm/vmctl && cargo build --release"
    exit 1
fi

mkdir -p "$LOG_DIR"
info "Test logs: $LOG_DIR"
echo ""

# ── Test runner ──

PASSED=0
FAILED=0
SKIPPED=0

run_test() {
    local name="$1"
    local iso="$2"
    local ram="$3"
    local timeout="$4"
    local keys="$5"  # e.g. "--key 5000:enter"
    local expect="$6" # string to search in VGA screen or serial output

    local log_stdout="$LOG_DIR/${name}-stdout.log"
    local log_stderr="$LOG_DIR/${name}-stderr.log"

    if [ ! -f "$iso" ]; then
        warn "$name: ISO not found: $iso — SKIPPED"
        SKIPPED=$((SKIPPED + 1))
        return
    fi

    info "$name: Starting (RAM=${ram}MB, timeout=${timeout}s)..."

    local cmd="$VMCTL run -r $ram -i $iso -b seabios -t $timeout -s -g"
    if [ -n "$keys" ]; then
        cmd="$cmd $keys"
    fi

    # Run vmctl, capture stdout and stderr separately
    set +e
    eval "$cmd" >"$log_stdout" 2>"$log_stderr"
    local rc=$?
    set -e

    # Extract exit reason from summary
    local exit_reason
    exit_reason=$(grep -oP 'exit_reason: \K\S+' "$log_stdout" 2>/dev/null || echo "unknown")
    local exit_count
    exit_count=$(grep -oP 'exit_count: \K\S+' "$log_stdout" 2>/dev/null || echo "0")
    local serial_bytes
    serial_bytes=$(grep -oP 'serial_bytes: \K\S+' "$log_stdout" 2>/dev/null || echo "0")

    # Check for expected string in output
    local found=0
    if [ -n "$expect" ]; then
        if grep -qi "$expect" "$log_stdout" 2>/dev/null || grep -qi "$expect" "$log_stderr" 2>/dev/null; then
            found=1
        fi
    else
        # No specific expectation — pass if we got meaningful VM exits
        if [ "$exit_count" != "0" ] && [ "$exit_reason" != "unknown" ]; then
            found=1
        fi
    fi

    # Print result
    echo -n "  exit=$exit_reason exits=$exit_count serial=$serial_bytes "
    if [ "$found" -eq 1 ]; then
        pass "$name"
        PASSED=$((PASSED + 1))
    else
        fail "$name (expected: '$expect')"
        # Show last few lines of VGA screen for debugging
        echo "  --- VGA Screen (last 10 lines) ---"
        sed -n '/--- VGA TEXT SCREEN/,/--- END SCREEN/p' "$log_stdout" | tail -12 | sed 's/^/  /'
        echo "  --- stderr (last 5 lines) ---"
        tail -5 "$log_stderr" | sed 's/^/  /'
        FAILED=$((FAILED + 1))
    fi
}

# ── Test definitions ──

for test in "${TESTS[@]}"; do
    case "$test" in
        tinycore)
            run_test \
                "tinycore" \
                "$ISO_DIR/TinyCore-current.iso" \
                256 \
                60 \
                "--key 5000:enter" \
                "boot"
            ;;
        ventoy)
            run_test \
                "ventoy" \
                "$ISO_DIR/ventoy-1.1.10-livecd.iso" \
                1024 \
                60 \
                "" \
                "Ventoy"
            ;;
        memtest)
            run_test \
                "memtest" \
                "$ISO_DIR/memtest86+.iso" \
                64 \
                30 \
                "" \
                "Memtest"
            ;;
    esac
done

# ── Summary ──

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "  ${GREEN}PASSED${NC}: $PASSED  ${RED}FAILED${NC}: $FAILED  ${YELLOW}SKIPPED${NC}: $SKIPPED"
echo "  Logs: $LOG_DIR"
echo "  Framebuffer: /tmp/corevm-framebuffer.raw (1024x768 BGRA)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ "$FAILED" -gt 0 ]; then
    exit 1
fi

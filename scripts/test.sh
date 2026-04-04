#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# anyOS test runner
# Usage: ./scripts/test.sh [OPTIONS]
#
# Options:
#   --kunit              Build kernel with KUnit and run in-kernel self-tests in QEMU
#   --kunit-timeout N    QEMU timeout in seconds for KUnit run (default: 120)
#   --js                 Run all 12 libjs ECMAScript unit-test suites
#   --js-suite NAME      Run a single suite (e.g. --js-suite 07_functions)
#   --tc39               Run tc39/test262 conformance tests
#   --tc39-download      Download the test262 subset first, then run
#   --tc39-dir DIR       Use a custom test262 directory (default: libs/libjs_tests/test262)
#   --verbose            Show full output / console.log from JS; show full QEMU serial log for KUnit
#   --release            Build in release mode
#   -h, --help           Show this help

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${SCRIPT_DIR}/.."
LIBJS_TESTS_DIR="${PROJECT_DIR}/libs/libjs_tests"
BUILD_DIR="${PROJECT_DIR}/build"
CARGO="${HOME}/.cargo/bin/cargo"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

RUN_JS=0
RUN_TC39=0
RUN_KUNIT=0
TC39_DOWNLOAD=0
JS_SUITE=""
TC39_DIR="${LIBJS_TESTS_DIR}/test262"
VERBOSE=0
RELEASE_FLAG=""
SHOW_HELP=0
KUNIT_TIMEOUT=120

if [ $# -eq 0 ]; then SHOW_HELP=1; fi

while [[ $# -gt 0 ]]; do
    case "$1" in
        --kunit)          RUN_KUNIT=1 ;;
        --kunit-timeout)  shift; KUNIT_TIMEOUT="$1" ;;
        --js)             RUN_JS=1 ;;
        --js-suite)       RUN_JS=1; shift; JS_SUITE="$1" ;;
        --tc39)           RUN_TC39=1 ;;
        --tc39-download)  RUN_TC39=1; TC39_DOWNLOAD=1 ;;
        --tc39-dir)       shift; TC39_DIR="$1" ;;
        --verbose)        VERBOSE=1 ;;
        --release)        RELEASE_FLAG="--release" ;;
        -h|--help)        SHOW_HELP=1 ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}" >&2
            SHOW_HELP=1
            ;;
    esac
    shift
done

if [ "$SHOW_HELP" -eq 1 ]; then
    echo ""
    echo -e "${BOLD}anyOS test runner${NC}"
    echo ""
    echo "Usage: ./scripts/test.sh [OPTIONS]"
    echo ""
    echo "  --kunit              Build with KUnit feature and run in-kernel tests in QEMU"
    echo "  --kunit-timeout N    QEMU timeout seconds for KUnit run (default: 120)"
    echo "  --js                 Run all 12 libjs ECMAScript unit-test suites"
    echo "  --js-suite NAME      Run one suite, e.g. --js-suite 07_functions"
    echo "  --tc39               Run tc39/test262 conformance suite"
    echo "  --tc39-download      Download test262 subset, then run"
    echo "  --tc39-dir DIR       Custom test262 directory"
    echo "  --verbose            Show full serial output (kunit) / console.log (js)"
    echo "  --release            Build in release mode"
    echo "  -h, --help           Show this help"
    echo ""
    echo "Examples:"
    echo "  ./scripts/test.sh --kunit"
    echo "  ./scripts/test.sh --kunit --verbose"
    echo "  ./scripts/test.sh --kunit --kunit-timeout 180"
    echo "  ./scripts/test.sh --js"
    echo "  ./scripts/test.sh --js-suite 07_functions --verbose"
    echo "  ./scripts/test.sh --tc39-download          # first time"
    echo "  ./scripts/test.sh --tc39                   # subsequent runs"
    echo "  ./scripts/test.sh --kunit --js --tc39      # everything"
    echo ""
    exit 0
fi

# ── KUnit: in-kernel test runner ─────────────────────────────────────────────

run_kunit_tests() {
    echo ""
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}${BOLD}  KUnit — in-kernel self-tests${NC}"
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""

    # ── 1. Check dependencies ────────────────────────────────────────────────
    if ! command -v qemu-system-x86_64 &>/dev/null; then
        echo -e "${RED}  ERROR: qemu-system-x86_64 not found.${NC}"
        echo -e "  Install with: ${BOLD}sudo apt-get install qemu-system-x86${NC}"
        return 1
    fi

    local cmake_bin
    cmake_bin="$(command -v cmake 2>/dev/null || true)"
    if [ -z "$cmake_bin" ]; then
        echo -e "${RED}  ERROR: cmake not found.${NC}"
        return 1
    fi

    local ninja_bin
    ninja_bin="$(command -v ninja 2>/dev/null || true)"
    if [ -z "$ninja_bin" ]; then
        echo -e "${RED}  ERROR: ninja not found.${NC}"
        return 1
    fi

    # ── 2. Build kernel with KUnit feature ───────────────────────────────────
    echo -e "${BOLD}  [1/3] Building kernel with --features kunit ...${NC}"

    local version
    version="$(tr -d '[:space:]' < "${PROJECT_DIR}/VERSION" 2>/dev/null || echo "0.0.0")"

    # Configure (or reconfigure) with ANYOS_KUNIT=ON and --nover semantics.
    cmake -B "$BUILD_DIR" -G Ninja \
        -DANYOS_KUNIT=ON \
        -DANYOS_DEBUG_VERBOSE=OFF \
        -DANYOS_VERSION="${version}" \
        "$PROJECT_DIR" > /dev/null 2>&1

    # Build only the BIOS disk image (default target).
    set +e
    ninja -C "$BUILD_DIR" 2>&1 | grep -E '^(FAILED|ninja: error|error\[)' || true
    local build_rc=${PIPESTATUS[0]}
    set -e

    if [ "$build_rc" -ne 0 ]; then
        echo -e "${RED}  Build failed (exit $build_rc). Aborting KUnit run.${NC}"
        # Reconfigure without KUnit so subsequent normal builds are not affected.
        cmake -B "$BUILD_DIR" -G Ninja \
            -DANYOS_KUNIT=OFF \
            -DANYOS_VERSION="${version}" \
            "$PROJECT_DIR" > /dev/null 2>&1
        return 1
    fi

    local image="${BUILD_DIR}/anyos.img"
    if [ ! -f "$image" ]; then
        echo -e "${RED}  ERROR: Disk image not found: ${image}${NC}"
        return 1
    fi
    echo -e "  ${GREEN}Build OK${NC} — image: ${image}"

    # ── 3. Boot in QEMU (headless) and capture serial output ─────────────────
    echo -e "${BOLD}  [2/3] Booting in QEMU (timeout: ${KUNIT_TIMEOUT}s) ...${NC}"

    local log_file
    log_file="$(mktemp /tmp/anyos-kunit-XXXXXX.log)"
    # Ensure cleanup on exit/interrupt.
    trap 'rm -f "$log_file"; kill "$QEMU_PID" 2>/dev/null || true' EXIT INT TERM

    qemu-system-x86_64 \
        -cpu qemu64,+sse3,+ssse3,+sse4.1,+sse4.2,+popcnt \
        -drive id=hd0,if=none,format=raw,file="$image" \
            -device ich9-ahci,id=ahci \
            -device ide-hd,drive=hd0,bus=ahci.0 \
        -m 512M \
        -smp cpus=2 \
        -nographic \
        -serial file:"$log_file" \
        -no-reboot \
        -no-shutdown \
        > /dev/null 2>&1 &
    QEMU_PID=$!

    # ── 4. Poll log for KUnit result lines (two phases) ──────────────────────
    local elapsed=0
    local unit_line=""
    local integ_line=""
    local panic_seen=0

    while [ $elapsed -lt "$KUNIT_TIMEOUT" ]; do
        sleep 1
        elapsed=$((elapsed + 1))

        # Check if QEMU exited unexpectedly.
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            break
        fi

        if [ -f "$log_file" ]; then
            # Collect latest summary line for each phase.
            unit_line=$(grep -oE 'KUnit unit: (ALL PASS|FAIL)[^$]*' "$log_file" 2>/dev/null | tail -1 || true)
            integ_line=$(grep -oE 'KUnit integration: (ALL PASS|FAIL)[^$]*' "$log_file" 2>/dev/null | tail -1 || true)

            # Done when both phases have reported.
            if [ -n "$unit_line" ] && [ -n "$integ_line" ]; then
                break
            fi
            # Detect kernel panic.
            if grep -q "KERNEL PANIC\|=== KERNEL PANIC ===" "$log_file" 2>/dev/null; then
                panic_seen=1
                break
            fi
        fi
    done

    # Kill QEMU now that we have what we need (or timed out).
    kill "$QEMU_PID" 2>/dev/null || true
    wait "$QEMU_PID" 2>/dev/null || true
    trap - EXIT INT TERM

    # ── 5. Report results ────────────────────────────────────────────────────
    echo -e "${BOLD}  [3/3] Results${NC}"
    echo ""

    if [ "$VERBOSE" -eq 1 ] && [ -f "$log_file" ]; then
        echo -e "${BOLD}  --- Serial output ---${NC}"
        grep -A2 -B1 -E '(KUnit|\[suite\]|\[PASS\]|\[FAIL\]|\[OK\]|assertion)' \
            "$log_file" 2>/dev/null | sed 's/^/  /' || true
        echo -e "${BOLD}  --- End of serial output ---${NC}"
        echo ""
    fi

    # Print per-suite summary from the log.
    if [ -f "$log_file" ]; then
        grep -E '(\[OK\]\s+\S|\[FAIL\]\s+\S|\[suite\])' "$log_file" 2>/dev/null \
            | sed 's/^/  /' || true
        echo ""
    fi

    local kunit_rc=0

    if [ "$panic_seen" -eq 1 ]; then
        echo -e "  ${RED}${BOLD}KERNEL PANIC detected during KUnit run.${NC}"
        if [ -f "$log_file" ]; then
            grep -A5 "KERNEL PANIC" "$log_file" | sed 's/^/  /' || true
        fi
        kunit_rc=1
    elif [ -z "$unit_line" ] && [ -z "$integ_line" ]; then
        echo -e "  ${YELLOW}${BOLD}TIMEOUT${NC} — no KUnit summary found within ${KUNIT_TIMEOUT}s."
        echo -e "  (Run with ${BOLD}--verbose${NC} to inspect the serial log.)"
        if [ -f "$log_file" ]; then
            echo -e "  Last 5 serial lines:"
            tail -5 "$log_file" | sed 's/^/    /' || true
        fi
        kunit_rc=1
    else
        # Report phase A — unit tests.
        if [ -n "$unit_line" ]; then
            if echo "$unit_line" | grep -q "ALL PASS"; then
                echo -e "  ${GREEN}${BOLD}${unit_line}${NC}"
            else
                echo -e "  ${RED}${BOLD}${unit_line}${NC}"
                kunit_rc=1
            fi
        else
            echo -e "  ${YELLOW}${BOLD}KUnit unit: (no summary — phase timed out)${NC}"
            kunit_rc=1
        fi

        # Report phase B — integration tests.
        if [ -n "$integ_line" ]; then
            if echo "$integ_line" | grep -q "ALL PASS"; then
                echo -e "  ${GREEN}${BOLD}${integ_line}${NC}"
            else
                echo -e "  ${RED}${BOLD}${integ_line}${NC}"
                kunit_rc=1
            fi
        else
            echo -e "  ${YELLOW}${BOLD}KUnit integration: (no summary — phase timed out)${NC}"
            kunit_rc=1
        fi
    fi

    rm -f "$log_file"

    # ── 6. Restore build config without KUnit ────────────────────────────────
    cmake -B "$BUILD_DIR" -G Ninja \
        -DANYOS_KUNIT=OFF \
        -DANYOS_VERSION="${version}" \
        "$PROJECT_DIR" > /dev/null 2>&1

    echo ""
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""

    return $kunit_rc
}

# ── build helper ─────────────────────────────────────────────────────────────

build_libjs_tests() {
    echo -e "${BOLD}  Building libjs_tests …${NC}"
    (cd "$LIBJS_TESTS_DIR" && "$CARGO" +stable build ${RELEASE_FLAG} 2>&1 \
        | grep -v '^warning:' || true)
}

# ── unit-test suites ──────────────────────────────────────────────────────────

run_js_tests() {
    echo ""
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}${BOLD}  libjs ECMAScript Unit-Test Suites${NC}"
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""

    local suites=(
        "01_primitives"
        "02_operators"
        "03_variables"
        "04_strings"
        "05_arrays"
        "06_objects"
        "07_functions"
        "08_control_flow"
        "09_classes"
        "10_error_handling"
        "11_builtins"
        "12_advanced"
    )

    if [ -n "$JS_SUITE" ]; then suites=("$JS_SUITE"); fi

    local total_pass=0
    local total_fail=0
    local failed_suites=()

    for suite in "${suites[@]}"; do
        printf "  %-22s" "${suite}"

        local cargo_cmd="${CARGO} +stable test ${RELEASE_FLAG} --test ${suite}"
        if [ "$VERBOSE" -eq 1 ]; then cargo_cmd="${cargo_cmd} -- --nocapture"; fi

        set +e
        output=$(cd "$LIBJS_TESTS_DIR" && eval "$cargo_cmd" 2>&1)
        rc=$?
        set -e

        pass=$(echo "$output" | grep -E '^test result:' | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+' || echo "0")
        fail=$(echo "$output" | grep -E '^test result:' | grep -oE '[0-9]+ failed' | grep -oE '[0-9]+' || echo "0")
        pass=${pass:-0}; fail=${fail:-0}
        total_pass=$((total_pass + pass))
        total_fail=$((total_fail + fail))

        if [ "$rc" -eq 0 ]; then
            echo -e "${GREEN}✓ ${pass} passed${NC}"
        else
            echo -e "${RED}✗ ${pass} passed  ${fail} failed${NC}"
            failed_suites+=("$suite")
            if [ "$VERBOSE" -eq 1 ]; then
                echo "$output" | grep -E 'panicked at' | sed 's/^/             /' || true
            fi
        fi
    done

    echo ""
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BOLD}  Total: ${GREEN}${total_pass} passed${NC}  ${RED}${total_fail} failed${NC}"
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""

    if [ ${#failed_suites[@]} -gt 0 ]; then
        echo -e "  ${RED}Failed: ${failed_suites[*]}${NC}"
        echo ""
        return 1
    fi
}

# ── tc39/test262 conformance ──────────────────────────────────────────────────

run_tc39_tests() {
    echo ""
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}${BOLD}  tc39/test262 Conformance Suite${NC}"
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""

    # Download if requested or if directory doesn't exist
    if [ "$TC39_DOWNLOAD" -eq 1 ] || [ ! -d "${TC39_DIR}/language" ]; then
        if [ ! -d "${TC39_DIR}/language" ]; then
            echo -e "${YELLOW}  test262 not found — downloading automatically …${NC}"
            echo ""
        fi
        "${SCRIPT_DIR}/download_test262.sh"
    fi

    if [ ! -d "${TC39_DIR}" ]; then
        echo -e "${RED}  ERROR: test262 directory not found: ${TC39_DIR}${NC}"
        echo -e "  Run:  ${BOLD}./scripts/test.sh --tc39-download${NC}"
        echo ""
        return 1
    fi

    # Build the tc39_runner binary
    echo -e "${BOLD}  Building tc39_runner …${NC}"
    (cd "$LIBJS_TESTS_DIR" && "$CARGO" +stable build ${RELEASE_FLAG} --bin tc39_runner 2>&1 \
        | grep -v '^warning:' || true)
    echo ""

    if [ "$RELEASE_FLAG" == "--release" ]; then
        RUNNER="${LIBJS_TESTS_DIR}/target/x86_64-unknown-linux-gnu/release/tc39_runner"
    else
        RUNNER="${LIBJS_TESTS_DIR}/target/x86_64-unknown-linux-gnu/debug/tc39_runner"
    fi

    if [ ! -f "$RUNNER" ]; then
        echo -e "${RED}  ERROR: tc39_runner binary not found at ${RUNNER}${NC}"
        return 1
    fi

    local verbose_flag=""
    if [ "$VERBOSE" -eq 1 ]; then verbose_flag="--verbose"; fi

    # Run each category separately so we get a per-category summary
    local categories=()
    while IFS= read -r -d '' dir; do
        categories+=("$dir")
    done < <(find "$TC39_DIR" -mindepth 1 -maxdepth 2 -type d ! -name harness -print0 | sort -z)

    local grand_pass=0
    local grand_fail=0
    local grand_skip=0

    for cat_dir in "${categories[@]}"; do
        rel=$(realpath --relative-to="$TC39_DIR" "$cat_dir" 2>/dev/null || echo "$cat_dir")
        count=$(find "$cat_dir" -maxdepth 1 -name '*.js' | wc -l | tr -d ' ')
        [ "$count" -eq 0 ] && continue

        printf "  %-40s" "${rel}"

        set +e
        result=$("$RUNNER" "$cat_dir" ${verbose_flag} 2>&1)
        rc=$?
        set -e

        p=$(echo "$result" | grep -oE '[0-9]+ passed'  | grep -oE '[0-9]+' || echo "0")
        f=$(echo "$result" | grep -oE '[0-9]+ failed'  | grep -oE '[0-9]+' || echo "0")
        s=$(echo "$result" | grep -oE '[0-9]+ skipped' | grep -oE '[0-9]+' || echo "0")
        p=${p:-0}; f=${f:-0}; s=${s:-0}

        grand_pass=$((grand_pass + p))
        grand_fail=$((grand_fail + f))
        grand_skip=$((grand_skip + s))

        if [ "$f" -eq 0 ]; then
            echo -e "${GREEN}✓ ${p}p ${s}s${NC}"
        else
            echo -e "${RED}✗ ${p}p ${f}f ${s}s${NC}"
            if [ "$VERBOSE" -eq 1 ]; then
                echo "$result" | grep "^  FAIL" | head -5 | sed 's/^/             /' || true
            fi
        fi
    done

    echo ""
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BOLD}  Total: ${GREEN}${grand_pass} passed${NC}  ${RED}${grand_fail} failed${NC}  ${YELLOW}${grand_skip} skipped${NC}"
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
}

# ── dispatch ──────────────────────────────────────────────────────────────────

FAILED=0

if [ "$RUN_KUNIT" -eq 1 ]; then
    run_kunit_tests || FAILED=1
fi

if [ "$RUN_JS" -eq 1 ]; then
    run_js_tests || FAILED=1
fi

if [ "$RUN_TC39" -eq 1 ]; then
    run_tc39_tests || FAILED=1
fi

exit "$FAILED"

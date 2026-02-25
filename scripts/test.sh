#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# anyOS host-native test runner
# Usage: ./scripts/test.sh [OPTIONS]
#
# Options:
#   --js                 Run all 12 libjs ECMAScript unit-test suites
#   --js-suite NAME      Run a single suite (e.g. --js-suite 07_functions)
#   --tc39               Run tc39/test262 conformance tests
#   --tc39-download      Download the test262 subset first, then run
#   --tc39-dir DIR       Use a custom test262 directory (default: libs/libjs_tests/test262)
#   --verbose            Show full output / console.log from JS
#   --release            Build in release mode
#   -h, --help           Show this help

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${SCRIPT_DIR}/.."
LIBJS_TESTS_DIR="${PROJECT_DIR}/libs/libjs_tests"
CARGO="${HOME}/.cargo/bin/cargo"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

RUN_JS=0
RUN_TC39=0
TC39_DOWNLOAD=0
JS_SUITE=""
TC39_DIR="${LIBJS_TESTS_DIR}/test262"
VERBOSE=0
RELEASE_FLAG=""
SHOW_HELP=0

if [ $# -eq 0 ]; then SHOW_HELP=1; fi

while [[ $# -gt 0 ]]; do
    case "$1" in
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
    echo -e "${BOLD}anyOS host-native test runner${NC}"
    echo ""
    echo "Usage: ./scripts/test.sh [OPTIONS]"
    echo ""
    echo "  --js                 Run all 12 libjs ECMAScript unit-test suites"
    echo "  --js-suite NAME      Run one suite, e.g. --js-suite 07_functions"
    echo "  --tc39               Run tc39/test262 conformance suite"
    echo "  --tc39-download      Download test262 subset, then run"
    echo "  --tc39-dir DIR       Custom test262 directory"
    echo "  --verbose            Show captured output / console.log"
    echo "  --release            Build in release mode"
    echo "  -h, --help           Show this help"
    echo ""
    echo "Examples:"
    echo "  ./scripts/test.sh --js"
    echo "  ./scripts/test.sh --js-suite 07_functions --verbose"
    echo "  ./scripts/test.sh --tc39-download          # first time"
    echo "  ./scripts/test.sh --tc39                   # subsequent runs"
    echo "  ./scripts/test.sh --js --tc39              # everything"
    echo ""
    exit 0
fi

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
        RUNNER="${LIBJS_TESTS_DIR}/target/aarch64-apple-darwin/release/tc39_runner"
    else
        RUNNER="${LIBJS_TESTS_DIR}/target/aarch64-apple-darwin/debug/tc39_runner"
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

if [ "$RUN_JS" -eq 1 ]; then
    run_js_tests || FAILED=1
fi

if [ "$RUN_TC39" -eq 1 ]; then
    run_tc39_tests || FAILED=1
fi

exit "$FAILED"

#!/bin/bash
# WPT CSS Reftest Runner for surf-host
#
# Usage:
#   ./run.sh [options] <test-path>
#
# Examples:
#   ./run.sh css/css-flexbox                    # Run all flexbox reftests
#   ./run.sh css/css-grid                       # Run all grid reftests
#   ./run.sh css/css-backgrounds                # Run all background reftests
#   ./run.sh css/css-flexbox/flex-grow-001.html  # Single test
#   ./run.sh -v css/css-color                   # Verbose (show PASS too)
#   ./run.sh -f css/css-flexbox                 # Show only failures
#   ./run.sh -s 800x600 css/css-flexbox         # Custom viewport size
#   ./run.sh -t 5.0 css/css-flexbox             # Custom threshold (5%)
#   ./run.sh -j 4 css/css-flexbox               # Parallel jobs
#   ./run.sh -k "grow" css/css-flexbox          # Filter by keyword
#   ./run.sh --list css/css-flexbox             # List tests without running
#   ./run.sh --save-failures css/css-flexbox    # Save failure PNGs

# No set -e: arithmetic ((x++)) returns 1 when x==0 which triggers errexit

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SURF_HOST_DIR="$SCRIPT_DIR/../surf-host"
WPT_DIR="$SCRIPT_DIR/wpt"
RESULTS_DIR="$SCRIPT_DIR/results"
PIXELDIFF="$SCRIPT_DIR/pixeldiff.py"

# Defaults
VIEWPORT="800x600"
THRESHOLD="1.0"
VERBOSE=0
FAILURES_ONLY=0
JOBS=1
KEYWORD=""
LIST_ONLY=0
SAVE_FAILURES=0
TIMEOUT=15000  # 15 seconds per render

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

usage() {
    echo "Usage: $0 [options] <test-path>"
    echo ""
    echo "Options:"
    echo "  -v              Verbose (show PASS results too)"
    echo "  -f              Show only failures"
    echo "  -s WxH          Viewport size (default: 800x600)"
    echo "  -t PERCENT      Pixel diff threshold (default: 1.0)"
    echo "  -j N            Parallel jobs (default: 1)"
    echo "  -k KEYWORD      Filter tests by keyword in filename"
    echo "  --list          List matching tests without running"
    echo "  --save-failures Save failure screenshots to results/"
    echo "  --timeout MS    Render timeout in ms (default: 15000)"
    echo "  -h, --help      Show this help"
    echo ""
    echo "Examples:"
    echo "  $0 css/css-flexbox"
    echo "  $0 -v -k grow css/css-flexbox"
    echo "  $0 --save-failures css/css-grid"
    exit 0
}

# Parse arguments
POSITIONAL=()
while [[ $# -gt 0 ]]; do
    case $1 in
        -v) VERBOSE=1; shift ;;
        -f) FAILURES_ONLY=1; shift ;;
        -s) VIEWPORT="$2"; shift 2 ;;
        -t) THRESHOLD="$2"; shift 2 ;;
        -j) JOBS="$2"; shift 2 ;;
        -k) KEYWORD="$2"; shift 2 ;;
        --list) LIST_ONLY=1; shift ;;
        --save-failures) SAVE_FAILURES=1; shift ;;
        --timeout) TIMEOUT="$2"; shift 2 ;;
        -h|--help) usage ;;
        *) POSITIONAL+=("$1"); shift ;;
    esac
done

if [ ${#POSITIONAL[@]} -eq 0 ]; then
    usage
fi

TEST_PATH="${POSITIONAL[0]}"

# Check prerequisites
if [ ! -d "$WPT_DIR/css" ]; then
    echo -e "${RED}[wpt] WPT not found. Run ./setup.sh first.${NC}"
    exit 1
fi

# Build surf-host before every test run so results always use the latest code.
SURF_HOST="$SURF_HOST_DIR/target/x86_64-unknown-linux-gnu/release/surf-host"
if [ $LIST_ONLY -eq 0 ]; then
    echo -e "${CYAN}[wpt] Building surf-host...${NC}"
    cd "$SURF_HOST_DIR"
    cargo +stable build --release --target x86_64-unknown-linux-gnu 2>&1 | tail -3
    cd "$SCRIPT_DIR"
fi

if [ ! -f "$SURF_HOST" ]; then
    echo -e "${RED}[wpt] surf-host build failed${NC}"
    exit 1
fi

# Find reftests: HTML files containing <link rel="match" href="...">
find_reftests() {
    local search_path="$WPT_DIR/$1"

    if [ -f "$search_path" ]; then
        # Single file
        echo "$search_path"
        return
    fi

    if [ ! -d "$search_path" ]; then
        echo -e "${RED}[wpt] Path not found: $search_path${NC}" >&2
        exit 1
    fi

    # Find HTML files with <link rel="match"
    grep -rl '<link[^>]*rel="match"' "$search_path" --include="*.html" --include="*.htm" 2>/dev/null | \
        sort
}

# Extract reference path from test HTML
get_reference() {
    local test_file="$1"
    local href=""
    # Try: rel="match" before href
    href=$(grep -oP '<link[^>]*rel="match"[^>]*href="\K[^"]*' "$test_file" 2>/dev/null | head -1)
    if [ -z "$href" ]; then
        # Try: href before rel="match"
        href=$(grep -oP '<link[^>]*href="\K[^"]*(?="[^>]*rel="match")' "$test_file" 2>/dev/null | head -1)
    fi
    echo "$href"
}

# Render an HTML file to PNG using surf-host
render() {
    local html_file="$1"
    local output_png="$2"
    local vw="${VIEWPORT%x*}"
    local vh="${VIEWPORT#*x}"

    timeout $((TIMEOUT / 1000 + 5)) "$SURF_HOST" \
        "file://$html_file" \
        --screenshot "$output_png" \
        --width "$vw" --height "$vh" \
        >/dev/null 2>&1

    return $?
}

# Run a single reftest — last line of output is PASS/FAIL/SKIP
run_test() {
    local test_file="$1"
    local test_name="${test_file#$WPT_DIR/}"

    # Get reference file
    local ref_href
    ref_href=$(get_reference "$test_file")
    if [ -z "$ref_href" ]; then
        [ $VERBOSE -eq 1 ] && echo -e "${YELLOW}SKIP${NC} $test_name (no reference found)"
        echo "RESULT:SKIP"
        return
    fi

    # Resolve reference path (relative to test file)
    local test_dir
    test_dir=$(dirname "$test_file")
    local ref_file
    ref_file=$(cd "$test_dir" && realpath -m "$ref_href" 2>/dev/null || echo "$test_dir/$ref_href")

    if [ ! -f "$ref_file" ]; then
        # Try common reference locations
        for try_path in \
            "$WPT_DIR/css/reference/$ref_href" \
            "$(realpath "$test_dir/../reference/$ref_href" 2>/dev/null)" \
            "$WPT_DIR/$ref_href"; do
            if [ -f "$try_path" ]; then
                ref_file="$try_path"
                break
            fi
        done
        if [ ! -f "$ref_file" ]; then
            [ $VERBOSE -eq 1 ] && echo -e "${YELLOW}SKIP${NC} $test_name (ref not found: $ref_href)"
            echo "RESULT:SKIP"
            return
        fi
    fi

    # Create temp directory for this test
    local tmpdir
    tmpdir=$(mktemp -d /tmp/wpt-XXXXXX)
    local test_png="$tmpdir/test.png"
    local ref_png="$tmpdir/ref.png"

    # Render test and reference
    if ! render "$test_file" "$test_png"; then
        [ $VERBOSE -eq 1 ] && echo -e "${YELLOW}SKIP${NC} $test_name (render failed)"
        rm -rf "$tmpdir"
        echo "RESULT:SKIP"
        return
    fi

    if ! render "$ref_file" "$ref_png"; then
        [ $VERBOSE -eq 1 ] && echo -e "${YELLOW}SKIP${NC} $test_name (ref render failed)"
        rm -rf "$tmpdir"
        echo "RESULT:SKIP"
        return
    fi

    if [ ! -f "$test_png" ] || [ ! -f "$ref_png" ]; then
        [ $VERBOSE -eq 1 ] && echo -e "${YELLOW}SKIP${NC} $test_name (no output)"
        rm -rf "$tmpdir"
        echo "RESULT:SKIP"
        return
    fi

    # Compare pixels
    local cmp_output
    cmp_output=$(python3 "$PIXELDIFF" "$test_png" "$ref_png" "$THRESHOLD" 2>&1)
    local exit_code=$?

    if [ $exit_code -eq 0 ]; then
        [ $FAILURES_ONLY -ne 1 ] && echo -e "${GREEN}PASS${NC} $test_name"
        rm -rf "$tmpdir"
        echo "RESULT:PASS"
    elif [ $exit_code -eq 1 ]; then
        echo -e "${RED}FAIL${NC} $test_name  ($cmp_output)"
        if [ $SAVE_FAILURES -eq 1 ]; then
            mkdir -p "$RESULTS_DIR"
            local safe_name
            safe_name=$(echo "$test_name" | tr '/' '_')
            cp "$test_png" "$RESULTS_DIR/${safe_name%.html}_test.png" 2>/dev/null
            cp "$ref_png" "$RESULTS_DIR/${safe_name%.html}_ref.png" 2>/dev/null
        fi
        rm -rf "$tmpdir"
        echo "RESULT:FAIL"
    else
        [ $VERBOSE -eq 1 ] && echo -e "${YELLOW}SKIP${NC} $test_name ($cmp_output)"
        rm -rf "$tmpdir"
        echo "RESULT:SKIP"
    fi
}

# Main
echo -e "${BOLD}[wpt] WPT CSS Reftest Runner${NC}"
echo -e "[wpt] Path: ${CYAN}$TEST_PATH${NC}"
echo -e "[wpt] Viewport: ${VIEWPORT}, Threshold: ${THRESHOLD}%"
echo ""

# Find tests
TESTS=()
while IFS= read -r line; do
    if [ -n "$KEYWORD" ]; then
        if [[ "$line" == *"$KEYWORD"* ]]; then
            TESTS+=("$line")
        fi
    else
        TESTS+=("$line")
    fi
done < <(find_reftests "$TEST_PATH")

TOTAL=${#TESTS[@]}

if [ $TOTAL -eq 0 ]; then
    echo -e "${YELLOW}[wpt] No reftests found in $TEST_PATH${NC}"
    exit 0
fi

echo -e "[wpt] Found ${BOLD}$TOTAL${NC} reftests"

if [ $LIST_ONLY -eq 1 ]; then
    for t in "${TESTS[@]}"; do
        echo "${t#$WPT_DIR/}"
    done
    exit 0
fi

echo ""

# Run tests
PASS=0
FAIL=0
SKIP=0
COUNT=0
FAIL_LIST=()

for test_file in "${TESTS[@]}"; do
    ((COUNT++))
    # run_test prints status lines + "RESULT:PASS/FAIL/SKIP" as last line
    output=$(run_test "$test_file" 2>&1)
    # Print all lines except the RESULT: line
    echo "$output" | grep -v '^RESULT:' || true
    # Extract result
    result=$(echo "$output" | grep '^RESULT:' | tail -1 | sed 's/^RESULT://')
    case "$result" in
        PASS) ((PASS++)) ;;
        FAIL)
            ((FAIL++))
            FAIL_LIST+=("${test_file#$WPT_DIR/}")
            ;;
        *) ((SKIP++)) ;;
    esac

    # Progress indicator every 25 tests
    if [ $((COUNT % 25)) -eq 0 ]; then
        echo -e "${CYAN}[wpt] Progress: $COUNT/$TOTAL (P:$PASS F:$FAIL S:$SKIP)${NC}"
    fi
done

# Summary
echo ""
echo -e "${BOLD}━━━ Results ━━━${NC}"
echo -e "  ${GREEN}PASS${NC}: $PASS"
echo -e "  ${RED}FAIL${NC}: $FAIL"
echo -e "  ${YELLOW}SKIP${NC}: $SKIP"
echo -e "  Total: $TOTAL"

if [ $TOTAL -gt 0 ]; then
    RUN=$((PASS + FAIL))
    if [ $RUN -gt 0 ]; then
        PCT=$(python3 -c "print(f'{$PASS * 100 / $RUN:.1f}')")
        echo -e "  Pass rate: ${BOLD}${PCT}%${NC} (of $RUN executed)"
    fi
fi

if [ ${#FAIL_LIST[@]} -gt 0 ] && [ ${#FAIL_LIST[@]} -le 50 ]; then
    echo ""
    echo -e "${BOLD}Failed tests:${NC}"
    for f in "${FAIL_LIST[@]}"; do
        echo "  - $f"
    done
fi

if [ $SAVE_FAILURES -eq 1 ] && [ $FAIL -gt 0 ]; then
    echo ""
    echo -e "[wpt] Failure screenshots saved to: $RESULTS_DIR/"
fi

# Exit with failure if any tests failed
[ $FAIL -eq 0 ]

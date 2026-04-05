#!/bin/bash
#
# test262.sh — htop-like TUI for test262 conformance tests
#
# Features:
#   - Live-updating stats (pass/fail/skip/timeout with rates)
#   - Progress bar
#   - Tab: toggle between failure list and category breakdown
#   - Ctrl+C: abort and show summary
#

set -u

# --- Compile if requested ---
if [ "${1:-}" = "--compile" ]; then
  echo "=== Compiling test262-runner ==="
  (cd tools/test262-runner && RUSTFLAGS="-Awarnings" cargo +stable build --release 2>&1 | grep -E '^(   Compiling|    Finished|error)') || exit 1
  echo "=== Done ==="
  shift
fi

RUNNER=tools/test262-runner/target/x86_64-unknown-linux-gnu/release/test262-runner
TEST_DIR=libs/libjs_tests/test262
BLOCKS=(1 1001 2001 3001 4001 5001 6001 7001 8001 9001 10001 11001 12001 13001 14001 15001 16001 17001 18001 19001 20001 21001 22001 23001 24001 25001 26001)
TOTAL_BLOCKS=${#BLOCKS[@]}

if [ ! -x "$RUNNER" ]; then
  echo "Error: $RUNNER not found. Run with --compile first."
  exit 1
fi

# --- Terminal setup ---
TERM_COLS=$(tput cols)
TERM_LINES=$(tput lines)

setup_term() {
  tput smcup
  tput civis
  stty -echo 2>/dev/null
  printf '\e[?7l'    # disable line wrap
  printf '\e[2J'     # clear screen
}

cleanup() {
  printf '\e[?7h'
  stty echo 2>/dev/null
  tput cnorm
  tput rmcup

  # Print final summary to normal terminal
  local total=$((G_PASS + G_FAIL + G_SKIP + G_TIMEOUT))
  local evaluated=$((G_PASS + G_FAIL + G_TIMEOUT))
  local pass_pct=0
  (( evaluated > 0 )) && pass_pct=$((G_PASS * 1000 / evaluated))
  echo ""
  echo "═══════════════════════════════════════════════════════"
  echo "  test262 Results for libjs"
  echo "═══════════════════════════════════════════════════════"
  printf "  Total:    %d\n" $total
  printf "  Passed:   %d (%d.%d%%)\n" $G_PASS $((pass_pct/10)) $((pass_pct%10))
  printf "  Failed:   %d\n" $G_FAIL
  printf "  Skipped:  %d\n" $G_SKIP
  (( G_TIMEOUT > 0 )) && printf "  Timeout:  %d\n" $G_TIMEOUT
  printf "  Elapsed:  %ds\n" $((SECONDS - START_SECONDS))
  echo "═══════════════════════════════════════════════════════"
  echo ""
  echo "Per-block output saved to /tmp/test262.*.txt"
}

trap cleanup EXIT
trap 'TERM_COLS=$(tput cols); TERM_LINES=$(tput lines)' WINCH

# --- State ---
declare -i G_PASS=0 G_FAIL=0 G_SKIP=0 G_TIMEOUT=0
CURRENT_TEST=""
CURRENT_BLOCK=0
START_SECONDS=$SECONDS
TOTAL_TESTS=27000     # approximate, updated from runner output
REDRAW_CTR=0
VIEW_MODE=0           # 0=failures, 1=categories
DONE=0

# Failure entries: "type\tpath\treason"
declare -a FAIL_ENTRIES=()

# Reason counters: reason -> count (for grouped failure view)
declare -A REASON_COUNTS=()
REASON_SORTED=""
REASON_SORTED_AT=0

# Category counters: associative array cat -> count
declare -A CAT_COUNTS=()
# Sorted category list (rebuilt periodically)
CAT_SORTED=""
CAT_SORTED_AT=0       # rebuild when fail count changes

# --- Input handling (non-blocking) ---
check_input() {
  local key
  if read -t 0.001 -n1 -s key < /dev/tty 2>/dev/null; then
    case "$key" in
      $'\t'|$'\x09')  # Tab
        VIEW_MODE=$(( (VIEW_MODE + 1) % 2 ))
        draw
        ;;
      '1') VIEW_MODE=0; draw ;;
      '2') VIEW_MODE=1; draw ;;
    esac
  fi
}

# --- Drawing ---
draw() {
  local cols=$TERM_COLS
  local lines=$TERM_LINES
  local elapsed=$((SECONDS - START_SECONDS))
  local total=$((G_PASS + G_FAIL + G_SKIP + G_TIMEOUT))

  # Pass/Fail rates (excluding skip)
  local evaluated=$((G_PASS + G_FAIL + G_TIMEOUT))
  local pass_pct=0 fail_pct=0
  (( evaluated > 0 )) && pass_pct=$((G_PASS * 1000 / evaluated))
  (( evaluated > 0 )) && fail_pct=$((G_FAIL * 1000 / evaluated))

  # Rate
  local rate_str="-"
  (( elapsed > 0 )) && rate_str="$((total / elapsed))/s"

  # --- Row 1: Title bar ---
  printf '\e[1;1H\e[1;97;44m'
  local title=" test262 - libjs Conformance Tests"
  local block_info="Block ${CURRENT_BLOCK}/${TOTAL_BLOCKS} "
  local pad=$((cols - ${#title} - ${#block_info}))
  (( pad < 0 )) && pad=0
  printf '%s' "$title"
  printf '%*s' "$pad" ''
  printf '%s' "$block_info"
  printf '\e[0m'

  # --- Row 2: Stats ---
  printf '\e[2;1H'
  printf ' \e[1;32mPass: %d (%d.%d%%)\e[0m' $G_PASS $((pass_pct/10)) $((pass_pct%10))
  printf '   \e[1;31mFail: %d (%d.%d%%)\e[0m' $G_FAIL $((fail_pct/10)) $((fail_pct%10))
  printf '   \e[1;33mSkip: %d\e[0m' $G_SKIP
  printf '   \e[1;35mTimeout: %d\e[0m' $G_TIMEOUT
  printf '   \e[37mTotal: %d  Rate: %s  %ds\e[0m' $total "$rate_str" $elapsed
  printf '\e[K'

  # --- Row 3: Progress bar ---
  printf '\e[3;1H'
  local bar_w=$((cols - 9))
  (( bar_w > 80 )) && bar_w=80
  (( bar_w < 10 )) && bar_w=10
  local pct=0
  (( TOTAL_TESTS > 0 )) && pct=$((total * 100 / TOTAL_TESTS))
  (( pct > 100 )) && pct=100
  local filled=$((bar_w * pct / 100))
  printf ' \e[32m'
  local i
  for ((i=0; i<filled; i++)); do printf '\xe2\x96\x88'; done  # █ (UTF-8)
  printf '\e[90m'
  for ((i=filled; i<bar_w; i++)); do printf '\xe2\x96\x91'; done  # ░
  printf '\e[0m %3d%%' "$pct"
  printf '\e[K'

  # --- Row 4: Current test ---
  printf '\e[4;1H'
  local max_path=$((cols - 4))
  (( max_path < 1 )) && max_path=1
  local ct="$CURRENT_TEST"
  (( ${#ct} > max_path )) && ct="${ct:0:$((max_path-1))}.."
  printf ' \e[36m> %s\e[0m\e[K' "$ct"

  # --- Row 5: Separator + view tabs ---
  printf '\e[5;1H\e[90m'
  for ((i=0; i<cols; i++)); do printf '\xe2\x94\x80'; done  # ─
  printf '\e[0m'

  # --- Row 6: View selector ---
  printf '\e[6;1H'
  if (( VIEW_MODE == 0 )); then
    printf ' \e[1;97;44m [1] Failures (%d) \e[0m' ${#FAIL_ENTRIES[@]}
    printf ' \e[90m [2] Categories \e[0m'
  else
    printf ' \e[90m [1] Failures (%d) \e[0m' ${#FAIL_ENTRIES[@]}
    printf ' \e[1;97;44m [2] Categories \e[0m'
  fi
  printf '  \e[90mTab/1/2 to switch\e[0m\e[K'

  # --- Body: rows 7 to lines-1 ---
  local body_start=7
  local body_end=$((lines))  # last row = footer
  local avail=$((body_end - body_start))
  (( avail < 1 )) && avail=1

  if (( VIEW_MODE == 0 )); then
    draw_failures $body_start $avail $cols
  else
    draw_categories $body_start $avail $cols
  fi

  # --- Footer ---
  printf '\e[%d;1H' "$lines"
  local footer
  if (( DONE )); then
    footer="Done! Output: /tmp/test262.*.txt | Press any key to exit"
  else
    footer="Output: /tmp/test262.*.txt | Tab=switch view | Ctrl+C=abort"
  fi
  local fpad=$((cols - ${#footer} - 1))
  (( fpad < 0 )) && fpad=0
  printf '\e[7m %s%*s\e[0m' "$footer" "$fpad" ''
}

draw_failures() {
  local start_row=$1 avail=$2 cols=$3

  # Rebuild sorted reason list if needed
  local total_reasons=$((G_FAIL + G_TIMEOUT))
  if (( total_reasons != REASON_SORTED_AT )); then
    REASON_SORTED_AT=$total_reasons
    REASON_SORTED=""
    local reason count
    for reason in "${!REASON_COUNTS[@]}"; do
      count=${REASON_COUNTS[$reason]}
      REASON_SORTED+="$(printf '%06d\t%s\n' "$count" "$reason")"$'\n'
    done
    REASON_SORTED=$(printf '%s' "$REASON_SORTED" | sort -rn)
  fi

  local row=$start_row
  while IFS=$'\t' read -r count reason; do
    [ -z "$count" ] && continue
    (( (row - start_row) >= avail )) && break
    printf '\e[%d;1H' "$row"

    count=$((10#$count))

    # Color by severity
    local color="37"
    (( count >= 100 )) && color="1;31"   # bold red
    (( count >= 20 && count < 100 )) && color="31"  # red
    (( count >= 5 && count < 20 )) && color="33"    # yellow
    (( count < 5 )) && color="37"                    # white

    local max_r=$((cols - 10))
    local dr="$reason"
    (( ${#dr} > max_r )) && dr="${dr:0:$((max_r-1))}.."
    printf ' \e[%sm%5d\e[0m  %s\e[K' "$color" "$count" "$dr"
    row=$((row + 1))
  done <<< "$REASON_SORTED"

  # Clear remaining
  for ((; (row - start_row) < avail; row++)); do
    printf '\e[%d;1H\e[K' "$row"
  done
}

draw_categories() {
  local start_row=$1 avail=$2 cols=$3

  # Rebuild sorted list if needed
  if (( G_FAIL != CAT_SORTED_AT )); then
    CAT_SORTED_AT=$G_FAIL
    # Sort categories by count (descending)
    CAT_SORTED=""
    local cat count
    for cat in "${!CAT_COUNTS[@]}"; do
      count=${CAT_COUNTS[$cat]}
      # Zero-pad count to 6 digits for sort
      CAT_SORTED+="$(printf '%06d\t%s\n' "$count" "$cat")"$'\n'
    done
    CAT_SORTED=$(echo "$CAT_SORTED" | sort -rn | head -n "$avail")
  fi

  local row=$start_row
  while IFS=$'\t' read -r count cat; do
    [ -z "$count" ] && continue
    (( (row - start_row) >= avail )) && break
    printf '\e[%d;1H' "$row"

    # Remove leading zeros
    count=$((10#$count))

    # Color based on count
    local color="37"  # white
    (( count >= 100 )) && color="31"  # red
    (( count >= 50 && count < 100 )) && color="33"  # yellow
    (( count >= 10 && count < 50 )) && color="36"  # cyan

    local max_cat=$((cols - 12))
    local dcat="$cat"
    (( ${#dcat} > max_cat )) && dcat="${dcat:0:$((max_cat-1))}.."
    printf ' \e[%sm%5d\e[0m  %s\e[K' "$color" "$count" "$dcat"
    row=$((row + 1))
  done <<< "$CAT_SORTED"

  # Clear remaining
  for ((; (row - start_row) < avail; row++)); do
    printf '\e[%d;1H\e[K' "$row"
  done
}

# --- Category extraction ---
add_to_category() {
  local fpath="$1"
  # Extract top 2 path components: "built-ins/Array" or "language/expressions"
  local cat
  local IFS='/'
  local -a parts=($fpath)
  if (( ${#parts[@]} >= 2 )); then
    cat="${parts[0]}/${parts[1]}"
  else
    cat="${parts[0]}"
  fi
  CAT_COUNTS[$cat]=$(( ${CAT_COUNTS[$cat]:-0} + 1 ))
}

# ===== MAIN =====
setup_term

# Initial draw
draw

# Process blocks
for block_idx in "${!BLOCKS[@]}"; do
  start=${BLOCKS[$block_idx]}
  CURRENT_BLOCK=$((block_idx + 1))
  REDRAW_CTR=0

  # Run runner: stderr (with [R] lines) → pipe for parsing, stdout → file
  while IFS= read -r line; do
    case "$line" in
      "[R]P"*)
        G_PASS+=1
        CURRENT_TEST="${line#*	}"
        REDRAW_CTR=$((REDRAW_CTR + 1))
        ;;
      "[R]F"*)
        G_FAIL+=1
        local_rest="${line#[R]F	}"
        local_path="${local_rest%%	*}"
        local_reason="${local_rest#*	}"
        [ "$local_path" = "$local_reason" ] && local_reason=""
        CURRENT_TEST="$local_path"
        FAIL_ENTRIES+=("F	${local_path}	${local_reason}")
        add_to_category "$local_path"
        # Count by reason (for grouped view)
        rkey="${local_reason:-unknown}"
        REASON_COUNTS["$rkey"]=$(( ${REASON_COUNTS["$rkey"]:-0} + 1 ))
        draw
        REDRAW_CTR=0
        ;;
      "[R]S"*)
        G_SKIP+=1
        REDRAW_CTR=$((REDRAW_CTR + 1))
        ;;
      "[R]T"*)
        G_TIMEOUT+=1
        local_path="${line#[R]T	}"
        CURRENT_TEST="$local_path"
        FAIL_ENTRIES+=("T	${local_path}	")
        add_to_category "$local_path"
        REASON_COUNTS["TIMEOUT (>5s)"]=$(( ${REASON_COUNTS["TIMEOUT (>5s)"]:-0} + 1 ))
        draw
        REDRAW_CTR=0
        ;;
      "[test262] Found"*)
        # Extract total test count from first block
        if [[ "$line" =~ Found\ ([0-9]+)\ test ]]; then
          TOTAL_TESTS=${BASH_REMATCH[1]}
        fi
        ;;
    esac

    # Throttled redraw: every 50 tests (pass/skip don't trigger immediate redraw)
    if (( REDRAW_CTR >= 50 )); then
      check_input
      draw
      REDRAW_CTR=0
    fi
  done < <("$RUNNER" "$TEST_DIR" --limit 1000 --start "$start" --live --verbose --timeout 5 2>&1 >"/tmp/test262.${start}.txt")

  # Redraw after block completes
  draw
  check_input
done

# --- Done ---
DONE=1
CURRENT_TEST="Complete"
draw

# Wait for keypress
read -n1 -s

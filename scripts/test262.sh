#!/bin/bash
#
# test262.sh — parallel test262 runner with htop-like TUI
#
# Features:
#   - Parallel execution (uses half of CPU cores)
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

if [ ! -x "$RUNNER" ]; then
  echo "Error: $RUNNER not found. Run with --compile first."
  exit 1
fi

# Parallel worker count — use half of CPU cores (tests are CPU-bound)
NPROC=$(nproc 2>/dev/null || echo 4)
PARALLEL=$((NPROC / 2))
(( PARALLEL < 2 )) && PARALLEL=2
(( PARALLEL > 8 )) && PARALLEL=8

# Total test count
TOTAL_TESTS=26081
BLOCK_SIZE=$(( TOTAL_TESTS / PARALLEL + 1 ))

# --- Terminal setup ---
TERM_COLS=$(tput cols 2>/dev/null || echo 80)
TERM_LINES=$(tput lines 2>/dev/null || echo 24)
(( TERM_COLS < 40 )) && TERM_COLS=80
(( TERM_LINES < 10 )) && TERM_LINES=24

setup_term() {
  tput smcup
  tput civis
  stty -echo 2>/dev/null
  printf '\e[?7l'    # disable line wrap
  printf '\e[2J'     # clear screen
}

cleanup() {
  # Kill all worker processes
  for pid in "${WORKER_PIDS[@]:-}"; do
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
  done

  printf '\e[?7h'
  stty echo 2>/dev/null
  tput cnorm
  tput rmcup

  # Print final summary to normal terminal
  parse_results
}

trap cleanup EXIT
trap 'TERM_COLS=$(tput cols 2>/dev/null || echo 80); TERM_LINES=$(tput lines 2>/dev/null || echo 24)' WINCH

# --- State ---
declare -i G_PASS=0 G_FAIL=0 G_SKIP=0 G_TIMEOUT=0
CURRENT_TEST=""
START_SECONDS=$SECONDS
TOTAL_TESTS=26081
VIEW_MODE=0           # 0=failures, 1=categories
DONE=0
WORKERS_DONE=0

# Result files (one per worker, appended line by line)
RESULT_DIR="/tmp/test262_results_$$"
mkdir -p "$RESULT_DIR"

WORKER_PIDS=()

# --- Parse final results from worker output files ---
parse_results() {
  local total_pass=0 total_fail=0 total_skip=0 total_timeout=0
  local elapsed=$((SECONDS - START_SECONDS))

  # Parse all worker result files
  for f in "$RESULT_DIR"/worker_*.stderr; do
    [ -f "$f" ] || continue
    while IFS= read -r line; do
      case "$line" in
        "[R]P"*) total_pass=$((total_pass + 1)) ;;
        "[R]F"*) total_fail=$((total_fail + 1)) ;;
        "[R]S"*) total_skip=$((total_skip + 1)) ;;
        "[R]T"*) total_timeout=$((total_timeout + 1)) ;;
      esac
    done < "$f"
  done

  local total=$((total_pass + total_fail + total_skip + total_timeout))
  local evaluated=$((total_pass + total_fail + total_timeout))
  local pass_pct=0
  (( evaluated > 0 )) && pass_pct=$((total_pass * 1000 / evaluated))

  echo ""
  echo "═══════════════════════════════════════════════════════"
  echo "  test262 Results for libjs"
  echo "═══════════════════════════════════════════════════════"
  printf "  Total:    %d\n" $total
  printf "  Passed:   %d (%d.%d%%)\n" $total_pass $((pass_pct/10)) $((pass_pct%10))
  printf "  Failed:   %d\n" $total_fail
  printf "  Skipped:  %d\n" $total_skip
  (( total_timeout > 0 )) && printf "  Timeout:  %d\n" $total_timeout
  printf "  Workers:  %d parallel\n" $PARALLEL
  printf "  Elapsed:  %ds\n" $elapsed
  echo "═══════════════════════════════════════════════════════"

  # Show top failure reasons
  echo ""
  echo "Top failure reasons:"
  local tmpf=$(mktemp)
  for f in "$RESULT_DIR"/worker_*.stderr; do
    [ -f "$f" ] || continue
    grep '^\[R\]F' "$f" | while IFS=$'\t' read -r _ _ reason; do
      echo "$reason"
    done
  done | sort | uniq -c | sort -rn | head -20 > "$tmpf"
  while read -r count reason; do
    printf "  %5d  %s\n" "$count" "$reason"
  done < "$tmpf"
  rm -f "$tmpf"

  echo ""
  echo "Per-worker output saved to $RESULT_DIR/"
  # Clean up old /tmp/test262.*.txt
  rm -f /tmp/test262.[0-9]*.txt
}

# --- Input handling (non-blocking) ---
check_input() {
  local key
  if read -t 0.001 -n1 -s key < /dev/tty 2>/dev/null; then
    case "$key" in
      $'\t'|$'\x09')  VIEW_MODE=$(( (VIEW_MODE + 1) % 2 )) ;;
      '1') VIEW_MODE=0 ;;
      '2') VIEW_MODE=1 ;;
    esac
  fi
}

# --- Drawing ---
draw() {
  local cols=$TERM_COLS
  local lines=$TERM_LINES
  local elapsed=$((SECONDS - START_SECONDS))
  local total=$((G_PASS + G_FAIL + G_SKIP + G_TIMEOUT))

  local evaluated=$((G_PASS + G_FAIL + G_TIMEOUT))
  local pass_pct=0 fail_pct=0
  (( evaluated > 0 )) && pass_pct=$((G_PASS * 1000 / evaluated))
  (( evaluated > 0 )) && fail_pct=$((G_FAIL * 1000 / evaluated))

  local rate_str="-"
  (( elapsed > 0 )) && rate_str="$((total / elapsed))/s"

  # --- Row 1: Title bar ---
  printf '\e[1;1H\e[1;97;44m'
  local title=" test262 - libjs Conformance Tests"
  local worker_info="${PARALLEL} workers "
  local pad=$((cols - ${#title} - ${#worker_info}))
  (( pad < 0 )) && pad=0
  printf '%s' "$title"
  printf '%*s' "$pad" ''
  printf '%s' "$worker_info"
  printf '\e[0m'

  # --- Row 2: Stats ---
  printf '\e[2;1H'
  printf ' \e[1;32mPass: %d (%d.%d%%)\e[0m' $G_PASS $((pass_pct/10)) $((pass_pct%10))
  printf '   \e[1;31mFail: %d (%d.%d%%)\e[0m' $G_FAIL $((fail_pct/10)) $((fail_pct%10))
  printf '   \e[1;33mSkip: %d\e[0m' $G_SKIP
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
  for ((i=0; i<filled; i++)); do printf '\xe2\x96\x88'; done
  printf '\e[90m'
  for ((i=filled; i<bar_w; i++)); do printf '\xe2\x96\x91'; done
  printf '\e[0m %3d%%' "$pct"
  printf '\e[K'

  # --- Row 4: Current test ---
  printf '\e[4;1H'
  local max_path=$((cols - 4))
  (( max_path < 1 )) && max_path=1
  local ct="$CURRENT_TEST"
  (( ${#ct} > max_path )) && ct="${ct:0:$((max_path-1))}.."
  printf ' \e[36m> %s\e[0m\e[K' "$ct"

  # --- Row 5: Separator ---
  printf '\e[5;1H\e[90m'
  for ((i=0; i<cols; i++)); do printf '\xe2\x94\x80'; done
  printf '\e[0m'

  # --- Row 6: View selector ---
  printf '\e[6;1H'
  if (( VIEW_MODE == 0 )); then
    printf ' \e[1;97;44m [1] Failures \e[0m'
    printf ' \e[90m [2] Categories \e[0m'
  else
    printf ' \e[90m [1] Failures \e[0m'
    printf ' \e[1;97;44m [2] Categories \e[0m'
  fi
  printf '  \e[90mTab/1/2 to switch\e[0m\e[K'

  # --- Body: recent failures or category counts (from tail of result files) ---
  local body_start=7
  local body_end=$lines
  local avail=$((body_end - body_start))
  (( avail < 1 )) && avail=1

  if (( VIEW_MODE == 0 )); then
    draw_recent_failures $body_start $avail $cols
  else
    draw_categories_live $body_start $avail $cols
  fi

  # --- Footer ---
  printf '\e[%d;1H' "$lines"
  local footer
  if (( DONE )); then
    footer="Done! Press any key to exit"
  else
    footer="Tab=switch view | Ctrl+C=abort"
  fi
  local fpad=$((cols - ${#footer} - 1))
  (( fpad < 0 )) && fpad=0
  printf '\e[7m %s%*s\e[0m' "$footer" "$fpad" ''
}

# Show recent failure reasons grouped by count (from in-memory counters)
draw_recent_failures() {
  local start_row=$1 avail=$2 cols=$3
  local row=$start_row

  # Use the REASON_TOP array (rebuilt periodically)
  local idx=0
  while (( idx < ${#REASON_TOP[@]} && (row - start_row) < avail )); do
    local entry="${REASON_TOP[$idx]}"
    local count="${entry%%	*}"
    local reason="${entry#*	}"
    printf '\e[%d;1H' "$row"

    local color="37"
    (( count >= 100 )) && color="1;31"
    (( count >= 20 && count < 100 )) && color="31"
    (( count >= 5 && count < 20 )) && color="33"

    local max_r=$((cols - 10))
    local dr="$reason"
    (( ${#dr} > max_r )) && dr="${dr:0:$((max_r-1))}.."
    printf ' \e[%sm%5d\e[0m  %s\e[K' "$color" "$count" "$dr"
    row=$((row + 1))
    idx=$((idx + 1))
  done

  for ((; (row - start_row) < avail; row++)); do
    printf '\e[%d;1H\e[K' "$row"
  done
}

draw_categories_live() {
  local start_row=$1 avail=$2 cols=$3
  local row=$start_row

  local idx=0
  while (( idx < ${#CAT_TOP[@]} && (row - start_row) < avail )); do
    local entry="${CAT_TOP[$idx]}"
    local count="${entry%%	*}"
    local cat="${entry#*	}"
    printf '\e[%d;1H' "$row"

    local color="37"
    (( count >= 100 )) && color="31"
    (( count >= 50 && count < 100 )) && color="33"
    (( count >= 10 && count < 50 )) && color="36"

    local max_cat=$((cols - 12))
    local dcat="$cat"
    (( ${#dcat} > max_cat )) && dcat="${dcat:0:$((max_cat-1))}.."
    printf ' \e[%sm%5d\e[0m  %s\e[K' "$color" "$count" "$dcat"
    row=$((row + 1))
    idx=$((idx + 1))
  done

  for ((; (row - start_row) < avail; row++)); do
    printf '\e[%d;1H\e[K' "$row"
  done
}

# ===== MAIN =====
setup_term

# Pre-sorted arrays for display (rebuilt periodically from assoc arrays)
declare -a REASON_TOP=()
declare -a CAT_TOP=()
declare -A REASON_COUNTS=()
declare -A CAT_COUNTS=()
LAST_SORT_AT=0

# Rebuild sorted display arrays from associative arrays
rebuild_sorted() {
  local tmpf=$(mktemp)
  local reason count
  for reason in "${!REASON_COUNTS[@]}"; do
    count=${REASON_COUNTS[$reason]}
    printf '%d\t%s\n' "$count" "$reason"
  done | sort -rn | head -30 > "$tmpf"
  REASON_TOP=()
  while IFS= read -r line; do
    REASON_TOP+=("$line")
  done < "$tmpf"
  rm -f "$tmpf"

  tmpf=$(mktemp)
  local cat
  for cat in "${!CAT_COUNTS[@]}"; do
    count=${CAT_COUNTS[$cat]}
    printf '%d\t%s\n' "$count" "$cat"
  done | sort -rn | head -30 > "$tmpf"
  CAT_TOP=()
  while IFS= read -r line; do
    CAT_TOP+=("$line")
  done < "$tmpf"
  rm -f "$tmpf"

  LAST_SORT_AT=$G_FAIL
}

draw

# Launch workers in parallel, each writes stderr to its own file
for ((block_idx=0; block_idx<PARALLEL; block_idx++)); do
  start=$((block_idx * BLOCK_SIZE + 1))
  "$RUNNER" "$TEST_DIR" --limit "$BLOCK_SIZE" --start "$start" --live --verbose --timeout 5 \
    2>"$RESULT_DIR/worker_${block_idx}.stderr" \
    >"$RESULT_DIR/worker_${block_idx}.stdout" &
  WORKER_PIDS+=($!)
done

# Poll worker output files for live stats
REDRAW_CTR=0
declare -A FILE_OFFSETS=()

poll_workers() {
  for ((w=0; w<PARALLEL; w++)); do
    local f="$RESULT_DIR/worker_${w}.stderr"
    [ -f "$f" ] || continue
    local off=${FILE_OFFSETS[$w]:-0}
    local new_lines
    new_lines=$(tail -c +$((off + 1)) "$f" 2>/dev/null) || continue
    [ -z "$new_lines" ] && continue
    local bytes_read=${#new_lines}
    FILE_OFFSETS[$w]=$((off + bytes_read))

    while IFS= read -r line; do
      case "$line" in
        "[R]P"*)
          G_PASS+=1
          CURRENT_TEST="${line#*	}"
          ;;
        "[R]F"*)
          G_FAIL+=1
          local_rest="${line#\[R\]F	}"
          local_path="${local_rest%%	*}"
          local_reason="${local_rest#*	}"
          [ "$local_path" = "$local_reason" ] && local_reason=""
          CURRENT_TEST="$local_path"
          rkey="${local_reason:-unknown}"
          REASON_COUNTS["$rkey"]=$(( ${REASON_COUNTS["$rkey"]:-0} + 1 ))
          # Category
          local IFS='/'
          local -a parts=($local_path)
          if (( ${#parts[@]} >= 2 )); then
            local cat="${parts[0]}/${parts[1]}"
          else
            local cat="${parts[0]}"
          fi
          CAT_COUNTS[$cat]=$(( ${CAT_COUNTS[$cat]:-0} + 1 ))
          ;;
        "[R]S"*)
          G_SKIP+=1
          ;;
        "[R]T"*)
          G_TIMEOUT+=1
          local_path="${line#\[R\]T	}"
          CURRENT_TEST="$local_path"
          REASON_COUNTS["TIMEOUT (>5s)"]=$(( ${REASON_COUNTS["TIMEOUT (>5s)"]:-0} + 1 ))
          ;;
      esac
    done <<< "$new_lines"
  done
}

# Main poll loop
while true; do
  # Check if all workers are done
  WORKERS_DONE=1
  for pid in "${WORKER_PIDS[@]}"; do
    if kill -0 "$pid" 2>/dev/null; then
      WORKERS_DONE=0
      break
    fi
  done

  poll_workers

  # Rebuild sorted arrays every 100 new failures
  if (( G_FAIL > LAST_SORT_AT + 50 || (WORKERS_DONE && G_FAIL != LAST_SORT_AT) )); then
    rebuild_sorted
  fi

  check_input
  draw

  if (( WORKERS_DONE )); then
    # Final poll to catch remaining output
    sleep 0.2
    poll_workers
    if (( G_FAIL != LAST_SORT_AT )); then
      rebuild_sorted
    fi
    draw
    break
  fi

  sleep 0.3
done

# --- Done ---
DONE=1
CURRENT_TEST="Complete"
draw

# Wait for keypress
read -n1 -s < /dev/tty

#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

usage() {
  cat <<'EOF'
Usage:
  scripts/test-agit.sh [repo-url]

Default:
  scripts/test-agit.sh
    clones https://github.com/brianmayclone/serodesk with the host-built agit/cgit,
    verifies refs, HEAD, remote config, object resolution, and clean checkout,
    then deletes the temporary clone.

Environment:
  AGIT_TEST_REPO=<url>  repository URL to clone
  AGIT_BIN=<path>       use an existing agit/cgit binary instead of building one
  AGIT_KEEP_TMP=1       keep the temporary directory for debugging
  AGIT_TIMEOUT=60s      maximum runtime for each agit command
  AGIT_MAX_VMEM_KB=2097152
                         virtual memory ceiling for each agit command

Examples:
  scripts/test-agit.sh
  AGIT_KEEP_TMP=1 scripts/test-agit.sh https://github.com/brianmayclone/serodesk
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

repo_url="${1:-${AGIT_TEST_REPO:-https://github.com/brianmayclone/serodesk}}"
agit_timeout="${AGIT_TIMEOUT:-60s}"
agit_max_vmem_kb="${AGIT_MAX_VMEM_KB:-2097152}"
tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/agit-github-test.XXXXXX")"
repo_name="${repo_url%/}"
repo_name="${repo_name%.git}"
repo_name="${repo_name##*/}"
clone_dir="$tmp_root/${repo_name:-repo}"

cleanup() {
  if [[ "${AGIT_KEEP_TMP:-0}" == "1" ]]; then
    printf 'agit test workspace kept at %s\n' "$tmp_root"
  else
    rm -rf "$tmp_root"
  fi
}
trap cleanup EXIT

if [[ -n "${AGIT_BIN:-}" ]]; then
  agit_bin="$AGIT_BIN"
  if [[ "$agit_bin" != /* ]]; then
    agit_bin="$repo_root/$agit_bin"
  fi
else
  cargo build --manifest-path bin/agit/Cargo.toml --no-default-features --features host
  agit_bin="$repo_root/target/debug/cgit"
fi

if [[ ! -x "$agit_bin" ]]; then
  printf 'error: agit binary is not executable: %s\n' "$agit_bin" >&2
  exit 1
fi

fail() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}

run_agit() {
  local cwd="$1"
  local label="$2"
  shift 2

  local out_file="$tmp_root/${label//[^A-Za-z0-9_.-]/_}.out"
  printf '==> agit %s\n' "$label"
  if ! run_limited "$cwd" "$out_file" "$@"; then
    cat "$out_file"
    fail "command failed: agit $label"
  fi

  if rg -n "^(fatal:|error:|PANIC:|thread 'main'.*panicked|.*panicked at)" "$out_file" >/dev/null; then
    cat "$out_file"
    fail "command reported an agit error: agit $label"
  fi

  cat "$out_file"
}

capture_agit() {
  local cwd="$1"
  local label="$2"
  shift 2

  local out_file="$tmp_root/${label//[^A-Za-z0-9_.-]/_}.out"
  if ! run_limited "$cwd" "$out_file" "$@"; then
    cat "$out_file"
    fail "command failed: agit $label"
  fi

  if rg -n "^(fatal:|error:|PANIC:|thread 'main'.*panicked|.*panicked at)" "$out_file" >/dev/null; then
    cat "$out_file"
    fail "command reported an agit error: agit $label"
  fi

  cat "$out_file"
}

run_limited() {
  local cwd="$1"
  local out_file="$2"
  shift 2

  if command -v timeout >/dev/null 2>&1; then
    (
      cd "$cwd"
      ulimit -v "$agit_max_vmem_kb"
      timeout --kill-after=5s "$agit_timeout" "$agit_bin" "$@"
    ) >"$out_file" 2>&1
  else
    (
      cd "$cwd"
      ulimit -v "$agit_max_vmem_kb"
      "$agit_bin" "$@"
    ) >"$out_file" 2>&1
  fi
}

printf 'Repository: %s\n' "$repo_url"
printf 'Workspace:  %s\n' "$tmp_root"
printf 'Binary:     %s\n' "$agit_bin"
printf 'Limits:     timeout=%s vmem=%s KB\n' "$agit_timeout" "$agit_max_vmem_kb"

run_agit "$tmp_root" "clone $repo_url" clone "$repo_url" "$clone_dir"

[[ -d "$clone_dir/.git" ]] || fail "clone did not create .git"
[[ -f "$clone_dir/.git/HEAD" ]] || fail "clone did not create .git/HEAD"
[[ -d "$clone_dir/.git/objects" ]] || fail "clone did not create .git/objects"

branch="$(capture_agit "$clone_dir" "branch --show-current" branch --show-current | tr -d '\r\n')"
[[ -n "$branch" ]] || fail "branch --show-current returned nothing"
printf 'HEAD branch: %s\n' "$branch"

head_oid="$(capture_agit "$clone_dir" "rev-parse HEAD" rev-parse HEAD | tr -d '\r\n')"
[[ "$head_oid" =~ ^[0-9a-fA-F]{40}$ ]] || fail "rev-parse HEAD did not return a 40-char oid: $head_oid"
printf 'HEAD oid:    %s\n' "$head_oid"

head_type="$(capture_agit "$clone_dir" "cat-file -t HEAD" cat-file -t HEAD | tr -d '\r\n')"
[[ "$head_type" == "commit" ]] || fail "cat-file -t HEAD returned '$head_type', expected commit"

remote="$(capture_agit "$clone_dir" "remote" remote | tr -d '\r')"
if ! grep -qx 'origin' <<<"$remote"; then
  printf '%s\n' "$remote"
  fail "remote list does not contain origin"
fi

status="$(capture_agit "$clone_dir" "status --porcelain" status --porcelain)"
if [[ -n "$status" ]]; then
  printf '%s\n' "$status"
  fail "working tree is not clean after clone"
fi

probe_file="agit-status-probe.txt"
printf 'one\n' >"$clone_dir/$probe_file"
status="$(capture_agit "$clone_dir" "status untracked probe" status --porcelain)"
[[ "$status" == "?? $probe_file" ]] || fail "untracked file status was '$status'"

run_agit "$clone_dir" "add probe" add "$probe_file"
status="$(capture_agit "$clone_dir" "status added probe" status --porcelain)"
[[ "$status" == "A  $probe_file" ]] || fail "added file status was '$status'"

printf 'two\n' >"$clone_dir/$probe_file"
status="$(capture_agit "$clone_dir" "status modified staged probe" status --porcelain)"
grep -qx "A  $probe_file" <<<"$status" || fail "staged add missing after modify: '$status'"
grep -qx " M $probe_file" <<<"$status" || fail "unstaged modify missing after modify: '$status'"

run_agit "$clone_dir" "add modified probe" add "$probe_file"
status="$(capture_agit "$clone_dir" "status restaged probe" status --porcelain)"
[[ "$status" == "A  $probe_file" ]] || fail "restaged file status was '$status'"

local_repo="$tmp_root/local-status-repo"
mkdir "$local_repo"
run_agit "$local_repo" "init local status repo" init .
printf 'base\n' >"$local_repo/base.txt"
run_agit "$local_repo" "add local base" add base.txt
run_agit "$local_repo" "commit local base" commit -m "base"
status="$(capture_agit "$local_repo" "status after local base commit" status --porcelain)"
[[ -z "$status" ]] || fail "local working tree not clean after base commit: '$status'"

printf 'probe\n' >"$local_repo/probe.txt"
status="$(capture_agit "$local_repo" "status local untracked" status --porcelain)"
[[ "$status" == "?? probe.txt" ]] || fail "local untracked status was '$status'"

run_agit "$local_repo" "add local probe" add probe.txt
status="$(capture_agit "$local_repo" "status local added" status --porcelain)"
[[ "$status" == "A  probe.txt" ]] || fail "local added status was '$status'"

run_agit "$local_repo" "commit local probe" commit -m "probe"
status="$(capture_agit "$local_repo" "status after local probe commit" status --porcelain)"
[[ -z "$status" ]] || fail "local working tree not clean after probe commit: '$status'"

mkdir -p "$local_repo/big-untracked/a/b/c"
printf 'deep\n' >"$local_repo/big-untracked/a/b/c/file.txt"
status="$(capture_agit "$local_repo" "status local untracked dir collapsed" status --porcelain)"
[[ "$status" == "?? big-untracked/" ]] || fail "untracked directory was not collapsed: '$status'"
rm -rf "$local_repo/big-untracked"

mkdir -p "$local_repo/tracked-dir"
printf 'tracked\n' >"$local_repo/tracked-dir/tracked.txt"
run_agit "$local_repo" "add local tracked dir file" add tracked-dir/tracked.txt
run_agit "$local_repo" "commit local tracked dir file" commit -m "tracked dir"
printf 'nested\n' >"$local_repo/tracked-dir/nested-untracked.txt"
status="$(capture_agit "$local_repo" "status local nested untracked in tracked dir" status --porcelain)"
[[ "$status" == "?? tracked-dir/nested-untracked.txt" ]] || fail "nested untracked in tracked dir was wrong: '$status'"
rm "$local_repo/tracked-dir/nested-untracked.txt"

printf 'changed\n' >"$local_repo/probe.txt"
status="$(capture_agit "$local_repo" "status local modified tracked" status --porcelain)"
[[ "$status" == " M probe.txt" ]] || fail "local modified tracked status was '$status'"

rm "$local_repo/probe.txt"
status="$(capture_agit "$local_repo" "status local deleted tracked" status --porcelain)"
[[ "$status" == " D probe.txt" ]] || fail "local deleted tracked status was '$status'"
diff_out="$(capture_agit "$local_repo" "diff local deleted tracked" diff)"
grep -q '^-probe' <<<"$diff_out" || fail "deleted diff did not contain removed line: '$diff_out'"

run_agit "$local_repo" "add local deleted probe" add probe.txt
run_agit "$local_repo" "commit local deleted probe" commit -m "delete probe"
status="$(capture_agit "$local_repo" "status after local delete commit" status --porcelain)"
[[ -z "$status" ]] || fail "local working tree not clean after delete commit: '$status'"

run_agit "$local_repo" "remote add local origin" remote add origin https://example.invalid/repo.git
remote_verbose="$(capture_agit "$local_repo" "remote -v local" remote -v)"
grep -q "origin[[:space:]]https://example.invalid/repo.git (fetch)" <<<"$remote_verbose" || fail "remote -v did not show origin fetch URL: '$remote_verbose'"
run_agit "$local_repo" "remote set-url local origin" remote set-url origin https://example.invalid/other.git
remote_verbose="$(capture_agit "$local_repo" "remote -v local after set-url" remote -v)"
grep -q "origin[[:space:]]https://example.invalid/other.git (fetch)" <<<"$remote_verbose" || fail "remote set-url did not update origin: '$remote_verbose'"
mkdir -p "$local_repo/.git/refs/remotes/origin"
capture_agit "$local_repo" "rev-parse local main before remote-ref" rev-parse main >"$local_repo/.git/refs/remotes/origin/main"
remote_ref_oid="$(capture_agit "$local_repo" "rev-parse local origin/main" rev-parse origin/main | tr -d '\r\n')"
main_ref_oid="$(capture_agit "$local_repo" "rev-parse local main after remote-ref" rev-parse main | tr -d '\r\n')"
[[ "$remote_ref_oid" == "$main_ref_oid" ]] || fail "origin/main did not resolve to remote-tracking ref"

printf 'base changed\n' >"$local_repo/base.txt"
diff_out="$(capture_agit "$local_repo" "diff local unstaged" diff)"
grep -q '^+base changed' <<<"$diff_out" || fail "unstaged diff did not contain changed line: '$diff_out'"

run_agit "$local_repo" "add local changed base" add base.txt
diff_out="$(capture_agit "$local_repo" "diff local cached" diff --cached)"
grep -q '^+base changed' <<<"$diff_out" || fail "cached diff did not contain changed line: '$diff_out'"
run_agit "$local_repo" "commit local changed base" commit -m "change base"
status="$(capture_agit "$local_repo" "status after local changed base commit" status --porcelain)"
[[ -z "$status" ]] || fail "local working tree not clean after changed base commit: '$status'"

printf 'stashed\n' >"$local_repo/base.txt"
printf 'scratch\n' >"$local_repo/scratch.txt"
run_agit "$local_repo" "stash push local" stash push -u -m "stash probe"
status="$(capture_agit "$local_repo" "status after stash push" status --porcelain)"
[[ -z "$status" ]] || fail "local working tree not clean after stash push: '$status'"
stash_list="$(capture_agit "$local_repo" "stash list local" stash list)"
grep -q 'stash probe' <<<"$stash_list" || fail "stash list did not contain message: '$stash_list'"
run_agit "$local_repo" "stash pop local" stash pop
status="$(capture_agit "$local_repo" "status after stash pop" status --porcelain)"
grep -qx " M base.txt" <<<"$status" || fail "stash pop did not restore tracked change: '$status'"
grep -qx "?? scratch.txt" <<<"$status" || fail "stash pop did not restore untracked file: '$status'"
rm "$local_repo/scratch.txt"
run_agit "$local_repo" "add local stashed base" add base.txt
run_agit "$local_repo" "commit local stashed base" commit -m "stashed base"
status="$(capture_agit "$local_repo" "status after local stash commit" status --porcelain)"
[[ -z "$status" ]] || fail "local working tree not clean after stash commit: '$status'"

run_agit "$local_repo" "branch local feature" branch feature
run_agit "$local_repo" "checkout local feature" checkout feature
printf 'feature\n' >"$local_repo/feature.txt"
run_agit "$local_repo" "add local feature file" add feature.txt
run_agit "$local_repo" "commit local feature file" commit -m "feature"
feature_oid="$(capture_agit "$local_repo" "rev-parse local feature" rev-parse HEAD | tr -d '\r\n')"
run_agit "$local_repo" "checkout local main before merge" checkout main
[[ ! -e "$local_repo/feature.txt" ]] || fail "checkout main left feature-only tracked file in working tree"
run_agit "$local_repo" "merge local feature" merge feature
merged_oid="$(capture_agit "$local_repo" "rev-parse local merged main" rev-parse HEAD | tr -d '\r\n')"
[[ "$merged_oid" == "$feature_oid" ]] || fail "merge did not fast-forward main to feature"
[[ -f "$local_repo/feature.txt" ]] || fail "fast-forward merge did not restore feature file"
status="$(capture_agit "$local_repo" "status after local merge" status --porcelain)"
[[ -z "$status" ]] || fail "local working tree not clean after merge: '$status'"

run_agit "$local_repo" "branch local topic" branch topic
run_agit "$local_repo" "checkout local main for rebase target" checkout main
printf 'main advance\n' >"$local_repo/rebase-target.txt"
run_agit "$local_repo" "add local rebase target" add rebase-target.txt
run_agit "$local_repo" "commit local rebase target" commit -m "rebase target"
target_oid="$(capture_agit "$local_repo" "rev-parse local rebase target" rev-parse HEAD | tr -d '\r\n')"
run_agit "$local_repo" "checkout local topic before rebase" checkout topic
run_agit "$local_repo" "rebase local topic onto main" rebase main
rebased_oid="$(capture_agit "$local_repo" "rev-parse local rebased topic" rev-parse HEAD | tr -d '\r\n')"
[[ "$rebased_oid" == "$target_oid" ]] || fail "rebase did not fast-forward topic to main"
status="$(capture_agit "$local_repo" "status after local rebase" status --porcelain)"
[[ -z "$status" ]] || fail "local working tree not clean after rebase: '$status'"

run_agit "$local_repo" "checkout local main before non-ff merge" checkout main
run_agit "$local_repo" "branch local merge-side" branch merge-side
run_agit "$local_repo" "checkout local merge-side" checkout merge-side
printf 'side\n' >"$local_repo/side.txt"
run_agit "$local_repo" "add local merge-side file" add side.txt
run_agit "$local_repo" "commit local merge-side file" commit -m "merge side"
side_oid="$(capture_agit "$local_repo" "rev-parse local merge-side" rev-parse HEAD | tr -d '\r\n')"
run_agit "$local_repo" "checkout local main before non-ff merge commit" checkout main
[[ ! -e "$local_repo/side.txt" ]] || fail "checkout main left side-only tracked file in working tree"
printf 'main only\n' >"$local_repo/main-only.txt"
run_agit "$local_repo" "add local main-only file" add main-only.txt
run_agit "$local_repo" "commit local main-only file" commit -m "main only"
main_before_merge_oid="$(capture_agit "$local_repo" "rev-parse local main before non-ff merge" rev-parse HEAD | tr -d '\r\n')"
run_agit "$local_repo" "merge local non-ff side" merge merge-side
merged_oid="$(capture_agit "$local_repo" "rev-parse local non-ff merged main" rev-parse HEAD | tr -d '\r\n')"
[[ "$merged_oid" != "$side_oid" ]] || fail "non-ff merge incorrectly fast-forwarded to side"
[[ "$merged_oid" != "$main_before_merge_oid" ]] || fail "non-ff merge did not create a new commit"
[[ -f "$local_repo/side.txt" ]] || fail "non-ff merge did not check out side file"
[[ -f "$local_repo/main-only.txt" ]] || fail "non-ff merge lost main-only file"
status="$(capture_agit "$local_repo" "status after local non-ff merge" status --porcelain)"
[[ -z "$status" ]] || fail "local working tree not clean after non-ff merge: '$status'"

run_agit "$local_repo" "branch local rebase-linear" branch rebase-linear
run_agit "$local_repo" "checkout local rebase-linear" checkout rebase-linear
printf 'linear side\n' >"$local_repo/linear-side.txt"
run_agit "$local_repo" "add local linear side" add linear-side.txt
run_agit "$local_repo" "commit local linear side" commit -m "linear side"
old_linear_oid="$(capture_agit "$local_repo" "rev-parse local old linear side" rev-parse HEAD | tr -d '\r\n')"
run_agit "$local_repo" "checkout local main before linear rebase" checkout main
printf 'linear main\n' >"$local_repo/linear-main.txt"
run_agit "$local_repo" "add local linear main" add linear-main.txt
run_agit "$local_repo" "commit local linear main" commit -m "linear main"
run_agit "$local_repo" "checkout local rebase-linear before nonlinear rebase" checkout rebase-linear
run_agit "$local_repo" "rebase local linear branch onto main" rebase main
new_linear_oid="$(capture_agit "$local_repo" "rev-parse local rebased linear side" rev-parse HEAD | tr -d '\r\n')"
[[ "$new_linear_oid" != "$old_linear_oid" ]] || fail "linear rebase did not rewrite branch commit"
[[ -f "$local_repo/linear-side.txt" ]] || fail "linear rebase lost side file"
[[ -f "$local_repo/linear-main.txt" ]] || fail "linear rebase did not include upstream file"
status="$(capture_agit "$local_repo" "status after local linear rebase" status --porcelain)"
[[ -z "$status" ]] || fail "local working tree not clean after linear rebase: '$status'"

printf 'agit GitHub clone test passed.\n'

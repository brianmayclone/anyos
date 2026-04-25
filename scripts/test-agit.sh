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
    clones https://github.com/brianmayclone/anyos with the host-built agit/cgit,
    verifies refs, HEAD, remote config, object resolution, and clean checkout,
    then deletes the temporary clone.

Environment:
  AGIT_TEST_REPO=<url>  repository URL to clone
  AGIT_BIN=<path>       use an existing agit/cgit binary instead of building one
  AGIT_KEEP_TMP=1       keep the temporary directory for debugging

Examples:
  scripts/test-agit.sh
  AGIT_KEEP_TMP=1 scripts/test-agit.sh https://github.com/brianmayclone/anyos
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

repo_url="${1:-${AGIT_TEST_REPO:-https://github.com/brianmayclone/anyos}}"
tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/agit-github-test.XXXXXX")"
clone_dir="$tmp_root/anyos"

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
  if ! (cd "$cwd" && "$agit_bin" "$@" >"$out_file" 2>&1); then
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
  if ! (cd "$cwd" && "$agit_bin" "$@" >"$out_file" 2>&1); then
    cat "$out_file"
    fail "command failed: agit $label"
  fi

  if rg -n "^(fatal:|error:|PANIC:|thread 'main'.*panicked|.*panicked at)" "$out_file" >/dev/null; then
    cat "$out_file"
    fail "command reported an agit error: agit $label"
  fi

  cat "$out_file"
}

printf 'Repository: %s\n' "$repo_url"
printf 'Workspace:  %s\n' "$tmp_root"
printf 'Binary:     %s\n' "$agit_bin"

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

printf 'agit GitHub clone test passed.\n'

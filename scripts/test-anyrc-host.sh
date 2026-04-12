#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch_dir="${TMPDIR:-/tmp}"
manifest_path="$repo_root/tools/anyrc-hosttests/Cargo.toml"

if [[ "${1:-}" == "--manifest-path" ]]; then
  manifest_path="$2"
  shift 2
fi

cd "$scratch_dir"

cargo test \
  --manifest-path "$manifest_path" \
  "$@"

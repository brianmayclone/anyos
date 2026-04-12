#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch_dir="${TMPDIR:-/tmp}"

cd "$scratch_dir"

cargo test \
  --manifest-path "$repo_root/tools/anyrc-hosttests/Cargo.toml" \
  "$@"

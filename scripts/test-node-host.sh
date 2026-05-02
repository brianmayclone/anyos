#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "${TMPDIR:-/tmp}"

cargo test \
  --manifest-path "$repo_root/tools/node-hosttests/Cargo.toml" \
  "$@"

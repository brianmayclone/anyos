#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo test \
  --manifest-path "$repo_root/tools/anyrc-hosttests/Cargo.toml" \
  --target x86_64-unknown-linux-gnu \
  "$@"

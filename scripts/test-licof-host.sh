#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

"$repo_root/scripts/test-anyrc-host.sh" \
  --manifest-path "$repo_root/tools/licof-hosttests/Cargo.toml" \
  "$@"

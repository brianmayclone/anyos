#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

viewport="${SURF_TEST_VIEWPORT:-1365x900}"
width="${viewport%x*}"
height="${viewport#*x}"
out_dir="${SURF_SITE_OUT_DIR:-/tmp/surf-sites}"
mkdir -p "$out_dir"

cargo build --manifest-path tools/surf-host/Cargo.toml

surf_host="tools/surf-host/target/debug/surf-host"

run_site() {
  local name="$1"
  local url="$2"
  shift 2

  echo "[surf-sites] rendering $name: $url"
  "$surf_host" "$url" \
    --screenshot "$out_dir/$name.png" \
    --width "$width" \
    --height "$height" \
    --delay "${SURF_SITE_DELAY:-0}" \
    "$@" \
    >"$out_dir/$name.log" 2>&1
  echo "[surf-sites] wrote $out_dir/$name.png"
}

run_site google "https://www.google.de"
run_site heise "https://www.heise.de" --no-js
run_site bild "https://www.bild.de" --no-js

echo "[surf-sites] done: $out_dir"

#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

matrix="${SURF_SCROLL_MATRIX:-tests/browser/surf_scroll_pages.tsv}"
out_dir="${SURF_SCROLL_OUT_DIR:-/tmp/surf-scroll-pages}"
viewport="${SURF_TEST_VIEWPORT:-1365x900}"
width="${viewport%x*}"
height="${viewport#*x}"
delay="${SURF_SCROLL_DELAY:-2500}"
site_filter=",${SURF_SCROLL_SITES:-},"

mkdir -p "$out_dir"

cargo build --manifest-path tools/surf-host/Cargo.toml
surf_host="tools/surf-host/target/debug/surf-host"

echo -e "name\turl\tdoc_h\tscroll_y\tstatus" >"$out_dir/summary.tsv"

while IFS=$'\t' read -r name url focus; do
  [[ -z "${name:-}" || "$name" == \#* || "$name" == "name" ]] && continue
  if [[ "$site_filter" != ",," && "$site_filter" != *",$name,"* ]]; then
    continue
  fi

  echo "[surf-scroll] bottom probe $name: $url"
  log="$out_dir/${name}.bottom.log"
  png="$out_dir/${name}.bottom.png"
  if "$surf_host" "$url" \
      --screenshot "$png" \
      --bottom \
      --width "$width" \
      --height "$height" \
      --delay "$delay" \
      --anyos-image-path \
      >"$log" 2>&1; then
    status="ok"
  else
    status="failed"
  fi

  bottom_line="$(rg -n "\\[surf-host\\] bottom:" "$log" | tail -1 || true)"
  doc_h="$(sed -n 's/.*doc_h=\([0-9][0-9]*\).*/\1/p' <<<"$bottom_line")"
  scroll_y="$(sed -n 's/.*scroll_y=\([0-9][0-9]*\).*/\1/p' <<<"$bottom_line")"
  echo -e "${name}\t${url}\t${doc_h:-?}\t${scroll_y:-?}\t${status}" >>"$out_dir/summary.tsv"
done <"$matrix"

echo "[surf-scroll] done: $out_dir/summary.tsv"

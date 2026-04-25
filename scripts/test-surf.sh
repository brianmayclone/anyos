#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

usage() {
  cat <<'EOF'
Usage:
  scripts/test-surf.sh [url] [WxH] [surf-host args...]

Default:
  scripts/test-surf.sh
    builds surf-host and opens the local egui browser on about:blank.

Environment:
  SURF_TEST_URL=<url>       default URL when no URL argument is passed
  SURF_TEST_VIEWPORT=WxH    default viewport, e.g. 1280x900
  SURF_TEST_GL=system       do not force the software GL defaults

Examples:
  scripts/test-surf.sh
  scripts/test-surf.sh https://example.com
  scripts/test-surf.sh file:///tmp/test.html 1280x900
  scripts/test-surf.sh https://example.com --remote-listen 127.0.0.1:8790
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

args=(run)

if [[ "${SURF_TEST_GL:-software}" != "system" ]]; then
  export WINIT_UNIX_BACKEND="${WINIT_UNIX_BACKEND:-x11}"
  export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"
  export GALLIUM_DRIVER="${GALLIUM_DRIVER:-llvmpipe}"
fi

if [[ -n "${1:-}" && ! "${1:-}" =~ ^[0-9]+x[0-9]+$ && "${1:-}" != --* ]]; then
  args+=("$1")
  shift
elif [[ -n "${SURF_TEST_URL:-}" ]]; then
  args+=("$SURF_TEST_URL")
fi

if [[ -n "${1:-}" && "${1:-}" =~ ^[0-9]+x[0-9]+$ ]]; then
  args+=("$1")
  shift
elif [[ -n "${SURF_TEST_VIEWPORT:-}" ]]; then
  args+=("$SURF_TEST_VIEWPORT")
fi

exec "$repo_root/tools/surf-host/build.sh" "${args[@]}" "$@"

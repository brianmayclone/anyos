#!/bin/bash
# Build and optionally run surf-host
#
# Usage:
#   ./build.sh                                Build only
#   ./build.sh run [url]                      Build + run with default viewport
#   ./build.sh run <url> 1280x960             Build + run with custom viewport
#   ./build.sh run <url> --screenshot out.png Build + run with screenshot
#   ./build.sh screenshot <url> out.png       Build + screenshot (shorthand)
#   ./build.sh screenshot <url> out.png full  Build + full-page screenshot
#   ./build.sh clean                          Clean build artifacts

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

BINARY="target/release/surf-host"

build() {
    echo "[build] compiling surf-host (release)..."
    cargo +stable build --release 2>&1 | grep -E "Compiling surf-host|Finished|^error" || true
    if [ ! -f "$BINARY" ]; then
        echo "[build] FAILED — run 'cargo +stable build --release' for details"
        exit 1
    fi
    echo "[build] done: $BINARY"
}

case "${1:-build}" in
    build)
        build
        ;;

    clean)
        cargo +stable clean
        echo "[build] cleaned."
        ;;

    run)
        build
        shift

        if [[ "${SURF_HOST_GL:-software}" != "system" ]]; then
            export WINIT_UNIX_BACKEND="${WINIT_UNIX_BACKEND:-x11}"
            export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"
            export GALLIUM_DRIVER="${GALLIUM_DRIVER:-llvmpipe}"
        fi

        URL=""
        if [[ -n "${1:-}" && ! "${1:-}" =~ ^([0-9]+)x([0-9]+)$ && "${1:-}" != --* ]]; then
            URL="$1"
            shift || true
        fi

        # Check if next arg is a viewport size (e.g. 1280x960)
        EXTRA_ARGS=()
        if [[ "${1:-}" =~ ^([0-9]+)x([0-9]+)$ ]]; then
            EXTRA_ARGS+=(--width "${BASH_REMATCH[1]}" --height "${BASH_REMATCH[2]}")
            shift || true
        fi

        if [[ -n "$URL" ]]; then
            exec "./$BINARY" "$URL" "${EXTRA_ARGS[@]}" "$@"
        else
            exec "./$BINARY" "${EXTRA_ARGS[@]}" "$@"
        fi
        ;;

    screenshot)
        build
        URL="${2:?Usage: build.sh screenshot <url> [options...]}"
        shift 2 || true

        # Parse remaining args: any mix of out.png, WxH, full/fullpage, delay(ms)
        OUT="screenshot.png"
        EXTRA_ARGS=()

        for arg in "$@"; do
            if [[ "$arg" == "full" || "$arg" == "fullpage" ]]; then
                EXTRA_ARGS+=(--fullpage)
            elif [[ "$arg" =~ ^([0-9]+)-([0-9]+)(px)?$ ]]; then
                # Y range: 400-900 or 400-900px
                EXTRA_ARGS+=(-y "$arg")
            elif [[ "$arg" =~ ^([0-9]+)x([0-9]+)$ ]]; then
                EXTRA_ARGS+=(--width "${BASH_REMATCH[1]}" --height "${BASH_REMATCH[2]}")
            elif [[ "$arg" =~ ^[0-9]+$ ]]; then
                EXTRA_ARGS+=(--delay "$arg")
            elif [[ "$arg" == *.png || "$arg" == *.jpg || "$arg" == *.bmp || "$arg" == /* || "$arg" == ./* ]]; then
                OUT="$arg"
            fi
        done

        EXTRA_ARGS=(--screenshot "$OUT" "${EXTRA_ARGS[@]}")
        exec "./$BINARY" "$URL" "${EXTRA_ARGS[@]}"
        ;;

    *)
        echo "Usage:"
        echo "  ./build.sh                                     Build only"
        echo "  ./build.sh run [url] [WxH]                    Build + open in window"
        echo "  ./build.sh screenshot <url> [opts...]         Build + save screenshot"
        echo "  ./build.sh clean                              Clean build"
        echo ""
        echo "Screenshot options (any order):"
        echo "  out.png       Output file (default: screenshot.png)"
        echo "  1280x960      Viewport size"
        echo "  full          Full-page (entire document height)"
        echo "  400-900       Y range in pixels (crop vertically)"
        echo "  3000          Delay in ms before capture"
        echo ""
        echo "Examples:"
        echo "  ./build.sh run"
        echo "  ./build.sh run https://www.wikipedia.de"
        echo "  ./build.sh run https://www.wikipedia.de 1280x960"
        echo "  ./build.sh screenshot https://example.com"
        echo "  ./build.sh screenshot https://example.com wiki.png full 1280x960"
        echo "  ./build.sh screenshot https://example.com 1280x960 full 3000"
        echo "  ./build.sh screenshot https://example.com 400-900 crop.png"
        exit 1
        ;;
esac

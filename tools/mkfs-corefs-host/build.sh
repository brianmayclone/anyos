#!/usr/bin/env bash
# Copyright (c) 2026 Mike Strathmann
# SPDX-License-Identifier: MIT
#
# Build & test helper for the host-side CoreFS formatter.
#
# Usage:
#   ./build.sh                       # release build
#   ./build.sh test                  # build + run unit + smoke tests
#   ./build.sh run --output foo.img  # build + invoke the CLI
#   ./build.sh clean                 # remove target/
#
# The tool is excluded from the anyOS workspace and is built with the
# stable host toolchain (`cargo +stable`). The repository no longer
# forces the anyOS target globally, so host-side commands stay isolated
# from kernel/userland `build-std` builds.

set -euo pipefail

cd "$(dirname "$0")"

CMD="${1:-build}"
shift || true

case "$CMD" in
    build)
        cargo +stable build --release
        echo
        echo "Binary: $(pwd)/target/x86_64-unknown-linux-gnu/release/mkfs-corefs-host"
        ;;
    test)
        cargo +stable test --release
        ;;
    run)
        cargo +stable build --release
        exec ./target/x86_64-unknown-linux-gnu/release/mkfs-corefs-host "$@"
        ;;
    clean)
        cargo +stable clean
        ;;
    *)
        echo "Unknown command: $CMD" >&2
        echo "Usage: $0 [build|test|run|clean]" >&2
        exit 2
        ;;
esac

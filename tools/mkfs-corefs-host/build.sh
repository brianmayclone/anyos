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
# The tool is excluded from the anyOS workspace and must always be
# built with the stable toolchain (`cargo +stable`), otherwise the
# kernel-target settings in the top-level `.cargo/config.toml` leak in
# and break the x86_64-unknown-linux-gnu build.

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

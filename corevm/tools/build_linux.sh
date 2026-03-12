#!/bin/bash
set -e
SCRIPT_DIR="$(dirname "$0")"
cd "$SCRIPT_DIR/../vmmanager"

# Always clean libcorevm artifacts to avoid stale builds
cargo clean -p libcorevm 2>/dev/null || true

if [ "$1" = "--clean" ]; then
    cargo clean
    shift
    echo "Cleaned build artifacts."
fi

cargo +stable build --release --features libcorevm/linux
echo "Built: target/x86_64-unknown-linux-gnu/release/corevm-vmmanager"

if [ "$1" = "--run" ]; then
    exec cargo +stable run --release --features libcorevm/linux -- "${@:2}"
fi

#!/bin/bash
set -e
cd "$(dirname "$0")/../vmmanager"

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

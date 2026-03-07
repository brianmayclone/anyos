#!/bin/bash
set -e
cd "$(dirname "$0")/../vmmanager"
cargo +stable build --release
echo "Built: target/x86_64-unknown-linux-gnu/release/corevm-vmmanager"

if [ "$1" = "--run" ]; then
    exec cargo +stable run --release -- "${@:2}"
fi

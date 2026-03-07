#!/bin/bash
set -e
cd "$(dirname "$0")/../vmmanager"
cargo +stable build --release
echo "Built: target/x86_64-unknown-linux-gnu/release/corevm-vmmanager"

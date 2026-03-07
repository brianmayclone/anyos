#!/bin/bash
set -e
cd "$(dirname "$0")/../vmmanager"
cargo build --release -Zbuild-std=
echo "Built: target/release/corevm-vmmanager"

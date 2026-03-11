#!/usr/bin/env bash
set -euo pipefail

# Build vmmanager for Windows x86_64 from WSL using cargo.exe (Windows Rust toolchain).
# The key trick: cargo.exe needs a Windows-native path for --manifest-path.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VMMANAGER_DIR="$(cd "$SCRIPT_DIR/../vmmanager" && pwd)"

# Convert WSL path to Windows path for cargo.exe
WIN_MANIFEST="$(wslpath -w "$VMMANAGER_DIR/Cargo.toml")"

echo "[build_win64] manifest: $WIN_MANIFEST"

# Use a Windows-native target directory to avoid WSL filesystem permission issues.
WIN_TARGET_DIR="/mnt/c/tmp/corevm-build"
mkdir -p "$WIN_TARGET_DIR"
WIN_TARGET_DIR_W="$(wslpath -w "$WIN_TARGET_DIR")"

# Remove stale WSL target dir that cargo.exe can't clean
rm -rf "$VMMANAGER_DIR/target"

cargo.exe +stable build \
    --release \
    --target x86_64-pc-windows-msvc \
    --features libcorevm/windows \
    --manifest-path "$WIN_MANIFEST" \
    --target-dir "$WIN_TARGET_DIR_W"

echo "[build_win64] Built: $WIN_TARGET_DIR/x86_64-pc-windows-msvc/release/corevm-vmmanager.exe"

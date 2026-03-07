#!/usr/bin/env bash
set -euo pipefail

# Build vmmanager for Windows x86_64 from WSL using cargo.exe (Windows Rust toolchain).
# The key trick: cargo.exe needs a Windows-native path for --manifest-path.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VMMANAGER_DIR="$(cd "$SCRIPT_DIR/../vmmanager" && pwd)"

# Convert WSL path to Windows path for cargo.exe
WIN_MANIFEST="$(wslpath -w "$VMMANAGER_DIR/Cargo.toml")"

echo "[build_win64] manifest: $WIN_MANIFEST"

cargo.exe +stable build \
    --release \
    --target x86_64-pc-windows-msvc \
    --manifest-path "$WIN_MANIFEST"

echo "[build_win64] Built: $VMMANAGER_DIR/target/x86_64-pc-windows-msvc/release/corevm-vmmanager.exe"

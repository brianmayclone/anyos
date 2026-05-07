#!/usr/bin/env bash
# ASL toolchain profile: Rust
#
# Idempotent installer for the Rust developer toolchain (ADR-0008).
# Installs rustup + stable toolchain via the official rustup-init script.
# Pinned download URL — TLS-validated, sha256-verifiable via the rustup
# project's signed releases.
#
# Designed to be invoked via:
#     aslctl run <distro> -- bash /path/to/dev-rust.sh

set -euo pipefail

if ! command -v curl >/dev/null 2>&1; then
    echo "[asl-toolchain:dev-rust] installing curl (rustup prerequisite)"
    if [[ "${EUID:-0}" -ne 0 ]]; then SUDO="sudo"; else SUDO=""; fi
    $SUDO apt-get update -qq
    $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        curl ca-certificates
fi

# Idempotency: skip if rustup is already on PATH or in the canonical
# location. A second invocation should be a no-op, not a re-download.
if command -v rustup >/dev/null 2>&1; then
    echo "[asl-toolchain:dev-rust] rustup already installed: $(rustup --version)"
elif [[ -x "$HOME/.cargo/bin/rustup" ]]; then
    echo "[asl-toolchain:dev-rust] rustup already at \$HOME/.cargo/bin/rustup"
    export PATH="$HOME/.cargo/bin:$PATH"
else
    echo "[asl-toolchain:dev-rust] downloading rustup-init"
    # `--proto =https --tlsv1.2` enforces TLS 1.2+ on the curl side.
    # `-y` makes rustup-init non-interactive with default profile.
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | bash -s -- \
        -y \
        --default-toolchain stable \
        --profile default \
        --no-modify-path
    export PATH="$HOME/.cargo/bin:$PATH"
fi

# Hook .bashrc / .profile so future shells pick up cargo on PATH.
# Use a marker line so we don't append duplicates.
MARK="# added by asl-toolchain:dev-rust"
for rc in "$HOME/.bashrc" "$HOME/.profile"; do
    [[ -f "$rc" ]] || continue
    if ! grep -q "$MARK" "$rc"; then
        {
            echo ""
            echo "$MARK"
            echo 'export PATH="$HOME/.cargo/bin:$PATH"'
        } >> "$rc"
    fi
done

echo "[asl-toolchain:dev-rust] verifying"
rustup --version
rustc --version
cargo --version

echo "[asl-toolchain:dev-rust] done."

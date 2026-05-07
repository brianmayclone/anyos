#!/usr/bin/env bash
# ASL toolchain profile: Node.js
#
# Idempotent installer for the Node.js developer toolchain (ADR-0008).
# Installs nvm and the latest Node.js LTS. nvm gives the user the
# ability to switch Node versions per project — required for many JS
# workflows that pin to specific majors.
#
# Designed to be invoked via:
#     aslctl run <distro> -- bash /path/to/dev-node.sh

set -euo pipefail

if [[ "${EUID:-0}" -ne 0 ]]; then SUDO="sudo"; else SUDO=""; fi

# Bootstrap prerequisites. ca-certificates is needed for the HTTPS
# fetch of the nvm install script, curl is the fetch tool itself,
# git lets nvm clone its own update repo on later invocations.
PREREQS=(curl ca-certificates git)
NEED_INSTALL=()
for pkg in "${PREREQS[@]}"; do
    if ! dpkg -s "$pkg" >/dev/null 2>&1; then
        NEED_INSTALL+=("$pkg")
    fi
done
if [[ ${#NEED_INSTALL[@]} -gt 0 ]]; then
    echo "[asl-toolchain:dev-node] installing prerequisites: ${NEED_INSTALL[*]}"
    $SUDO apt-get update -qq
    $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${NEED_INSTALL[@]}"
fi

# nvm — pinned to a known-good tag. Update procedure mirrors
# DEBIAN_RAW_SHA512_HEX: bump the tag, re-test, document in release notes.
NVM_VERSION="v0.40.1"
export NVM_DIR="$HOME/.nvm"

if [[ -s "$NVM_DIR/nvm.sh" ]]; then
    echo "[asl-toolchain:dev-node] nvm already installed at $NVM_DIR"
else
    echo "[asl-toolchain:dev-node] installing nvm $NVM_VERSION"
    curl --proto '=https' --tlsv1.2 -sSf \
        "https://raw.githubusercontent.com/nvm-sh/nvm/${NVM_VERSION}/install.sh" \
        | bash
fi

# Source nvm in the current shell so we can call it.
# shellcheck disable=SC1091
. "$NVM_DIR/nvm.sh"

# Install / re-confirm LTS. `nvm install --lts` is itself idempotent —
# if the latest LTS is already there it just reports the version.
echo "[asl-toolchain:dev-node] installing/confirming Node.js LTS"
nvm install --lts
nvm use --lts
nvm alias default 'lts/*'

echo "[asl-toolchain:dev-node] verifying"
node --version
npm --version

echo "[asl-toolchain:dev-node] done."

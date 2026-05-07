#!/usr/bin/env bash
# ASL toolchain profile: C / C++
#
# Idempotent installer for the C/C++ developer toolchain (ADR-0008).
# Designed to be invoked via:
#     aslctl run <distro> -- bash /path/to/dev-c.sh
#
# Adds: gcc, g++, clang, gdb, make, cmake, ninja-build, pkg-config,
#       build-essential meta-package.

set -euo pipefail

if ! command -v apt-get >/dev/null 2>&1; then
    echo "ERROR: this profile requires a Debian/Ubuntu-based distro." >&2
    exit 1
fi

if [[ "${EUID:-0}" -ne 0 ]]; then
    SUDO="sudo"
else
    SUDO=""
fi

PACKAGES=(
    build-essential
    clang
    gdb
    make
    cmake
    ninja-build
    pkg-config
)

echo "[asl-toolchain:dev-c] refreshing apt index"
$SUDO apt-get update -qq

echo "[asl-toolchain:dev-c] installing: ${PACKAGES[*]}"
$SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${PACKAGES[@]}"

echo "[asl-toolchain:dev-c] verifying tools"
gcc --version | head -1
g++ --version | head -1
clang --version | head -1
make --version | head -1
cmake --version | head -1
ninja --version

echo "[asl-toolchain:dev-c] done."

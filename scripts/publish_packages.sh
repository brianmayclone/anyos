#!/usr/bin/env bash
# publish_packages.sh — Build anyOS package archives and generate repository index.
#
# Reads package definitions from packages/<name>/pkg.json, copies built binaries
# from the build sysroot, runs apkg-build + apkg-index, and outputs a ready-to-deploy
# repository structure into firebase-hosting/public/.
#
# Usage:
#   ./scripts/publish_packages.sh [--arch x86_64|aarch64] [--clean]
#
# Prerequisites:
#   - Project must be built first (./scripts/build.sh)
#   - apkg-build and apkg-index are compiled as part of the build

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BUILD_DIR="${PROJECT_DIR}/build"
SYSROOT_DIR="${BUILD_DIR}/sysroot"

# Build tools
APKG_BUILD="${BUILD_DIR}/buildsystem/apkg-build"
APKG_INDEX="${BUILD_DIR}/buildsystem/apkg-index"

# Package definitions
PACKAGES_DIR="${PROJECT_DIR}/packages"

# Output directories
HOSTING_DIR="${PROJECT_DIR}/firebase-hosting/public"
STAGING_DIR="${BUILD_DIR}/pkg-staging"

# Defaults
ARCH="x86_64"
CLEAN=0

# ── Parse arguments ──────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch)
            ARCH="$2"
            shift 2
            ;;
        --clean)
            CLEAN=1
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [--arch x86_64|aarch64] [--clean]"
            echo ""
            echo "Options:"
            echo "  --arch <arch>   Target architecture (default: x86_64)"
            echo "  --clean         Remove staging and output dirs before building"
            echo "  -h, --help      Show this help"
            exit 0
            ;;
        *)
            echo "Error: unknown option '$1'"
            exit 1
            ;;
    esac
done

# ── Validate prerequisites ───────────────────────────────────────────

if [[ ! -x "$APKG_BUILD" ]]; then
    echo "Error: apkg-build not found at ${APKG_BUILD}"
    echo "Run ./scripts/build.sh first to compile build tools."
    exit 1
fi

if [[ ! -x "$APKG_INDEX" ]]; then
    echo "Error: apkg-index not found at ${APKG_INDEX}"
    echo "Run ./scripts/build.sh first to compile build tools."
    exit 1
fi

if [[ ! -d "$PACKAGES_DIR" ]]; then
    echo "Error: packages/ directory not found."
    echo "Create package definitions in packages/<name>/pkg.json first."
    exit 1
fi

# ── Clean if requested ───────────────────────────────────────────────

if [[ "$CLEAN" -eq 1 ]]; then
    echo "Cleaning staging and output directories..."
    rm -rf "$STAGING_DIR" "$HOSTING_DIR"
fi

# ── Create output directories ────────────────────────────────────────

mkdir -p "$STAGING_DIR"
mkdir -p "${HOSTING_DIR}/packages/${ARCH}"

# ── Build each package ───────────────────────────────────────────────

PKG_COUNT=0
FAIL_COUNT=0

for pkg_json in "${PACKAGES_DIR}"/*/pkg.json; do
    [[ -f "$pkg_json" ]] || continue

    pkg_dir="$(dirname "$pkg_json")"
    pkg_name="$(basename "$pkg_dir")"

    # Read version from pkg.json (simple grep, no jq dependency)
    version=$(grep -o '"version"[[:space:]]*:[[:space:]]*"[^"]*"' "$pkg_json" | head -1 | sed 's/.*"\([^"]*\)"$/\1/')
    if [[ -z "$version" ]]; then
        echo "Warning: ${pkg_name}/pkg.json missing version, skipping."
        ((FAIL_COUNT++)) || true
        continue
    fi

    echo "── Building package: ${pkg_name} ${version} (${ARCH}) ──"

    # Create staging directory for this package
    stage="${STAGING_DIR}/${pkg_name}"
    rm -rf "$stage"
    mkdir -p "${stage}/files"

    # Copy pkg.json
    cp "$pkg_json" "${stage}/pkg.json"

    # Copy files from build sysroot
    # Check if the package definition has a files.list (explicit file mapping)
    files_list="${pkg_dir}/files.list"
    if [[ -f "$files_list" ]]; then
        # files.list format: one source path per line (relative to sysroot)
        while IFS= read -r line; do
            # Skip comments and empty lines
            [[ -z "$line" || "$line" == \#* ]] && continue
            src="${SYSROOT_DIR}/${line}"
            if [[ -f "$src" ]]; then
                dest_dir="${stage}/files/$(dirname "$line")"
                mkdir -p "$dest_dir"
                cp "$src" "${stage}/files/${line}"
            else
                echo "  Warning: ${line} not found in sysroot, skipping."
            fi
        done < "$files_list"
    else
        # Default: copy System/bin/<pkg_name> from sysroot
        bin="${SYSROOT_DIR}/System/bin/${pkg_name}"
        if [[ -f "$bin" ]]; then
            mkdir -p "${stage}/files/System/bin"
            cp "$bin" "${stage}/files/System/bin/${pkg_name}"
        else
            echo "  Warning: ${pkg_name} binary not found in sysroot at ${bin}"
            echo "  Create a files.list in packages/${pkg_name}/ to specify custom file paths."
            ((FAIL_COUNT++)) || true
            continue
        fi
    fi

    # Calculate installed size and update pkg.json
    installed_size=$(du -sb "${stage}/files" 2>/dev/null | cut -f1 || echo "0")
    # Use a temp file to update size_installed in pkg.json if not already set
    if ! grep -q '"size_installed"' "${stage}/pkg.json"; then
        # Insert size_installed before the closing brace
        sed -i.bak '$ s/}/,\n  "size_installed": '"${installed_size}"'\n}/' "${stage}/pkg.json"
        rm -f "${stage}/pkg.json.bak"
    fi

    # Run apkg-build
    output_pkg="${HOSTING_DIR}/packages/${ARCH}/${pkg_name}-${version}.tar.gz"
    if "$APKG_BUILD" -d "$stage" -o "$output_pkg"; then
        echo "  Created: ${pkg_name}-${version}.tar.gz"
        ((PKG_COUNT++)) || true
    else
        echo "  Error: failed to build ${pkg_name}"
        ((FAIL_COUNT++)) || true
    fi
done

# ── Generate repository index ────────────────────────────────────────

if [[ "$PKG_COUNT" -gt 0 ]]; then
    echo ""
    echo "── Generating repository index ──"
    "$APKG_INDEX" \
        -d "${HOSTING_DIR}/packages/${ARCH}" \
        -o "${HOSTING_DIR}/index.json" \
        -n "anyOS Packages" \
        -a "$ARCH"
    echo ""
fi

# ── Summary ──────────────────────────────────────────────────────────

echo "════════════════════════════════════════════"
echo "  Packages built:  ${PKG_COUNT}"
echo "  Failures:        ${FAIL_COUNT}"
echo "  Output:          ${HOSTING_DIR}/"
echo "  Architecture:    ${ARCH}"
echo "════════════════════════════════════════════"

if [[ "$PKG_COUNT" -eq 0 ]]; then
    echo ""
    echo "No packages were built. Create package definitions in packages/<name>/pkg.json"
    echo "See packages/README.md for the expected format."
fi

exit $((FAIL_COUNT > 0 ? 1 : 0))

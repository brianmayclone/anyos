#!/usr/bin/env bash
# deploy_packages.sh — Build packages and deploy to Firebase Hosting.
#
# Usage:
#   ./scripts/deploy_packages.sh [--arch x86_64|aarch64] [--skip-build] [--init]
#
# Prerequisites:
#   - Firebase CLI: npm install -g firebase-tools
#   - Logged in: firebase login
#   - Project built: ./scripts/build.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
HOSTING_DIR="${PROJECT_DIR}/firebase-hosting/public"

# Defaults
ARCH="x86_64"
SKIP_BUILD=0
DO_INIT=0

# ── Parse arguments ──────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch)
            ARCH="$2"
            shift 2
            ;;
        --skip-build)
            SKIP_BUILD=1
            shift
            ;;
        --init)
            DO_INIT=1
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [--arch x86_64|aarch64] [--skip-build] [--init]"
            echo ""
            echo "Options:"
            echo "  --arch <arch>    Target architecture (default: x86_64)"
            echo "  --skip-build     Skip package building, deploy existing files"
            echo "  --init           Run firebase init before deploying"
            echo "  -h, --help       Show this help"
            exit 0
            ;;
        *)
            echo "Error: unknown option '$1'"
            exit 1
            ;;
    esac
done

# ── Check Firebase CLI ───────────────────────────────────────────────

if ! command -v firebase &>/dev/null; then
    echo "Error: Firebase CLI not found."
    echo "Install it with: npm install -g firebase-tools"
    echo "Then log in with: firebase login"
    exit 1
fi

# ── Firebase init (optional) ─────────────────────────────────────────

if [[ "$DO_INIT" -eq 1 ]]; then
    echo "Initializing Firebase project..."
    cd "$PROJECT_DIR/firebase-hosting"
    firebase init hosting --project serocash-18126
    cd "$PROJECT_DIR"
    echo "Firebase initialized. Review firebase-hosting/firebase.json if needed."
fi

# ── Build packages ───────────────────────────────────────────────────

if [[ "$SKIP_BUILD" -eq 0 ]]; then
    echo "Building packages..."
    "${SCRIPT_DIR}/publish_packages.sh" --arch "$ARCH" --clean
fi

# ── Validate output ──────────────────────────────────────────────────

if [[ ! -f "${HOSTING_DIR}/index.json" ]]; then
    echo "Error: ${HOSTING_DIR}/index.json not found."
    echo "Run without --skip-build or run publish_packages.sh first."
    exit 1
fi

# ── Deploy to Firebase Hosting ───────────────────────────────────────

echo ""
echo "Deploying to Firebase Hosting..."
cd "$PROJECT_DIR/firebase-hosting"
firebase deploy --only hosting:anyos-pkg --project serocash-18126

echo ""
echo "Deployed successfully!"
echo "Repository URL: https://anyos-pkg.web.app"
echo ""
echo "Users can add this mirror with:"
echo "  apkg mirror add https://anyos-pkg.web.app"
echo "  apkg update"

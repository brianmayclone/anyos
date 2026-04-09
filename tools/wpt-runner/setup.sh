#!/bin/bash
# WPT CSS Test Suite Setup
# Sparse-clones only the css/ directory from web-platform-tests/wpt (~500MB instead of ~100GB)

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WPT_DIR="$SCRIPT_DIR/wpt"

if [ -d "$WPT_DIR/css" ]; then
    echo "[wpt-setup] WPT css/ already present at $WPT_DIR/css"
    echo "[wpt-setup] To update: cd $WPT_DIR && git pull"
    echo "[wpt-setup] To re-download: rm -rf $WPT_DIR && re-run this script"
    exit 0
fi

echo "[wpt-setup] Sparse-cloning WPT (css/ + common resources only)..."

# Remove partial clone if exists
rm -rf "$WPT_DIR"

git clone --filter=blob:none --no-checkout --depth=1 \
    https://github.com/web-platform-tests/wpt.git "$WPT_DIR"

cd "$WPT_DIR"

# Enable sparse checkout
git sparse-checkout init --cone

# Only fetch css tests + shared support files (fonts, images, common CSS)
git sparse-checkout set \
    css/CSS2 \
    css/css-align \
    css/css-backgrounds \
    css/css-box \
    css/css-break \
    css/css-cascade \
    css/css-color \
    css/css-display \
    css/css-flexbox \
    css/css-fonts \
    css/css-grid \
    css/css-images \
    css/css-inline \
    css/css-lists \
    css/css-multicol \
    css/css-overflow \
    css/css-position \
    css/css-pseudo \
    css/css-selectors \
    css/css-sizing \
    css/css-text \
    css/css-text-decor \
    css/css-transforms \
    css/css-ui \
    css/css-values \
    css/css-writing-modes \
    css/support \
    css/reference \
    css/fonts \
    common \
    fonts

git checkout

echo ""
echo "[wpt-setup] Done! WPT CSS tests at: $WPT_DIR/css/"
echo "[wpt-setup] Run tests with: ./run.sh css/css-flexbox"

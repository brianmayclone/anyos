#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Download a curated subset of the tc39/test262 ECMAScript conformance suite.
#
# Only tests that can run without a full browser/Node environment are fetched:
#   - language/expressions/*
#   - language/statements/*
#   - language/types/*
#   - language/literals/*
#   - built-ins/Array/*
#   - built-ins/String/*
#   - built-ins/Number/*
#   - built-ins/Math/*
#   - built-ins/JSON/*
#   - built-ins/Object/*
#
# The tests land in:  libs/libjs_tests/test262/
#
# Usage: ./scripts/download_test262.sh [--force]
#   --force   Re-download even if the directory already exists

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${SCRIPT_DIR}/.."
OUT_DIR="${PROJECT_DIR}/libs/libjs_tests/test262"

REPO="https://github.com/tc39/test262"
BRANCH="main"

# GitHub raw content base URL
RAW="https://raw.githubusercontent.com/tc39/test262/${BRANCH}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

FORCE=0
if [[ "${1:-}" == "--force" ]]; then
    FORCE=1
fi

echo ""
echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${CYAN}${BOLD}  tc39/test262 — curated subset download${NC}"
echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

# Check dependencies
for dep in git curl; do
    if ! command -v "$dep" &>/dev/null; then
        echo -e "${RED}ERROR: '$dep' is required but not found.${NC}"
        exit 1
    fi
done

# ── Strategy: sparse git checkout ────────────────────────────────────────────
# We use a sparse checkout so we only get the test subdirectories we care about,
# not the full 300 MB repository.

CLONE_DIR="${OUT_DIR}/.git-sparse"

if [[ -d "${OUT_DIR}/language" && "$FORCE" -eq 0 ]]; then
    echo -e "${YELLOW}test262 already present at ${OUT_DIR}${NC}"
    echo "Use --force to re-download."
    echo ""
    # Count existing tests
    count=$(find "${OUT_DIR}" -name '*.js' | wc -l | tr -d ' ')
    echo -e "${GREEN}  ${count} test files ready.${NC}"
    echo ""
    exit 0
fi

echo -e "  Source:  ${REPO}"
echo -e "  Target:  ${OUT_DIR}"
echo ""

rm -rf "${CLONE_DIR}" "${OUT_DIR}"
mkdir -p "${CLONE_DIR}" "${OUT_DIR}"

echo -e "${BOLD}  Cloning (sparse, depth=1) …${NC}"

git -C "${CLONE_DIR}" init -q
git -C "${CLONE_DIR}" remote add origin "${REPO}"
git -C "${CLONE_DIR}" config core.sparseCheckout true

# Sparse checkout paths — only what our engine can handle
cat > "${CLONE_DIR}/.git/info/sparse-checkout" << 'EOF'
harness/assert.js
harness/sta.js
test/language/expressions/
test/language/statements/
test/language/types/
test/language/literals/
test/language/block-scope/
test/built-ins/Array/prototype/
test/built-ins/String/prototype/
test/built-ins/Number/prototype/
test/built-ins/Math/
test/built-ins/JSON/
test/built-ins/Object/prototype/
test/built-ins/Boolean/
EOF

echo -e "  Fetching …"
git -C "${CLONE_DIR}" fetch --depth=1 origin "${BRANCH}" -q

echo -e "  Checking out …"
git -C "${CLONE_DIR}" checkout -q FETCH_HEAD

# ── Copy test files into our output directory ─────────────────────────────────
echo -e "  Copying tests …"

# Copy harness
mkdir -p "${OUT_DIR}/harness"
cp -r "${CLONE_DIR}/harness/." "${OUT_DIR}/harness/" 2>/dev/null || true

# Copy test directories
for subdir in \
    "test/language/expressions" \
    "test/language/statements" \
    "test/language/types" \
    "test/language/literals" \
    "test/language/block-scope" \
    "test/built-ins/Array/prototype" \
    "test/built-ins/String/prototype" \
    "test/built-ins/Number/prototype" \
    "test/built-ins/Math" \
    "test/built-ins/JSON" \
    "test/built-ins/Object/prototype" \
    "test/built-ins/Boolean"
do
    src="${CLONE_DIR}/${subdir}"
    if [[ -d "$src" ]]; then
        rel="${subdir#test/}"          # strip leading "test/"
        dst="${OUT_DIR}/${rel}"
        mkdir -p "$dst"
        cp -r "${src}/." "${dst}/"
    fi
done

# Clean up sparse clone
rm -rf "${CLONE_DIR}"

# ── Write .gitignore so the downloaded tests aren't committed ─────────────────
cat > "${OUT_DIR}/.gitignore" << 'EOF'
# test262 files are downloaded at dev time, not committed.
# Run: ./scripts/download_test262.sh
*
!.gitignore
!README.md
EOF

cat > "${OUT_DIR}/README.md" << 'EOF'
# test262 — downloaded subset

This directory contains a curated subset of the [tc39/test262](https://github.com/tc39/test262)
ECMAScript conformance test suite.

Run `./scripts/download_test262.sh` to (re-)download it.

Run the tests with:
```
./scripts/test.sh --tc39
```
EOF

echo ""
count=$(find "${OUT_DIR}" -name '*.js' ! -path '*/harness/*' | wc -l | tr -d ' ')
echo -e "${GREEN}${BOLD}  ✓ Done — ${count} test files downloaded.${NC}"
echo ""
echo -e "  Run tests with:  ${BOLD}./scripts/test.sh --tc39${NC}"
echo ""

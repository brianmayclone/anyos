#!/bin/bash
# Script to add copyright headers to all source files in the anyOS project.
# Usage: ./scripts/add_copyright.sh [--check] [--remove]
#   --check   Only report files missing the header (no modifications)
#   --remove  Remove existing copyright headers

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Copyright header templates
read -r -d '' RUST_HEADER << 'EOF' || true
// Copyright (c) 2024-2026 Christian Moeller
// Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
//
// This project is open source and community-driven.
// Contributions are welcome! See README.md for details.
//
// SPDX-License-Identifier: MIT
EOF

read -r -d '' ASM_HEADER << 'EOF' || true
; Copyright (c) 2024-2026 Christian Moeller
; Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
;
; This project is open source and community-driven.
; Contributions are welcome! See README.md for details.
;
; SPDX-License-Identifier: MIT
EOF

read -r -d '' C_HEADER << 'EOF' || true
/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */
EOF

read -r -d '' PYTHON_HEADER << 'EOF' || true
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT
EOF

read -r -d '' CMAKE_HEADER << 'EOF' || true
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT
EOF

read -r -d '' LD_HEADER << 'EOF' || true
/* Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 * SPDX-License-Identifier: MIT */
EOF

MODE="${1:-add}"
ADDED=0
SKIPPED=0
CHECKED=0

has_copyright() {
    local file="$1"
    # Check first 10 lines for copyright marker
    head -n 10 "$file" 2>/dev/null | grep -qi "copyright" && return 0
    return 1
}

remove_copyright() {
    local file="$1"
    local ext="$2"
    local temp
    temp=$(mktemp)

    case "$ext" in
        rs)
            # Remove leading // Copyright block + blank line
            awk '
                BEGIN { in_header=1 }
                in_header && /^\/\/ (Copyright|Email|This project|Contributions|SPDX)/ { next }
                in_header && /^\/\/$/ { next }
                in_header && /^$/ { in_header=0; next }
                { in_header=0; print }
            ' "$file" > "$temp"
            ;;
        asm|inc)
            awk '
                BEGIN { in_header=1 }
                in_header && /^; (Copyright|Email|This project|Contributions|SPDX)/ { next }
                in_header && /^;$/ { next }
                in_header && /^$/ { in_header=0; next }
                { in_header=0; print }
            ' "$file" > "$temp"
            ;;
        c|h|S)
            awk '
                BEGIN { in_header=1 }
                in_header && /^\/\*/ && /Copyright/ { in_block=1; next }
                in_block && /\*\// { in_block=0; next }
                in_block { next }
                in_header && /^$/ { in_header=0; next }
                { in_header=0; print }
            ' "$file" > "$temp"
            ;;
        py|sh|cmake)
            awk '
                BEGIN { in_header=1 }
                in_header && NR==1 && /^#!/ { print; next }
                in_header && /^# (Copyright|Email|This project|Contributions|SPDX)/ { next }
                in_header && /^#$/ { next }
                in_header && /^$/ { in_header=0; next }
                { in_header=0; print }
            ' "$file" > "$temp"
            ;;
        ld)
            awk '
                BEGIN { in_header=1 }
                in_header && /^\/\* Copyright/ { in_block=1; next }
                in_block && /\*\// { in_block=0; next }
                in_block { next }
                in_header && /^$/ { in_header=0; next }
                { in_header=0; print }
            ' "$file" > "$temp"
            ;;
    esac

    mv "$temp" "$file"
}

add_header() {
    local file="$1"
    local header="$2"
    local ext="$3"
    local temp
    temp=$(mktemp)

    # For scripts with shebang, preserve it
    if [[ "$ext" == "py" || "$ext" == "sh" ]] && head -n 1 "$file" | grep -q "^#!"; then
        head -n 1 "$file" > "$temp"
        echo "" >> "$temp"
        echo "$header" >> "$temp"
        echo "" >> "$temp"
        tail -n +2 "$file" >> "$temp"
    else
        echo "$header" >> "$temp"
        echo "" >> "$temp"
        cat "$file" >> "$temp"
    fi

    mv "$temp" "$file"
}

process_file() {
    local file="$1"
    local rel_path="${file#$PROJECT_ROOT/}"

    # Skip build directory, third_party, .git, target dirs, venv
    case "$rel_path" in
        build/*|third_party/*|.git/*|*/target/*|*/.git/*|.venv/*) return ;;
    esac

    local ext="${file##*.}"
    local basename="$(basename "$file")"
    local header=""
    local file_ext=""

    case "$ext" in
        rs)    header="$RUST_HEADER"; file_ext="rs" ;;
        asm)   header="$ASM_HEADER"; file_ext="asm" ;;
        inc)   header="$ASM_HEADER"; file_ext="inc" ;;
        c)     header="$C_HEADER"; file_ext="c" ;;
        h)     header="$C_HEADER"; file_ext="h" ;;
        S)     header="$C_HEADER"; file_ext="S" ;;
        py)    header="$PYTHON_HEADER"; file_ext="py" ;;
        sh)    header="$CMAKE_HEADER"; file_ext="sh" ;;
        ld)    header="$LD_HEADER"; file_ext="ld" ;;
        *)     return ;;
    esac

    # Handle CMakeLists.txt specially
    if [[ "$basename" == "CMakeLists.txt" ]]; then
        header="$CMAKE_HEADER"
        file_ext="cmake"
    fi

    CHECKED=$((CHECKED + 1))

    if [[ "$MODE" == "--check" ]]; then
        if ! has_copyright "$file"; then
            echo "  MISSING: $rel_path"
            ADDED=$((ADDED + 1))
        fi
        return
    fi

    if [[ "$MODE" == "--remove" ]]; then
        if has_copyright "$file"; then
            remove_copyright "$file" "$file_ext"
            echo "  REMOVED: $rel_path"
            ADDED=$((ADDED + 1))
        fi
        return
    fi

    # Default: add mode
    if has_copyright "$file"; then
        SKIPPED=$((SKIPPED + 1))
        return
    fi

    add_header "$file" "$header" "$file_ext"
    echo "  ADDED:   $rel_path"
    ADDED=$((ADDED + 1))
}

echo "anyOS Copyright Header Tool"
echo "==========================="
echo "Mode: $MODE"
echo ""

# Find all source files (excluding build, third_party, target, .git)
while IFS= read -r -d '' file; do
    process_file "$file"
done < <(find "$PROJECT_ROOT" \
    -path "$PROJECT_ROOT/build" -prune -o \
    -path "$PROJECT_ROOT/third_party" -prune -o \
    -path "$PROJECT_ROOT/.git" -prune -o \
    -path "$PROJECT_ROOT/.venv" -prune -o \
    -path "*/target" -prune -o \
    \( -name "*.rs" -o -name "*.asm" -o -name "*.inc" \
       -o -name "*.c" -o -name "*.h" -o -name "*.S" \
       -o -name "*.py" -o -name "*.sh" -o -name "*.ld" \
       -o -name "CMakeLists.txt" \) \
    -type f -print0 2>/dev/null)

echo ""
echo "Done! Checked: $CHECKED, Added/Matched: $ADDED, Skipped (already has): $SKIPPED"

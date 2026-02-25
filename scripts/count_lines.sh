#!/usr/bin/env bash
# Count source lines of code (excluding comments and blank lines) for anyOS.
# Usage: ./scripts/count_lines.sh

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

DIRS=(kernel libs scripts system buildsystem bootloader bin apps tools)
EXTS=(c cpp h rs asm nasm def yaml yml xml json)

# Build find -name pattern
name_args=()
for ext in "${EXTS[@]}"; do
    [[ ${#name_args[@]} -gt 0 ]] && name_args+=(-o)
    name_args+=(-name "*.${ext}")
done

strip_comments() {
    # Remove C/Rust style line comments (// ...), hash comments (# ...),
    # semicolon comments (; ... for asm), then drop blank lines.
    # Block comments (/* ... */) are handled with a simple state machine via awk.
    awk '
    BEGIN { in_block = 0 }
    {
        line = $0
        # Handle block comments /* ... */
        while (1) {
            if (in_block) {
                idx = index(line, "*/")
                if (idx > 0) {
                    line = substr(line, idx + 2)
                    in_block = 0
                } else {
                    line = ""
                    break
                }
            } else {
                idx = index(line, "/*")
                if (idx > 0) {
                    prefix = substr(line, 1, idx - 1)
                    line = substr(line, idx + 2)
                    in_block = 1
                    # Keep the prefix before /*
                    if (length(prefix) > 0) {
                        out = prefix
                    }
                } else {
                    break
                }
            }
        }
        if (in_block) next

        # Remove line comments: //, #, ;
        # But not inside strings (simple heuristic: first occurrence outside quotes)
        gsub(/\/\/.*/, "", line)

        # For .asm/.nasm: ; is comment. For others # is comment.
        # We detect file type from FILENAME variable set per-file.
        if (filetype == "asm" || filetype == "nasm") {
            gsub(/;.*/, "", line)
        } else if (filetype != "json" && filetype != "xml" && filetype != "yaml" && filetype != "yml" && filetype != "def") {
            # For C/Rust/build files: # could be preprocessor or comment
            # Skip # stripping for C/H (preprocessor) and Rust (attributes)
            if (filetype != "c" && filetype != "cpp" && filetype != "h" && filetype != "rs") {
                gsub(/#.*/, "", line)
            }
        }

        # For shell/yaml/python scripts
        if (filetype == "yaml" || filetype == "yml") {
            gsub(/#.*/, "", line)
        }

        # Trim whitespace
        gsub(/^[[:space:]]+/, "", line)
        gsub(/[[:space:]]+$/, "", line)

        if (length(line) > 0) count++
    }
    END { printf "%d", count }
    ' count=0 filetype="$1"
}

printf "\n"
printf "  %-14s %8s %8s %8s\n" "Directory" "Files" "Total" "Code"
printf "  %-14s %8s %8s %8s\n" "---------" "-----" "-----" "----"

grand_files=0
grand_total=0
grand_code=0

for dir in "${DIRS[@]}"; do
    dirpath="${ROOT}/${dir}"
    [[ -d "$dirpath" ]] || continue

    dir_files=0
    dir_total=0
    dir_code=0

    while IFS= read -r file; do
        [[ -f "$file" ]] || continue
        dir_files=$((dir_files + 1))

        # Count total lines
        total=$(wc -l < "$file" | tr -d ' ')
        dir_total=$((dir_total + total))

        # Determine file type for comment stripping
        ext="${file##*.}"
        code=$(strip_comments "$ext" < "$file")
        dir_code=$((dir_code + code))

    done < <(find "$dirpath" \( "${name_args[@]}" \) -not -path "*/target/*" -not -path "*/build/*" -not -path "*/.git/*" -not -path "*/third_party/*")

    if [[ $dir_files -gt 0 ]]; then
        printf "  %-14s %8d %8d %8d\n" "$dir" "$dir_files" "$dir_total" "$dir_code"
    fi

    grand_files=$((grand_files + dir_files))
    grand_total=$((grand_total + dir_total))
    grand_code=$((grand_code + dir_code))
done

printf "  %-14s %8s %8s %8s\n" "---------" "-----" "-----" "----"
printf "  %-14s %8d %8d %8d\n" "TOTAL" "$grand_files" "$grand_total" "$grand_code"

# Breakdown by file extension
printf "\n  By extension:\n"
printf "  %-8s %8s %8s\n" "Ext" "Files" "Code"
printf "  %-8s %8s %8s\n" "---" "-----" "----"

for ext in "${EXTS[@]}"; do
    ext_files=0
    ext_code=0
    for dir in "${DIRS[@]}"; do
        dirpath="${ROOT}/${dir}"
        [[ -d "$dirpath" ]] || continue
        while IFS= read -r file; do
            [[ -f "$file" ]] || continue
            ext_files=$((ext_files + 1))
            code=$(strip_comments "$ext" < "$file")
            ext_code=$((ext_code + code))
        done < <(find "$dirpath" -name "*.${ext}" -not -path "*/target/*" -not -path "*/build/*" -not -path "*/.git/*" -not -path "*/third_party/*")
    done
    if [[ $ext_files -gt 0 ]]; then
        printf "  %-8s %8d %8d\n" ".${ext}" "$ext_files" "$ext_code"
    fi
done

printf "\n"

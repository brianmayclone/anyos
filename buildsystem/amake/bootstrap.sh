#!/bin/bash
# Bootstrap script — compile amake from source using the host cc.
# No dependencies other than a C99 compiler and POSIX headers.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SRC="${SCRIPT_DIR}/src"
OUT="${SCRIPT_DIR}/../../buildsystem/build/amake"

mkdir -p "$(dirname "$OUT")"

echo "Bootstrapping amake..."
cc -Wall -O2 -std=c99 -o "$OUT" \
    "${SRC}/amake.c" \
    "${SRC}/vars.c" \
    "${SRC}/lexer.c" \
    "${SRC}/parser.c" \
    "${SRC}/glob.c" \
    "${SRC}/track.c" \
    "${SRC}/eval.c" \
    "${SRC}/graph.c" \
    "${SRC}/exec.c"

echo "Built: ${OUT}"
echo "Usage: ${OUT} -B build [target]"

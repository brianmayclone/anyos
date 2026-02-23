#!/bin/bash
# Build dash 0.5.12 for anyOS (cross-compilation)
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
DASH_DIR="${ROOT_DIR}/third_party/dash-0.5.12"
LIBC_DIR="${ROOT_DIR}/libs/libc"
GCC="${I686_ELF_GCC:-i686-elf-gcc}"
AR="${I686_ELF_AR:-i686-elf-ar}"

OBJ_DIR="${DASH_DIR}/obj"
rm -rf "${OBJ_DIR}"
mkdir -p "${OBJ_DIR}" "${OBJ_DIR}/bltin"

# Common flags
CFLAGS="-ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector"
CFLAGS="${CFLAGS} -O2 -m32 -Wall -Wno-unused-but-set-variable -Wno-unused-parameter"
CFLAGS="${CFLAGS} -include ${DASH_DIR}/config.h"
CFLAGS="${CFLAGS} -DBSD=1 -DSHELL"
CFLAGS="${CFLAGS} -I${DASH_DIR}/generated"
CFLAGS="${CFLAGS} -I${DASH_DIR}/src"
CFLAGS="${CFLAGS} -I${LIBC_DIR}/include"

echo "=== Compiling dash ==="

# Source files from src/
SRC_FILES="
  alias arith_yacc arith_yylex cd error eval exec expand
  histedit input jobs mail main memalloc miscbltin
  mystring options output parser redir show system trap var
"

for f in ${SRC_FILES}; do
    echo "  CC ${f}.c"
    ${GCC} ${CFLAGS} -c "${DASH_DIR}/src/${f}.c" -o "${OBJ_DIR}/${f}.o"
done

# Builtin files from src/bltin/
for f in printf test times; do
    echo "  CC bltin/${f}.c"
    ${GCC} ${CFLAGS} -c "${DASH_DIR}/src/bltin/${f}.c" -o "${OBJ_DIR}/bltin/${f}.o"
done

# Generated files
GEN_FILES="builtins init nodes signames syntax"
for f in ${GEN_FILES}; do
    echo "  CC generated/${f}.c"
    ${GCC} ${CFLAGS} -c "${DASH_DIR}/generated/${f}.c" -o "${OBJ_DIR}/${f}.o"
done

echo "=== Creating dash.a ==="
ALL_OBJS=""
for f in ${SRC_FILES}; do ALL_OBJS="${ALL_OBJS} ${OBJ_DIR}/${f}.o"; done
ALL_OBJS="${ALL_OBJS} ${OBJ_DIR}/bltin/printf.o ${OBJ_DIR}/bltin/test.o ${OBJ_DIR}/bltin/times.o"
for f in ${GEN_FILES}; do ALL_OBJS="${ALL_OBJS} ${OBJ_DIR}/${f}.o"; done

${AR} rcs "${DASH_DIR}/dash.a" ${ALL_OBJS}
echo "dash.a created ($(wc -c < "${DASH_DIR}/dash.a") bytes)"

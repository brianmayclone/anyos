#!/bin/bash
# Build NASM assembler for anyOS (cross-compiled with i686-elf-gcc)
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
NASM_DIR="$PROJECT_DIR/third_party/nasm"
LIBC_DIR="$PROJECT_DIR/libs/libc"
OBJ_DIR="$NASM_DIR/obj"
OUTPUT="$NASM_DIR/nasm.elf"

CC=i686-elf-gcc
AR=i686-elf-ar

CFLAGS="-m32 -O2 -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector -fcommon -w"
CFLAGS="$CFLAGS -DHAVE_CONFIG_H"
CFLAGS="$CFLAGS -I$NASM_DIR -I$NASM_DIR/include -I$NASM_DIR/x86 -I$NASM_DIR/asm -I$NASM_DIR/output"
CFLAGS="$CFLAGS -I$NASM_DIR/nasmlib -I$NASM_DIR/macros -I$NASM_DIR/common -I$NASM_DIR/disasm"
CFLAGS="$CFLAGS -I$LIBC_DIR/include"

mkdir -p "$OBJ_DIR"

# NASM main
NASM_MAIN="asm/nasm.c"

# Library sources (LIBOBJ_NW + warnings)
LIBSRCS="
  stdlib/snprintf.c
  stdlib/vsnprintf.c
  stdlib/strlcpy.c
  stdlib/strnlen.c
  stdlib/strrchrnul.c
  nasmlib/ver.c
  nasmlib/alloc.c
  nasmlib/asprintf.c
  nasmlib/errfile.c
  nasmlib/crc32.c
  nasmlib/crc64.c
  nasmlib/md5c.c
  nasmlib/string.c
  nasmlib/nctype.c
  nasmlib/file.c
  nasmlib/mmap.c
  nasmlib/ilog2.c
  nasmlib/realpath.c
  nasmlib/path.c
  nasmlib/filename.c
  nasmlib/rlimit.c
  nasmlib/readnum.c
  nasmlib/numstr.c
  nasmlib/zerobuf.c
  nasmlib/bsi.c
  nasmlib/rbtree.c
  nasmlib/hashtbl.c
  nasmlib/raa.c
  nasmlib/saa.c
  nasmlib/strlist.c
  nasmlib/perfhash.c
  nasmlib/badenum.c
  common/common.c
  x86/insnsa.c
  x86/insnsb.c
  x86/insnsd.c
  x86/insnsn.c
  x86/regs.c
  x86/regvals.c
  x86/regflags.c
  x86/regdis.c
  x86/disp8.c
  x86/iflag.c
  asm/error.c
  asm/floats.c
  asm/directiv.c
  asm/directbl.c
  asm/pragma.c
  asm/assemble.c
  asm/labels.c
  asm/parser.c
  asm/preproc.c
  asm/quote.c
  asm/pptok.c
  asm/listing.c
  asm/eval.c
  asm/exprlib.c
  asm/exprdump.c
  asm/stdscan.c
  asm/strfunc.c
  asm/tokhash.c
  asm/segalloc.c
  asm/rdstrnum.c
  asm/srcfile.c
  asm/warnings.c
  macros/macros.c
  output/outform.c
  output/outlib.c
  output/legacy.c
  output/nulldbg.c
  output/nullout.c
  output/outbin.c
  output/outaout.c
  output/outcoff.c
  output/outelf.c
  output/outobj.c
  output/outas86.c
  output/outdbg.c
  output/outieee.c
  output/outmacho.c
  output/codeview.c
  disasm/disasm.c
  disasm/sync.c
"

echo "=== Compiling NASM for anyOS ==="

# Compile all source files
OBJS=""
for src in $LIBSRCS $NASM_MAIN; do
    obj="$OBJ_DIR/$(echo $src | sed 's|/|_|g; s|\.c$|.o|')"
    echo "  CC $src"
    $CC $CFLAGS -c "$NASM_DIR/$src" -o "$obj"
    OBJS="$OBJS $obj"
done

echo "=== Linking NASM ==="
$CC -nostdlib -static -m32 \
    -T "$LIBC_DIR/link.ld" \
    -o "$OUTPUT" \
    "$LIBC_DIR/obj/crt0.o" \
    $OBJS \
    "$LIBC_DIR/libc.a" \
    -lgcc

echo "=== Done: $OUTPUT ==="
ls -la "$OUTPUT"

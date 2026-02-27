#!/bin/bash
# Copyright (c) 2024-2026 Christian Moeller
# SPDX-License-Identifier: MIT
#
# Build x86_64-anyos cross-compiler (GCC 12.4.0 + Binutils 2.41).
#
# Produces a GCC cross-compiler that runs on the host (macOS/Linux) and
# targets anyOS.  Also builds libgcc.a for linking into anyOS programs.
#
# Usage:
#   ./scripts/build_gcc_toolchain.sh [--prefix DIR] [--sysroot DIR] [--jobs N]
#
# After install, add to PATH:
#   export PATH="$HOME/opt/anyos-toolchain/bin:$PATH"

set -euo pipefail

# ── Configuration ────────────────────────────────────────────────────────────

TARGET="x86_64-anyos"
BINUTILS_VERSION="2.41"
GCC_VERSION="12.4.0"

# Defaults (overridable via env or flags)
PREFIX="${ANYOS_TOOLCHAIN:-$HOME/opt/anyos-toolchain}"
SYSROOT=""
JOBS=""

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PATCHES_DIR="$PROJECT_DIR/third_party/gcc-12.4.0/anyos-patches"

SRC_DIR="$HOME/src/anyos-toolchain"
BUILD_DIR="$HOME/build/anyos-toolchain"

# ── Parse arguments ──────────────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)  PREFIX="$2"; shift 2 ;;
    --sysroot) SYSROOT="$2"; shift 2 ;;
    --jobs)    JOBS="$2"; shift 2 ;;
    --help|-h)
      echo "Usage: $0 [--prefix DIR] [--sysroot DIR] [--jobs N]"
      exit 0
      ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# Auto-detect job count
if [[ -z "$JOBS" ]]; then
  if command -v nproc &>/dev/null; then
    JOBS="$(nproc)"
  elif command -v sysctl &>/dev/null; then
    JOBS="$(sysctl -n hw.ncpu)"
  else
    JOBS=4
  fi
fi

# ── Banner ───────────────────────────────────────────────────────────────────

echo "========================================="
echo " anyOS C++ Toolchain Builder"
echo "========================================="
echo "  Target:   $TARGET"
echo "  Prefix:   $PREFIX"
echo "  Binutils: $BINUTILS_VERSION"
echo "  GCC:      $GCC_VERSION"
echo "  Jobs:     $JOBS"
echo "  Patches:  $PATCHES_DIR"
echo ""

# ── Prerequisites check ─────────────────────────────────────────────────────

check_tool() {
  if ! command -v "$1" &>/dev/null; then
    echo "ERROR: Required tool '$1' not found."
    echo "  Install it with your package manager."
    exit 1
  fi
}

check_tool make
check_tool gcc
check_tool g++
check_tool tar
check_tool wget

# Check for GMP, MPC, MPFR (needed by GCC)
echo "--- Checking build dependencies ---"
if [[ "$(uname)" == "Darwin" ]]; then
  # macOS: use Homebrew
  for pkg in gmp mpfr libmpc; do
    if ! brew list "$pkg" &>/dev/null 2>&1; then
      echo "Installing $pkg via Homebrew..."
      brew install "$pkg"
    fi
  done
  # Homebrew paths for configure
  GMP_DIR="$(brew --prefix gmp)"
  MPFR_DIR="$(brew --prefix mpfr)"
  MPC_DIR="$(brew --prefix libmpc)"
  EXTRA_GCC_CONFIGURE="--with-gmp=$GMP_DIR --with-mpfr=$MPFR_DIR --with-mpc=$MPC_DIR"
  # macOS sed needs '' for -i
  SED_INPLACE=(sed -i '')
else
  # Linux: check for dev packages
  EXTRA_GCC_CONFIGURE=""
  SED_INPLACE=(sed -i)
fi

# ── Directories ──────────────────────────────────────────────────────────────

mkdir -p "$PREFIX" "$SRC_DIR" "$BUILD_DIR"
export PATH="$PREFIX/bin:$PATH"

# ── Download ─────────────────────────────────────────────────────────────────

cd "$SRC_DIR"

if [ ! -f "binutils-${BINUTILS_VERSION}.tar.xz" ]; then
  echo "--- Downloading binutils-${BINUTILS_VERSION} ---"
  wget -q --show-progress "https://ftp.gnu.org/gnu/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
fi

if [ ! -f "gcc-${GCC_VERSION}.tar.xz" ]; then
  echo "--- Downloading gcc-${GCC_VERSION} ---"
  wget -q --show-progress "https://ftp.gnu.org/gnu/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"
fi

# ── Extract ──────────────────────────────────────────────────────────────────

echo "--- Extracting sources ---"
[ ! -d "binutils-${BINUTILS_VERSION}" ] && tar xf "binutils-${BINUTILS_VERSION}.tar.xz"
[ ! -d "gcc-${GCC_VERSION}" ]           && tar xf "gcc-${GCC_VERSION}.tar.xz"

# ── Patch binutils for anyOS ────────────────────────────────────────────────

BINUTILS_SRC="$SRC_DIR/binutils-${BINUTILS_VERSION}"

echo ""
echo "--- Patching binutils for x86_64-anyos ---"

# 1. config.sub: Teach the system about anyos as a valid OS.
if ! grep -q 'anyos' "$BINUTILS_SRC/config.sub" 2>/dev/null; then
  # Add anyos* to the OS list (near "none)" in the first os case block)
  "${SED_INPLACE[@]}" '/^	      -none)$/i\
	      -anyos*)
' "$BINUTILS_SRC/config.sub"
  echo "  patched config.sub"
fi

# Also patch the top-level config.sub used by configure
for f in "$BINUTILS_SRC"/*/config.sub; do
  if [ -f "$f" ] && ! grep -q 'anyos' "$f" 2>/dev/null; then
    "${SED_INPLACE[@]}" '/^	      -none)$/i\
	      -anyos*)
' "$f"
  fi
done

# 2. bfd/config.bfd: Map x86_64-*-anyos* to ELF64 x86-64.
if ! grep -q 'anyos' "$BINUTILS_SRC/bfd/config.bfd" 2>/dev/null; then
  "${SED_INPLACE[@]}" '/x86_64-\*-linux-\*/i\
  x86_64-*-anyos*)\
    targ_defvec=x86_64_elf64_vec\
    targ_selvecs="i386_elf32_vec"\
    want64=true\
    ;;\
' "$BINUTILS_SRC/bfd/config.bfd"
  echo "  patched bfd/config.bfd"
fi

# 3. gas/configure.tgt: Assembler target mapping.
if ! grep -q 'anyos' "$BINUTILS_SRC/gas/configure.tgt" 2>/dev/null; then
  "${SED_INPLACE[@]}" '/x86_64-\*-linux-\*/i\
  x86_64-*-anyos*)			fmt=elf ;;\
' "$BINUTILS_SRC/gas/configure.tgt"
  echo "  patched gas/configure.tgt"
fi

# 4. ld/configure.tgt: Linker target mapping.
if ! grep -q 'anyos' "$BINUTILS_SRC/ld/configure.tgt" 2>/dev/null; then
  "${SED_INPLACE[@]}" '/x86_64-\*-linux-\*/i\
x86_64-*-anyos*)	targ_emul=elf_x86_64\
			targ_extra_emuls="elf_i386" ;;\
' "$BINUTILS_SRC/ld/configure.tgt"
  echo "  patched ld/configure.tgt"
fi

echo "  binutils patching complete"

# ── Patch GCC for anyOS ─────────────────────────────────────────────────────

GCC_SRC="$SRC_DIR/gcc-${GCC_VERSION}"

echo ""
echo "--- Patching GCC for x86_64-anyos ---"

# 1. Copy anyos.h target header.
cp "$PATCHES_DIR/gcc-config-anyos.h" "$GCC_SRC/gcc/config/anyos.h"
echo "  installed gcc/config/anyos.h"

# 2. config.sub: Teach GCC about anyos.
if ! grep -q 'anyos' "$GCC_SRC/config.sub" 2>/dev/null; then
  "${SED_INPLACE[@]}" '/^	      -none)$/i\
	      -anyos*)
' "$GCC_SRC/config.sub"
  echo "  patched config.sub"
fi

# Also patch sub-project config.sub files
for f in "$GCC_SRC"/*/config.sub; do
  if [ -f "$f" ] && ! grep -q 'anyos' "$f" 2>/dev/null; then
    "${SED_INPLACE[@]}" '/^	      -none)$/i\
	      -anyos*)
' "$f"
  fi
done

# 3. gcc/config.gcc: Add the x86_64-anyos target.
if ! grep -q 'anyos' "$GCC_SRC/gcc/config.gcc" 2>/dev/null; then
  # Add common OS stanza (near the "Common parts" section)
  "${SED_INPLACE[@]}" '/^# Common parts for widely ported systems\./a\
\
# anyOS -- bare-metal x86_64 OS with custom libc64/libcxx\
*-*-anyos*)\
  gas=yes\
  gnu_ld=yes\
  default_use_cxa_atexit=yes\
  use_gcc_stdint=provide\
  ;;\
' "$GCC_SRC/gcc/config.gcc"

  # Add machine-specific stanza (before "# Architecture descriptions" or similar)
  "${SED_INPLACE[@]}" '/^x86_64-\*-linux\*/i\
x86_64-*-anyos*)\
	tm_file="${tm_file} i386/unix.h i386/att.h dbxelf.h elfos.h i386/i386elf.h i386/x86-64.h anyos.h"\
	tmake_file="${tmake_file} i386/t-i386elf"\
	extra_options="${extra_options} i386/elf.opt"\
	;;\
' "$GCC_SRC/gcc/config.gcc"
  echo "  patched gcc/config.gcc"
fi

# 4. libgcc/config.host: Configure libgcc for anyOS.
if ! grep -q 'anyos' "$GCC_SRC/libgcc/config.host" 2>/dev/null; then
  "${SED_INPLACE[@]}" '/x86_64-\*-linux\*/i\
x86_64-*-anyos*)\
	extra_parts="crtbegin.o crtend.o"\
	tmake_file="${tmake_file} t-crtstuff"\
	;;\
' "$GCC_SRC/libgcc/config.host"
  echo "  patched libgcc/config.host"
fi

# 5. fixincludes: Disable fixincludes (not needed for freestanding).
if [ -f "$GCC_SRC/fixincludes/mkfixinc.sh" ]; then
  if ! grep -q 'anyos' "$GCC_SRC/fixincludes/mkfixinc.sh" 2>/dev/null; then
    "${SED_INPLACE[@]}" '/    \*-\*-none/a\
    *-*-anyos* | \\' "$GCC_SRC/fixincludes/mkfixinc.sh"
    echo "  patched fixincludes/mkfixinc.sh"
  fi
fi

echo "  GCC patching complete"

# ── Build binutils ───────────────────────────────────────────────────────────

echo ""
echo "--- Building binutils for ${TARGET} ---"
rm -rf "$BUILD_DIR/binutils"
mkdir -p "$BUILD_DIR/binutils"
cd "$BUILD_DIR/binutils"

"$BINUTILS_SRC/configure" \
  --target="$TARGET" \
  --prefix="$PREFIX" \
  --with-sysroot \
  --disable-nls \
  --disable-werror

make -j"$JOBS"
make install
echo "--- binutils installed ---"

# ── Build GCC (C and C++ compilers) ─────────────────────────────────────────

echo ""
echo "--- Building GCC for ${TARGET} ---"
rm -rf "$BUILD_DIR/gcc"
mkdir -p "$BUILD_DIR/gcc"
cd "$BUILD_DIR/gcc"

SYSROOT_FLAGS=""
if [[ -n "$SYSROOT" ]]; then
  SYSROOT_FLAGS="--with-sysroot=$SYSROOT"
fi

"$GCC_SRC/configure" \
  --target="$TARGET" \
  --prefix="$PREFIX" \
  --enable-languages=c,c++ \
  --disable-nls \
  --disable-shared \
  --disable-threads \
  --disable-libssp \
  --disable-libquadmath \
  --disable-libgomp \
  --disable-libatomic \
  --disable-libstdcxx \
  --disable-hosted-libstdcxx \
  --disable-libstdcxx-pch \
  --disable-multilib \
  --without-headers \
  --with-newlib \
  $SYSROOT_FLAGS \
  $EXTRA_GCC_CONFIGURE

# Build compiler + libgcc
make -j"$JOBS" all-gcc all-target-libgcc
make install-gcc install-target-libgcc
echo "--- GCC installed ---"

# ── Copy libgcc.a to project sysroot (if specified) ─────────────────────────

LIBGCC_A="$PREFIX/lib/gcc/$TARGET/${GCC_VERSION}/libgcc.a"

if [[ -n "$SYSROOT" ]] && [[ -f "$LIBGCC_A" ]]; then
  echo ""
  echo "--- Installing libgcc.a to sysroot ---"
  mkdir -p "$SYSROOT/Libraries/libc64/lib"
  cp "$LIBGCC_A" "$SYSROOT/Libraries/libc64/lib/libgcc.a"
  echo "  copied libgcc.a to $SYSROOT/Libraries/libc64/lib/"

  # Also copy crtbegin.o / crtend.o if they were built
  for crt in crtbegin.o crtend.o; do
    CRT_PATH="$PREFIX/lib/gcc/$TARGET/${GCC_VERSION}/$crt"
    if [[ -f "$CRT_PATH" ]]; then
      cp "$CRT_PATH" "$SYSROOT/Libraries/libc64/lib/$crt"
      echo "  copied $crt to sysroot"
    fi
  done
fi

# ── Verify ───────────────────────────────────────────────────────────────────

echo ""
echo "========================================="
echo " Installation complete!"
echo "========================================="
echo ""

"$PREFIX/bin/${TARGET}-gcc" --version | head -1
"$PREFIX/bin/${TARGET}-g++" --version | head -1
"$PREFIX/bin/${TARGET}-ld"  --version | head -1
"$PREFIX/bin/${TARGET}-as"  --version | head -1
"$PREFIX/bin/${TARGET}-ar"  --version | head -1

echo ""
echo "Installed tools:"
ls -1 "$PREFIX/bin/${TARGET}-"* 2>/dev/null | while read f; do
  echo "  $(basename "$f")"
done

if [[ -f "$LIBGCC_A" ]]; then
  echo ""
  echo "libgcc.a: $(wc -c < "$LIBGCC_A") bytes"
fi

echo ""
echo "Add to your shell profile:"
echo "  export PATH=\"$PREFIX/bin:\$PATH\""
echo ""
echo "Usage example:"
echo "  ${TARGET}-g++ -ffreestanding -nostdlib -O2 \\"
echo "    -I /path/to/anyos/libs/libcxx/include \\"
echo "    -I /path/to/anyos/libs/libc64/include \\"
echo "    main.cpp -o main.elf \\"
echo "    -L /path/to/anyos/libs -lcxx -lc++abi -lunwind -lc64 -lgcc"
echo ""

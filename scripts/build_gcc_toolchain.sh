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
#   ./scripts/build_gcc_toolchain.sh --clean [--all]
#
# Options:
#   --clean       Remove build directories (keeps downloaded tarballs)
#   --clean --all Remove build directories AND downloaded sources + installed toolchain
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
DO_CLEAN=false
CLEAN_ALL=false

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
    --clean)   DO_CLEAN=true; shift ;;
    --all)     CLEAN_ALL=true; shift ;;
    --help|-h)
      echo "Usage: $0 [--prefix DIR] [--sysroot DIR] [--jobs N]"
      echo "       $0 --clean [--all]"
      echo ""
      echo "Options:"
      echo "  --prefix DIR    Install cross-compiler to DIR (default: ~/opt/anyos-toolchain)"
      echo "  --sysroot DIR   Copy libgcc.a and CRT files to sysroot"
      echo "  --jobs N        Parallel build jobs (default: auto-detect)"
      echo "  --clean         Remove build directories (keeps downloaded tarballs)"
      echo "  --clean --all   Remove everything: builds, sources, and installed toolchain"
      exit 0
      ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# ── Handle --clean ──────────────────────────────────────────────────────────

if $DO_CLEAN; then
  echo "========================================="
  echo " anyOS Toolchain — Clean"
  echo "========================================="
  echo ""

  # Always remove build directories
  if [[ -d "$BUILD_DIR" ]]; then
    echo "Removing build directory: $BUILD_DIR"
    rm -rf "$BUILD_DIR"
  else
    echo "Build directory not found: $BUILD_DIR (already clean)"
  fi

  if $CLEAN_ALL; then
    # Remove extracted source trees (keep tarballs unless --all)
    if [[ -d "$SRC_DIR/binutils-${BINUTILS_VERSION}" ]]; then
      echo "Removing extracted sources: $SRC_DIR/binutils-${BINUTILS_VERSION}"
      rm -rf "$SRC_DIR/binutils-${BINUTILS_VERSION}"
    fi
    if [[ -d "$SRC_DIR/gcc-${GCC_VERSION}" ]]; then
      echo "Removing extracted sources: $SRC_DIR/gcc-${GCC_VERSION}"
      rm -rf "$SRC_DIR/gcc-${GCC_VERSION}"
    fi
    # Remove downloaded tarballs
    if [[ -f "$SRC_DIR/binutils-${BINUTILS_VERSION}.tar.xz" ]]; then
      echo "Removing tarball: binutils-${BINUTILS_VERSION}.tar.xz"
      rm -f "$SRC_DIR/binutils-${BINUTILS_VERSION}.tar.xz"
    fi
    if [[ -f "$SRC_DIR/gcc-${GCC_VERSION}.tar.xz" ]]; then
      echo "Removing tarball: gcc-${GCC_VERSION}.tar.xz"
      rm -f "$SRC_DIR/gcc-${GCC_VERSION}.tar.xz"
    fi
    # Remove installed toolchain
    if [[ -d "$PREFIX" ]]; then
      echo "Removing installed toolchain: $PREFIX"
      rm -rf "$PREFIX"
    fi
    # Clean up empty source directory
    if [[ -d "$SRC_DIR" ]] && [[ -z "$(ls -A "$SRC_DIR" 2>/dev/null)" ]]; then
      rmdir "$SRC_DIR"
    fi
  else
    echo ""
    echo "Kept downloaded sources in: $SRC_DIR"
    echo "Kept installed toolchain in: $PREFIX"
    echo "Use --clean --all to remove everything."
  fi

  echo ""
  echo "Clean complete."
  exit 0
fi

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

# Mirror list — tries faster mirrors first, falls back to main FTP
BINUTILS_URLS=(
  "https://ftpmirror.gnu.org/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
  "https://mirror.dogado.de/gnu/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
  "https://mirror.netcologne.de/gnu/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
  "https://ftp.fau.de/gnu/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
  "https://ftp.gnu.org/gnu/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
)

GCC_URLS=(
  "https://ftpmirror.gnu.org/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"
  "https://mirror.dogado.de/gnu/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"
  "https://mirror.netcologne.de/gnu/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"
  "https://ftp.fau.de/gnu/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"
  "https://ftp.gnu.org/gnu/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"
)

download_with_mirrors() {
  local dest="$1"
  shift
  local urls=("$@")
  for url in "${urls[@]}"; do
    echo "  Trying: $url"
    if wget -q --show-progress --timeout=10 --tries=1 -O "$dest.part" "$url"; then
      mv "$dest.part" "$dest"
      echo "  OK"
      return 0
    fi
    rm -f "$dest.part"
  done
  echo "ERROR: All mirrors failed for $dest"
  exit 1
}

cd "$SRC_DIR"

if [ ! -f "binutils-${BINUTILS_VERSION}.tar.xz" ]; then
  echo "--- Downloading binutils-${BINUTILS_VERSION} ---"
  download_with_mirrors "binutils-${BINUTILS_VERSION}.tar.xz" "${BINUTILS_URLS[@]}"
fi

if [ ! -f "gcc-${GCC_VERSION}.tar.xz" ]; then
  echo "--- Downloading gcc-${GCC_VERSION} ---"
  download_with_mirrors "gcc-${GCC_VERSION}.tar.xz" "${GCC_URLS[@]}"
fi

# ── Extract ──────────────────────────────────────────────────────────────────

echo "--- Extracting sources ---"
if [ ! -d "binutils-${BINUTILS_VERSION}" ]; then
  tar xf "binutils-${BINUTILS_VERSION}.tar.xz" || { echo "ERROR: Failed to extract binutils tarball."; exit 1; }
fi
if [ ! -d "gcc-${GCC_VERSION}" ]; then
  tar xf "gcc-${GCC_VERSION}.tar.xz" || { echo "ERROR: Failed to extract GCC tarball."; exit 1; }
fi

# Verify extraction succeeded
BINUTILS_SRC="$SRC_DIR/binutils-${BINUTILS_VERSION}"
GCC_SRC="$SRC_DIR/gcc-${GCC_VERSION}"

if [[ ! -f "$BINUTILS_SRC/config.sub" ]]; then
  echo "ERROR: binutils source not found at $BINUTILS_SRC/config.sub"
  echo "  Tarball may be corrupted. Try: $0 --clean && $0"
  exit 1
fi
if [[ ! -f "$GCC_SRC/config.sub" ]]; then
  echo "ERROR: GCC source not found at $GCC_SRC/config.sub"
  echo "  Tarball may be corrupted. Try: $0 --clean && $0"
  exit 1
fi

echo "  binutils source: $BINUTILS_SRC"
echo "  GCC source:      $GCC_SRC"

# ── Patch binutils for anyOS ────────────────────────────────────────────────

# Robust config.sub patcher: inserts "anyos*)" before the "*)" catch-all
# that prints "OS not recognized". Works regardless of indentation.
patch_config_sub() {
  local file="$1"
  if [[ ! -f "$file" ]] || grep -q 'anyos' "$file" 2>/dev/null; then
    return 0
  fi
  # Structure in config.sub:
  #   none)
  #       ;;
  #   *)
  #       echo "Invalid configuration ... OS ... not recognized"
  #
  # We need to find the line number of the *) that precedes "OS.*not recognized"
  # and insert our anyos case BEFORE it.
  local err_line
  err_line=$(grep -n "Invalid configuration.*OS.*not recognized" "$file" | head -1 | cut -d: -f1)
  if [[ -z "$err_line" ]]; then
    echo "  WARNING: could not find OS validation in $file"
    return 1
  fi
  # The *) case pattern is 1 line before the echo
  local insert_line=$((err_line - 1))
  "${SED_INPLACE[@]}" "${insert_line}i\\
	anyos*)\\
		;;
" "$file"
  echo "  patched $(basename "$(dirname "$file")")/$(basename "$file")"
}

echo ""
echo "--- Patching binutils for x86_64-anyos ---"

# 1. config.sub: Teach the system about anyos as a valid OS
patch_config_sub "$BINUTILS_SRC/config.sub"

# Also patch sub-project config.sub files
for f in "$BINUTILS_SRC"/*/config.sub; do
  patch_config_sub "$f"
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
#    GAS uses i386 cpu_type for x86_64 (see line ~117: x86_64* -> cpu_type=i386).
#    So the target pattern must be i386-*-anyos*, inserted before i386-*-elf*.
if ! grep -q 'anyos' "$BINUTILS_SRC/gas/configure.tgt" 2>/dev/null; then
  "${SED_INPLACE[@]}" '/i386-\*-elf\*)/i\
  i386-*-anyos*)				fmt=elf ;;
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

echo ""
echo "--- Patching GCC for x86_64-anyos ---"

# 1. Copy anyos.h target header.
cp "$PATCHES_DIR/gcc-config-anyos.h" "$GCC_SRC/gcc/config/anyos.h"
echo "  installed gcc/config/anyos.h"

# 2. config.sub: Teach GCC about anyos.
patch_config_sub "$GCC_SRC/config.sub"

# Also patch sub-project config.sub files
for f in "$GCC_SRC"/*/config.sub; do
  patch_config_sub "$f"
done

# 3. gcc/config.gcc: Add the x86_64-anyos target.
if ! grep -q 'anyos' "$GCC_SRC/gcc/config.gcc" 2>/dev/null; then
  # Add common OS stanza INSIDE the "case ${target} in" block after "Common parts"
  # Insert anyos before the first real case (*-*-darwin*)
  "${SED_INPLACE[@]}" '/^\*-\*-darwin\*)/i\
*-*-anyos*)\
  gas=yes\
  gnu_ld=yes\
  default_use_cxa_atexit=yes\
  use_gcc_stdint=provide\
  ;;\
' "$GCC_SRC/gcc/config.gcc"

  # Add machine-specific stanza for x86_64-anyos
  "${SED_INPLACE[@]}" '/^x86_64-\*-linux\*/i\
x86_64-*-anyos*)\
	tm_file="${tm_file} i386/unix.h i386/att.h dbxelf.h elfos.h i386/i386elf.h i386/x86-64.h anyos.h"\
	tmake_file="${tmake_file} i386/t-i386elf"\
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
if [[ -x "$PREFIX/bin/${TARGET}-as" ]] && [[ -x "$PREFIX/bin/${TARGET}-ld" ]]; then
  echo "--- binutils already installed, skipping (use --clean to rebuild) ---"
else
  echo "--- Building binutils for ${TARGET} ---"
  rm -rf "$BUILD_DIR/binutils"
  mkdir -p "$BUILD_DIR/binutils"
  cd "$BUILD_DIR/binutils"

  "$BINUTILS_SRC/configure" \
    --target="$TARGET" \
    --prefix="$PREFIX" \
    --with-sysroot \
    --with-system-zlib \
    --disable-nls \
    --disable-werror \
    MAKEINFO=true

  make -j"$JOBS" MAKEINFO=true
  make install MAKEINFO=true
  echo "--- binutils installed ---"
fi

# ── Build GCC (C and C++ compilers) ─────────────────────────────────────────

echo ""
LIBGCC_A="$PREFIX/lib/gcc/$TARGET/${GCC_VERSION}/libgcc.a"
if [[ -x "$PREFIX/bin/${TARGET}-gcc" ]] && [[ -f "$LIBGCC_A" ]]; then
  echo "--- GCC already installed, skipping (use --clean to rebuild) ---"
else
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
    --with-system-zlib \
    $SYSROOT_FLAGS \
    $EXTRA_GCC_CONFIGURE \
    MAKEINFO=true

  # Build compiler first
  make -j"$JOBS" all-gcc MAKEINFO=true

  # Build libgcc with inhibit_libc to skip gcov (needs libc features we don't have)
  make -j"$JOBS" all-target-libgcc MAKEINFO=true CFLAGS_FOR_TARGET="-g -O2 -Dinhibit_libc"

  make install-gcc install-target-libgcc MAKEINFO=true
  echo "--- GCC installed ---"
fi

# ── Copy libgcc.a to project sysroot (if specified) ─────────────────────────

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

# ── Verify Stage 1 ──────────────────────────────────────────────────────────

echo ""
echo "========================================="
echo " Stage 1 complete (cross-compiler)!"
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

# =============================================================================
# STAGE 2: Canadian Cross — Build GCC to run ON anyOS
# =============================================================================
# This cross-compiles GCC itself so it produces ELF64 binaries that execute
# natively on anyOS.  Requires libc64 libraries to be already built.
#
# Build triplet:  x86_64-apple-darwin (macOS) or x86_64-pc-linux-gnu
# Host triplet:   x86_64-anyos        (the OS that will RUN the compiler)
# Target triplet: x86_64-anyos        (the OS the compiler produces code FOR)

NATIVE_PREFIX="/System/Toolchain"
NATIVE_INSTALL="$BUILD_DIR/native-toolchain"

# Check if libc64 libraries exist for Stage 2
LIBC64_LIB_DIR="$PROJECT_DIR/libs/libc64"
LIBCXX_LIB_DIR="$PROJECT_DIR/libs/libcxx"
LIBUNWIND_LIB_DIR="$PROJECT_DIR/libs/libunwind"
LIBCXXABI_LIB_DIR="$PROJECT_DIR/libs/libcxxabi"

if [[ ! -f "$LIBC64_LIB_DIR/libc64.a" ]]; then
  echo ""
  echo "========================================="
  echo " Stage 2 skipped — libc64.a not built"
  echo "========================================="
  echo ""
  echo "Build libc64 first (cmake --build build), then re-run this script."
  echo ""
  echo "Add to your shell profile:"
  echo "  export PATH=\"$PREFIX/bin:\$PATH\""
  exit 0
fi

echo ""
echo "========================================="
echo " Stage 2: Building GCC for native anyOS"
echo "========================================="
echo ""

# Create a sysroot that the cross-compiler will use to find headers and libs
# when building the native GCC.
CROSS_SYSROOT="$BUILD_DIR/cross-sysroot"
mkdir -p "$CROSS_SYSROOT/Libraries/libc64/include"
mkdir -p "$CROSS_SYSROOT/Libraries/libc64/lib"
mkdir -p "$CROSS_SYSROOT/Libraries/libcxx/include"
mkdir -p "$CROSS_SYSROOT/Libraries/libcxx/lib"
mkdir -p "$CROSS_SYSROOT/usr/include"
mkdir -p "$CROSS_SYSROOT/usr/lib"

# Copy libc64 headers and library
cp -r "$LIBC64_LIB_DIR/include/"* "$CROSS_SYSROOT/Libraries/libc64/include/"
cp "$LIBC64_LIB_DIR/libc64.a" "$CROSS_SYSROOT/Libraries/libc64/lib/"
cp "$LIBC64_LIB_DIR/obj/crt0.o" "$CROSS_SYSROOT/Libraries/libc64/lib/"
cp "$LIBC64_LIB_DIR/obj/crti.o" "$CROSS_SYSROOT/Libraries/libc64/lib/"
cp "$LIBC64_LIB_DIR/obj/crtn.o" "$CROSS_SYSROOT/Libraries/libc64/lib/"
[[ -f "$LIBGCC_A" ]] && cp "$LIBGCC_A" "$CROSS_SYSROOT/Libraries/libc64/lib/"
cp "$LIBC64_LIB_DIR/link.ld" "$CROSS_SYSROOT/Libraries/libc64/lib/"

# Copy libcxx/libunwind/libc++abi headers and libraries
if [[ -f "$LIBCXX_LIB_DIR/libcxx.a" ]]; then
  cp -r "$LIBCXX_LIB_DIR/include/"* "$CROSS_SYSROOT/Libraries/libcxx/include/"
  cp "$LIBCXX_LIB_DIR/libcxx.a" "$CROSS_SYSROOT/Libraries/libcxx/lib/"
fi
if [[ -f "$LIBUNWIND_LIB_DIR/libunwind.a" ]]; then
  cp "$LIBUNWIND_LIB_DIR/libunwind.a" "$CROSS_SYSROOT/Libraries/libcxx/lib/"
  cp "$LIBUNWIND_LIB_DIR/include/unwind.h" "$CROSS_SYSROOT/Libraries/libcxx/include/"
fi
if [[ -f "$LIBCXXABI_LIB_DIR/libc++abi.a" ]]; then
  cp "$LIBCXXABI_LIB_DIR/libc++abi.a" "$CROSS_SYSROOT/Libraries/libcxx/lib/"
  cp "$LIBCXXABI_LIB_DIR/include/cxxabi.h" "$CROSS_SYSROOT/Libraries/libcxx/include/"
fi

# Symlink headers into /usr/include (some GCC configure checks look here)
ln -sf "$CROSS_SYSROOT/Libraries/libc64/include/"* "$CROSS_SYSROOT/usr/include/" 2>/dev/null || true
ln -sf "$CROSS_SYSROOT/Libraries/libc64/lib/"* "$CROSS_SYSROOT/usr/lib/" 2>/dev/null || true

echo "  Cross-compilation sysroot prepared at $CROSS_SYSROOT"

# --- Build native binutils (runs ON anyOS) ---

echo ""
echo "--- Building native binutils (host=x86_64-anyos) ---"
rm -rf "$BUILD_DIR/native-binutils"
mkdir -p "$BUILD_DIR/native-binutils"
cd "$BUILD_DIR/native-binutils"

CC_FOR_HOST="$PREFIX/bin/${TARGET}-gcc"
CXX_FOR_HOST="$PREFIX/bin/${TARGET}-g++"
AR_FOR_HOST="$PREFIX/bin/${TARGET}-ar"
RANLIB_FOR_HOST="$PREFIX/bin/${TARGET}-ranlib"

# Configure cross-compiling flags for the host
HOST_CFLAGS="-O2 -ffreestanding -nostdinc -I$CROSS_SYSROOT/Libraries/libc64/include"
HOST_LDFLAGS="-nostdlib -static -L$CROSS_SYSROOT/Libraries/libc64/lib -T $CROSS_SYSROOT/Libraries/libc64/lib/link.ld $CROSS_SYSROOT/Libraries/libc64/lib/crt0.o $CROSS_SYSROOT/Libraries/libc64/lib/crti.o"
HOST_LIBS="-lc64 -lgcc $CROSS_SYSROOT/Libraries/libc64/lib/crtn.o"

"$BINUTILS_SRC/configure" \
  --host="$TARGET" \
  --target="$TARGET" \
  --prefix="$NATIVE_PREFIX" \
  --with-sysroot="$CROSS_SYSROOT" \
  --disable-nls \
  --disable-werror \
  CC="$CC_FOR_HOST" \
  CXX="$CXX_FOR_HOST" \
  AR="$AR_FOR_HOST" \
  RANLIB="$RANLIB_FOR_HOST" \
  CFLAGS="$HOST_CFLAGS" \
  LDFLAGS="$HOST_LDFLAGS" \
  LIBS="$HOST_LIBS"

make -j"$JOBS" || {
  echo ""
  echo "WARNING: Native binutils build failed (this may require additional libc64 stubs)."
  echo "Stage 1 cross-compiler is still functional."
  echo ""
}

if [[ -f "$BUILD_DIR/native-binutils/binutils/ar" ]]; then
  mkdir -p "$NATIVE_INSTALL/bin"
  for tool in as ld ar nm objdump objcopy ranlib strip; do
    src="$BUILD_DIR/native-binutils/binutils/$tool"
    [[ -f "$src" ]] || src="$BUILD_DIR/native-binutils/gas/as-new"
    [[ "$tool" == "as" ]] && src="$BUILD_DIR/native-binutils/gas/as-new"
    [[ "$tool" == "ld" ]] && src="$BUILD_DIR/native-binutils/ld/ld-new"
    if [[ -f "$src" ]]; then
      cp "$src" "$NATIVE_INSTALL/bin/$tool"
      echo "  installed native $tool"
    fi
  done
fi

# --- Build native GCC (runs ON anyOS) ---

echo ""
echo "--- Building native GCC (host=x86_64-anyos) ---"
rm -rf "$BUILD_DIR/native-gcc"
mkdir -p "$BUILD_DIR/native-gcc"
cd "$BUILD_DIR/native-gcc"

"$GCC_SRC/configure" \
  --host="$TARGET" \
  --target="$TARGET" \
  --prefix="$NATIVE_PREFIX" \
  --with-sysroot="$CROSS_SYSROOT" \
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
  --disable-bootstrap \
  --with-newlib \
  CC="$CC_FOR_HOST" \
  CXX="$CXX_FOR_HOST" \
  AR="$AR_FOR_HOST" \
  RANLIB="$RANLIB_FOR_HOST" \
  CC_FOR_TARGET="$CC_FOR_HOST" \
  CXX_FOR_TARGET="$CXX_FOR_HOST" \
  AR_FOR_TARGET="$AR_FOR_HOST" \
  RANLIB_FOR_TARGET="$RANLIB_FOR_HOST" \
  CFLAGS="$HOST_CFLAGS" \
  CXXFLAGS="$HOST_CFLAGS -I$CROSS_SYSROOT/Libraries/libcxx/include" \
  LDFLAGS="$HOST_LDFLAGS" \
  LIBS="$HOST_LIBS" \
  $EXTRA_GCC_CONFIGURE

make -j"$JOBS" all-gcc || {
  echo ""
  echo "WARNING: Native GCC build failed (this is expected for initial ports)."
  echo "Stage 1 cross-compiler is still functional."
  echo ""
}

# Install native GCC if build succeeded
if [[ -f "$BUILD_DIR/native-gcc/gcc/cc1" ]]; then
  mkdir -p "$NATIVE_INSTALL/bin"
  mkdir -p "$NATIVE_INSTALL/libexec/gcc/$TARGET/${GCC_VERSION}"
  for tool in gcc g++ cpp; do
    src="$BUILD_DIR/native-gcc/gcc/$tool"
    [[ "$tool" == "gcc" ]] && src="$BUILD_DIR/native-gcc/gcc/xgcc"
    [[ "$tool" == "g++" ]] && src="$BUILD_DIR/native-gcc/gcc/xg++"
    [[ "$tool" == "cpp" ]] && src="$BUILD_DIR/native-gcc/gcc/cpp"
    if [[ -f "$src" ]]; then
      cp "$src" "$NATIVE_INSTALL/bin/$tool"
      echo "  installed native $tool"
    fi
  done
  # cc1, cc1plus go into libexec
  for comp in cc1 cc1plus; do
    if [[ -f "$BUILD_DIR/native-gcc/gcc/$comp" ]]; then
      cp "$BUILD_DIR/native-gcc/gcc/$comp" "$NATIVE_INSTALL/libexec/gcc/$TARGET/${GCC_VERSION}/$comp"
      echo "  installed native $comp"
    fi
  done
fi

# ── Verify ───────────────────────────────────────────────────────────────────

echo ""
echo "========================================="
echo " Build complete!"
echo "========================================="
echo ""
echo "Stage 1 — Cross-compiler (runs on this machine):"
"$PREFIX/bin/${TARGET}-gcc" --version | head -1
echo ""

if [[ -d "$NATIVE_INSTALL/bin" ]] && [[ -n "$(ls "$NATIVE_INSTALL/bin" 2>/dev/null)" ]]; then
  echo "Stage 2 — Native compiler (runs ON anyOS):"
  ls -1 "$NATIVE_INSTALL/bin/" | while read f; do
    echo "  $f ($(wc -c < "$NATIVE_INSTALL/bin/$f") bytes)"
  done
  if [[ -d "$NATIVE_INSTALL/libexec/gcc" ]]; then
    echo ""
    echo "Native compiler components:"
    find "$NATIVE_INSTALL/libexec" -type f | while read f; do
      echo "  $(basename "$f") ($(wc -c < "$f") bytes)"
    done
  fi
  echo ""
  echo "To install to anyOS sysroot:"
  echo "  cp -r $NATIVE_INSTALL/* \$SYSROOT/System/Toolchain/"
else
  echo "Stage 2 — Native compiler: build failed or skipped."
  echo "  This is expected for initial ports. The cross-compiler is fully functional."
fi

echo ""
echo "Add to your shell profile:"
echo "  export PATH=\"$PREFIX/bin:\$PATH\""
echo ""
echo "Usage example (cross-compile from host):"
echo "  ${TARGET}-g++ -ffreestanding -nostdlib -O2 \\"
echo "    -I $PROJECT_DIR/libs/libcxx/include \\"
echo "    -I $PROJECT_DIR/libs/libc64/include \\"
echo "    main.cpp -o main.elf \\"
echo "    -L $PROJECT_DIR/libs/libc64 -L $PROJECT_DIR/libs/libcxx \\"
echo "    -lcxx -lc++abi -lunwind -lc64 -lgcc"
echo ""

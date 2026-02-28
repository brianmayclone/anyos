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
DO_STAGE2=false

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
    --stage2)  DO_STAGE2=true; shift ;;
    --help|-h)
      echo "Usage: $0 [--prefix DIR] [--sysroot DIR] [--jobs N]"
      echo "       $0 --clean [--all]"
      echo ""
      echo "Options:"
      echo "  --prefix DIR    Install cross-compiler to DIR (default: ~/opt/anyos-toolchain)"
      echo "  --sysroot DIR   Copy libgcc.a and CRT files to sysroot"
      echo "  --jobs N        Parallel build jobs (default: auto-detect)"
      echo "  --stage2        Also build native compiler (runs ON anyOS, requires libc64)"
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

# Download GCC prerequisites (GMP, MPFR, MPC) into the GCC source tree.
# This enables in-tree builds so Stage 2 can cross-compile them for anyOS.
if [[ ! -d "$GCC_SRC/gmp" ]]; then
  echo "--- Downloading GCC prerequisites (GMP, MPFR, MPC) ---"
  cd "$GCC_SRC"
  # Use GCC's built-in script if available, otherwise download manually
  if [[ -x contrib/download_prerequisites ]]; then
    contrib/download_prerequisites
  else
    GMP_VER="6.2.1"
    MPFR_VER="4.1.0"
    MPC_VER="1.2.1"
    for pkg in "gmp-${GMP_VER}" "mpfr-${MPFR_VER}" "mpc-${MPC_VER}"; do
      if [[ ! -d "${pkg}" ]]; then
        download_with_mirrors "${pkg}.tar.xz" \
          "https://ftpmirror.gnu.org/${pkg%%-*}/${pkg}.tar.xz" \
          "https://ftp.gnu.org/gnu/${pkg%%-*}/${pkg}.tar.xz"
        tar xf "${pkg}.tar.xz"
        rm -f "${pkg}.tar.xz"
      fi
    done
    ln -sf "gmp-${GMP_VER}" gmp
    ln -sf "mpfr-${MPFR_VER}" mpfr
    ln -sf "mpc-${MPC_VER}" mpc
  fi
  cd "$SRC_DIR"
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
  # Different config.sub versions use "OS" or "system" in their error message
  err_line=$(grep -n "Invalid configuration.*\(OS\|system\).*not recognized" "$file" | head -1 | cut -d: -f1 || true)
  if [[ -z "$err_line" ]]; then
    return 0
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

# Also patch sub-project config.sub files (and GMP's configfsf.sub wrapper)
for f in "$GCC_SRC"/*/config.sub "$GCC_SRC"/*/configfsf.sub; do
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
    --disable-gcov \
    --without-headers \
    --with-newlib \
    --with-system-zlib \
    $SYSROOT_FLAGS \
    $EXTRA_GCC_CONFIGURE \
    CFLAGS_FOR_TARGET="-g -O2 -Dinhibit_libc" \
    MAKEINFO=true

  # Build compiler first
  make -j"$JOBS" all-gcc MAKEINFO=true

  # Build libgcc (inhibit_libc set in configure to skip gcov/libc-dependent code)
  make -j"$JOBS" all-target-libgcc MAKEINFO=true

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

echo ""
echo "Add to your shell profile:"
echo "  export PATH=\"$PREFIX/bin:\$PATH\""

# =============================================================================
# STAGE 2: Canadian Cross — Build GCC to run ON anyOS
# =============================================================================
# This cross-compiles GCC itself so it produces ELF64 binaries that execute
# natively on anyOS.  Requires libc64 libraries to be already built.
#
# Build triplet:  x86_64-apple-darwin (macOS) or x86_64-pc-linux-gnu
# Host triplet:   x86_64-anyos        (the OS that will RUN the compiler)
# Target triplet: x86_64-anyos        (the OS the compiler produces code FOR)

if ! $DO_STAGE2; then
  echo ""
  echo "Stage 2 (native compiler for anyOS) skipped."
  echo "Use --stage2 to build a GCC that runs ON anyOS (requires libc64)."
  exit 0
fi

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
  echo " Stage 2 requires libc64.a"
  echo "========================================="
  echo ""
  echo "Build libc64 first (cmake --build build), then re-run with --stage2."
  exit 1
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

# Also install into the cross-compiler's standard search path so the linker
# finds libraries automatically (configure tests use the cross-compiler).
CROSS_LIB="$PREFIX/$TARGET/lib"
mkdir -p "$CROSS_LIB"
cp "$LIBC64_LIB_DIR/libc64.a" "$CROSS_LIB/"
"$PREFIX/bin/${TARGET}-ranlib" "$CROSS_LIB/libc64.a"
cp "$LIBC64_LIB_DIR/obj/crt0.o" "$CROSS_LIB/"
cp "$LIBC64_LIB_DIR/obj/crti.o" "$CROSS_LIB/"
cp "$LIBC64_LIB_DIR/obj/crtn.o" "$CROSS_LIB/"
[[ -f "$LIBGCC_A" ]] && cp "$LIBGCC_A" "$CROSS_LIB/"
cp "$LIBC64_LIB_DIR/link.ld" "$CROSS_LIB/"

# Install headers into cross-compiler search path first (needed for libstdc++ build below).
CROSS_INC="$PREFIX/$TARGET/include"
mkdir -p "$CROSS_INC"
cp -r "$LIBC64_LIB_DIR/include/"* "$CROSS_INC/"

# Create stub libraries that configure tests expect.
# libm.a: math functions live in libc64, empty archive satisfies -lm.
# libc.a: alias for libc64.a (some tools link -lc instead of -lc64).
# libstdc++.a: minimal C++ runtime (operator new/delete) — GCC itself is C++.
"$PREFIX/bin/${TARGET}-ar" rcs "$CROSS_LIB/libm.a"
ln -sf libc64.a "$CROSS_LIB/libc.a"

# Build a minimal libstdc++.a with operator new/delete implementations.
CXXSTUB_SRC="/tmp/anyos_cxxstub_$$.cc"
cat > "$CXXSTUB_SRC" << 'CXXEOF'
#include <stdlib.h>
#include <new>
namespace std { const nothrow_t nothrow{}; }
void* operator new(size_t sz)  { void* p = malloc(sz ? sz : 1); if (!p) abort(); return p; }
void* operator new[](size_t sz)  { return operator new(sz); }
void  operator delete(void* p) noexcept { free(p); }
void  operator delete[](void* p) noexcept { free(p); }
void  operator delete(void* p, size_t) noexcept { free(p); }
void  operator delete[](void* p, size_t) noexcept { free(p); }
void* operator new(size_t sz, const std::nothrow_t&) noexcept { return malloc(sz ? sz : 1); }
void* operator new[](size_t sz, const std::nothrow_t&) noexcept { return malloc(sz ? sz : 1); }
void  operator delete(void* p, const std::nothrow_t&) noexcept { free(p); }
void  operator delete[](void* p, const std::nothrow_t&) noexcept { free(p); }
/* Pure virtual handler */
extern "C" void __cxa_pure_virtual() { abort(); }
/* Guard acquire/release for thread-safe statics (single-threaded stub) */
extern "C" int  __cxa_guard_acquire(long long *g) { return !*(char*)g; }
extern "C" void __cxa_guard_release(long long *g) { *(char*)g = 1; }
extern "C" void __cxa_guard_abort(long long *) {}
/* atexit registration */
extern "C" int  __cxa_atexit(void (*)(void*), void*, void*) { return 0; }
CXXEOF
CXXSTUB_OBJ="/tmp/anyos_cxxstub_$$.o"
"$PREFIX/bin/${TARGET}-g++" -c -O2 -ffreestanding -isystem "$CROSS_INC" \
    "$CXXSTUB_SRC" -o "$CXXSTUB_OBJ"
"$PREFIX/bin/${TARGET}-ar" rcs "$CROSS_LIB/libstdc++.a" "$CXXSTUB_OBJ"
cp "$CROSS_LIB/libstdc++.a" "$CROSS_SYSROOT/Libraries/libc64/lib/"
rm -f "$CXXSTUB_SRC" "$CXXSTUB_OBJ"
echo "  built minimal libstdc++.a (operator new/delete + cxa stubs)"

"$PREFIX/bin/${TARGET}-ar" rcs "$CROSS_SYSROOT/Libraries/libc64/lib/libm.a"
ln -sf libc64.a "$CROSS_SYSROOT/Libraries/libc64/lib/libc.a"

# Symlink headers into /usr/include (some GCC configure checks look here)
ln -sf "$CROSS_SYSROOT/Libraries/libc64/include/"* "$CROSS_SYSROOT/usr/include/" 2>/dev/null || true
ln -sf "$CROSS_SYSROOT/Libraries/libc64/lib/"* "$CROSS_SYSROOT/usr/lib/" 2>/dev/null || true

echo "  Cross-compilation sysroot prepared at $CROSS_SYSROOT"
echo "  Libraries installed to $CROSS_LIB"

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

# Configure's test programs fail to link on our freestanding target, so it
# incorrectly reports almost all headers/functions as missing.  We use a
# CONFIG_SITE file to pre-seed autoconf's cache with correct values, so
# configure writes the right #defines into config.h.
ANYOS_CONFIG_SITE="$BUILD_DIR/anyos-config.site"
cat > "$ANYOS_CONFIG_SITE" << 'SITE_EOF'
# Autoconf cache seed for anyOS (libc64) cross-compilation.
# Configure tests fail to link because we're cross-compiling to a freestanding
# target.  These values reflect what libc64 actually provides.

# Header availability
ac_cv_header_limits_h=yes
ac_cv_header_string_h=yes
ac_cv_header_strings_h=yes
ac_cv_header_stdint_h=yes
ac_cv_header_stdlib_h=yes
ac_cv_header_stdio_h=yes
ac_cv_header_errno_h=yes
ac_cv_header_memory_h=yes
ac_cv_header_unistd_h=yes
ac_cv_header_fcntl_h=yes
ac_cv_header_inttypes_h=yes
ac_cv_header_alloca_h=yes
ac_cv_header_sys_types_h=yes
ac_cv_header_sys_stat_h=yes
ac_cv_header_sys_param_h=yes
ac_cv_header_sys_time_h=yes
ac_cv_header_time_h=yes
ac_cv_header_signal_h=yes
ac_cv_header_math_h=yes
ac_cv_header_ctype_h=yes
ac_cv_header_locale_h=yes
ac_cv_header_dirent_h=yes
ac_cv_header_setjmp_h=yes
ac_cv_header_stdc=yes

# Function availability
ac_cv_func_memcpy=yes
ac_cv_func_memset=yes
ac_cv_func_memmove=yes
ac_cv_func_memcmp=yes
ac_cv_func_memchr=yes
ac_cv_func_strchr=yes
ac_cv_func_strrchr=yes
ac_cv_func_strdup=yes
ac_cv_func_strtol=yes
ac_cv_func_strtoul=yes
ac_cv_func_strtoll=yes
ac_cv_func_strtoull=yes
ac_cv_func_strtod=yes
ac_cv_func_malloc=yes
ac_cv_func_realloc=yes
ac_cv_func_calloc=yes
ac_cv_func_free=yes
ac_cv_func_atexit=yes
ac_cv_func_getenv=yes
ac_cv_func_putenv=yes
ac_cv_func_setenv=yes
ac_cv_func_qsort=yes
ac_cv_func_bsearch=yes
ac_cv_func_strerror=yes
ac_cv_func_strsignal=yes
ac_cv_func_strstr=yes
ac_cv_func_stpcpy=yes
ac_cv_func_stpncpy=yes
ac_cv_func_strcasecmp=yes
ac_cv_func_strncasecmp=yes
ac_cv_func_strndup=yes
ac_cv_func_strnlen=yes
ac_cv_func_memchr=yes
ac_cv_func_mempcpy=yes
ac_cv_func_snprintf=yes
ac_cv_func_vsnprintf=yes
ac_cv_func_vfprintf=yes
ac_cv_func_vsprintf=yes
ac_cv_func_printf=yes
ac_cv_func_fprintf=yes
ac_cv_func_sprintf=yes
ac_cv_func_abort=yes
ac_cv_func_exit=yes
ac_cv_func_open=yes
ac_cv_func_close=yes
ac_cv_func_read=yes
ac_cv_func_write=yes
ac_cv_func_lseek=yes
ac_cv_func_stat=yes
ac_cv_func_fstat=yes
ac_cv_func_access=yes
ac_cv_func_getcwd=yes
ac_cv_func_getpid=yes
ac_cv_func_kill=yes
ac_cv_func_raise=yes
ac_cv_func_signal=yes
ac_cv_func_clock=yes
ac_cv_func_time=yes
ac_cv_func_mkstemp=yes
ac_cv_func_mmap=yes
ac_cv_func_munmap=yes
ac_cv_func_sbrk=yes
ac_cv_func_realpath=yes
ac_cv_func_rename=yes
ac_cv_func_unlink=yes
ac_cv_func_mkdir=yes
ac_cv_func_rmdir=yes
ac_cv_func_fork=yes
ac_cv_func_execve=yes
ac_cv_func_execvp=yes
ac_cv_func_wait=yes
ac_cv_func_waitpid=yes
ac_cv_func_pipe=yes
ac_cv_func_dup=yes
ac_cv_func_dup2=yes
ac_cv_func_fcntl=yes
ac_cv_func_select=yes
ac_cv_func_poll=yes
ac_cv_func_nanosleep=yes
ac_cv_func_sysconf=yes
ac_cv_func_strtok_r=yes
ac_cv_func_times=yes
ac_cv_func_gettimeofday=yes
ac_cv_func_strftime=yes
ac_cv_func_strcoll=yes
ac_cv_func_strxfrm=yes
ac_cv_func_fnmatch=yes
ac_cv_func_getopt=yes
ac_cv_func_getopt_long=yes
ac_cv_header_fnmatch_h=yes
ac_cv_header_getopt_h=yes

# Unlocked I/O — available as inline wrappers in stdio.h
ac_cv_func_putc_unlocked=yes
ac_cv_func_getc_unlocked=yes
# Functions NOT available in libc64 (prevent false positives from cross-compile tests)
ac_cv_func_fputc_unlocked=no
ac_cv_func_fwrite_unlocked=no
ac_cv_func_fprintf_unlocked=no
ac_cv_func_fputs_unlocked=no

# Declaration tests (ac_cv_have_decl_*)
ac_cv_have_decl_calloc=yes
ac_cv_have_decl_malloc=yes
ac_cv_have_decl_realloc=yes
ac_cv_have_decl_free=yes
ac_cv_have_decl_getenv=yes
ac_cv_have_decl_abort=yes
ac_cv_have_decl_strtol=yes
ac_cv_have_decl_strtoul=yes
ac_cv_have_decl_strtoll=yes
ac_cv_have_decl_strtoull=yes
ac_cv_have_decl_basename=no
ac_cv_have_decl_basename_char_p_=no
ac_cv_have_decl_ffs=no
ac_cv_have_decl_getopt=yes
ac_cv_have_decl_sbrk=yes
ac_cv_have_decl_strnlen=yes
ac_cv_have_decl_asprintf=no
ac_cv_have_decl_vasprintf=no
ac_cv_have_decl_strverscmp=no
ac_cv_have_decl_snprintf=yes
ac_cv_have_decl_vsnprintf=yes

# Endianness (x86_64 is little-endian)
ac_cv_c_bigendian=no

# Type sizes (x86_64)
ac_cv_sizeof_int=4
ac_cv_sizeof_long=8
ac_cv_sizeof_long_long=8
ac_cv_sizeof_void_p=8
ac_cv_sizeof_short=2
ac_cv_sizeof_char=1
ac_cv_sizeof_size_t=8
ac_cv_sizeof_off_t=8
ac_cv_type_signal=void
SITE_EOF
echo "  created CONFIG_SITE at $ANYOS_CONFIG_SITE"

# With CONFIG_SITE properly setting HAVE_* macros, the conditional includes in
# source files (#ifdef HAVE_LIMITS_H / #include <limits.h>) work correctly.
# We do NOT use -include flags here because they conflict with libiberty's own
# replacement function signatures.
HOST_CFLAGS="-O2 -ffreestanding -isystem $CROSS_INC"
HOST_CXXFLAGS="$HOST_CFLAGS -fpermissive"
HOST_LDFLAGS="-L$CROSS_LIB"
export CONFIG_SITE="$ANYOS_CONFIG_SITE"

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
  CXXFLAGS="$HOST_CXXFLAGS" \
  LDFLAGS="$HOST_LDFLAGS" \
  CFLAGS_FOR_BUILD="-O2" \
  CXXFLAGS_FOR_BUILD="-O2" \
  LDFLAGS_FOR_BUILD="" \
  MAKEINFO=true

# Build only the binary tools (skip doc generation which requires makeinfo).
make -j"$JOBS" MAKEINFO=true all-binutils all-gas all-ld 2>&1 || {
  echo ""
  echo "WARNING: Native binutils build had errors (some tools may still be usable)."
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

# Use in-tree GMP/MPFR/MPC (don't use Homebrew paths — those are host-only)
"$GCC_SRC/configure" \
  --host="$TARGET" \
  --target="$TARGET" \
  --prefix="$NATIVE_PREFIX" \
  --with-sysroot="$CROSS_SYSROOT" \
  --enable-languages=c \
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
  --disable-gcov \
  --disable-isl \
  --disable-libcc1 \
  --with-newlib \
  CC="$CC_FOR_HOST" \
  CXX="$CXX_FOR_HOST" \
  AR="$AR_FOR_HOST" \
  RANLIB="$RANLIB_FOR_HOST" \
  CC_FOR_TARGET="$CC_FOR_HOST" \
  AR_FOR_TARGET="$AR_FOR_HOST" \
  RANLIB_FOR_TARGET="$RANLIB_FOR_HOST" \
  CFLAGS="$HOST_CFLAGS" \
  CXXFLAGS="$HOST_CXXFLAGS" \
  LDFLAGS="$HOST_LDFLAGS" \
  CFLAGS_FOR_BUILD="-O2" \
  CXXFLAGS_FOR_BUILD="-O2" \
  LDFLAGS_FOR_BUILD="" \
  CFLAGS_FOR_TARGET="$HOST_CFLAGS" \
  MAKEINFO=true

# libcody is a C++20 module mapper library — it requires C++ standard library
# headers (<memory>, <string>, etc.) which don't exist on anyOS yet.
# Stub it out with a no-op Makefile so all-gcc can proceed without it.
mkdir -p "$BUILD_DIR/native-gcc/libcody"
cat > "$BUILD_DIR/native-gcc/libcody/Makefile" << 'LIBCODY_EOF'
all:
	@true
install:
	@true
clean:
	@true
LIBCODY_EOF
# Create empty libcody.a so the linker doesn't complain
"$AR_FOR_HOST" rcs "$BUILD_DIR/native-gcc/libcody/libcody.a"

# Force BUILD_CPPLIB to use the build-side library (not host-side).
# In Canadian Cross builds, GCC's Makefile defaults BUILD_CPPLIB to the HOST
# libcpp which is cross-compiled for anyOS — causing linker errors when linking
# build tools (e.g., genmatch) that must run on macOS.
BUILD_LIBCPP_PATH="$BUILD_DIR/native-gcc/build-$(gcc -dumpmachine)/libcpp/libcpp.a"
BUILD_LIBIBERTY_PATH="$BUILD_DIR/native-gcc/build-$(gcc -dumpmachine)/libiberty/libiberty.a"

make -j"$JOBS" all-gcc MAKEINFO=true \
  BUILD_CPPLIB="$BUILD_LIBCPP_PATH $BUILD_LIBIBERTY_PATH" || {
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

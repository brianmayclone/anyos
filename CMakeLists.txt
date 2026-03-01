cmake_minimum_required(VERSION 3.20)
project(anyOS NONE)

# ============================================================
# Configuration
# ============================================================
set(NASM_EXECUTABLE "nasm")
set(QEMU_EXECUTABLE "qemu-system-x86_64")
find_program(CARGO_EXECUTABLE cargo
    HINTS "$ENV{USERPROFILE}/.cargo/bin" "$ENV{HOME}/.cargo/bin")
if(NOT CARGO_EXECUTABLE)
    message(FATAL_ERROR "cargo not found in PATH. Install Rust from https://rustup.rs/")
endif()

# find make without putting MSYS2/usr/bin in the Windows PATH
# (that directory contains a GNU coreutils link.exe that conflicts with MSVC link.exe)
find_program(MAKE_EXECUTABLE NAMES make
    HINTS "C:/msys64/usr/bin" "/usr/bin" "/usr/local/bin")
if(NOT MAKE_EXECUTABLE)
    message(STATUS "make not found — C library and game builds will be skipped")
endif()

# find Python — on Windows the executable is "python", on Unix "python3"
# (kept as fallback; native C tools are preferred)
find_program(PYTHON_EXECUTABLE NAMES python3 python)

option(ANYOS_DEBUG_VERBOSE "Enable verbose debug output" OFF)
option(ANYOS_DEBUG_SURF "Enable Surf browser debug logging" OFF)
option(ANYOS_NO_CROSS "Disable cross-compilation (skip libc, TCC, games, curl)" OFF)
option(ANYOS_RESET "Force fresh disk image rebuild (destroy runtime data)" OFF)
set(ANYOS_ARCH "x86_64" CACHE STRING "Target architecture (x86_64 or arm64)")

# Version — passed from build.sh/build.ps1 via -DANYOS_VERSION or read from VERSION file
if(NOT ANYOS_VERSION)
  file(READ "${CMAKE_SOURCE_DIR}/VERSION" ANYOS_VERSION)
  string(STRIP "${ANYOS_VERSION}" ANYOS_VERSION)
endif()
message(STATUS "anyOS version: ${ANYOS_VERSION}")

# Auto-detect i686-elf cross-compiler (needed for libc, TCC, Doom, Quake)
if(ANYOS_NO_CROSS)
  set(HAS_CROSS_COMPILER FALSE)
  message(STATUS "Cross-compilation disabled (ANYOS_NO_CROSS=ON) — skipping C library, TCC, and games")
else()
  find_program(I686_ELF_GCC NAMES i686-elf-gcc i386-elf-gcc)
  if(I686_ELF_GCC)
    set(HAS_CROSS_COMPILER TRUE)
    message(STATUS "Found cross-compiler: ${I686_ELF_GCC}")
  else()
    set(HAS_CROSS_COMPILER FALSE)
    message(STATUS "i686-elf-gcc not found — skipping C library, TCC, and games (Rust-only build)")
  endif()
endif()

set(DISK_IMAGE "${CMAKE_BINARY_DIR}/anyos.img")
if(ANYOS_ARCH STREQUAL "arm64")
  set(KERNEL_ELF "${CMAKE_BINARY_DIR}/kernel/aarch64-anyos/release/anyos_kernel.elf")
  message(STATUS "Target architecture: ARM64 (AArch64)")
else()
  set(KERNEL_ELF "${CMAKE_BINARY_DIR}/kernel/x86_64-anyos/release/anyos_kernel.elf")
  message(STATUS "Target architecture: x86_64")
endif()

# Detect CPU count for parallel sub-builds (make -j)
include(ProcessorCount)
ProcessorCount(NPROC)
if(NOT NPROC OR NPROC EQUAL 0)
  set(NPROC 4)
endif()
message(STATUS "Parallel sub-builds: -j${NPROC}")

# ============================================================
# Module includes (order matters: each may depend on the prior)
# ============================================================
include(${CMAKE_CURRENT_SOURCE_DIR}/cmake/BuildTools.cmake)
include(${CMAKE_CURRENT_SOURCE_DIR}/cmake/KernelBoot.cmake)
include(${CMAKE_CURRENT_SOURCE_DIR}/cmake/UserPrograms.cmake)
include(${CMAKE_CURRENT_SOURCE_DIR}/cmake/Toolchains.cmake)
include(${CMAKE_CURRENT_SOURCE_DIR}/cmake/Targets.cmake)

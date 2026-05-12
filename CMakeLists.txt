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
option(ANYOS_RESET "Force fresh disk image rebuild (destroy runtime data)" OFF)
set(ANYOS_ARCH "x86_64" CACHE STRING "Target architecture (x86_64 or arm64)")
set(ANYOS_IMAGE_SIZE_MIB "4096" CACHE STRING "Disk image size in MiB")
if(NOT ANYOS_IMAGE_SIZE_MIB MATCHES "^[1-9][0-9]*$")
  message(FATAL_ERROR
    "ANYOS_IMAGE_SIZE_MIB must be a positive integer MiB value, got '${ANYOS_IMAGE_SIZE_MIB}'")
endif()
message(STATUS "Disk image size: ${ANYOS_IMAGE_SIZE_MIB} MiB")

set(ANYOS_BOOT_MODE "uefi" CACHE STRING "Default x86_64 boot mode: uefi | bios")
set_property(CACHE ANYOS_BOOT_MODE PROPERTY STRINGS uefi bios)
if(NOT ANYOS_BOOT_MODE MATCHES "^(uefi|bios)$")
  message(FATAL_ERROR
    "ANYOS_BOOT_MODE must be 'uefi' or 'bios', got '${ANYOS_BOOT_MODE}'")
endif()
message(STATUS "Default boot mode: ${ANYOS_BOOT_MODE}")

# System-filesystem selection. `exfat` is the system filesystem for now.
# CoreFS remains buildable as an explicit experimental option, but it is not
# used for the default system/root partition.
set(ANYOS_SYSTEM_FS "exfat" CACHE STRING "System filesystem: exfat")
set_property(CACHE ANYOS_SYSTEM_FS PROPERTY STRINGS exfat)
set(ANYOS_SYSTEM_FS_SIZE_MIB "128" CACHE STRING
    "Size of the CoreFS system partition in MiB (only used when ANYOS_SYSTEM_FS=corefs)")
if(ANYOS_SYSTEM_FS STREQUAL "corefs")
  message(FATAL_ERROR
    "CoreFS as system filesystem is temporarily disabled; use ANYOS_SYSTEM_FS=exfat")
endif()
if(NOT ANYOS_SYSTEM_FS STREQUAL "exfat")
  message(FATAL_ERROR
    "ANYOS_SYSTEM_FS must be 'exfat', got '${ANYOS_SYSTEM_FS}'")
endif()
message(STATUS "System filesystem: exfat")

# Dual-partition image layout (Phase 5):
#   Partition 1 (exFAT /boot, small)     — kernel + boot assets
#   Partition 2 (optional)               — /System, /Applications, /Users, ...
# When OFF, the C builder keeps the legacy single-partition exFAT layout.
option(ANYOS_DUAL_PARTITION "Create a dual-partition disk image (/boot + /)" OFF)
set(ANYOS_BOOT_PARTITION_SIZE_MIB "32" CACHE STRING
    "Size of the /boot partition in MiB (only used when ANYOS_DUAL_PARTITION=ON)")
if(ANYOS_DUAL_PARTITION)
  message(STATUS "Image layout: dual-partition (/boot ${ANYOS_BOOT_PARTITION_SIZE_MIB} MiB + /)")
else()
  message(STATUS "Image layout: single-partition (legacy)")
endif()

# Version — passed from build.sh/build.ps1 via -DANYOS_VERSION or read from VERSION file
if(NOT ANYOS_VERSION)
  file(READ "${CMAKE_SOURCE_DIR}/VERSION" ANYOS_VERSION)
  string(STRIP "${ANYOS_VERSION}" ANYOS_VERSION)
endif()
message(STATUS "anyOS version: ${ANYOS_VERSION}")

# 32-bit cross-compiler (i686-elf-gcc) is no longer used. The 32-bit
# C world (libc, TCC, NASM, make, dash, ssh suite, curl, libgit2,
# ClassiCube) was removed; all user-space programs are now 64-bit
# (Rust workspace + libc64/clang for the small remaining C/C++ bits).
set(HAS_CROSS_COMPILER FALSE)

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

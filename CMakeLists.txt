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

# ============================================================
# Build System Tools (anyelf, mkimage — built first, used later)
# ============================================================
# These are native C tools that replace the Python scripts.
# They are compiled at configure/build time for the host, and also
# copied to the sysroot for self-hosting on anyOS.

set(BUILDSYSTEM_DIR "${CMAKE_SOURCE_DIR}/buildsystem")

# anyelf — ELF conversion tool (replaces elf2bin.py, elf2kdrv.py)
set(ANYELF_EXECUTABLE "${CMAKE_BINARY_DIR}/buildsystem/anyelf${CMAKE_EXECUTABLE_SUFFIX}")
file(GLOB ANYELF_SRCS "${BUILDSYSTEM_DIR}/anyelf/src/*.c" "${BUILDSYSTEM_DIR}/anyelf/src/*.h")
add_custom_command(
  OUTPUT ${ANYELF_EXECUTABLE}
  COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/buildsystem"
  COMMAND cc -Wall -O2 -std=c99 -o ${ANYELF_EXECUTABLE}
    ${BUILDSYSTEM_DIR}/anyelf/src/anyelf.c
    ${BUILDSYSTEM_DIR}/anyelf/src/convert.c
  DEPENDS ${ANYELF_SRCS}
  COMMENT "Building buildsystem tool: anyelf"
)

# mkimage — disk image builder (replaces mkimage.py)
set(MKIMAGE_EXECUTABLE "${CMAKE_BINARY_DIR}/buildsystem/mkimage${CMAKE_EXECUTABLE_SUFFIX}")
file(GLOB MKIMAGE_SRCS "${BUILDSYSTEM_DIR}/mkimage/src/*.c" "${BUILDSYSTEM_DIR}/mkimage/src/*.h")
add_custom_command(
  OUTPUT ${MKIMAGE_EXECUTABLE}
  COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/buildsystem"
  COMMAND cc -Wall -O2 -std=c99 ${POSIX_FLAG} -o ${MKIMAGE_EXECUTABLE}
    ${BUILDSYSTEM_DIR}/mkimage/src/mkimage.c
    ${BUILDSYSTEM_DIR}/mkimage/src/elf.c
    ${BUILDSYSTEM_DIR}/mkimage/src/fat16.c
    ${BUILDSYSTEM_DIR}/mkimage/src/exfat.c
    ${BUILDSYSTEM_DIR}/mkimage/src/gpt.c
    ${BUILDSYSTEM_DIR}/mkimage/src/iso9660.c
  DEPENDS ${MKIMAGE_SRCS}
  COMMENT "Building buildsystem tool: mkimage"
)

# On Linux, -std=c99 hides POSIX functions like strdup(); macOS exposes them by default.
if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
  set(POSIX_FLAG "-D_POSIX_C_SOURCE=200809L")
else()
  set(POSIX_FLAG "")
endif()

# anyld — ELF64 shared object linker (for future .so builds)
set(ANYLD_EXECUTABLE "${CMAKE_BINARY_DIR}/buildsystem/anyld${CMAKE_EXECUTABLE_SUFFIX}")
file(GLOB ANYLD_SRCS "${BUILDSYSTEM_DIR}/anyld/src/*.c" "${BUILDSYSTEM_DIR}/anyld/src/*.h")
add_custom_command(
  OUTPUT ${ANYLD_EXECUTABLE}
  COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/buildsystem"
  COMMAND cc -Wall -O2 -std=c99 ${POSIX_FLAG} -o ${ANYLD_EXECUTABLE}
    ${BUILDSYSTEM_DIR}/anyld/src/anyld.c
    ${BUILDSYSTEM_DIR}/anyld/src/input.c
    ${BUILDSYSTEM_DIR}/anyld/src/link.c
    ${BUILDSYSTEM_DIR}/anyld/src/output.c
    ${BUILDSYSTEM_DIR}/anyld/src/defs.c
  DEPENDS ${ANYLD_SRCS}
  COMMENT "Building buildsystem tool: anyld"
)

# mkappbundle — .app bundle creator (validates & assembles .app directories)
set(MKAPPBUNDLE_EXECUTABLE "${CMAKE_BINARY_DIR}/buildsystem/mkappbundle${CMAKE_EXECUTABLE_SUFFIX}")
file(GLOB MKAPPBUNDLE_SRCS "${BUILDSYSTEM_DIR}/mkappbundle/src/*.c" "${BUILDSYSTEM_DIR}/mkappbundle/src/*.h")
add_custom_command(
  OUTPUT ${MKAPPBUNDLE_EXECUTABLE}
  COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/buildsystem"
  COMMAND cc -Wall -O2 -std=c99 -o ${MKAPPBUNDLE_EXECUTABLE}
    ${BUILDSYSTEM_DIR}/mkappbundle/src/mkappbundle.c
  DEPENDS ${MKAPPBUNDLE_SRCS}
  COMMENT "Building buildsystem tool: mkappbundle"
)

# Target to build all buildsystem tools
add_custom_target(buildsystem-tools
  DEPENDS ${ANYELF_EXECUTABLE} ${MKIMAGE_EXECUTABLE} ${ANYLD_EXECUTABLE} ${MKAPPBUNDLE_EXECUTABLE}
)

option(ANYOS_DEBUG_VERBOSE "Enable verbose debug output" OFF)
option(ANYOS_NO_CROSS "Disable cross-compilation (skip libc, TCC, games, curl)" OFF)
option(ANYOS_RESET "Force fresh disk image rebuild (destroy runtime data)" OFF)

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
set(KERNEL_ELF "${CMAKE_BINARY_DIR}/kernel/x86_64-anyos/release/anyos_kernel.elf")

# ============================================================
# 1. Bootloader Stage 1 (MBR)
# ============================================================
add_custom_command(
  OUTPUT ${CMAKE_BINARY_DIR}/stage1.bin
  COMMAND ${NASM_EXECUTABLE} -f bin
    -o ${CMAKE_BINARY_DIR}/stage1.bin
    ${CMAKE_SOURCE_DIR}/bootloader/stage1/boot.asm
  DEPENDS ${CMAKE_SOURCE_DIR}/bootloader/stage1/boot.asm
  COMMENT "Assembling Stage 1 bootloader (MBR)"
)

# ============================================================
# 2. Bootloader Stage 2
# ============================================================
set(STAGE2_SOURCES
  ${CMAKE_SOURCE_DIR}/bootloader/stage2/stage2.asm
)

add_custom_command(
  OUTPUT ${CMAKE_BINARY_DIR}/stage2.bin
  COMMAND ${NASM_EXECUTABLE} -f bin
    -I ${CMAKE_SOURCE_DIR}/bootloader/stage2/
    -o ${CMAKE_BINARY_DIR}/stage2.bin
    ${CMAKE_SOURCE_DIR}/bootloader/stage2/stage2.asm
  DEPENDS
    ${CMAKE_SOURCE_DIR}/bootloader/stage2/stage2.asm
    ${CMAKE_SOURCE_DIR}/bootloader/stage2/a20.asm
    ${CMAKE_SOURCE_DIR}/bootloader/stage2/memory_map.asm
    ${CMAKE_SOURCE_DIR}/bootloader/stage2/disk.asm
    ${CMAKE_SOURCE_DIR}/bootloader/stage2/protected_mode.asm
  COMMENT "Assembling Stage 2 bootloader"
)

# ============================================================
# 3. Kernel ASM stubs
# ============================================================
set(KERNEL_ASM_SOURCES
  ${CMAKE_SOURCE_DIR}/kernel/asm/boot.asm
  ${CMAKE_SOURCE_DIR}/kernel/asm/interrupts.asm
  ${CMAKE_SOURCE_DIR}/kernel/asm/context_switch.asm
  ${CMAKE_SOURCE_DIR}/kernel/asm/syscall_entry.asm
  ${CMAKE_SOURCE_DIR}/kernel/asm/syscall_fast.asm
)

set(KERNEL_ASM_OBJECTS "")
foreach(ASM_SRC ${KERNEL_ASM_SOURCES})
  get_filename_component(ASM_NAME ${ASM_SRC} NAME_WE)
  set(ASM_OBJ ${CMAKE_BINARY_DIR}/kernel_asm_${ASM_NAME}.o)
  add_custom_command(
    OUTPUT ${ASM_OBJ}
    COMMAND ${NASM_EXECUTABLE} -f elf64
      -o ${ASM_OBJ}
      ${ASM_SRC}
    DEPENDS ${ASM_SRC}
    COMMENT "Assembling kernel/${ASM_NAME}.asm"
  )
  list(APPEND KERNEL_ASM_OBJECTS ${ASM_OBJ})
endforeach()

# Build comma-separated list for passing to Cargo (avoid CMake ';' and CMD '|' pipe)
string(REPLACE ";" "," KERNEL_ASM_OBJECTS_STR "${KERNEL_ASM_OBJECTS}")

# AP trampoline — assembled as flat binary (not ELF), included via include_bytes!
set(AP_TRAMPOLINE_BIN ${CMAKE_BINARY_DIR}/ap_trampoline.bin)
add_custom_command(
  OUTPUT ${AP_TRAMPOLINE_BIN}
  COMMAND ${NASM_EXECUTABLE} -f bin
    -o ${AP_TRAMPOLINE_BIN}
    ${CMAKE_SOURCE_DIR}/kernel/asm/ap_trampoline.asm
  DEPENDS ${CMAKE_SOURCE_DIR}/kernel/asm/ap_trampoline.asm
  COMMENT "Assembling AP trampoline (flat binary)"
)

# ============================================================
# 4. Kernel (Cargo build)
# ============================================================
set(CARGO_FEATURES_ARG "")
if(ANYOS_DEBUG_VERBOSE)
  set(CARGO_FEATURES_ARG "--features;debug_verbose")
endif()

# Collect all kernel .rs source files so CMake re-invokes Cargo when any change.
# CONFIGURE_DEPENDS makes CMake re-glob at build time if files are added/removed.
file(GLOB_RECURSE KERNEL_RS_SOURCES CONFIGURE_DEPENDS
  "${CMAKE_SOURCE_DIR}/kernel/src/*.rs"
)

add_custom_command(
  OUTPUT ${KERNEL_ELF}
  COMMAND ${CMAKE_COMMAND} -E env
    "ANYOS_ASM_OBJECTS=${KERNEL_ASM_OBJECTS_STR}"
    "ANYOS_AP_TRAMPOLINE=${AP_TRAMPOLINE_BIN}"
    "RUSTFLAGS=-C force-frame-pointers=yes"
    ${CARGO_EXECUTABLE} build --release
    --manifest-path ${CMAKE_SOURCE_DIR}/kernel/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/x86_64-anyos.json
    --target-dir ${CMAKE_BINARY_DIR}/kernel
    ${CARGO_FEATURES_ARG}
  DEPENDS
    ${KERNEL_ASM_OBJECTS}
    ${AP_TRAMPOLINE_BIN}
    ${CMAKE_SOURCE_DIR}/kernel/Cargo.toml
    ${CMAKE_SOURCE_DIR}/kernel/build.rs
    ${CMAKE_SOURCE_DIR}/kernel/link.ld
    ${CMAKE_SOURCE_DIR}/x86_64-anyos.json
    ${KERNEL_RS_SOURCES}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building kernel with Cargo"
)

# ============================================================
# 5. User Programs (flat binaries)
# ============================================================
set(SYSROOT_DIR "${CMAKE_BINARY_DIR}/sysroot")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/bin")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/sbin")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/users")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Users")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/system/tests")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libc/include")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libc/include/sys")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libc/include/netinet")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libc/include/arpa")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libc/lib")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Applications")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/src")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/Drivers")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/Drivers/gpu")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/Drivers/storage")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/Drivers/network")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/Drivers/input")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/Drivers/audio")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/Drivers/bus")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/Drivers/system")

# Copy buildsystem tool sources to sysroot for self-hosting on anyOS
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/system/buildsystem/.stamp
  COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/Libraries/system/buildsystem
  COMMAND ${CMAKE_COMMAND} -E copy_directory
    ${BUILDSYSTEM_DIR}/anyelf ${SYSROOT_DIR}/Libraries/system/buildsystem/anyelf
  COMMAND ${CMAKE_COMMAND} -E copy_directory
    ${BUILDSYSTEM_DIR}/mkimage ${SYSROOT_DIR}/Libraries/system/buildsystem/mkimage
  COMMAND ${CMAKE_COMMAND} -E copy_directory
    ${BUILDSYSTEM_DIR}/anyld ${SYSROOT_DIR}/Libraries/system/buildsystem/anyld
  COMMAND ${CMAKE_COMMAND} -E touch ${SYSROOT_DIR}/Libraries/system/buildsystem/.stamp
  DEPENDS ${ANYELF_SRCS} ${MKIMAGE_SRCS} ${ANYLD_SRCS}
  COMMENT "Installing buildsystem tool sources to sysroot"
)

# --- Helper: build a Rust user program ---
set(STDLIB_DEPS
  ${CMAKE_SOURCE_DIR}/libs/stdlib/Cargo.toml
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/lib.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/raw.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/dll.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/process.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/fs.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/sys.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/net.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/ipc.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/io.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/heap.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/anim.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/icons.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/ui/mod.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/src/ui/window.rs
  ${CMAKE_SOURCE_DIR}/libs/stdlib/link.ld
  ${CMAKE_SOURCE_DIR}/libs/libheap/Cargo.toml
  ${CMAKE_SOURCE_DIR}/libs/libheap/src/lib.rs
  ${CMAKE_SOURCE_DIR}/x86_64-anyos.json
)

function(add_rust_user_program NAME SRC_DIR)
  set(ELF "${CMAKE_BINARY_DIR}/programs/${NAME}/x86_64-anyos-user/release/${NAME}.elf")
  file(GLOB_RECURSE _PROG_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${ELF}
    COMMAND ${CARGO_EXECUTABLE} build --release
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
      --target-dir ${CMAKE_BINARY_DIR}/programs/${NAME}
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${SRC_DIR}/build.rs
      ${_PROG_RS}
      ${STDLIB_DEPS}
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building Rust user program: ${NAME}"
  )
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/System/bin/${NAME}
    COMMAND ${ANYELF_EXECUTABLE} bin
      ${ELF}
      ${SYSROOT_DIR}/System/bin/${NAME}
    DEPENDS ${ELF} ${ANYELF_EXECUTABLE}
    COMMENT "Converting ${NAME} ELF to flat binary"
  )
  set(RUST_USER_BINS ${RUST_USER_BINS} ${SYSROOT_DIR}/System/bin/${NAME} PARENT_SCOPE)
endfunction()

# Variant for system programs (placed in /System/ instead of /bin/)
function(add_rust_system_program NAME SRC_DIR)
  set(ELF "${CMAKE_BINARY_DIR}/programs/${NAME}/x86_64-anyos-user/release/${NAME}.elf")
  file(GLOB_RECURSE _PROG_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${ELF}
    COMMAND ${CARGO_EXECUTABLE} build --release
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
      --target-dir ${CMAKE_BINARY_DIR}/programs/${NAME}
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${SRC_DIR}/build.rs
      ${_PROG_RS}
      ${STDLIB_DEPS}
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building system program: ${NAME}"
  )
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/System/${NAME}
    COMMAND ${ANYELF_EXECUTABLE} bin
      ${ELF}
      ${SYSROOT_DIR}/System/${NAME}
    DEPENDS ${ELF} ${ANYELF_EXECUTABLE}
    COMMENT "Converting ${NAME} ELF to flat binary"
  )
  set(SYSTEM_BINS ${SYSTEM_BINS} ${SYSROOT_DIR}/System/${NAME} PARENT_SCOPE)
endfunction()

# Variant for privileged sbin programs (placed in /System/sbin/)
function(add_rust_sbin_program NAME SRC_DIR)
  set(ELF "${CMAKE_BINARY_DIR}/programs/${NAME}/x86_64-anyos-user/release/${NAME}.elf")
  file(GLOB_RECURSE _PROG_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${ELF}
    COMMAND ${CARGO_EXECUTABLE} build --release
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
      --target-dir ${CMAKE_BINARY_DIR}/programs/${NAME}
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${SRC_DIR}/build.rs
      ${_PROG_RS}
      ${STDLIB_DEPS}
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building sbin program: ${NAME}"
  )
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/System/sbin/${NAME}
    COMMAND ${ANYELF_EXECUTABLE} bin
      ${ELF}
      ${SYSROOT_DIR}/System/sbin/${NAME}
    DEPENDS ${ELF} ${ANYELF_EXECUTABLE}
    COMMENT "Converting ${NAME} ELF to flat binary (sbin)"
  )
  set(RUST_USER_BINS ${RUST_USER_BINS} ${SYSROOT_DIR}/System/sbin/${NAME} PARENT_SCOPE)
endfunction()

# Variant for .app bundles (placed in /Applications/{DISPLAY_NAME}.app/)
# Uses mkappbundle for validated bundling with ELF auto-conversion via anyelf.
function(add_app NAME SRC_DIR DISPLAY_NAME)
  set(APP_DIR "${SYSROOT_DIR}/Applications/${DISPLAY_NAME}.app")
  set(ELF "${CMAKE_BINARY_DIR}/programs/${NAME}/x86_64-anyos-user/release/${NAME}.elf")
  file(GLOB_RECURSE _PROG_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${ELF}
    COMMAND ${CARGO_EXECUTABLE} build --release
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
      --target-dir ${CMAKE_BINARY_DIR}/programs/${NAME}
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${SRC_DIR}/build.rs
      ${_PROG_RS}
      ${STDLIB_DEPS}
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building app: ${DISPLAY_NAME}"
  )
  # Collect mkappbundle arguments and dependencies
  set(_BUNDLE_ARGS
    -i "${SRC_DIR}/Info.conf"
    -e ${ELF}
    --anyelf-path ${ANYELF_EXECUTABLE}
    -o "${APP_DIR}"
    --force
  )
  set(_BUNDLE_DEPS ${ELF} "${SRC_DIR}/Info.conf" ${ANYELF_EXECUTABLE} ${MKAPPBUNDLE_EXECUTABLE})
  if(EXISTS "${SRC_DIR}/Icon.ico")
    list(APPEND _BUNDLE_ARGS -c "${SRC_DIR}/Icon.ico")
    list(APPEND _BUNDLE_DEPS "${SRC_DIR}/Icon.ico")
  endif()
  foreach(_RESDIR syntax)
    if(IS_DIRECTORY "${SRC_DIR}/${_RESDIR}")
      list(APPEND _BUNDLE_ARGS -r "${SRC_DIR}/${_RESDIR}")
      file(GLOB _RES_FILES "${SRC_DIR}/${_RESDIR}/*")
      list(APPEND _BUNDLE_DEPS ${_RES_FILES})
    endif()
  endforeach()
  # Copy extra resource files (e.g. build.conf)
  if(EXISTS "${SRC_DIR}/build.conf")
    list(APPEND _BUNDLE_ARGS -r "${SRC_DIR}/build.conf")
    list(APPEND _BUNDLE_DEPS "${SRC_DIR}/build.conf")
  endif()
  add_custom_command(
    OUTPUT "${APP_DIR}/${DISPLAY_NAME}"
    COMMAND ${CMAKE_COMMAND} -E rm -rf "${APP_DIR}"
    COMMAND ${MKAPPBUNDLE_EXECUTABLE} ${_BUNDLE_ARGS}
    DEPENDS ${_BUNDLE_DEPS}
    COMMENT "Packaging ${DISPLAY_NAME}.app (mkappbundle)"
  )
  set(APP_BINS ${APP_BINS} "${APP_DIR}/${DISPLAY_NAME}" PARENT_SCOPE)
endfunction()

# Variant for DLLs (placed in /Libraries/)
function(add_dll NAME SRC_DIR)
  set(DLL_ELF "${CMAKE_BINARY_DIR}/dll/${NAME}/x86_64-anyos-user/release/${NAME}.elf")
  file(GLOB_RECURSE _DLL_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${DLL_ELF}
    COMMAND ${CARGO_EXECUTABLE} build --release
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
      --target-dir ${CMAKE_BINARY_DIR}/dll/${NAME}
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${SRC_DIR}/build.rs
      ${_DLL_RS}
      ${SRC_DIR}/link.ld
      ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building DLL: ${NAME}"
  )
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/Libraries/${NAME}.dlib
    COMMAND ${ANYELF_EXECUTABLE} dlib
      ${DLL_ELF}
      ${SYSROOT_DIR}/Libraries/${NAME}.dlib
    DEPENDS ${DLL_ELF} ${ANYELF_EXECUTABLE}
    COMMENT "Converting ${NAME} ELF to DLIB v3"
  )
  set(DLL_BINS ${DLL_BINS} ${SYSROOT_DIR}/Libraries/${NAME}.dlib PARENT_SCOPE)
endfunction()

# Variant for loadable kernel drivers (.ddv bundles in /System/Drivers/{CATEGORY}/)
# Builds with kernel target (Ring 0), converts to KDRV format via anyelf
function(add_driver NAME SRC_DIR DISPLAY_NAME CATEGORY)
  set(DDV_DIR "${SYSROOT_DIR}/System/Drivers/${CATEGORY}/${DISPLAY_NAME}.ddv")
  set(DRV_ELF "${CMAKE_BINARY_DIR}/drivers/${NAME}/x86_64-anyos/release/${NAME}.elf")
  file(GLOB_RECURSE _DRV_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${DRV_ELF}
    COMMAND ${CARGO_EXECUTABLE} build --release
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos.json
      --target-dir ${CMAKE_BINARY_DIR}/drivers/${NAME}
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${_DRV_RS}
      ${CMAKE_SOURCE_DIR}/x86_64-anyos.json
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building driver: ${DISPLAY_NAME}"
  )
  add_custom_command(
    OUTPUT "${DDV_DIR}/${DISPLAY_NAME}"
    COMMAND ${CMAKE_COMMAND} -E make_directory "${DDV_DIR}"
    COMMAND ${ANYELF_EXECUTABLE} kdrv
      ${DRV_ELF}
      "${DDV_DIR}/${DISPLAY_NAME}"
    COMMAND ${CMAKE_COMMAND} -E copy "${SRC_DIR}/Info.conf" "${DDV_DIR}/Info.conf"
    DEPENDS ${DRV_ELF} "${SRC_DIR}/Info.conf" ${ANYELF_EXECUTABLE}
    COMMENT "Packaging ${DISPLAY_NAME}.ddv"
  )
  set(DRIVER_BINS ${DRIVER_BINS} "${DDV_DIR}/${DISPLAY_NAME}" PARENT_SCOPE)
endfunction()

set(DRIVER_BINS "")
# add_driver(example_drv ${CMAKE_SOURCE_DIR}/drivers/example "Example Driver" "network")

set(DLL_BINS "")
add_dll(uisys ${CMAKE_SOURCE_DIR}/libs/uisys)
add_dll(libimage ${CMAKE_SOURCE_DIR}/libs/libimage)
add_dll(librender ${CMAKE_SOURCE_DIR}/libs/librender)
add_dll(libcompositor ${CMAKE_SOURCE_DIR}/libs/libcompositor)

# Shared libraries (.so) — built via Cargo → .a → anyld → ET_DYN .so
function(add_shared_lib NAME SRC_DIR BASE_ADDR)
  set(LIB_A "${CMAKE_BINARY_DIR}/shlib/${NAME}/x86_64-anyos-user/release/lib${NAME}.a")
  set(LIB_SO "${CMAKE_BINARY_DIR}/shlib/${NAME}.so")
  file(GLOB_RECURSE _SL_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  # Step 1: Cargo → static archive (.a)
  add_custom_command(
    OUTPUT ${LIB_A}
    COMMAND ${CARGO_EXECUTABLE} build --release
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
      --target-dir ${CMAKE_BINARY_DIR}/shlib/${NAME}
      -Z build-std=core,alloc
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${_SL_RS}
      ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building shared library: ${NAME} (Cargo)"
  )
  # Step 2: anyld → .so (ET_DYN shared object)
  add_custom_command(
    OUTPUT ${LIB_SO}
    COMMAND ${ANYLD_EXECUTABLE}
      -o ${LIB_SO}
      -b ${BASE_ADDR}
      -e ${SRC_DIR}/exports.def
      ${LIB_A}
    DEPENDS ${LIB_A} ${SRC_DIR}/exports.def ${ANYLD_EXECUTABLE}
    COMMENT "Linking ${NAME}.so (anyld)"
  )
  # Step 3: Copy to sysroot
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/Libraries/${NAME}.so
    COMMAND ${CMAKE_COMMAND} -E copy ${LIB_SO} ${SYSROOT_DIR}/Libraries/${NAME}.so
    DEPENDS ${LIB_SO}
    COMMENT "Installing ${NAME}.so to sysroot"
  )
  set(DLL_BINS ${DLL_BINS} ${SYSROOT_DIR}/Libraries/${NAME}.so PARENT_SCOPE)
endfunction()

add_shared_lib(libanyui ${CMAKE_SOURCE_DIR}/libs/libanyui "0x04400000")
add_shared_lib(libfont ${CMAKE_SOURCE_DIR}/libs/libfont "0x05000000")
add_shared_lib(libdb ${CMAKE_SOURCE_DIR}/libs/libdb "0x07000000")

set(RUST_USER_BINS "")
add_rust_user_program(ping       ${CMAKE_SOURCE_DIR}/bin/ping)
add_rust_user_program(dhcp       ${CMAKE_SOURCE_DIR}/bin/dhcp)
add_rust_user_program(dns        ${CMAKE_SOURCE_DIR}/bin/dns)
add_rust_user_program(ls         ${CMAKE_SOURCE_DIR}/bin/ls)
add_rust_user_program(cat        ${CMAKE_SOURCE_DIR}/bin/cat)
add_rust_user_program(ifconfig   ${CMAKE_SOURCE_DIR}/bin/ifconfig)
add_rust_user_program(arp        ${CMAKE_SOURCE_DIR}/bin/arp)
add_rust_user_program(sysinfo    ${CMAKE_SOURCE_DIR}/bin/sysinfo)
add_rust_user_program(dmesg      ${CMAKE_SOURCE_DIR}/bin/dmesg)
add_rust_user_program(mkdir      ${CMAKE_SOURCE_DIR}/bin/mkdir)
add_rust_user_program(rm         ${CMAKE_SOURCE_DIR}/bin/rm)
add_rust_user_program(touch      ${CMAKE_SOURCE_DIR}/bin/touch)
add_rust_user_program(cp         ${CMAKE_SOURCE_DIR}/bin/cp)
add_rust_user_program(mv         ${CMAKE_SOURCE_DIR}/bin/mv)
add_rust_user_program(date       ${CMAKE_SOURCE_DIR}/bin/date)
add_rust_user_program(sleep      ${CMAKE_SOURCE_DIR}/bin/sleep)
add_rust_user_program(hostname   ${CMAKE_SOURCE_DIR}/bin/hostname)
add_rust_user_program(ftp        ${CMAKE_SOURCE_DIR}/bin/ftp)
add_rust_user_program(wget       ${CMAKE_SOURCE_DIR}/bin/wget)
add_rust_user_program(play       ${CMAKE_SOURCE_DIR}/bin/play)
add_rust_user_program(pipes      ${CMAKE_SOURCE_DIR}/bin/pipes)
add_rust_user_program(devlist    ${CMAKE_SOURCE_DIR}/bin/devlist)
add_rust_user_program(echo       ${CMAKE_SOURCE_DIR}/bin/echo)
add_rust_user_program(ps         ${CMAKE_SOURCE_DIR}/bin/ps)
add_rust_user_program(top        ${CMAKE_SOURCE_DIR}/bin/top)
add_rust_user_program(htop       ${CMAKE_SOURCE_DIR}/bin/htop)
add_rust_user_program(kill       ${CMAKE_SOURCE_DIR}/bin/kill)
add_rust_user_program(nice       ${CMAKE_SOURCE_DIR}/bin/nice)
add_rust_user_program(free       ${CMAKE_SOURCE_DIR}/bin/free)
add_rust_user_program(uptime     ${CMAKE_SOURCE_DIR}/bin/uptime)
add_rust_user_program(uname      ${CMAKE_SOURCE_DIR}/bin/uname)
add_rust_user_program(pwd        ${CMAKE_SOURCE_DIR}/bin/pwd)
add_rust_user_program(wc         ${CMAKE_SOURCE_DIR}/bin/wc)
add_rust_user_program(hexdump    ${CMAKE_SOURCE_DIR}/bin/hexdump)
add_rust_user_program(head       ${CMAKE_SOURCE_DIR}/bin/head)
add_rust_user_program(tail       ${CMAKE_SOURCE_DIR}/bin/tail)
add_rust_user_program(clear      ${CMAKE_SOURCE_DIR}/bin/clear)
add_rust_user_program(env        ${CMAKE_SOURCE_DIR}/bin/env)
add_rust_user_program(grep       ${CMAKE_SOURCE_DIR}/bin/grep)
add_rust_user_program(find       ${CMAKE_SOURCE_DIR}/bin/find)
add_rust_user_program(sort       ${CMAKE_SOURCE_DIR}/bin/sort)
add_rust_user_program(uniq       ${CMAKE_SOURCE_DIR}/bin/uniq)
add_rust_user_program(rev        ${CMAKE_SOURCE_DIR}/bin/rev)
add_rust_user_program(stat       ${CMAKE_SOURCE_DIR}/bin/stat)
add_rust_user_program(ln         ${CMAKE_SOURCE_DIR}/bin/ln)
add_rust_user_program(readlink   ${CMAKE_SOURCE_DIR}/bin/readlink)
add_rust_user_program(df         ${CMAKE_SOURCE_DIR}/bin/df)
add_rust_user_program(cal        ${CMAKE_SOURCE_DIR}/bin/cal)
add_rust_user_program(seq        ${CMAKE_SOURCE_DIR}/bin/seq)
add_rust_user_program(yes        ${CMAKE_SOURCE_DIR}/bin/yes)
add_rust_user_program(whoami     ${CMAKE_SOURCE_DIR}/bin/whoami)
add_rust_user_program(which      ${CMAKE_SOURCE_DIR}/bin/which)
add_rust_user_program(strings    ${CMAKE_SOURCE_DIR}/bin/strings)
add_rust_user_program(base64     ${CMAKE_SOURCE_DIR}/bin/base64)
add_rust_user_program(xxd        ${CMAKE_SOURCE_DIR}/bin/xxd)
add_rust_user_program(set        ${CMAKE_SOURCE_DIR}/bin/set)
add_rust_user_program(export     ${CMAKE_SOURCE_DIR}/bin/export)
add_rust_user_program(mount      ${CMAKE_SOURCE_DIR}/bin/mount)
add_rust_user_program(umount     ${CMAKE_SOURCE_DIR}/bin/umount)
add_rust_user_program(open       ${CMAKE_SOURCE_DIR}/bin/open)
add_rust_user_program(listuser   ${CMAKE_SOURCE_DIR}/bin/listuser)
add_rust_user_program(listgroups ${CMAKE_SOURCE_DIR}/bin/listgroups)
add_rust_user_program(chmod      ${CMAKE_SOURCE_DIR}/bin/chmod)
add_rust_user_program(chown      ${CMAKE_SOURCE_DIR}/bin/chown)
add_rust_user_program(su         ${CMAKE_SOURCE_DIR}/bin/su)
add_rust_user_program(echoserver ${CMAKE_SOURCE_DIR}/bin/echoserver)
add_rust_user_program(netstat    ${CMAKE_SOURCE_DIR}/bin/netstat)
add_rust_user_program(svc        ${CMAKE_SOURCE_DIR}/bin/svc)
add_rust_user_program(logd       ${CMAKE_SOURCE_DIR}/bin/logd)
add_rust_user_program(ami        ${CMAKE_SOURCE_DIR}/bin/ami)
add_rust_user_program(vi         ${CMAKE_SOURCE_DIR}/bin/vi)
# Privileged sbin programs
add_rust_sbin_program(adduser    ${CMAKE_SOURCE_DIR}/bin/adduser)
add_rust_sbin_program(deluser    ${CMAKE_SOURCE_DIR}/bin/deluser)
add_rust_sbin_program(addgroup   ${CMAKE_SOURCE_DIR}/bin/addgroup)
add_rust_sbin_program(delgroup   ${CMAKE_SOURCE_DIR}/bin/delgroup)
add_rust_sbin_program(passwd     ${CMAKE_SOURCE_DIR}/bin/passwd)
add_rust_sbin_program(fdisk      ${CMAKE_SOURCE_DIR}/bin/fdisk)

# true/false: package names are true_cmd/false_cmd (Rust keywords) but binaries named true/false
set(TRUE_ELF "${CMAKE_BINARY_DIR}/programs/true_cmd/x86_64-anyos-user/release/true_cmd.elf")
add_custom_command(
  OUTPUT ${TRUE_ELF}
  COMMAND ${CARGO_EXECUTABLE} build --release
    --manifest-path ${CMAKE_SOURCE_DIR}/bin/true/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
    --target-dir ${CMAKE_BINARY_DIR}/programs/true_cmd
  DEPENDS
    ${CMAKE_SOURCE_DIR}/bin/true/Cargo.toml
    ${CMAKE_SOURCE_DIR}/bin/true/build.rs
    ${CMAKE_SOURCE_DIR}/bin/true/src/main.rs
    ${STDLIB_DEPS}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building Rust user program: true"
)
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/true
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${TRUE_ELF}
    ${SYSROOT_DIR}/System/bin/true
  DEPENDS ${TRUE_ELF} ${ANYELF_EXECUTABLE}
  COMMENT "Converting true ELF to flat binary"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/System/bin/true)

set(FALSE_ELF "${CMAKE_BINARY_DIR}/programs/false_cmd/x86_64-anyos-user/release/false_cmd.elf")
add_custom_command(
  OUTPUT ${FALSE_ELF}
  COMMAND ${CARGO_EXECUTABLE} build --release
    --manifest-path ${CMAKE_SOURCE_DIR}/bin/false/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
    --target-dir ${CMAKE_BINARY_DIR}/programs/false_cmd
  DEPENDS
    ${CMAKE_SOURCE_DIR}/bin/false/Cargo.toml
    ${CMAKE_SOURCE_DIR}/bin/false/build.rs
    ${CMAKE_SOURCE_DIR}/bin/false/src/main.rs
    ${STDLIB_DEPS}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building Rust user program: false"
)
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/false
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${FALSE_ELF}
    ${SYSROOT_DIR}/System/bin/false
  DEPENDS ${FALSE_ELF} ${ANYELF_EXECUTABLE}
  COMMENT "Converting false ELF to flat binary"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/System/bin/false)

set(SYSTEM_BINS "")
add_rust_system_program(init        ${CMAKE_SOURCE_DIR}/system/init)
add_rust_system_program(audiomon    ${CMAKE_SOURCE_DIR}/system/audiomon)
add_rust_system_program(netmon      ${CMAKE_SOURCE_DIR}/system/netmon)
add_rust_system_program(inputmon    ${CMAKE_SOURCE_DIR}/system/inputmon)
add_rust_system_program(login       ${CMAKE_SOURCE_DIR}/system/login)
add_rust_system_program(permdialog  ${CMAKE_SOURCE_DIR}/system/permdialog)
add_rust_user_program(amid          ${CMAKE_SOURCE_DIR}/system/amid)

# Desktop GUI applications → .app bundles in /Applications/
set(APP_BINS "")
add_app(terminal    ${CMAKE_SOURCE_DIR}/system/terminal     "Terminal")
add_app(shell       ${CMAKE_SOURCE_DIR}/system/shell        "Shell")
add_app(taskmanager ${CMAKE_SOURCE_DIR}/system/taskmanager  "Activity Monitor")
add_app(settings    ${CMAKE_SOURCE_DIR}/system/settings     "Settings")
add_app(finder      ${CMAKE_SOURCE_DIR}/system/finder       "Finder")
add_app(diskutil   ${CMAKE_SOURCE_DIR}/system/diskutil     "Disk Utility")
add_app(eventviewer ${CMAKE_SOURCE_DIR}/system/eventviewer  "Event Viewer")
add_app(notepad     ${CMAKE_SOURCE_DIR}/apps/notepad        "Notepad")
add_app(imgview     ${CMAKE_SOURCE_DIR}/apps/imgview        "Image Viewer")
add_app(videoplayer ${CMAKE_SOURCE_DIR}/apps/videoplayer    "Video Player")
add_app(diagnostics ${CMAKE_SOURCE_DIR}/apps/diagnostics    "Diagnostics")
add_app(calc        ${CMAKE_SOURCE_DIR}/apps/calc           "Calculator")
add_app(fontviewer  ${CMAKE_SOURCE_DIR}/apps/fontviewer     "Font Viewer")
add_app(clock       ${CMAKE_SOURCE_DIR}/apps/clock          "Clock")
add_app(screenshot  ${CMAKE_SOURCE_DIR}/apps/screenshot     "Screenshot")
add_app(surf        ${CMAKE_SOURCE_DIR}/apps/surf            "Surf")
add_app(demo_anyui  ${CMAKE_SOURCE_DIR}/apps/demo_anyui     "anyUI Demo")
add_app(anycode     ${CMAKE_SOURCE_DIR}/apps/anycode        "anyOS Code")
add_app(paint       ${CMAKE_SOURCE_DIR}/apps/paint          "Paint")
add_app(minesweeper ${CMAKE_SOURCE_DIR}/apps/minesweeper   "Minesweeper")

# Build compositor binary (nested path: /System/compositor/compositor)
set(COMPOSITOR_SRC_DIR ${CMAKE_SOURCE_DIR}/system/compositor/compositor)
set(COMPOSITOR_ELF "${CMAKE_BINARY_DIR}/programs/compositor/x86_64-anyos-user/release/compositor.elf")
add_custom_command(
  OUTPUT ${COMPOSITOR_ELF}
  COMMAND ${CARGO_EXECUTABLE} build --release
    --manifest-path ${COMPOSITOR_SRC_DIR}/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
    --target-dir ${CMAKE_BINARY_DIR}/programs/compositor
  DEPENDS
    ${COMPOSITOR_SRC_DIR}/Cargo.toml
    ${COMPOSITOR_SRC_DIR}/build.rs
    ${COMPOSITOR_SRC_DIR}/src/main.rs
    ${STDLIB_DEPS}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building system program: compositor"
)
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/compositor/compositor
  COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/System/compositor
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${COMPOSITOR_ELF}
    ${SYSROOT_DIR}/System/compositor/compositor
  DEPENDS ${COMPOSITOR_ELF} ${ANYELF_EXECUTABLE}
  COMMENT "Converting compositor ELF to flat binary"
)
list(APPEND SYSTEM_BINS ${SYSROOT_DIR}/System/compositor/compositor)

# Build dock program (nested path: /System/compositor/dock)
set(DOCK_SRC_DIR ${CMAKE_SOURCE_DIR}/system/compositor/dock)
set(DOCK_ELF "${CMAKE_BINARY_DIR}/programs/dock/x86_64-anyos/release/dock.elf")
add_custom_command(
  OUTPUT ${DOCK_ELF}
  COMMAND ${CARGO_EXECUTABLE} build --release
    --manifest-path ${DOCK_SRC_DIR}/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/x86_64-anyos.json
    --target-dir ${CMAKE_BINARY_DIR}/programs/dock
  DEPENDS
    ${DOCK_SRC_DIR}/Cargo.toml
    ${DOCK_SRC_DIR}/build.rs
    ${DOCK_SRC_DIR}/src/main.rs
    ${STDLIB_DEPS}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building system program: dock"
)
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/compositor/dock
  COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/System/compositor
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${DOCK_ELF}
    ${SYSROOT_DIR}/System/compositor/dock
  DEPENDS ${DOCK_ELF} ${ANYELF_EXECUTABLE}
  COMMENT "Converting dock ELF to flat binary"
)
list(APPEND SYSTEM_BINS ${SYSROOT_DIR}/System/compositor/dock)


# Copy any extra sysroot files from tools/sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/.stamp
  COMMAND ${CMAKE_COMMAND} -E copy_directory
    ${CMAKE_SOURCE_DIR}/sysroot
    ${SYSROOT_DIR}
  COMMAND ${CMAKE_COMMAND} -E touch ${SYSROOT_DIR}/.stamp
  DEPENDS ${CMAKE_SOURCE_DIR}/sysroot
          ${CMAKE_SOURCE_DIR}/sysroot/System/fonts/sfpro.ttf
          ${CMAKE_SOURCE_DIR}/sysroot/System/fonts/sfpro-bold.ttf
          ${CMAKE_SOURCE_DIR}/sysroot/System/fonts/sfpro-thin.ttf
          ${CMAKE_SOURCE_DIR}/sysroot/System/fonts/sfpro-italic.ttf
          ${CMAKE_SOURCE_DIR}/sysroot/System/fonts/andale-mono.ttf
          ${CMAKE_SOURCE_DIR}/sysroot/media/wallpapers/default.png
          ${CMAKE_SOURCE_DIR}/sysroot/media/wallpapers
          ${CMAKE_SOURCE_DIR}/sysroot/System/users/wallpapers
          ${CMAKE_SOURCE_DIR}/sysroot/System/media/icons
          ${CMAKE_SOURCE_DIR}/sysroot/System/media/icons/controls
          ${CMAKE_SOURCE_DIR}/sysroot/System/compositor/compositor.conf
          ${CMAKE_SOURCE_DIR}/sysroot/System/etc/inputmon.conf
  COMMENT "Populating sysroot from tools/sysroot"
)

# ============================================================
# C Library, TCC, and Games (require i686-elf cross-compiler)
# ============================================================
if(HAS_CROSS_COMPILER)
# ── C Library (libc.a + crt0.o for C programs) ──
set(LIBC_DIR "${CMAKE_SOURCE_DIR}/libs/libc")
set(LIBC_A "${LIBC_DIR}/libc.a")
set(LIBC_CRT0 "${LIBC_DIR}/obj/crt0.o")

get_filename_component(CROSS_BIN_DIR "${I686_ELF_GCC}" DIRECTORY)
set(I686_ELF_AR  "${CROSS_BIN_DIR}/i686-elf-ar${CMAKE_EXECUTABLE_SUFFIX}")

add_custom_command(
  OUTPUT ${LIBC_A} ${LIBC_CRT0}
  COMMAND ${MAKE_EXECUTABLE} -C ${LIBC_DIR} clean CC=${I686_ELF_GCC} AR=${I686_ELF_AR} AS=${I686_ELF_GCC}
  COMMAND ${MAKE_EXECUTABLE} -C ${LIBC_DIR} CC=${I686_ELF_GCC} AR=${I686_ELF_AR} AS=${I686_ELF_GCC}
  DEPENDS
    ${LIBC_DIR}/Makefile
    ${LIBC_DIR}/src/crt0.S
    ${LIBC_DIR}/src/syscall.S
    ${LIBC_DIR}/src/string.c
    ${LIBC_DIR}/src/stdlib.c
    ${LIBC_DIR}/src/stdio.c
    ${LIBC_DIR}/src/unistd.c
    ${LIBC_DIR}/src/ctype.c
    ${LIBC_DIR}/src/signal.c
    ${LIBC_DIR}/src/setjmp.S
    ${LIBC_DIR}/src/stat.c
    ${LIBC_DIR}/src/time.c
    ${LIBC_DIR}/src/math.c
    ${LIBC_DIR}/src/mman.c
    ${LIBC_DIR}/src/start.c
    ${LIBC_DIR}/src/socket.c
    ${LIBC_DIR}/src/stubs.c
  COMMENT "Building C library (libc.a + crt0.o)"
)

# Create empty crti.o, crtn.o stubs and empty libtcc1.a for TCC
set(LIBC_CRTI "${LIBC_DIR}/obj/crti.o")
set(LIBC_CRTN "${LIBC_DIR}/obj/crtn.o")
set(LIBC_LIBTCC1 "${LIBC_DIR}/libtcc1.a")
add_custom_command(
  OUTPUT ${LIBC_CRTI} ${LIBC_CRTN} ${LIBC_LIBTCC1}
  COMMAND ${I686_ELF_GCC} -m32 -c ${LIBC_DIR}/src/crti.S -o ${LIBC_CRTI}
  COMMAND ${I686_ELF_GCC} -m32 -c ${LIBC_DIR}/src/crtn.S -o ${LIBC_CRTN}
  COMMAND ${I686_ELF_AR} rcs ${LIBC_LIBTCC1}
  DEPENDS ${LIBC_A} ${LIBC_DIR}/src/crti.S ${LIBC_DIR}/src/crtn.S
  COMMENT "Creating CRT stubs and libtcc1.a for TCC"
)

# Copy libc artifacts to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/libc/lib/libc.a ${SYSROOT_DIR}/Libraries/libc/lib/crt0.o
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_A} ${SYSROOT_DIR}/Libraries/libc/lib/libc.a
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_CRT0} ${SYSROOT_DIR}/Libraries/libc/lib/crt0.o
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_CRT0} ${SYSROOT_DIR}/Libraries/libc/lib/crt1.o
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_CRTI} ${SYSROOT_DIR}/Libraries/libc/lib/crti.o
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_CRTN} ${SYSROOT_DIR}/Libraries/libc/lib/crtn.o
  DEPENDS ${LIBC_A} ${LIBC_CRT0} ${LIBC_CRTI} ${LIBC_CRTN}
  COMMENT "Installing libc.a and crt0.o to sysroot"
)

# Copy libc headers to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/libc/include/.stamp
  COMMAND ${CMAKE_COMMAND} -E copy_directory ${LIBC_DIR}/include ${SYSROOT_DIR}/Libraries/libc/include
  COMMAND ${CMAKE_COMMAND} -E touch ${SYSROOT_DIR}/Libraries/libc/include/.stamp
  DEPENDS ${LIBC_A}
  COMMENT "Installing C headers to sysroot"
)

# ============================================================
# TCC (Tiny C Compiler for anyOS)
# ============================================================
set(TCC_DIR "${CMAKE_SOURCE_DIR}/third_party/tcc-0.9.27")
set(TCC_OBJ "${TCC_DIR}/tcc.o")
set(TCC_ELF "${TCC_DIR}/tcc.elf")

set(TCC_CFLAGS
  -DONE_SOURCE=1
  -DTCC_TARGET_I386
  -DCONFIG_TCC_STATIC
  -DCONFIG_TCCBOOT
  "-DCONFIG_TCCDIR=\"/Libraries/libc/lib/tcc\""
  "-DCONFIG_TCC_SYSINCLUDEPATHS=\"/Libraries/libc/include\""
  "-DCONFIG_TCC_LIBPATHS=\"/Libraries/libc/lib\""
  "-DCONFIG_TCC_CRTPREFIX=\"/Libraries/libc/lib\""
  "-DCONFIG_TCC_ELFINTERP=\"\""
  "-DTCC_VERSION=\"0.9.27\""
)

add_custom_command(
  OUTPUT ${TCC_OBJ}
  COMMAND ${I686_ELF_GCC}
    ${TCC_CFLAGS}
    -I${TCC_DIR}
    -I${LIBC_DIR}/include
    -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector
    -O2 -m32 -w
    -c ${TCC_DIR}/tcc.c -o ${TCC_OBJ}
  DEPENDS ${LIBC_A} ${TCC_DIR}/tcc.c ${TCC_DIR}/tcc.h ${TCC_DIR}/libtcc.c
  COMMENT "Compiling TCC for anyOS"
)

add_custom_command(
  OUTPUT ${TCC_ELF}
  COMMAND ${I686_ELF_GCC}
    -nostdlib -static -m32
    -T ${LIBC_DIR}/link.ld
    -o ${TCC_ELF}
    ${LIBC_CRT0}
    ${TCC_OBJ}
    ${LIBC_A}
    -lgcc
  DEPENDS ${TCC_OBJ} ${LIBC_A} ${LIBC_CRT0} ${LIBC_DIR}/link.ld
  COMMENT "Linking TCC for anyOS"
)

# Copy TCC to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/cc
  COMMAND ${CMAKE_COMMAND} -E copy ${TCC_ELF} ${SYSROOT_DIR}/System/bin/cc
  DEPENDS ${TCC_ELF}
  COMMENT "Installing TCC as /System/bin/cc"
)

# Copy TCC internal headers
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libc/lib/tcc/include")
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/libc/lib/tcc/include/.stamp
  COMMAND ${CMAKE_COMMAND} -E copy_directory ${TCC_DIR}/include ${SYSROOT_DIR}/Libraries/libc/lib/tcc/include
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_DIR}/link.ld ${SYSROOT_DIR}/Libraries/libc/lib/tcc/link.ld
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_LIBTCC1} ${SYSROOT_DIR}/Libraries/libc/lib/tcc/libtcc1.a
  COMMAND ${CMAKE_COMMAND} -E touch ${SYSROOT_DIR}/Libraries/libc/lib/tcc/include/.stamp
  DEPENDS ${TCC_ELF} ${LIBC_LIBTCC1}
  COMMENT "Installing TCC headers to sysroot"
)

# ============================================================
# NASM (Netwide Assembler for anyOS)
# ============================================================
set(NASM_DIR "${CMAKE_SOURCE_DIR}/third_party/nasm")
set(NASM_ELF "${NASM_DIR}/nasm.elf")

add_custom_command(
  OUTPUT ${NASM_ELF}
  COMMAND ${CMAKE_SOURCE_DIR}/scripts/build_nasm.sh
  DEPENDS ${LIBC_A} ${LIBC_CRT0}
  COMMENT "Building NASM assembler for anyOS"
)

# Copy NASM to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/nasm
  COMMAND ${CMAKE_COMMAND} -E copy ${NASM_ELF} ${SYSROOT_DIR}/System/bin/nasm
  DEPENDS ${NASM_ELF}
  COMMENT "Installing NASM as /System/bin/nasm"
)

# ============================================================
# libgit2 + mini git CLI
# ============================================================
set(LG2_DIR "${CMAKE_SOURCE_DIR}/third_party/libgit2")
set(LG2_A "${LG2_DIR}/libgit2.a")
set(GIT_DIR "${CMAKE_SOURCE_DIR}/bin/git")
set(GIT_ELF "${GIT_DIR}/git.elf")

add_custom_command(
  OUTPUT ${LG2_A}
  COMMAND ${CMAKE_SOURCE_DIR}/scripts/build_libgit2.sh
  DEPENDS ${LIBC_A} ${LIBC_CRT0}
  COMMENT "Building libgit2 for anyOS"
)

add_custom_command(
  OUTPUT ${GIT_ELF}
  COMMAND ${CMAKE_SOURCE_DIR}/scripts/build_git.sh
  DEPENDS ${LG2_A} ${BEARSSL_A} ${LIBC_A} ${LIBC_CRT0}
    ${GIT_DIR}/src/main.c
    ${GIT_DIR}/src/bearssl_stream.c
  COMMENT "Building mini git CLI for anyOS"
)

# Copy git to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/git
  COMMAND ${CMAKE_COMMAND} -E copy ${GIT_ELF} ${SYSROOT_DIR}/System/bin/git
  DEPENDS ${GIT_ELF}
  COMMENT "Installing git as /System/bin/git"
)

# ============================================================
# make (minimal POSIX make utility, cross-compiled C)
# ============================================================
set(MAKE_SRC "${CMAKE_SOURCE_DIR}/bin/make/src/make.c")
set(MAKE_OBJ "${CMAKE_SOURCE_DIR}/bin/make/make.o")
set(MAKE_ELF "${CMAKE_SOURCE_DIR}/bin/make/make.elf")

add_custom_command(
  OUTPUT ${MAKE_OBJ}
  COMMAND ${I686_ELF_GCC}
    -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector
    -I${LIBC_DIR}/include -O2 -m32 -w
    -c ${MAKE_SRC} -o ${MAKE_OBJ}
  DEPENDS ${MAKE_SRC} ${LIBC_A}
  COMMENT "Compiling make for anyOS"
)

add_custom_command(
  OUTPUT ${MAKE_ELF}
  COMMAND ${I686_ELF_GCC}
    -nostdlib -static -m32
    -T ${LIBC_DIR}/link.ld
    -o ${MAKE_ELF}
    ${LIBC_CRT0}
    ${MAKE_OBJ}
    ${LIBC_A}
    -lgcc
  DEPENDS ${MAKE_OBJ} ${LIBC_A} ${LIBC_CRT0} ${LIBC_DIR}/link.ld
  COMMENT "Linking make for anyOS"
)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/make
  COMMAND ${CMAKE_COMMAND} -E copy ${MAKE_ELF} ${SYSROOT_DIR}/System/bin/make
  DEPENDS ${MAKE_ELF}
  COMMENT "Installing make as /System/bin/make"
)

# ============================================================
# DOOM (doomgeneric, cross-compiled C program) → DOOM.app
# ============================================================
set(DOOM_DIR "${CMAKE_SOURCE_DIR}/third_party/doom")
set(DOOM_ELF "${DOOM_DIR}/doom.elf")
set(DOOM_APP "${SYSROOT_DIR}/Applications/DOOM.app")

file(GLOB DOOM_SOURCES "${DOOM_DIR}/src/*.c" "${DOOM_DIR}/src/*.h" "${DOOM_DIR}/Makefile")
add_custom_command(
  OUTPUT ${DOOM_ELF}
  COMMAND ${MAKE_EXECUTABLE} -C ${DOOM_DIR} clean CC=${I686_ELF_GCC} AR=${I686_ELF_AR} AS=${I686_ELF_GCC}
  COMMAND ${MAKE_EXECUTABLE} -C ${DOOM_DIR} CC=${I686_ELF_GCC} AR=${I686_ELF_AR} AS=${I686_ELF_GCC} LIBC_DIR=${LIBC_DIR}
  DEPENDS ${LIBC_A} ${LIBC_CRT0} ${DOOM_SOURCES}
  COMMENT "Building DOOM for anyOS"
)

set(DOOM_WAD "${CMAKE_SOURCE_DIR}/sysroot/apps/doom/doom.wad")

add_custom_command(
  OUTPUT ${DOOM_APP}/DOOM
  COMMAND ${CMAKE_COMMAND} -E rm -rf "${DOOM_APP}"
  COMMAND ${MKAPPBUNDLE_EXECUTABLE}
    -i "${DOOM_DIR}/Info.conf"
    -e ${DOOM_ELF}
    -c "${DOOM_DIR}/Icon.ico"
    -r ${DOOM_WAD}
    --keep-elf
    --force
    -o "${DOOM_APP}"
  DEPENDS ${DOOM_ELF} ${DOOM_WAD} "${DOOM_DIR}/Info.conf" "${DOOM_DIR}/Icon.ico" ${MKAPPBUNDLE_EXECUTABLE}
  COMMENT "Packaging DOOM.app (mkappbundle)"
)

# Quake (WinQuake software renderer, cross-compiled C program) → Quake.app
# ============================================================
set(QUAKE_DIR "${CMAKE_SOURCE_DIR}/third_party/quake")
set(QUAKE_ELF "${QUAKE_DIR}/quake.elf")
set(QUAKE_APP "${SYSROOT_DIR}/Applications/Quake.app")

file(GLOB QUAKE_SOURCES "${QUAKE_DIR}/WinQuake/*.c" "${QUAKE_DIR}/WinQuake/*.h" "${QUAKE_DIR}/Makefile")
add_custom_command(
  OUTPUT ${QUAKE_ELF}
  COMMAND ${MAKE_EXECUTABLE} -C ${QUAKE_DIR} clean CC=${I686_ELF_GCC} AR=${I686_ELF_AR} AS=${I686_ELF_GCC}
  COMMAND ${MAKE_EXECUTABLE} -C ${QUAKE_DIR} CC=${I686_ELF_GCC} AR=${I686_ELF_AR} AS=${I686_ELF_GCC} LIBC_DIR=${LIBC_DIR}
  DEPENDS ${LIBC_A} ${LIBC_CRT0} ${QUAKE_SOURCES}
  COMMENT "Building Quake for anyOS"
)

set(QUAKE_PAK "${CMAKE_SOURCE_DIR}/sysroot/apps/quake/id1/pak0.pak")
set(QUAKE_CFG "${CMAKE_SOURCE_DIR}/sysroot/apps/quake/id1/config.cfg")

set(QUAKE_ID1_DIR "${CMAKE_SOURCE_DIR}/sysroot/apps/quake/id1")
add_custom_command(
  OUTPUT ${QUAKE_APP}/Quake
  COMMAND ${CMAKE_COMMAND} -E rm -rf "${QUAKE_APP}"
  COMMAND ${MKAPPBUNDLE_EXECUTABLE}
    -i "${QUAKE_DIR}/Info.conf"
    -e ${QUAKE_ELF}
    -r "${QUAKE_ID1_DIR}"
    --keep-elf
    --force
    -o "${QUAKE_APP}"
  DEPENDS ${QUAKE_ELF} ${QUAKE_PAK} ${QUAKE_CFG} "${QUAKE_DIR}/Info.conf" ${MKAPPBUNDLE_EXECUTABLE}
  COMMENT "Packaging Quake.app (mkappbundle)"
)

# ============================================================
# TinyGL Library (software OpenGL 1.x rasterizer)
# ============================================================
set(TINYGL_DIR "${CMAKE_SOURCE_DIR}/third_party/tinygl")
set(TINYGL_A "${TINYGL_DIR}/libtinygl.a")

file(GLOB TINYGL_SOURCES "${TINYGL_DIR}/src/*.c" "${TINYGL_DIR}/src/*.h" "${TINYGL_DIR}/Makefile")
add_custom_command(
  OUTPUT ${TINYGL_A}
  COMMAND ${MAKE_EXECUTABLE} -C ${TINYGL_DIR} clean CC=${I686_ELF_GCC} AR=${I686_ELF_AR}
  COMMAND ${MAKE_EXECUTABLE} -C ${TINYGL_DIR} CC=${I686_ELF_GCC} AR=${I686_ELF_AR} LIBC_DIR=${LIBC_DIR}
  DEPENDS ${LIBC_A} ${TINYGL_SOURCES}
  COMMENT "Building TinyGL library for anyOS"
)

# ============================================================
# GLCube Demo (TinyGL spinning cube) → GLCube.app
# ============================================================
set(GLCUBE_DIR "${CMAKE_SOURCE_DIR}/third_party/tinygl/demo")
set(GLCUBE_ELF "${GLCUBE_DIR}/glcube.elf")
set(GLCUBE_APP "${SYSROOT_DIR}/Applications/GLCube.app")

file(GLOB GLCUBE_SOURCES "${GLCUBE_DIR}/*.c" "${GLCUBE_DIR}/Makefile")
add_custom_command(
  OUTPUT ${GLCUBE_ELF}
  COMMAND ${MAKE_EXECUTABLE} -C ${GLCUBE_DIR} clean CC=${I686_ELF_GCC}
  COMMAND ${MAKE_EXECUTABLE} -C ${GLCUBE_DIR} CC=${I686_ELF_GCC} LIBC_DIR=${LIBC_DIR} TINYGL_DIR=${TINYGL_DIR}
  DEPENDS ${LIBC_A} ${LIBC_CRT0} ${TINYGL_A} ${GLCUBE_SOURCES}
  COMMENT "Building GLCube demo for anyOS"
)

add_custom_command(
  OUTPUT ${GLCUBE_APP}/GLCube
  COMMAND ${CMAKE_COMMAND} -E rm -rf "${GLCUBE_APP}"
  COMMAND ${MKAPPBUNDLE_EXECUTABLE}
    -i "${GLCUBE_DIR}/Info.conf"
    -e ${GLCUBE_ELF}
    --keep-elf
    --force
    -o "${GLCUBE_APP}"
  DEPENDS ${GLCUBE_ELF} "${GLCUBE_DIR}/Info.conf" ${MKAPPBUNDLE_EXECUTABLE}
  COMMENT "Packaging GLCube.app (mkappbundle)"
)

# ============================================================
# ClassiCube (Minecraft Classic, software renderer) → ClassiCube.app
# (only built if third_party/classicube is present — not a git submodule)
# ============================================================
set(CLASSICUBE_DIR "${CMAKE_SOURCE_DIR}/third_party/classicube")
if(EXISTS "${CLASSICUBE_DIR}/src/Core.h")
  set(HAS_CLASSICUBE TRUE)
  set(CLASSICUBE_ELF "${CLASSICUBE_DIR}/classicube.elf")
  set(CLASSICUBE_APP "${SYSROOT_DIR}/Applications/ClassiCube.app")

  file(GLOB CLASSICUBE_SOURCES
    "${CLASSICUBE_DIR}/src/*.c"
    "${CLASSICUBE_DIR}/src/*.h"
    "${CLASSICUBE_DIR}/misc/anyos/Makefile"
  )
  add_custom_command(
    OUTPUT ${CLASSICUBE_ELF}
    COMMAND ${MAKE_EXECUTABLE} -C ${CLASSICUBE_DIR} -f misc/anyos/Makefile clean
    COMMAND ${MAKE_EXECUTABLE} -C ${CLASSICUBE_DIR} -f misc/anyos/Makefile CC=${I686_ELF_GCC} LIBC_DIR=${LIBC_DIR}
    DEPENDS ${LIBC_A} ${LIBC_CRT0} ${CLASSICUBE_SOURCES}
    COMMENT "Building ClassiCube for anyOS"
  )

  add_custom_command(
    OUTPUT ${CLASSICUBE_APP}/ClassiCube
    COMMAND ${CMAKE_COMMAND} -E rm -rf "${CLASSICUBE_APP}"
    COMMAND ${MKAPPBUNDLE_EXECUTABLE}
      -i "${CLASSICUBE_DIR}/Info.conf"
      -e ${CLASSICUBE_ELF}
      --keep-elf
      --force
      -o "${CLASSICUBE_APP}"
    DEPENDS ${CLASSICUBE_ELF} "${CLASSICUBE_DIR}/Info.conf" ${MKAPPBUNDLE_EXECUTABLE}
    COMMENT "Packaging ClassiCube.app (mkappbundle)"
  )
else()
  set(HAS_CLASSICUBE FALSE)
  message(STATUS "ClassiCube not found in third_party/classicube — skipping")
endif()

# Clean targets for third-party builds (outside build dir)
set(THIRD_PARTY_CLEAN_FILES
  ${DOOM_ELF} "${DOOM_DIR}/obj"
  ${QUAKE_ELF} "${QUAKE_DIR}/obj"
  ${TINYGL_A} "${TINYGL_DIR}/obj"
  ${GLCUBE_ELF} "${GLCUBE_DIR}/obj"
)
if(HAS_CLASSICUBE)
  list(APPEND THIRD_PARTY_CLEAN_FILES ${CLASSICUBE_ELF} "${CLASSICUBE_DIR}/build/anyos")
endif()
set_property(DIRECTORY APPEND PROPERTY ADDITIONAL_CLEAN_FILES ${THIRD_PARTY_CLEAN_FILES})

# ============================================================
# BearSSL (TLS library, used by curl)
# ============================================================
set(BEARSSL_DIR "${CMAKE_SOURCE_DIR}/third_party/bearssl")
set(BEARSSL_A "${BEARSSL_DIR}/build/libbearssl.a")
add_custom_command(
  OUTPUT ${BEARSSL_A}
  COMMAND bash ${CMAKE_SOURCE_DIR}/scripts/build_bearssl.sh
  DEPENDS ${LIBC_A}
  COMMENT "Building BearSSL for anyOS"
)

# ============================================================
# curl (HTTP/FTP client, cross-compiled C program)
# ============================================================
set(CURL_DIR "${CMAKE_SOURCE_DIR}/third_party/curl")
set(CURL_LIB "${CURL_DIR}/libcurl.a")
set(CURL_ELF "${CURL_DIR}/curl.elf")

add_custom_command(
  OUTPUT ${CURL_LIB}
  COMMAND bash ${CMAKE_SOURCE_DIR}/scripts/build_curl.sh
  DEPENDS
    ${LIBC_A}
    ${CMAKE_SOURCE_DIR}/scripts/build_curl.sh
    ${CURL_DIR}/lib/config-anyos.h
  COMMENT "Building libcurl for anyOS"
)

add_custom_command(
  OUTPUT ${CURL_ELF}
  COMMAND ${I686_ELF_GCC}
    -nostdlib -static -m32
    -T ${LIBC_DIR}/link.ld
    -o ${CURL_ELF}
    ${LIBC_CRT0}
    -Wl,--start-group
    ${CURL_LIB}
    ${BEARSSL_A}
    ${LIBC_A}
    -lgcc
    -Wl,--end-group
  DEPENDS ${CURL_LIB} ${BEARSSL_A} ${LIBC_A} ${LIBC_CRT0} ${LIBC_DIR}/link.ld
  COMMENT "Linking curl for anyOS"
)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/curl
  COMMAND ${CMAKE_COMMAND} -E copy ${CURL_ELF} ${SYSROOT_DIR}/System/bin/curl
  DEPENDS ${CURL_ELF}
  COMMENT "Installing curl to /System/bin/curl"
)

# ============================================================
# dash (Debian Almquist Shell for anyOS)
# ============================================================
set(DASH_DIR "${CMAKE_SOURCE_DIR}/third_party/dash-0.5.12")
set(DASH_LIB "${DASH_DIR}/dash.a")
set(DASH_ELF "${DASH_DIR}/dash.elf")

add_custom_command(
  OUTPUT ${DASH_LIB}
  COMMAND bash ${CMAKE_SOURCE_DIR}/scripts/build_dash.sh
  DEPENDS
    ${LIBC_A}
    ${CMAKE_SOURCE_DIR}/scripts/build_dash.sh
    ${DASH_DIR}/config.h
  COMMENT "Building dash for anyOS"
)

add_custom_command(
  OUTPUT ${DASH_ELF}
  COMMAND ${I686_ELF_GCC}
    -nostdlib -static -m32
    -T ${LIBC_DIR}/link.ld
    -o ${DASH_ELF}
    ${LIBC_CRT0}
    -Wl,--start-group
    ${DASH_LIB}
    ${LIBC_A}
    -lgcc
    -Wl,--end-group
  DEPENDS ${DASH_LIB} ${LIBC_A} ${LIBC_CRT0} ${LIBC_DIR}/link.ld
  COMMENT "Linking dash for anyOS"
)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/sh
  COMMAND ${CMAKE_COMMAND} -E copy ${DASH_ELF} ${SYSROOT_DIR}/System/bin/sh
  DEPENDS ${DASH_ELF}
  COMMENT "Installing dash as /System/bin/sh"
)

# ============================================================
# SSH library + client + server
# ============================================================
set(SSH_DIR "${CMAKE_SOURCE_DIR}/third_party/ssh")
set(SSH_LIB "${SSH_DIR}/build/libssh.a")
set(SSH_CLIENT_DIR "${CMAKE_SOURCE_DIR}/bin/ssh")
set(SSH_CLIENT_ELF "${SSH_CLIENT_DIR}/ssh.elf")
set(SSHD_DIR "${CMAKE_SOURCE_DIR}/bin/sshd")
set(SSHD_ELF "${SSHD_DIR}/sshd.elf")

# Build libssh.a
add_custom_command(
  OUTPUT ${SSH_LIB}
  COMMAND bash ${CMAKE_SOURCE_DIR}/scripts/build_ssh.sh
  DEPENDS
    ${BEARSSL_A}
    ${LIBC_A}
    ${SSH_DIR}/src/ssh.c
    ${SSH_DIR}/include/ssh.h
    ${CMAKE_SOURCE_DIR}/scripts/build_ssh.sh
  COMMENT "Building SSH library for anyOS"
)

# Build SSH client (ssh.elf)
add_custom_command(
  OUTPUT ${SSH_CLIENT_ELF}
  COMMAND ${I686_ELF_GCC}
    -m32 -O2 -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector -std=c99 -w
    -I${SSH_DIR}/include -I${BEARSSL_DIR}/inc -I${LIBC_DIR}/include
    -c ${SSH_CLIENT_DIR}/src/main.c -o ${SSH_CLIENT_DIR}/main.o
  COMMAND ${I686_ELF_GCC}
    -nostdlib -static -m32
    -T ${LIBC_DIR}/link.ld
    -o ${SSH_CLIENT_ELF}
    ${LIBC_CRT0}
    ${SSH_CLIENT_DIR}/main.o
    -Wl,--start-group
    ${SSH_LIB}
    ${BEARSSL_A}
    ${LIBC_A}
    -lgcc
    -Wl,--end-group
  DEPENDS ${SSH_LIB} ${BEARSSL_A} ${LIBC_A} ${LIBC_CRT0} ${LIBC_DIR}/link.ld
    ${SSH_CLIENT_DIR}/src/main.c
  COMMENT "Building SSH client for anyOS"
)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/ssh
  COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/System/bin
  COMMAND ${CMAKE_COMMAND} -E copy ${SSH_CLIENT_ELF} ${SYSROOT_DIR}/System/bin/ssh
  DEPENDS ${SSH_CLIENT_ELF}
  COMMENT "Installing SSH client to /System/bin/ssh"
)

# Build SSH server (sshd.elf)
add_custom_command(
  OUTPUT ${SSHD_ELF}
  COMMAND ${I686_ELF_GCC}
    -m32 -O2 -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector -std=c99 -w
    -I${SSH_DIR}/include -I${BEARSSL_DIR}/inc -I${LIBC_DIR}/include
    -c ${SSHD_DIR}/src/main.c -o ${SSHD_DIR}/main.o
  COMMAND ${I686_ELF_GCC}
    -nostdlib -static -m32
    -T ${LIBC_DIR}/link.ld
    -o ${SSHD_ELF}
    ${LIBC_CRT0}
    ${SSHD_DIR}/main.o
    -Wl,--start-group
    ${SSH_LIB}
    ${BEARSSL_A}
    ${LIBC_A}
    -lgcc
    -Wl,--end-group
  DEPENDS ${SSH_LIB} ${BEARSSL_A} ${LIBC_A} ${LIBC_CRT0} ${LIBC_DIR}/link.ld
    ${SSHD_DIR}/src/main.c
  COMMENT "Building SSH server for anyOS"
)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/sshd
  COMMAND ${CMAKE_COMMAND} -E copy ${SSHD_ELF} ${SYSROOT_DIR}/System/bin/sshd
  DEPENDS ${SSHD_ELF}
  COMMENT "Installing SSH server to /System/bin/sshd"
)

# Copy test source files to /Libraries/system/tests/ on disk
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/system/tests/.stamp
  COMMAND ${CMAKE_COMMAND} -E copy_directory
    ${CMAKE_SOURCE_DIR}/libs/tests
    ${SYSROOT_DIR}/Libraries/system/tests
  COMMAND ${CMAKE_COMMAND} -E touch ${SYSROOT_DIR}/Libraries/system/tests/.stamp
  DEPENDS ${CMAKE_SOURCE_DIR}/libs/tests/Makefile
          ${CMAKE_SOURCE_DIR}/libs/tests/fork_test.c
          ${CMAKE_SOURCE_DIR}/libs/tests/pipe_test.c
          ${CMAKE_SOURCE_DIR}/libs/tests/dup_test.c
          ${CMAKE_SOURCE_DIR}/libs/tests/pipe_chain.c
          ${CMAKE_SOURCE_DIR}/libs/tests/signal_test.c
          ${CMAKE_SOURCE_DIR}/libs/tests/setjmp_test.c
          ${CMAKE_SOURCE_DIR}/libs/tests/testsuite.c
  COMMENT "Installing test sources to /Libraries/system/tests/"
)

set(C_TOOLCHAIN_DEPS
  ${SYSROOT_DIR}/Libraries/system/tests/.stamp
  ${SYSROOT_DIR}/Libraries/libc/lib/libc.a
  ${SYSROOT_DIR}/Libraries/libc/lib/crt0.o
  ${SYSROOT_DIR}/Libraries/libc/include/.stamp
  ${SYSROOT_DIR}/System/bin/cc
  ${SYSROOT_DIR}/System/bin/nasm
  ${SYSROOT_DIR}/System/bin/make
  ${SYSROOT_DIR}/Libraries/libc/lib/tcc/include/.stamp
  ${DOOM_APP}/DOOM
  ${QUAKE_APP}/Quake
  ${GLCUBE_APP}/GLCube
  ${SYSROOT_DIR}/System/bin/curl
  ${SYSROOT_DIR}/System/bin/git
  ${SYSROOT_DIR}/System/bin/sh
  ${SYSROOT_DIR}/System/bin/ssh
  ${SYSROOT_DIR}/System/bin/sshd
)

if(HAS_CLASSICUBE)
  list(APPEND C_TOOLCHAIN_DEPS ${CLASSICUBE_APP}/ClassiCube)
endif()

endif() # HAS_CROSS_COMPILER

add_custom_target(programs DEPENDS
  ${RUST_USER_BINS}
  ${SYSTEM_BINS}
  ${APP_BINS}
  ${DLL_BINS}
  ${DRIVER_BINS}
  ${SYSROOT_DIR}/.stamp
  ${SYSROOT_DIR}/Libraries/system/buildsystem/.stamp
  ${C_TOOLCHAIN_DEPS}
)

# ============================================================
# 6. Disk Image
# ============================================================
set(MKIMAGE_RESET_FLAG "")
if(ANYOS_RESET)
  set(MKIMAGE_RESET_FLAG "--reset")
endif()

add_custom_command(
  OUTPUT ${DISK_IMAGE}
  COMMAND ${MKIMAGE_EXECUTABLE}
    --stage1 ${CMAKE_BINARY_DIR}/stage1.bin
    --stage2 ${CMAKE_BINARY_DIR}/stage2.bin
    --kernel ${KERNEL_ELF}
    --output ${DISK_IMAGE}
    --image-size 256
    --sysroot ${SYSROOT_DIR}
    --fs-start 8192
    ${MKIMAGE_RESET_FLAG}
  DEPENDS
    ${CMAKE_BINARY_DIR}/stage1.bin
    ${CMAKE_BINARY_DIR}/stage2.bin
    ${KERNEL_ELF}
    ${RUST_USER_BINS}
    ${SYSTEM_BINS}
    ${APP_BINS}
    ${DLL_BINS}
    ${DRIVER_BINS}
    ${SYSROOT_DIR}/.stamp
    ${C_TOOLCHAIN_DEPS}
    ${MKIMAGE_EXECUTABLE}
  COMMENT "Creating bootable disk image (256 MiB, exFAT filesystem)"
)

# ============================================================
# Targets
# ============================================================
add_custom_target(bootloader DEPENDS
  ${CMAKE_BINARY_DIR}/stage1.bin
  ${CMAKE_BINARY_DIR}/stage2.bin
)

add_custom_target(kernel DEPENDS ${KERNEL_ELF})

add_custom_target(image ALL DEPENDS ${DISK_IMAGE} programs)

add_custom_target(run
  COMMAND ${QEMU_EXECUTABLE}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU"
)

add_custom_target(run-vmware
  COMMAND ${QEMU_EXECUTABLE}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with VMware SVGA II"
)

add_custom_target(run-vmware-debug
  COMMAND ${QEMU_EXECUTABLE}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
    -s
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with VMware SVGA II + GDB server on :1234"
)

add_custom_target(run-ahci
  COMMAND ${QEMU_EXECUTABLE}
    -drive id=hd0,if=none,format=raw,file=${DISK_IMAGE}
    -device ich9-ahci,id=ahci -device ide-hd,drive=hd0,bus=ahci.0
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with AHCI (SATA DMA)"
)

add_custom_target(run-ahci-vmware
  COMMAND ${QEMU_EXECUTABLE}
    -drive id=hd0,if=none,format=raw,file=${DISK_IMAGE}
    -device ich9-ahci,id=ahci -device ide-hd,drive=hd0,bus=ahci.0
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with AHCI + VMware SVGA II"
)

add_custom_target(run-audio
  COMMAND ${QEMU_EXECUTABLE}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -device AC97,audiodev=audio0 -audiodev coreaudio,id=audio0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with AC'97 audio"
)

add_custom_target(run-usb
  COMMAND ${QEMU_EXECUTABLE}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -usb -device usb-kbd -device usb-mouse
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with UHCI USB keyboard + mouse"
)

add_custom_target(run-usb-ehci
  COMMAND ${QEMU_EXECUTABLE}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -device ich9-usb-ehci1,id=ehci
    -device ich9-usb-uhci1,masterbus=ehci.0,firstport=0,multifunction=on
    -device usb-kbd,bus=ehci.0,port=1
    -device usb-mouse,bus=ehci.0,port=2
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with EHCI USB keyboard + mouse"
)

add_custom_target(debug
  COMMAND ${QEMU_EXECUTABLE}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -s -S
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU (debug mode, GDB on :1234)"
)

# ============================================================
# UEFI Boot Support
# ============================================================
set(UEFI_BOOTLOADER_EFI "${CMAKE_BINARY_DIR}/bootx64.efi")
set(UEFI_DISK_IMAGE "${CMAKE_BINARY_DIR}/anyos-uefi.img")
set(OVMF_FW "/opt/homebrew/share/qemu/edk2-x86_64-code.fd")

# Build UEFI bootloader
add_custom_command(
  OUTPUT ${UEFI_BOOTLOADER_EFI}
  COMMAND ${CARGO_EXECUTABLE} build --release
    --manifest-path ${CMAKE_SOURCE_DIR}/bootloader/uefi/Cargo.toml
    --target-dir ${CMAKE_BINARY_DIR}/uefi-boot
    --target x86_64-unknown-uefi
    -Zbuild-std=core,alloc
    -Zbuild-std-features=compiler-builtins-mem
  COMMAND ${CMAKE_COMMAND} -E copy
    ${CMAKE_BINARY_DIR}/uefi-boot/x86_64-unknown-uefi/release/anyos-uefi-boot.efi
    ${UEFI_BOOTLOADER_EFI}
  DEPENDS
    ${CMAKE_SOURCE_DIR}/bootloader/uefi/Cargo.toml
    ${CMAKE_SOURCE_DIR}/bootloader/uefi/src/main.rs
  COMMENT "Building UEFI bootloader"
)

# Create UEFI disk image (GPT + ESP + exFAT data partition)
add_custom_command(
  OUTPUT ${UEFI_DISK_IMAGE}
  COMMAND ${MKIMAGE_EXECUTABLE} --uefi
    --bootloader ${UEFI_BOOTLOADER_EFI}
    --kernel ${KERNEL_ELF}
    --output ${UEFI_DISK_IMAGE}
    --image-size 64
    --sysroot ${SYSROOT_DIR}
    ${MKIMAGE_RESET_FLAG}
  DEPENDS
    ${UEFI_BOOTLOADER_EFI}
    ${KERNEL_ELF}
    ${RUST_USER_BINS}
    ${SYSTEM_BINS}
    ${DLL_BINS}
    ${SYSROOT_DIR}/.stamp
    ${C_TOOLCHAIN_DEPS}
    ${MKIMAGE_EXECUTABLE}
  COMMENT "Creating UEFI bootable disk image (GPT + ESP + exFAT)"
)

add_custom_target(uefi-bootloader DEPENDS ${UEFI_BOOTLOADER_EFI})
add_custom_target(uefi-image DEPENDS ${UEFI_DISK_IMAGE} programs)

add_custom_target(run-uefi
  COMMAND ${QEMU_EXECUTABLE}
    -drive if=pflash,format=raw,readonly=on,file=${OVMF_FW}
    -drive format=raw,file=${UEFI_DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${UEFI_DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with UEFI (OVMF + VMware SVGA)"
)

add_custom_target(run-uefi-std
  COMMAND ${QEMU_EXECUTABLE}
    -drive if=pflash,format=raw,readonly=on,file=${OVMF_FW}
    -drive format=raw,file=${UEFI_DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${UEFI_DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with UEFI (OVMF + Bochs VGA)"
)

# ============================================================
# ISO 9660 Live CD Support (El Torito BIOS boot)
# ============================================================
set(ISO_IMAGE "${CMAKE_BINARY_DIR}/anyos.iso")

add_custom_command(
  OUTPUT ${ISO_IMAGE}
  COMMAND ${MKIMAGE_EXECUTABLE} --iso
    --stage1 ${CMAKE_BINARY_DIR}/stage1.bin
    --stage2 ${CMAKE_BINARY_DIR}/stage2.bin
    --kernel ${KERNEL_ELF}
    --output ${ISO_IMAGE}
    --sysroot ${SYSROOT_DIR}
  DEPENDS
    ${CMAKE_BINARY_DIR}/stage1.bin
    ${CMAKE_BINARY_DIR}/stage2.bin
    ${KERNEL_ELF}
    ${RUST_USER_BINS}
    ${SYSTEM_BINS}
    ${DLL_BINS}
    ${SYSROOT_DIR}/.stamp
    ${C_TOOLCHAIN_DEPS}
    ${MKIMAGE_EXECUTABLE}
  COMMENT "Creating bootable ISO 9660 image (El Torito, BIOS boot)"
)

add_custom_target(iso DEPENDS ${ISO_IMAGE} programs)

add_custom_target(run-cdrom
  COMMAND ${QEMU_EXECUTABLE}
    -cdrom ${ISO_IMAGE}
    -boot d
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${ISO_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS from ISO (CD-ROM boot with VMware SVGA)"
)

add_custom_target(run-cdrom-std
  COMMAND ${QEMU_EXECUTABLE}
    -cdrom ${ISO_IMAGE}
    -boot d
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${ISO_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS from ISO (CD-ROM boot with Bochs VGA)"
)

add_custom_target(run-cdrom-with-disk
  COMMAND ${QEMU_EXECUTABLE}
    -drive format=raw,file=${DISK_IMAGE}
    -cdrom ${ISO_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE} ${ISO_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS with HDD + CD-ROM (VMware SVGA)"
)

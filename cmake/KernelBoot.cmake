# ============================================================
# 1. Bootloader Stage 1 (MBR)
# ============================================================
add_custom_command(
  OUTPUT ${CMAKE_BINARY_DIR}/stage1.bin
  COMMAND ${NASM_EXECUTABLE} -w-all -f bin
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
  COMMAND ${NASM_EXECUTABLE} -w-all -f bin
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
    COMMAND ${NASM_EXECUTABLE} -w-all -f elf64
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
  COMMAND ${NASM_EXECUTABLE} -w-all -f bin
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

# ── x86_64 kernel build ──────────────────────────────────────────────
add_custom_command(
  OUTPUT ${CMAKE_BINARY_DIR}/kernel/x86_64-anyos/release/anyos_kernel.elf
  COMMAND ${CMAKE_COMMAND} -E env
    "ANYOS_ASM_OBJECTS=${KERNEL_ASM_OBJECTS_STR}"
    "ANYOS_AP_TRAMPOLINE=${AP_TRAMPOLINE_BIN}"
    "ANYOS_VERSION=${ANYOS_VERSION}"
    "RUSTFLAGS=-C force-frame-pointers=yes -Awarnings"
    ${CARGO_EXECUTABLE} build --release --quiet
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
  COMMENT "Building kernel with Cargo (x86_64)"
)

# ============================================================
# 5. ARM64 Kernel Build
# ============================================================

# ARM64 ASM stubs (assembled with clang, not NASM)
set(KERNEL_ARM64_ASM_SOURCES
  ${CMAKE_SOURCE_DIR}/kernel/asm_arm64/boot.S
  ${CMAKE_SOURCE_DIR}/kernel/asm_arm64/exceptions.S
  ${CMAKE_SOURCE_DIR}/kernel/asm_arm64/context_switch.S
  ${CMAKE_SOURCE_DIR}/kernel/asm_arm64/ap_startup.S
)

set(KERNEL_ARM64_ASM_OBJECTS "")
foreach(ASM_SRC ${KERNEL_ARM64_ASM_SOURCES})
  get_filename_component(ASM_NAME ${ASM_SRC} NAME_WE)
  set(ASM_OBJ ${CMAKE_BINARY_DIR}/kernel_arm64_${ASM_NAME}.o)
  add_custom_command(
    OUTPUT ${ASM_OBJ}
    COMMAND clang --target=aarch64-none-elf -c -o ${ASM_OBJ} ${ASM_SRC}
    DEPENDS ${ASM_SRC}
    COMMENT "Assembling kernel/asm_arm64/${ASM_NAME}.S (AArch64)"
  )
  list(APPEND KERNEL_ARM64_ASM_OBJECTS ${ASM_OBJ})
endforeach()

string(REPLACE ";" "," KERNEL_ARM64_ASM_OBJECTS_STR "${KERNEL_ARM64_ASM_OBJECTS}")

# ARM64 kernel Cargo build
add_custom_command(
  OUTPUT ${CMAKE_BINARY_DIR}/kernel/aarch64-anyos/release/anyos_kernel.elf
  COMMAND ${CMAKE_COMMAND} -E env
    "ANYOS_ASM_OBJECTS=${KERNEL_ARM64_ASM_OBJECTS_STR}"
    "ANYOS_VERSION=${ANYOS_VERSION}"
    "RUSTFLAGS=-Awarnings"
    ${CARGO_EXECUTABLE} build --release --quiet
    --manifest-path ${CMAKE_SOURCE_DIR}/kernel/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/aarch64-anyos.json
    --target-dir ${CMAKE_BINARY_DIR}/kernel
    ${CARGO_FEATURES_ARG}
  DEPENDS
    ${KERNEL_ARM64_ASM_OBJECTS}
    ${CMAKE_SOURCE_DIR}/kernel/Cargo.toml
    ${CMAKE_SOURCE_DIR}/kernel/build.rs
    ${CMAKE_SOURCE_DIR}/kernel/link_arm64.ld
    ${CMAKE_SOURCE_DIR}/aarch64-anyos.json
    ${KERNEL_RS_SOURCES}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building kernel with Cargo (AArch64)"
)

add_custom_target(kernel-arm64 DEPENDS
  ${CMAKE_BINARY_DIR}/kernel/aarch64-anyos/release/anyos_kernel.elf
)

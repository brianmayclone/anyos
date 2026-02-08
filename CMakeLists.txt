cmake_minimum_required(VERSION 3.20)
project(anyOS NONE)

# ============================================================
# Configuration
# ============================================================
set(NASM_EXECUTABLE "nasm")
set(QEMU_EXECUTABLE "qemu-system-i386")
set(CARGO_EXECUTABLE "$ENV{HOME}/.cargo/bin/cargo")

option(ANYOS_DEBUG_VERBOSE "Enable verbose debug output" OFF)

set(DISK_IMAGE "${CMAKE_BINARY_DIR}/anyos.img")
set(KERNEL_ELF "${CMAKE_BINARY_DIR}/kernel/i686-anyos/debug/anyos_kernel.elf")

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
)

set(KERNEL_ASM_OBJECTS "")
foreach(ASM_SRC ${KERNEL_ASM_SOURCES})
  get_filename_component(ASM_NAME ${ASM_SRC} NAME_WE)
  set(ASM_OBJ ${CMAKE_BINARY_DIR}/kernel_asm_${ASM_NAME}.o)
  add_custom_command(
    OUTPUT ${ASM_OBJ}
    COMMAND ${NASM_EXECUTABLE} -f elf32
      -o ${ASM_OBJ}
      ${ASM_SRC}
    DEPENDS ${ASM_SRC}
    COMMENT "Assembling kernel/${ASM_NAME}.asm"
  )
  list(APPEND KERNEL_ASM_OBJECTS ${ASM_OBJ})
endforeach()

# Build semicolon-separated list for passing to Cargo
string(REPLACE ";" "\\;" KERNEL_ASM_OBJECTS_STR "${KERNEL_ASM_OBJECTS}")

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

add_custom_command(
  OUTPUT ${KERNEL_ELF}
  COMMAND ${CMAKE_COMMAND} -E env
    "ANYOS_ASM_OBJECTS=${KERNEL_ASM_OBJECTS_STR}"
    "ANYOS_AP_TRAMPOLINE=${AP_TRAMPOLINE_BIN}"
    ${CARGO_EXECUTABLE} build
    --manifest-path ${CMAKE_SOURCE_DIR}/kernel/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/i686-anyos.json
    --target-dir ${CMAKE_BINARY_DIR}/kernel
    ${CARGO_FEATURES_ARG}
  DEPENDS
    ${KERNEL_ASM_OBJECTS}
    ${AP_TRAMPOLINE_BIN}
    ${CMAKE_SOURCE_DIR}/kernel/Cargo.toml
    ${CMAKE_SOURCE_DIR}/kernel/build.rs
    ${CMAKE_SOURCE_DIR}/kernel/link.ld
    ${CMAKE_SOURCE_DIR}/i686-anyos.json
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building kernel with Cargo"
)

# ============================================================
# 5. User Programs (flat binaries)
# ============================================================
set(SYSROOT_DIR "${CMAKE_BINARY_DIR}/sysroot")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/bin")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/system")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/system/lib")

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/bin/hello
  COMMAND ${NASM_EXECUTABLE} -f bin
    -o ${SYSROOT_DIR}/bin/hello
    ${CMAKE_SOURCE_DIR}/programs/hello.asm
  DEPENDS ${CMAKE_SOURCE_DIR}/programs/hello.asm
  COMMENT "Assembling user program: hello"
)

# --- Helper: build a Rust user program ---
set(STDLIB_DEPS
  ${CMAKE_SOURCE_DIR}/stdlib/Cargo.toml
  ${CMAKE_SOURCE_DIR}/stdlib/src/lib.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/raw.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/dll.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/process.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/fs.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/sys.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/net.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/ipc.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/io.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/heap.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/ui/mod.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/ui/window.rs
  ${CMAKE_SOURCE_DIR}/stdlib/link.ld
  ${CMAKE_SOURCE_DIR}/i686-anyos.json
)

function(add_rust_user_program NAME SRC_DIR)
  set(ELF "${CMAKE_BINARY_DIR}/programs/${NAME}/i686-anyos/debug/${NAME}.elf")
  add_custom_command(
    OUTPUT ${ELF}
    COMMAND ${CARGO_EXECUTABLE} build
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/i686-anyos.json
      --target-dir ${CMAKE_BINARY_DIR}/programs/${NAME}
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${SRC_DIR}/build.rs
      ${SRC_DIR}/src/main.rs
      ${STDLIB_DEPS}
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building Rust user program: ${NAME}"
  )
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/bin/${NAME}
    COMMAND python3 ${CMAKE_SOURCE_DIR}/tools/elf2bin.py
      ${ELF}
      ${SYSROOT_DIR}/bin/${NAME}
    DEPENDS ${ELF}
    COMMENT "Converting ${NAME} ELF to flat binary"
  )
  set(RUST_USER_BINS ${RUST_USER_BINS} ${SYSROOT_DIR}/bin/${NAME} PARENT_SCOPE)
endfunction()

# Variant for system programs (placed in /system/ instead of /bin/)
function(add_rust_system_program NAME SRC_DIR)
  set(ELF "${CMAKE_BINARY_DIR}/programs/${NAME}/i686-anyos/debug/${NAME}.elf")
  add_custom_command(
    OUTPUT ${ELF}
    COMMAND ${CARGO_EXECUTABLE} build
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/i686-anyos.json
      --target-dir ${CMAKE_BINARY_DIR}/programs/${NAME}
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${SRC_DIR}/build.rs
      ${SRC_DIR}/src/main.rs
      ${STDLIB_DEPS}
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building system program: ${NAME}"
  )
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/system/${NAME}
    COMMAND python3 ${CMAKE_SOURCE_DIR}/tools/elf2bin.py
      ${ELF}
      ${SYSROOT_DIR}/system/${NAME}
    DEPENDS ${ELF}
    COMMENT "Converting ${NAME} ELF to flat binary"
  )
  set(SYSTEM_BINS ${SYSTEM_BINS} ${SYSROOT_DIR}/system/${NAME} PARENT_SCOPE)
endfunction()

# Variant for DLLs (placed in /system/lib/)
function(add_dll NAME SRC_DIR)
  set(DLL_ELF "${CMAKE_BINARY_DIR}/dll/${NAME}/i686-anyos/debug/${NAME}.elf")
  add_custom_command(
    OUTPUT ${DLL_ELF}
    COMMAND ${CARGO_EXECUTABLE} build
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/i686-anyos.json
      --target-dir ${CMAKE_BINARY_DIR}/dll/${NAME}
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${SRC_DIR}/build.rs
      ${SRC_DIR}/src/main.rs
      ${SRC_DIR}/link.ld
      ${CMAKE_SOURCE_DIR}/i686-anyos.json
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building DLL: ${NAME}"
  )
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/system/lib/${NAME}.dll
    COMMAND python3 ${CMAKE_SOURCE_DIR}/tools/elf2bin.py
      ${DLL_ELF}
      ${SYSROOT_DIR}/system/lib/${NAME}.dll
    DEPENDS ${DLL_ELF}
    COMMENT "Converting ${NAME} ELF to DLL flat binary"
  )
  set(DLL_BINS ${DLL_BINS} ${SYSROOT_DIR}/system/lib/${NAME}.dll PARENT_SCOPE)
endfunction()

set(DLL_BINS "")
add_dll(uisys ${CMAKE_SOURCE_DIR}/programs/dll/uisys)

set(RUST_USER_BINS "")
add_rust_user_program(hello_rust ${CMAKE_SOURCE_DIR}/programs/hello_rust)
add_rust_user_program(ping       ${CMAKE_SOURCE_DIR}/programs/ping)
add_rust_user_program(dhcp       ${CMAKE_SOURCE_DIR}/programs/dhcp)
add_rust_user_program(dns        ${CMAKE_SOURCE_DIR}/programs/dns)
add_rust_user_program(ls         ${CMAKE_SOURCE_DIR}/programs/ls)
add_rust_user_program(cat        ${CMAKE_SOURCE_DIR}/programs/cat)
add_rust_user_program(ifconfig   ${CMAKE_SOURCE_DIR}/programs/ifconfig)
add_rust_user_program(arp        ${CMAKE_SOURCE_DIR}/programs/arp)
add_rust_user_program(sysinfo    ${CMAKE_SOURCE_DIR}/programs/sysinfo)
add_rust_user_program(dmesg      ${CMAKE_SOURCE_DIR}/programs/dmesg)

set(SYSTEM_BINS "")
add_rust_system_program(init        ${CMAKE_SOURCE_DIR}/programs/system/init)
add_rust_system_program(terminal    ${CMAKE_SOURCE_DIR}/programs/system/terminal)
add_rust_system_program(taskmanager ${CMAKE_SOURCE_DIR}/programs/system/taskmanager)
add_rust_system_program(settings    ${CMAKE_SOURCE_DIR}/programs/system/settings)
add_rust_system_program(finder      ${CMAKE_SOURCE_DIR}/programs/system/finder)

# Build dock program (nested path: /system/compositor/dock)
set(DOCK_SRC_DIR ${CMAKE_SOURCE_DIR}/programs/system/compositor/dock)
set(DOCK_ELF "${CMAKE_BINARY_DIR}/programs/dock/i686-anyos/debug/dock.elf")
add_custom_command(
  OUTPUT ${DOCK_ELF}
  COMMAND ${CARGO_EXECUTABLE} build
    --manifest-path ${DOCK_SRC_DIR}/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/i686-anyos.json
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
  OUTPUT ${SYSROOT_DIR}/system/compositor/dock
  COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/system/compositor
  COMMAND python3 ${CMAKE_SOURCE_DIR}/tools/elf2bin.py
    ${DOCK_ELF}
    ${SYSROOT_DIR}/system/compositor/dock
  DEPENDS ${DOCK_ELF}
  COMMENT "Converting dock ELF to flat binary"
)
list(APPEND SYSTEM_BINS ${SYSROOT_DIR}/system/compositor/dock)

# Also place taskmanager and finder in /bin/ so the terminal shell can launch them
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/bin/taskmanager
  COMMAND ${CMAKE_COMMAND} -E copy
    ${SYSROOT_DIR}/system/taskmanager
    ${SYSROOT_DIR}/bin/taskmanager
  DEPENDS ${SYSROOT_DIR}/system/taskmanager
  COMMENT "Copying taskmanager to /bin/"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/bin/taskmanager)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/bin/finder
  COMMAND ${CMAKE_COMMAND} -E copy
    ${SYSROOT_DIR}/system/finder
    ${SYSROOT_DIR}/bin/finder
  DEPENDS ${SYSROOT_DIR}/system/finder
  COMMENT "Copying finder to /bin/"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/bin/finder)

# Copy any extra sysroot files from tools/sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/.stamp
  COMMAND ${CMAKE_COMMAND} -E copy_directory
    ${CMAKE_SOURCE_DIR}/tools/sysroot
    ${SYSROOT_DIR}
  COMMAND ${CMAKE_COMMAND} -E touch ${SYSROOT_DIR}/.stamp
  DEPENDS ${CMAKE_SOURCE_DIR}/tools/sysroot
  COMMENT "Populating sysroot from tools/sysroot"
)

# Generate dock icons
set(ICONS_DIR ${SYSROOT_DIR}/system/icons)
add_custom_command(
  OUTPUT ${ICONS_DIR}/terminal.icon ${ICONS_DIR}/taskmanager.icon ${ICONS_DIR}/settings.icon ${ICONS_DIR}/finder.icon
  COMMAND python3 ${CMAKE_SOURCE_DIR}/tools/gen_icons.py ${ICONS_DIR}
  DEPENDS ${CMAKE_SOURCE_DIR}/tools/gen_icons.py
  COMMENT "Generating dock icons"
)

add_custom_target(programs DEPENDS
  ${SYSROOT_DIR}/bin/hello
  ${RUST_USER_BINS}
  ${SYSTEM_BINS}
  ${DLL_BINS}
  ${SYSROOT_DIR}/.stamp
  ${ICONS_DIR}/terminal.icon
)

# ============================================================
# 6. Disk Image
# ============================================================
add_custom_command(
  OUTPUT ${DISK_IMAGE}
  COMMAND python3 ${CMAKE_SOURCE_DIR}/tools/mkimage.py
    --stage1 ${CMAKE_BINARY_DIR}/stage1.bin
    --stage2 ${CMAKE_BINARY_DIR}/stage2.bin
    --kernel ${KERNEL_ELF}
    --output ${DISK_IMAGE}
    --image-size 64
    --sysroot ${SYSROOT_DIR}
    --fs-start 4096
  DEPENDS
    ${CMAKE_BINARY_DIR}/stage1.bin
    ${CMAKE_BINARY_DIR}/stage2.bin
    ${KERNEL_ELF}
    ${SYSROOT_DIR}/bin/hello
    ${RUST_USER_BINS}
    ${SYSTEM_BINS}
    ${DLL_BINS}
    ${SYSROOT_DIR}/.stamp
  COMMENT "Creating bootable disk image (64 MiB, FAT16 filesystem)"
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
    -m 128M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU"
)

add_custom_target(debug
  COMMAND ${QEMU_EXECUTABLE}
    -drive format=raw,file=${DISK_IMAGE}
    -m 128M
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

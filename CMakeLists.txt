cmake_minimum_required(VERSION 3.20)
project(anyOS NONE)

# ============================================================
# Configuration
# ============================================================
set(NASM_EXECUTABLE "nasm")
set(QEMU_EXECUTABLE "qemu-system-x86_64")
set(CARGO_EXECUTABLE "$ENV{HOME}/.cargo/bin/cargo")

option(ANYOS_DEBUG_VERBOSE "Enable verbose debug output" OFF)

# Auto-detect i686-elf cross-compiler (needed for libc, TCC, Doom, Quake)
find_program(I686_ELF_GCC NAMES i686-elf-gcc i386-elf-gcc)
if(I686_ELF_GCC)
  set(HAS_CROSS_COMPILER TRUE)
  message(STATUS "Found cross-compiler: ${I686_ELF_GCC}")
else()
  set(HAS_CROSS_COMPILER FALSE)
  message(STATUS "i686-elf-gcc not found — skipping C library, TCC, and games (Rust-only build)")
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
file(MAKE_DIRECTORY "${SYSROOT_DIR}/bin")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/system")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/system/lib")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/include")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/include/sys")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/lib")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/src")

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
  ${CMAKE_SOURCE_DIR}/stdlib/src/anim.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/icons.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/ui/mod.rs
  ${CMAKE_SOURCE_DIR}/stdlib/src/ui/window.rs
  ${CMAKE_SOURCE_DIR}/stdlib/link.ld
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
add_dll(libimage ${CMAKE_SOURCE_DIR}/programs/dll/libimage)
add_dll(libfont ${CMAKE_SOURCE_DIR}/programs/dll/libfont)
add_dll(librender ${CMAKE_SOURCE_DIR}/programs/dll/librender)
add_dll(libcompositor ${CMAKE_SOURCE_DIR}/programs/dll/libcompositor)

set(RUST_USER_BINS "")
add_rust_user_program(ping       ${CMAKE_SOURCE_DIR}/programs/ping)
add_rust_user_program(dhcp       ${CMAKE_SOURCE_DIR}/programs/dhcp)
add_rust_user_program(dns        ${CMAKE_SOURCE_DIR}/programs/dns)
add_rust_user_program(ls         ${CMAKE_SOURCE_DIR}/programs/ls)
add_rust_user_program(cat        ${CMAKE_SOURCE_DIR}/programs/cat)
add_rust_user_program(ifconfig   ${CMAKE_SOURCE_DIR}/programs/ifconfig)
add_rust_user_program(arp        ${CMAKE_SOURCE_DIR}/programs/arp)
add_rust_user_program(sysinfo    ${CMAKE_SOURCE_DIR}/programs/sysinfo)
add_rust_user_program(dmesg      ${CMAKE_SOURCE_DIR}/programs/dmesg)
add_rust_user_program(mkdir      ${CMAKE_SOURCE_DIR}/programs/mkdir)
add_rust_user_program(rm         ${CMAKE_SOURCE_DIR}/programs/rm)
add_rust_user_program(touch      ${CMAKE_SOURCE_DIR}/programs/touch)
add_rust_user_program(cp         ${CMAKE_SOURCE_DIR}/programs/cp)
add_rust_user_program(mv         ${CMAKE_SOURCE_DIR}/programs/mv)
add_rust_user_program(date       ${CMAKE_SOURCE_DIR}/programs/date)
add_rust_user_program(sleep      ${CMAKE_SOURCE_DIR}/programs/sleep)
add_rust_user_program(hostname   ${CMAKE_SOURCE_DIR}/programs/hostname)
add_rust_user_program(ftp        ${CMAKE_SOURCE_DIR}/programs/ftp)
add_rust_user_program(wget       ${CMAKE_SOURCE_DIR}/programs/wget)
add_rust_user_program(play       ${CMAKE_SOURCE_DIR}/programs/play)
add_rust_user_program(pipes      ${CMAKE_SOURCE_DIR}/programs/pipes)
add_rust_user_program(devlist    ${CMAKE_SOURCE_DIR}/programs/devlist)
add_rust_user_program(echo       ${CMAKE_SOURCE_DIR}/programs/echo)
add_rust_user_program(ps         ${CMAKE_SOURCE_DIR}/programs/ps)
add_rust_user_program(kill       ${CMAKE_SOURCE_DIR}/programs/kill)
add_rust_user_program(free       ${CMAKE_SOURCE_DIR}/programs/free)
add_rust_user_program(uptime     ${CMAKE_SOURCE_DIR}/programs/uptime)
add_rust_user_program(uname      ${CMAKE_SOURCE_DIR}/programs/uname)
add_rust_user_program(pwd        ${CMAKE_SOURCE_DIR}/programs/pwd)
add_rust_user_program(wc         ${CMAKE_SOURCE_DIR}/programs/wc)
add_rust_user_program(hexdump    ${CMAKE_SOURCE_DIR}/programs/hexdump)
add_rust_user_program(head       ${CMAKE_SOURCE_DIR}/programs/head)
add_rust_user_program(tail       ${CMAKE_SOURCE_DIR}/programs/tail)
add_rust_user_program(clear      ${CMAKE_SOURCE_DIR}/programs/clear)
add_rust_user_program(env        ${CMAKE_SOURCE_DIR}/programs/env)
add_rust_user_program(grep       ${CMAKE_SOURCE_DIR}/programs/grep)
add_rust_user_program(find       ${CMAKE_SOURCE_DIR}/programs/find)
add_rust_user_program(sort       ${CMAKE_SOURCE_DIR}/programs/sort)
add_rust_user_program(uniq       ${CMAKE_SOURCE_DIR}/programs/uniq)
add_rust_user_program(rev        ${CMAKE_SOURCE_DIR}/programs/rev)
add_rust_user_program(stat       ${CMAKE_SOURCE_DIR}/programs/stat)
add_rust_user_program(df         ${CMAKE_SOURCE_DIR}/programs/df)
add_rust_user_program(cal        ${CMAKE_SOURCE_DIR}/programs/cal)
add_rust_user_program(seq        ${CMAKE_SOURCE_DIR}/programs/seq)
add_rust_user_program(yes        ${CMAKE_SOURCE_DIR}/programs/yes)
add_rust_user_program(whoami     ${CMAKE_SOURCE_DIR}/programs/whoami)
add_rust_user_program(which      ${CMAKE_SOURCE_DIR}/programs/which)
add_rust_user_program(strings    ${CMAKE_SOURCE_DIR}/programs/strings)
add_rust_user_program(base64     ${CMAKE_SOURCE_DIR}/programs/base64)
add_rust_user_program(xxd        ${CMAKE_SOURCE_DIR}/programs/xxd)
add_rust_user_program(set        ${CMAKE_SOURCE_DIR}/programs/set)
add_rust_user_program(export     ${CMAKE_SOURCE_DIR}/programs/export)

# true/false: package names are true_cmd/false_cmd (Rust keywords) but binaries named true/false
set(TRUE_ELF "${CMAKE_BINARY_DIR}/programs/true_cmd/x86_64-anyos-user/release/true_cmd.elf")
add_custom_command(
  OUTPUT ${TRUE_ELF}
  COMMAND ${CARGO_EXECUTABLE} build --release
    --manifest-path ${CMAKE_SOURCE_DIR}/programs/true/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
    --target-dir ${CMAKE_BINARY_DIR}/programs/true_cmd
  DEPENDS
    ${CMAKE_SOURCE_DIR}/programs/true/Cargo.toml
    ${CMAKE_SOURCE_DIR}/programs/true/build.rs
    ${CMAKE_SOURCE_DIR}/programs/true/src/main.rs
    ${STDLIB_DEPS}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building Rust user program: true"
)
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/bin/true
  COMMAND python3 ${CMAKE_SOURCE_DIR}/tools/elf2bin.py
    ${TRUE_ELF}
    ${SYSROOT_DIR}/bin/true
  DEPENDS ${TRUE_ELF}
  COMMENT "Converting true ELF to flat binary"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/bin/true)

set(FALSE_ELF "${CMAKE_BINARY_DIR}/programs/false_cmd/x86_64-anyos-user/release/false_cmd.elf")
add_custom_command(
  OUTPUT ${FALSE_ELF}
  COMMAND ${CARGO_EXECUTABLE} build --release
    --manifest-path ${CMAKE_SOURCE_DIR}/programs/false/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
    --target-dir ${CMAKE_BINARY_DIR}/programs/false_cmd
  DEPENDS
    ${CMAKE_SOURCE_DIR}/programs/false/Cargo.toml
    ${CMAKE_SOURCE_DIR}/programs/false/build.rs
    ${CMAKE_SOURCE_DIR}/programs/false/src/main.rs
    ${STDLIB_DEPS}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building Rust user program: false"
)
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/bin/false
  COMMAND python3 ${CMAKE_SOURCE_DIR}/tools/elf2bin.py
    ${FALSE_ELF}
    ${SYSROOT_DIR}/bin/false
  DEPENDS ${FALSE_ELF}
  COMMENT "Converting false ELF to flat binary"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/bin/false)

set(SYSTEM_BINS "")
add_rust_system_program(init        ${CMAKE_SOURCE_DIR}/programs/system/init)
add_rust_system_program(terminal    ${CMAKE_SOURCE_DIR}/programs/system/terminal)
add_rust_system_program(taskmanager ${CMAKE_SOURCE_DIR}/programs/system/taskmanager)
add_rust_system_program(settings    ${CMAKE_SOURCE_DIR}/programs/system/settings)
add_rust_system_program(finder      ${CMAKE_SOURCE_DIR}/programs/system/finder)
add_rust_system_program(notepad     ${CMAKE_SOURCE_DIR}/programs/system/notepad)
add_rust_system_program(imgview     ${CMAKE_SOURCE_DIR}/programs/system/imgview)
add_rust_system_program(videoplayer ${CMAKE_SOURCE_DIR}/programs/system/videoplayer)
add_rust_system_program(diagnostics ${CMAKE_SOURCE_DIR}/programs/system/diagnostics)
add_rust_system_program(calc        ${CMAKE_SOURCE_DIR}/programs/system/calc)
add_rust_system_program(fontviewer  ${CMAKE_SOURCE_DIR}/programs/system/fontviewer)
add_rust_system_program(clock       ${CMAKE_SOURCE_DIR}/programs/system/clock)
add_rust_system_program(screenshot  ${CMAKE_SOURCE_DIR}/programs/system/screenshot)
add_rust_system_program(netmon      ${CMAKE_SOURCE_DIR}/programs/system/netmon)
add_rust_system_program(audiomon    ${CMAKE_SOURCE_DIR}/programs/system/audiomon)

# Build compositor binary (nested path: /system/compositor/compositor)
set(COMPOSITOR_SRC_DIR ${CMAKE_SOURCE_DIR}/programs/system/compositor/compositor)
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
  OUTPUT ${SYSROOT_DIR}/system/compositor/compositor
  COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/system/compositor
  COMMAND python3 ${CMAKE_SOURCE_DIR}/tools/elf2bin.py
    ${COMPOSITOR_ELF}
    ${SYSROOT_DIR}/system/compositor/compositor
  DEPENDS ${COMPOSITOR_ELF}
  COMMENT "Converting compositor ELF to flat binary"
)
list(APPEND SYSTEM_BINS ${SYSROOT_DIR}/system/compositor/compositor)

# Build dock program (nested path: /system/compositor/dock)
set(DOCK_SRC_DIR ${CMAKE_SOURCE_DIR}/programs/system/compositor/dock)
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
          ${CMAKE_SOURCE_DIR}/tools/sysroot/src/hello.c
          ${CMAKE_SOURCE_DIR}/tools/sysroot/system/fonts/sfpro.ttf
          ${CMAKE_SOURCE_DIR}/tools/sysroot/system/fonts/sfpro-bold.ttf
          ${CMAKE_SOURCE_DIR}/tools/sysroot/system/fonts/sfpro-thin.ttf
          ${CMAKE_SOURCE_DIR}/tools/sysroot/system/fonts/sfpro-italic.ttf
          ${CMAKE_SOURCE_DIR}/tools/sysroot/media/wallpapers/default.png
          ${CMAKE_SOURCE_DIR}/tools/sysroot/system/media/icons
  COMMENT "Populating sysroot from tools/sysroot"
)

# ============================================================
# C Library, TCC, and Games (require i686-elf cross-compiler)
# ============================================================
if(HAS_CROSS_COMPILER)
# ── C Library (libc.a + crt0.o for C programs) ──
set(LIBC_DIR "${CMAKE_SOURCE_DIR}/programs/libc")
set(LIBC_A "${LIBC_DIR}/libc.a")
set(LIBC_CRT0 "${LIBC_DIR}/obj/crt0.o")

add_custom_command(
  OUTPUT ${LIBC_A} ${LIBC_CRT0}
  COMMAND make -C ${LIBC_DIR} clean
  COMMAND make -C ${LIBC_DIR}
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
  COMMENT "Building C library (libc.a + crt0.o)"
)

# Create empty crti.o, crtn.o stubs and empty libtcc1.a for TCC
set(LIBC_CRTI "${LIBC_DIR}/obj/crti.o")
set(LIBC_CRTN "${LIBC_DIR}/obj/crtn.o")
set(LIBC_LIBTCC1 "${LIBC_DIR}/libtcc1.a")
add_custom_command(
  OUTPUT ${LIBC_CRTI} ${LIBC_CRTN} ${LIBC_LIBTCC1}
  COMMAND i686-elf-gcc -m32 -c ${LIBC_DIR}/src/crti.S -o ${LIBC_CRTI}
  COMMAND i686-elf-gcc -m32 -c ${LIBC_DIR}/src/crtn.S -o ${LIBC_CRTN}
  COMMAND i686-elf-ar rcs ${LIBC_LIBTCC1}
  DEPENDS ${LIBC_A} ${LIBC_DIR}/src/crti.S ${LIBC_DIR}/src/crtn.S
  COMMENT "Creating CRT stubs and libtcc1.a for TCC"
)

# Copy libc artifacts to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/lib/libc.a ${SYSROOT_DIR}/lib/crt0.o
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_A} ${SYSROOT_DIR}/lib/libc.a
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_CRT0} ${SYSROOT_DIR}/lib/crt0.o
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_CRT0} ${SYSROOT_DIR}/lib/crt1.o
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_CRTI} ${SYSROOT_DIR}/lib/crti.o
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_CRTN} ${SYSROOT_DIR}/lib/crtn.o
  DEPENDS ${LIBC_A} ${LIBC_CRT0} ${LIBC_CRTI} ${LIBC_CRTN}
  COMMENT "Installing libc.a and crt0.o to sysroot"
)

# Copy libc headers to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/include/.stamp
  COMMAND ${CMAKE_COMMAND} -E copy_directory ${LIBC_DIR}/include ${SYSROOT_DIR}/include
  COMMAND ${CMAKE_COMMAND} -E touch ${SYSROOT_DIR}/include/.stamp
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
  "-DCONFIG_TCCDIR=\"/lib/tcc\""
  "-DCONFIG_TCC_SYSINCLUDEPATHS=\"/include\""
  "-DCONFIG_TCC_LIBPATHS=\"/lib\""
  "-DCONFIG_TCC_CRTPREFIX=\"/lib\""
  "-DCONFIG_TCC_ELFINTERP=\"\""
  "-DTCC_VERSION=\"0.9.27\""
)

add_custom_command(
  OUTPUT ${TCC_OBJ}
  COMMAND i686-elf-gcc
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
  COMMAND i686-elf-gcc
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
  OUTPUT ${SYSROOT_DIR}/bin/cc
  COMMAND ${CMAKE_COMMAND} -E copy ${TCC_ELF} ${SYSROOT_DIR}/bin/cc
  DEPENDS ${TCC_ELF}
  COMMENT "Installing TCC as /bin/cc"
)

# Copy TCC internal headers
file(MAKE_DIRECTORY "${SYSROOT_DIR}/lib/tcc/include")
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/lib/tcc/include/.stamp
  COMMAND ${CMAKE_COMMAND} -E copy_directory ${TCC_DIR}/include ${SYSROOT_DIR}/lib/tcc/include
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_DIR}/link.ld ${SYSROOT_DIR}/lib/tcc/link.ld
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC_LIBTCC1} ${SYSROOT_DIR}/lib/tcc/libtcc1.a
  COMMAND ${CMAKE_COMMAND} -E touch ${SYSROOT_DIR}/lib/tcc/include/.stamp
  DEPENDS ${TCC_ELF} ${LIBC_LIBTCC1}
  COMMENT "Installing TCC headers to sysroot"
)

# ============================================================
# DOOM (doomgeneric, cross-compiled C program)
# ============================================================
set(DOOM_DIR "${CMAKE_SOURCE_DIR}/third_party/doom")
set(DOOM_ELF "${DOOM_DIR}/doom.elf")

add_custom_command(
  OUTPUT ${DOOM_ELF}
  COMMAND make -C ${DOOM_DIR} clean
  COMMAND make -C ${DOOM_DIR} LIBC_DIR=${LIBC_DIR}
  DEPENDS ${LIBC_A} ${LIBC_CRT0}
  COMMENT "Building DOOM for anyOS"
)

set(DOOM_WAD "${CMAKE_SOURCE_DIR}/tools/sysroot/apps/doom/doom.wad")

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/apps/doom/doom ${SYSROOT_DIR}/apps/doom/doom.wad
  COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/apps/doom
  COMMAND ${CMAKE_COMMAND} -E copy ${DOOM_ELF} ${SYSROOT_DIR}/apps/doom/doom
  COMMAND ${CMAKE_COMMAND} -E copy ${DOOM_WAD} ${SYSROOT_DIR}/apps/doom/doom.wad
  DEPENDS ${DOOM_ELF} ${DOOM_WAD}
  COMMENT "Installing DOOM + WAD to sysroot"
)

# Quake (WinQuake software renderer, cross-compiled C program)
# ============================================================
set(QUAKE_DIR "${CMAKE_SOURCE_DIR}/third_party/quake")
set(QUAKE_ELF "${QUAKE_DIR}/quake.elf")

add_custom_command(
  OUTPUT ${QUAKE_ELF}
  COMMAND make -C ${QUAKE_DIR} clean
  COMMAND make -C ${QUAKE_DIR} LIBC_DIR=${LIBC_DIR}
  DEPENDS ${LIBC_A} ${LIBC_CRT0}
  COMMENT "Building Quake for anyOS"
)

set(QUAKE_PAK "${CMAKE_SOURCE_DIR}/tools/sysroot/apps/quake/id1/pak0.pak")
set(QUAKE_CFG "${CMAKE_SOURCE_DIR}/tools/sysroot/apps/quake/id1/config.cfg")

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/apps/quake/quake ${SYSROOT_DIR}/apps/quake/id1/pak0.pak
  COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/apps/quake/id1
  COMMAND ${CMAKE_COMMAND} -E copy ${QUAKE_ELF} ${SYSROOT_DIR}/apps/quake/quake
  COMMAND ${CMAKE_COMMAND} -E copy ${QUAKE_PAK} ${SYSROOT_DIR}/apps/quake/id1/pak0.pak
  COMMAND ${CMAKE_COMMAND} -E copy ${QUAKE_CFG} ${SYSROOT_DIR}/apps/quake/id1/config.cfg
  DEPENDS ${QUAKE_ELF} ${QUAKE_PAK} ${QUAKE_CFG}
  COMMENT "Installing Quake + PAK to sysroot"
)

# Quake 2 (id Software GPL, cross-compiled C program with ref_soft)
# ============================================================
set(QUAKE2_DIR "${CMAKE_SOURCE_DIR}/third_party/quake2")
set(QUAKE2_ELF "${QUAKE2_DIR}/quake2.elf")

add_custom_command(
  OUTPUT ${QUAKE2_ELF}
  COMMAND make -C ${QUAKE2_DIR} -f Makefile.anyos clean
  COMMAND make -C ${QUAKE2_DIR} -f Makefile.anyos LIBC_DIR=${LIBC_DIR}
  DEPENDS ${LIBC_A} ${LIBC_CRT0}
  COMMENT "Building Quake 2 for anyOS"
)

set(QUAKE2_PAK "${CMAKE_SOURCE_DIR}/tools/sysroot/apps/quake2/baseq2/pak0.pak")

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/apps/quake2/quake2 ${SYSROOT_DIR}/apps/quake2/baseq2/pak0.pak
  COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/apps/quake2/baseq2
  COMMAND ${CMAKE_COMMAND} -E copy ${QUAKE2_ELF} ${SYSROOT_DIR}/apps/quake2/quake2
  COMMAND ${CMAKE_COMMAND} -E copy ${QUAKE2_PAK} ${SYSROOT_DIR}/apps/quake2/baseq2/pak0.pak
  DEPENDS ${QUAKE2_ELF} ${QUAKE2_PAK}
  COMMENT "Installing Quake 2 + PAK to sysroot"
)

set(C_TOOLCHAIN_DEPS
  ${SYSROOT_DIR}/lib/libc.a
  ${SYSROOT_DIR}/lib/crt0.o
  ${SYSROOT_DIR}/include/.stamp
  ${SYSROOT_DIR}/bin/cc
  ${SYSROOT_DIR}/lib/tcc/include/.stamp
  ${SYSROOT_DIR}/apps/doom/doom
  ${SYSROOT_DIR}/apps/quake/quake
  ${SYSROOT_DIR}/apps/quake2/quake2
)

endif() # HAS_CROSS_COMPILER

add_custom_target(programs DEPENDS
  ${RUST_USER_BINS}
  ${SYSTEM_BINS}
  ${DLL_BINS}
  ${SYSROOT_DIR}/.stamp
  ${C_TOOLCHAIN_DEPS}
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
    --image-size 128
    --sysroot ${SYSROOT_DIR}
    --fs-start 8192
  DEPENDS
    ${CMAKE_BINARY_DIR}/stage1.bin
    ${CMAKE_BINARY_DIR}/stage2.bin
    ${KERNEL_ELF}
    ${RUST_USER_BINS}
    ${SYSTEM_BINS}
    ${DLL_BINS}
    ${SYSROOT_DIR}/.stamp
    ${C_TOOLCHAIN_DEPS}
  COMMENT "Creating bootable disk image (128 MiB, FAT16 filesystem)"
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

# Create UEFI disk image (GPT + ESP + FAT16 data partition)
add_custom_command(
  OUTPUT ${UEFI_DISK_IMAGE}
  COMMAND python3 ${CMAKE_SOURCE_DIR}/tools/mkimage.py --uefi
    --bootloader ${UEFI_BOOTLOADER_EFI}
    --kernel ${KERNEL_ELF}
    --output ${UEFI_DISK_IMAGE}
    --image-size 64
    --sysroot ${SYSROOT_DIR}
  DEPENDS
    ${UEFI_BOOTLOADER_EFI}
    ${KERNEL_ELF}
    ${RUST_USER_BINS}
    ${SYSTEM_BINS}
    ${DLL_BINS}
    ${SYSROOT_DIR}/.stamp
    ${C_TOOLCHAIN_DEPS}
  COMMENT "Creating UEFI bootable disk image (GPT + ESP + FAT16)"
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

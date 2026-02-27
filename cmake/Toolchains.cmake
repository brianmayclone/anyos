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
get_filename_component(MSYS_USR_BIN "${MAKE_EXECUTABLE}" DIRECTORY)

# On Windows, cross-compiler binaries (MSYS2) need msys-2.0.dll in PATH.
# Generate a wrapper script that prepends the cross-compiler and MSYS2 dirs to PATH.
if(WIN32)
  set(CROSS_WRAPPER "${CMAKE_BINARY_DIR}/cross_env.cmd")
  file(WRITE "${CROSS_WRAPPER}" "@set \"PATH=${CROSS_BIN_DIR};${MSYS_USR_BIN};%PATH%\"\n@%*\n")
  set(CROSS_ENV "${CROSS_WRAPPER}")
else()
  set(CROSS_ENV "")
endif()

add_custom_command(
  OUTPUT ${LIBC_A} ${LIBC_CRT0}
  COMMAND ${CROSS_ENV} ${MAKE_EXECUTABLE} -s -C ${LIBC_DIR} clean CC=${I686_ELF_GCC} AR=${I686_ELF_AR} AS=${I686_ELF_GCC}
  COMMAND ${CROSS_ENV} ${MAKE_EXECUTABLE} -s -j${NPROC} -C ${LIBC_DIR} CC=${I686_ELF_GCC} AR=${I686_ELF_AR} AS=${I686_ELF_GCC}
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
  COMMAND ${CROSS_ENV} ${I686_ELF_GCC} -m32 -c ${LIBC_DIR}/src/crti.S -o ${LIBC_CRTI}
  COMMAND ${CROSS_ENV} ${I686_ELF_GCC} -m32 -c ${LIBC_DIR}/src/crtn.S -o ${LIBC_CRTN}
  COMMAND ${CROSS_ENV} ${I686_ELF_AR} rcs ${LIBC_LIBTCC1}
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
  COMMAND ${CROSS_ENV} ${I686_ELF_GCC}
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
  COMMAND ${CROSS_ENV} ${I686_ELF_GCC}
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
  COMMAND ${CROSS_ENV} ${I686_ELF_GCC}
    -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector
    -I${LIBC_DIR}/include -O2 -m32 -w
    -c ${MAKE_SRC} -o ${MAKE_OBJ}
  DEPENDS ${MAKE_SRC} ${LIBC_A}
  COMMENT "Compiling make for anyOS"
)

add_custom_command(
  OUTPUT ${MAKE_ELF}
  COMMAND ${CROSS_ENV} ${I686_ELF_GCC}
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
  COMMAND ${CROSS_ENV} ${MAKE_EXECUTABLE} -s -C ${DOOM_DIR} clean CC=${I686_ELF_GCC} AR=${I686_ELF_AR} AS=${I686_ELF_GCC}
  COMMAND ${CROSS_ENV} ${MAKE_EXECUTABLE} -s -j${NPROC} -C ${DOOM_DIR} CC=${I686_ELF_GCC} AR=${I686_ELF_AR} AS=${I686_ELF_GCC} LIBC_DIR=${LIBC_DIR}
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
  COMMAND ${CROSS_ENV} ${MAKE_EXECUTABLE} -s -C ${QUAKE_DIR} clean CC=${I686_ELF_GCC} AR=${I686_ELF_AR} AS=${I686_ELF_GCC}
  COMMAND ${CROSS_ENV} ${MAKE_EXECUTABLE} -s -j${NPROC} -C ${QUAKE_DIR} CC=${I686_ELF_GCC} AR=${I686_ELF_AR} AS=${I686_ELF_GCC} LIBC_DIR=${LIBC_DIR}
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
    COMMAND ${CROSS_ENV} ${MAKE_EXECUTABLE} -s -C ${CLASSICUBE_DIR} -f misc/anyos/Makefile clean
    COMMAND ${CROSS_ENV} ${MAKE_EXECUTABLE} -s -j${NPROC} -C ${CLASSICUBE_DIR} -f misc/anyos/Makefile CC=${I686_ELF_GCC} LIBC_DIR=${LIBC_DIR}
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
if(WIN32)
  add_custom_command(
    OUTPUT ${BEARSSL_A}
    COMMAND ${CROSS_ENV} powershell -ExecutionPolicy Bypass -File ${CMAKE_SOURCE_DIR}/scripts/build_bearssl.ps1
    DEPENDS ${LIBC_A}
    COMMENT "Building BearSSL for anyOS"
  )
else()
  add_custom_command(
    OUTPUT ${BEARSSL_A}
    COMMAND bash ${CMAKE_SOURCE_DIR}/scripts/build_bearssl.sh
    DEPENDS ${LIBC_A}
    COMMENT "Building BearSSL for anyOS"
  )
endif()

# ============================================================
# curl (HTTP/FTP client, cross-compiled C program)
# ============================================================
set(CURL_DIR "${CMAKE_SOURCE_DIR}/third_party/curl")
set(CURL_LIB "${CURL_DIR}/libcurl.a")
set(CURL_ELF "${CURL_DIR}/curl.elf")

if(WIN32)
  add_custom_command(
    OUTPUT ${CURL_LIB}
    COMMAND ${CROSS_ENV} powershell -ExecutionPolicy Bypass -File ${CMAKE_SOURCE_DIR}/scripts/build_curl.ps1
    DEPENDS
      ${LIBC_A}
      ${CMAKE_SOURCE_DIR}/scripts/build_curl.ps1
      ${CURL_DIR}/lib/config-anyos.h
    COMMENT "Building libcurl for anyOS"
  )
else()
  add_custom_command(
    OUTPUT ${CURL_LIB}
    COMMAND bash ${CMAKE_SOURCE_DIR}/scripts/build_curl.sh
    DEPENDS
      ${LIBC_A}
      ${CMAKE_SOURCE_DIR}/scripts/build_curl.sh
      ${CURL_DIR}/lib/config-anyos.h
    COMMENT "Building libcurl for anyOS"
  )
endif()

add_custom_command(
  OUTPUT ${CURL_ELF}
  COMMAND ${CROSS_ENV} ${I686_ELF_GCC}
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

if(WIN32)
  add_custom_command(
    OUTPUT ${DASH_LIB}
    COMMAND ${CROSS_ENV} powershell -ExecutionPolicy Bypass -File ${CMAKE_SOURCE_DIR}/scripts/build_dash.ps1
    DEPENDS
      ${LIBC_A}
      ${CMAKE_SOURCE_DIR}/scripts/build_dash.ps1
      ${DASH_DIR}/config.h
    COMMENT "Building dash for anyOS"
  )
else()
  add_custom_command(
    OUTPUT ${DASH_LIB}
    COMMAND bash ${CMAKE_SOURCE_DIR}/scripts/build_dash.sh
    DEPENDS
      ${LIBC_A}
      ${CMAKE_SOURCE_DIR}/scripts/build_dash.sh
      ${DASH_DIR}/config.h
    COMMENT "Building dash for anyOS"
  )
endif()

add_custom_command(
  OUTPUT ${DASH_ELF}
  COMMAND ${CROSS_ENV} ${I686_ELF_GCC}
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
if(WIN32)
  add_custom_command(
    OUTPUT ${SSH_LIB}
    COMMAND ${CROSS_ENV} powershell -ExecutionPolicy Bypass -File ${CMAKE_SOURCE_DIR}/scripts/build_ssh.ps1
    DEPENDS
      ${BEARSSL_A}
      ${LIBC_A}
      ${SSH_DIR}/src/ssh.c
      ${SSH_DIR}/include/ssh.h
      ${CMAKE_SOURCE_DIR}/scripts/build_ssh.ps1
    COMMENT "Building SSH library for anyOS"
  )
else()
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
endif()

# Build SSH client (ssh.elf)
add_custom_command(
  OUTPUT ${SSH_CLIENT_ELF}
  COMMAND ${CROSS_ENV} ${I686_ELF_GCC}
    -m32 -O2 -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector -std=c99 -w
    -I${SSH_DIR}/include -I${BEARSSL_DIR}/inc -I${LIBC_DIR}/include
    -c ${SSH_CLIENT_DIR}/src/main.c -o ${SSH_CLIENT_DIR}/main.o
  COMMAND ${CROSS_ENV} ${I686_ELF_GCC}
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
  COMMAND ${CROSS_ENV} ${I686_ELF_GCC}
    -m32 -O2 -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector -std=c99 -w
    -I${SSH_DIR}/include -I${BEARSSL_DIR}/inc -I${LIBC_DIR}/include
    -c ${SSHD_DIR}/src/main.c -o ${SSHD_DIR}/main.o
  COMMAND ${CROSS_ENV} ${I686_ELF_GCC}
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

# ============================================================
# 64-bit C Library (libc64 — uses clang, no cross-compiler needed)
# ============================================================
find_program(CLANG_EXECUTABLE NAMES clang
  HINTS /usr/bin /usr/local/bin
        "$ENV{HOME}/opt/cross/bin"
        "C:/msys64/mingw64/bin"
        "C:/msys64/clang64/bin"
)
if(NOT CLANG_EXECUTABLE)
  message(FATAL_ERROR "clang not found. Install clang (e.g. pacman -S mingw-w64-x86_64-clang on MSYS2).")
endif()
find_program(CLANGXX_EXECUTABLE NAMES clang++
  HINTS /usr/bin /usr/local/bin
        "$ENV{HOME}/opt/cross/bin"
        "C:/msys64/mingw64/bin"
        "C:/msys64/clang64/bin"
)
get_filename_component(CLANG_BIN_DIR "${CLANG_EXECUTABLE}" DIRECTORY)
find_program(LLVM_AR_EXECUTABLE NAMES llvm-ar ar HINTS "${CLANG_BIN_DIR}")
message(STATUS "Found clang: ${CLANG_EXECUTABLE}")
message(STATUS "Found clang++: ${CLANGXX_EXECUTABLE}")
message(STATUS "Found ar (for 64-bit): ${LLVM_AR_EXECUTABLE}")

# On Windows, clang and MSYS2 utilities (rm, mkdir) may not be in PATH when
# invoked from cmd.exe.  Prepend both the clang bin dir and the MSYS2 usr/bin.
if(WIN32)
  get_filename_component(MSYS_USR_BIN_DIR "${MAKE_EXECUTABLE}" DIRECTORY)
  set(CLANG_WRAPPER "${CMAKE_BINARY_DIR}/clang_env.cmd")
  file(WRITE "${CLANG_WRAPPER}" "@set \"PATH=${CLANG_BIN_DIR};${MSYS_USR_BIN_DIR};%PATH%\"\n@%*\n")
  set(CLANG_ENV "${CLANG_WRAPPER}")
else()
  set(CLANG_ENV "")
endif()

set(LIBC64_DIR "${CMAKE_SOURCE_DIR}/libs/libc64")
set(LIBC64_A "${LIBC64_DIR}/libc64.a")
set(LIBC64_CRT0 "${LIBC64_DIR}/obj/crt0.o")
set(LIBC64_CRTI "${LIBC64_DIR}/obj/crti.o")
set(LIBC64_CRTN "${LIBC64_DIR}/obj/crtn.o")

add_custom_command(
  OUTPUT ${LIBC64_A} ${LIBC64_CRT0} ${LIBC64_CRTI} ${LIBC64_CRTN}
  COMMAND ${CLANG_ENV} ${MAKE_EXECUTABLE} -s -C ${LIBC64_DIR} clean CC=${CLANG_EXECUTABLE} AR=${LLVM_AR_EXECUTABLE} AS=${CLANG_EXECUTABLE}
  COMMAND ${CLANG_ENV} ${MAKE_EXECUTABLE} -s -j${NPROC} -C ${LIBC64_DIR} CC=${CLANG_EXECUTABLE} AR=${LLVM_AR_EXECUTABLE} AS=${CLANG_EXECUTABLE} EXTRA_CFLAGS=-w
  DEPENDS
    ${LIBC64_DIR}/Makefile
    ${LIBC64_DIR}/src/crt0.S
    ${LIBC64_DIR}/src/syscall.S
    ${LIBC64_DIR}/src/setjmp.S
    ${LIBC64_DIR}/src/crti.S
    ${LIBC64_DIR}/src/crtn.S
    ${LIBC64_DIR}/src/string.c
    ${LIBC64_DIR}/src/stdlib.c
    ${LIBC64_DIR}/src/stdio.c
    ${LIBC64_DIR}/src/unistd.c
    ${LIBC64_DIR}/src/ctype.c
    ${LIBC64_DIR}/src/signal.c
    ${LIBC64_DIR}/src/stat.c
    ${LIBC64_DIR}/src/time.c
    ${LIBC64_DIR}/src/math.c
    ${LIBC64_DIR}/src/mman.c
    ${LIBC64_DIR}/src/start.c
    ${LIBC64_DIR}/src/socket.c
    ${LIBC64_DIR}/src/stubs.c
    ${LIBC64_DIR}/src/pthread.c
  COMMENT "Building 64-bit C library (libc64.a)"
)

# Copy libc64 artifacts to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/libc64/lib/libc64.a
         ${SYSROOT_DIR}/Libraries/libc64/lib/crt0.o
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC64_A} ${SYSROOT_DIR}/Libraries/libc64/lib/libc64.a
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC64_CRT0} ${SYSROOT_DIR}/Libraries/libc64/lib/crt0.o
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC64_CRT0} ${SYSROOT_DIR}/Libraries/libc64/lib/crt1.o
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC64_CRTI} ${SYSROOT_DIR}/Libraries/libc64/lib/crti.o
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBC64_CRTN} ${SYSROOT_DIR}/Libraries/libc64/lib/crtn.o
  DEPENDS ${LIBC64_A} ${LIBC64_CRT0} ${LIBC64_CRTI} ${LIBC64_CRTN}
  COMMENT "Installing libc64 to sysroot"
)

# Copy libc64 headers to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/libc64/include/.stamp
  COMMAND ${CMAKE_COMMAND} -E copy_directory ${LIBC64_DIR}/include ${SYSROOT_DIR}/Libraries/libc64/include
  COMMAND ${CMAKE_COMMAND} -E touch ${SYSROOT_DIR}/Libraries/libc64/include/.stamp
  DEPENDS ${LIBC64_A}
  COMMENT "Installing 64-bit C headers to sysroot"
)

# ============================================================
# C++ Standard Library (libcxx — uses clang++, depends on libc64)
# ============================================================
set(LIBCXX_DIR "${CMAKE_SOURCE_DIR}/libs/libcxx")
set(LIBCXX_A "${LIBCXX_DIR}/libcxx.a")

add_custom_command(
  OUTPUT ${LIBCXX_A}
  COMMAND ${CLANG_ENV} ${MAKE_EXECUTABLE} -s -C ${LIBCXX_DIR} clean CXX=${CLANGXX_EXECUTABLE} AR=${LLVM_AR_EXECUTABLE}
  COMMAND ${CLANG_ENV} ${MAKE_EXECUTABLE} -s -j${NPROC} -C ${LIBCXX_DIR} CXX=${CLANGXX_EXECUTABLE} AR=${LLVM_AR_EXECUTABLE} EXTRA_CXXFLAGS=-w
  DEPENDS
    ${LIBC64_A}
    ${LIBCXX_DIR}/Makefile
    ${LIBCXX_DIR}/src/new.cpp
    ${LIBCXX_DIR}/src/iostream.cpp
  COMMENT "Building C++ standard library (libcxx.a)"
)

# Copy libcxx artifacts to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/libcxx/lib/libcxx.a
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBCXX_A} ${SYSROOT_DIR}/Libraries/libcxx/lib/libcxx.a
  DEPENDS ${LIBCXX_A}
  COMMENT "Installing libcxx.a to sysroot"
)

# Copy libcxx headers to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/libcxx/include/.stamp
  COMMAND ${CMAKE_COMMAND} -E copy_directory ${LIBCXX_DIR}/include ${SYSROOT_DIR}/Libraries/libcxx/include
  COMMAND ${CMAKE_COMMAND} -E touch ${SYSROOT_DIR}/Libraries/libcxx/include/.stamp
  DEPENDS ${LIBCXX_A}
  COMMENT "Installing C++ headers to sysroot"
)

# ============================================================
# Stack Unwinder (libunwind — depends on libc64)
# ============================================================
set(LIBUNWIND_DIR "${CMAKE_SOURCE_DIR}/libs/libunwind")
set(LIBUNWIND_A "${LIBUNWIND_DIR}/libunwind.a")

add_custom_command(
  OUTPUT ${LIBUNWIND_A}
  COMMAND ${CLANG_ENV} ${MAKE_EXECUTABLE} -s -C ${LIBUNWIND_DIR} clean CC=${CLANG_EXECUTABLE} AR=${LLVM_AR_EXECUTABLE} AS=${CLANG_EXECUTABLE}
  COMMAND ${CLANG_ENV} ${MAKE_EXECUTABLE} -s -j${NPROC} -C ${LIBUNWIND_DIR} CC=${CLANG_EXECUTABLE} AR=${LLVM_AR_EXECUTABLE} AS=${CLANG_EXECUTABLE} EXTRA_CFLAGS=-w
  DEPENDS
    ${LIBC64_A}
    ${LIBUNWIND_DIR}/Makefile
    ${LIBUNWIND_DIR}/src/unwind.c
    ${LIBUNWIND_DIR}/src/unwind_registers.S
    ${LIBUNWIND_DIR}/include/unwind.h
  COMMENT "Building stack unwinder (libunwind.a)"
)

# Copy libunwind to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/libcxx/lib/libunwind.a
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBUNWIND_A} ${SYSROOT_DIR}/Libraries/libcxx/lib/libunwind.a
  DEPENDS ${LIBUNWIND_A}
  COMMENT "Installing libunwind.a to sysroot"
)

# Copy libunwind headers to sysroot (unwind.h is needed by user code)
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/libcxx/include/unwind.h
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBUNWIND_DIR}/include/unwind.h ${SYSROOT_DIR}/Libraries/libcxx/include/unwind.h
  DEPENDS ${LIBUNWIND_A}
  COMMENT "Installing unwind.h to sysroot"
)

# ============================================================
# C++ ABI Runtime (libc++abi — depends on libc64, libunwind, libcxx headers)
# ============================================================
set(LIBCXXABI_DIR "${CMAKE_SOURCE_DIR}/libs/libcxxabi")
set(LIBCXXABI_A "${LIBCXXABI_DIR}/libc++abi.a")

add_custom_command(
  OUTPUT ${LIBCXXABI_A}
  COMMAND ${CLANG_ENV} ${MAKE_EXECUTABLE} -s -C ${LIBCXXABI_DIR} clean CXX=${CLANGXX_EXECUTABLE} AR=${LLVM_AR_EXECUTABLE}
  COMMAND ${CLANG_ENV} ${MAKE_EXECUTABLE} -s -j${NPROC} -C ${LIBCXXABI_DIR} CXX=${CLANGXX_EXECUTABLE} AR=${LLVM_AR_EXECUTABLE} EXTRA_CXXFLAGS=-w
  DEPENDS
    ${LIBC64_A}
    ${LIBUNWIND_A}
    ${LIBCXXABI_DIR}/Makefile
    ${LIBCXXABI_DIR}/src/cxa_exception.cpp
    ${LIBCXXABI_DIR}/src/cxa_guard.cpp
    ${LIBCXXABI_DIR}/src/cxa_handlers.cpp
    ${LIBCXXABI_DIR}/src/cxa_rtti.cpp
    ${LIBCXXABI_DIR}/include/cxxabi.h
  COMMENT "Building C++ ABI runtime (libc++abi.a)"
)

# Copy libc++abi to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/libcxx/lib/libc++abi.a
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBCXXABI_A} ${SYSROOT_DIR}/Libraries/libcxx/lib/libc++abi.a
  DEPENDS ${LIBCXXABI_A}
  COMMENT "Installing libc++abi.a to sysroot"
)

# Copy cxxabi.h to sysroot
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/Libraries/libcxx/include/cxxabi.h
  COMMAND ${CMAKE_COMMAND} -E copy ${LIBCXXABI_DIR}/include/cxxabi.h ${SYSROOT_DIR}/Libraries/libcxx/include/cxxabi.h
  DEPENDS ${LIBCXXABI_A}
  COMMENT "Installing cxxabi.h to sysroot"
)

# ============================================================
# GCC Cross-Compiler Detection (x86_64-anyos-gcc)
# ============================================================
# If the user has built the GCC cross-compiler (via scripts/build_gcc_toolchain.sh),
# detect it and install libgcc.a + toolchain binaries to the sysroot.
find_program(ANYOS_GCC NAMES x86_64-anyos-gcc
  HINTS "$ENV{HOME}/opt/anyos-toolchain/bin"
        "$ENV{ANYOS_TOOLCHAIN}/bin"
)
if(ANYOS_GCC)
  message(STATUS "Found x86_64-anyos-gcc: ${ANYOS_GCC}")
  get_filename_component(ANYOS_GCC_BIN_DIR "${ANYOS_GCC}" DIRECTORY)
  get_filename_component(ANYOS_GCC_PREFIX "${ANYOS_GCC_BIN_DIR}" DIRECTORY)

  # Detect libgcc.a location
  execute_process(
    COMMAND ${ANYOS_GCC} -print-libgcc-file-name
    OUTPUT_VARIABLE ANYOS_LIBGCC_PATH
    OUTPUT_STRIP_TRAILING_WHITESPACE
    ERROR_QUIET
  )

  if(EXISTS "${ANYOS_LIBGCC_PATH}")
    message(STATUS "Found libgcc.a: ${ANYOS_LIBGCC_PATH}")

    # Copy libgcc.a to sysroot
    add_custom_command(
      OUTPUT ${SYSROOT_DIR}/Libraries/libc64/lib/libgcc.a
      COMMAND ${CMAKE_COMMAND} -E copy ${ANYOS_LIBGCC_PATH} ${SYSROOT_DIR}/Libraries/libc64/lib/libgcc.a
      DEPENDS ${ANYOS_LIBGCC_PATH}
      COMMENT "Installing libgcc.a to sysroot"
    )
    set(LIBGCC_SYSROOT_DEP ${SYSROOT_DIR}/Libraries/libc64/lib/libgcc.a)
  else()
    message(STATUS "libgcc.a not found at ${ANYOS_LIBGCC_PATH} — skipping")
    set(LIBGCC_SYSROOT_DEP "")
  endif()

  # Install GCC toolchain binaries to /System/Toolchain/bin/ on disk
  set(ANYOS_TOOLCHAIN_BINS gcc g++ as ld ar nm objdump objcopy ranlib strip)
  set(TOOLCHAIN_SYSROOT_DEPS "")
  foreach(TOOL ${ANYOS_TOOLCHAIN_BINS})
    set(TOOL_PATH "${ANYOS_GCC_BIN_DIR}/x86_64-anyos-${TOOL}${CMAKE_EXECUTABLE_SUFFIX}")
    if(EXISTS "${TOOL_PATH}")
      set(DEST "${SYSROOT_DIR}/System/Toolchain/bin/${TOOL}")
      add_custom_command(
        OUTPUT ${DEST}
        COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/System/Toolchain/bin
        COMMAND ${CMAKE_COMMAND} -E copy ${TOOL_PATH} ${DEST}
        DEPENDS ${TOOL_PATH}
        COMMENT "Installing ${TOOL} to /System/Toolchain/bin/"
      )
      list(APPEND TOOLCHAIN_SYSROOT_DEPS ${DEST})
    endif()
  endforeach()

  # Copy the linker script for the toolchain
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/System/Toolchain/lib/link.ld
    COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/System/Toolchain/lib
    COMMAND ${CMAKE_COMMAND} -E copy ${LIBC64_DIR}/link.ld ${SYSROOT_DIR}/System/Toolchain/lib/link.ld
    DEPENDS ${LIBC64_DIR}/link.ld
    COMMENT "Installing linker script to /System/Toolchain/lib/"
  )
  list(APPEND TOOLCHAIN_SYSROOT_DEPS ${SYSROOT_DIR}/System/Toolchain/lib/link.ld)

  # Check for Stage 2 native compiler (built to run ON anyOS)
  set(NATIVE_TOOLCHAIN_DIR "$ENV{HOME}/build/anyos-toolchain/native-toolchain")
  if(IS_DIRECTORY "${NATIVE_TOOLCHAIN_DIR}/bin")
    message(STATUS "Found native GCC toolchain at ${NATIVE_TOOLCHAIN_DIR}")
    # Install native binaries (ELF64) to /System/Toolchain/ on disk
    file(GLOB NATIVE_BINS "${NATIVE_TOOLCHAIN_DIR}/bin/*")
    foreach(NATIVE_BIN ${NATIVE_BINS})
      get_filename_component(TOOL_NAME "${NATIVE_BIN}" NAME)
      set(DEST "${SYSROOT_DIR}/System/Toolchain/bin/${TOOL_NAME}")
      add_custom_command(
        OUTPUT ${DEST}
        COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/System/Toolchain/bin
        COMMAND ${CMAKE_COMMAND} -E copy ${NATIVE_BIN} ${DEST}
        DEPENDS ${NATIVE_BIN}
        COMMENT "Installing native ${TOOL_NAME} to /System/Toolchain/bin/"
      )
      list(APPEND TOOLCHAIN_SYSROOT_DEPS ${DEST})
    endforeach()
    # Install cc1/cc1plus to libexec on disk
    file(GLOB_RECURSE NATIVE_LIBEXEC "${NATIVE_TOOLCHAIN_DIR}/libexec/*")
    foreach(NATIVE_LE ${NATIVE_LIBEXEC})
      get_filename_component(LE_NAME "${NATIVE_LE}" NAME)
      set(DEST "${SYSROOT_DIR}/System/Toolchain/libexec/${LE_NAME}")
      add_custom_command(
        OUTPUT ${DEST}
        COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/System/Toolchain/libexec
        COMMAND ${CMAKE_COMMAND} -E copy ${NATIVE_LE} ${DEST}
        DEPENDS ${NATIVE_LE}
        COMMENT "Installing native ${LE_NAME} to /System/Toolchain/libexec/"
      )
      list(APPEND TOOLCHAIN_SYSROOT_DEPS ${DEST})
    endforeach()
  endif()

else()
  message(STATUS "x86_64-anyos-gcc not found — GCC toolchain will not be installed to sysroot")
  message(STATUS "  Run scripts/build_gcc_toolchain.sh to build it")
  set(LIBGCC_SYSROOT_DEP "")
  set(TOOLCHAIN_SYSROOT_DEPS "")
endif()

# Aggregate dependency list for 64-bit toolchain
set(CXX_TOOLCHAIN_DEPS
  ${SYSROOT_DIR}/Libraries/libc64/lib/libc64.a
  ${SYSROOT_DIR}/Libraries/libc64/lib/crt0.o
  ${SYSROOT_DIR}/Libraries/libc64/include/.stamp
  ${SYSROOT_DIR}/Libraries/libcxx/lib/libcxx.a
  ${SYSROOT_DIR}/Libraries/libcxx/include/.stamp
  ${SYSROOT_DIR}/Libraries/libcxx/lib/libunwind.a
  ${SYSROOT_DIR}/Libraries/libcxx/include/unwind.h
  ${SYSROOT_DIR}/Libraries/libcxx/lib/libc++abi.a
  ${SYSROOT_DIR}/Libraries/libcxx/include/cxxabi.h
  ${LIBGCC_SYSROOT_DEP}
  ${TOOLCHAIN_SYSROOT_DEPS}
)

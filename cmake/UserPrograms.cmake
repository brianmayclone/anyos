# ============================================================
# 5. User Programs (flat binaries)
# ============================================================
set(SYSROOT_DIR "${CMAKE_BINARY_DIR}/sysroot")

# ── Architecture-dependent target selection ──
if(ANYOS_ARCH STREQUAL "arm64")
  set(USER_TARGET_JSON "${CMAKE_SOURCE_DIR}/aarch64-anyos-user.json")
  set(USER_TARGET_TRIPLE "aarch64-anyos-user")
  set(KERNEL_TARGET_JSON "${CMAKE_SOURCE_DIR}/aarch64-anyos.json")
  set(KERNEL_TARGET_TRIPLE "aarch64-anyos")
else()
  set(USER_TARGET_JSON "${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json")
  set(USER_TARGET_TRIPLE "x86_64-anyos-user")
  set(KERNEL_TARGET_JSON "${CMAKE_SOURCE_DIR}/x86_64-anyos.json")
  set(KERNEL_TARGET_TRIPLE "x86_64-anyos")
endif()
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
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libc64/include")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libc64/include/sys")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libc64/include/netinet")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libc64/include/arpa")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libc64/include/net")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libc64/lib")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libcxx/include")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libcxx/lib")
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

# Shared target directory for all user-space Rust programs.
set(USER_TARGET_DIR "${CMAKE_BINARY_DIR}/user-target")

# Standard library dependencies (used by DLLs/shared libs that still build individually)
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
  ${USER_TARGET_JSON}
)

# ============================================================
# Workspace build — compiles ALL user-space programs in parallel
# ============================================================
# Instead of 100+ individual cargo build calls (serialized by Cargo's
# target-dir lockfile), one workspace build lets Cargo parallelize
# compilation across all CPU cores.

file(GLOB_RECURSE _WS_RS CONFIGURE_DEPENDS
  "${CMAKE_SOURCE_DIR}/bin/*/src/*.rs"
  "${CMAKE_SOURCE_DIR}/apps/*/src/*.rs"
  "${CMAKE_SOURCE_DIR}/system/*/src/*.rs"
  "${CMAKE_SOURCE_DIR}/system/compositor/compositor/src/*.rs"
)
file(GLOB _WS_TOMLS CONFIGURE_DEPENDS
  "${CMAKE_SOURCE_DIR}/bin/*/Cargo.toml"
  "${CMAKE_SOURCE_DIR}/apps/*/Cargo.toml"
  "${CMAKE_SOURCE_DIR}/system/*/Cargo.toml"
  "${CMAKE_SOURCE_DIR}/system/compositor/compositor/Cargo.toml"
)
file(GLOB _WS_BUILD_RS CONFIGURE_DEPENDS
  "${CMAKE_SOURCE_DIR}/bin/*/build.rs"
  "${CMAKE_SOURCE_DIR}/apps/*/build.rs"
  "${CMAKE_SOURCE_DIR}/system/*/build.rs"
  "${CMAKE_SOURCE_DIR}/system/compositor/compositor/build.rs"
)

set(WORKSPACE_STAMP "${USER_TARGET_DIR}/.workspace-stamp")

# Optional features for workspace build
set(_WS_FEATURES "")
if(ANYOS_DEBUG_SURF)
  set(_WS_FEATURES "--features;surf/debug_surf")
endif()

# Architecture-specific workspace exclusions
set(_WS_EXCLUDES "--exclude;anyos_kernel")
if(ANYOS_ARCH STREQUAL "arm64")
  # surf depends on BearSSL (x86_64-only) for TLS — exclude from ARM64 builds
  list(APPEND _WS_EXCLUDES "--exclude;surf")
endif()

add_custom_command(
  OUTPUT ${WORKSPACE_STAMP}
  COMMAND ${CMAKE_COMMAND} -E env "RUSTFLAGS=-Awarnings" "ANYOS_VERSION=${ANYOS_VERSION}"
    ${CARGO_EXECUTABLE} build --workspace
    ${_WS_EXCLUDES}
    --release --quiet
    --target ${USER_TARGET_JSON}
    --target-dir ${USER_TARGET_DIR}
    ${_WS_FEATURES}
  COMMAND ${CMAKE_COMMAND} -E touch ${WORKSPACE_STAMP}
  DEPENDS
    ${CMAKE_SOURCE_DIR}/Cargo.toml
    ${USER_TARGET_JSON}
    ${_WS_TOMLS}
    ${_WS_BUILD_RS}
    ${_WS_RS}
    ${STDLIB_DEPS}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building all user-space programs (cargo workspace, parallel)"
)

# ============================================================
# ELF-to-flat-binary conversion helpers (post-workspace-build)
# ============================================================
# These functions only perform the anyelf ELF -> flat binary conversion.
# The actual Rust compilation is handled by the workspace build above.

function(add_rust_user_program NAME)
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/System/bin/${NAME}
    COMMAND ${ANYELF_EXECUTABLE} bin
      ${USER_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/${NAME}.elf
      ${SYSROOT_DIR}/System/bin/${NAME}
    DEPENDS ${WORKSPACE_STAMP} ${ANYELF_EXECUTABLE}
    COMMENT "Converting ${NAME} ELF to flat binary"
  )
  set(RUST_USER_BINS ${RUST_USER_BINS} ${SYSROOT_DIR}/System/bin/${NAME} PARENT_SCOPE)
endfunction()

function(add_rust_system_program NAME)
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/System/${NAME}
    COMMAND ${ANYELF_EXECUTABLE} bin
      ${USER_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/${NAME}.elf
      ${SYSROOT_DIR}/System/${NAME}
    DEPENDS ${WORKSPACE_STAMP} ${ANYELF_EXECUTABLE}
    COMMENT "Converting ${NAME} ELF to flat binary"
  )
  set(SYSTEM_BINS ${SYSTEM_BINS} ${SYSROOT_DIR}/System/${NAME} PARENT_SCOPE)
endfunction()

function(add_rust_sbin_program NAME)
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/System/sbin/${NAME}
    COMMAND ${ANYELF_EXECUTABLE} bin
      ${USER_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/${NAME}.elf
      ${SYSROOT_DIR}/System/sbin/${NAME}
    DEPENDS ${WORKSPACE_STAMP} ${ANYELF_EXECUTABLE}
    COMMENT "Converting ${NAME} ELF to flat binary (sbin)"
  )
  set(RUST_USER_BINS ${RUST_USER_BINS} ${SYSROOT_DIR}/System/sbin/${NAME} PARENT_SCOPE)
endfunction()

# .app bundles (placed in /Applications/{DISPLAY_NAME}.app/)
# Uses mkappbundle for validated bundling with ELF auto-conversion via anyelf.
function(add_app NAME SRC_DIR DISPLAY_NAME)
  set(APP_DIR "${SYSROOT_DIR}/Applications/${DISPLAY_NAME}.app")
  set(ELF "${USER_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/${NAME}.elf")
  # Collect mkappbundle arguments and dependencies
  set(_BUNDLE_ARGS
    -i "${SRC_DIR}/Info.conf"
    -e ${ELF}
    --anyelf-path ${ANYELF_EXECUTABLE}
    --version ${ANYOS_VERSION}
    -o "${APP_DIR}"
    --force
  )
  set(_BUNDLE_DEPS ${WORKSPACE_STAMP} "${SRC_DIR}/Info.conf" ${ANYELF_EXECUTABLE} ${MKAPPBUNDLE_EXECUTABLE})
  if(EXISTS "${SRC_DIR}/Icon.ico")
    list(APPEND _BUNDLE_ARGS -c "${SRC_DIR}/Icon.ico")
    list(APPEND _BUNDLE_DEPS "${SRC_DIR}/Icon.ico")
  endif()
  foreach(_RESDIR syntax resources)
    if(IS_DIRECTORY "${SRC_DIR}/${_RESDIR}")
      list(APPEND _BUNDLE_ARGS -r "${SRC_DIR}/${_RESDIR}")
      file(GLOB_RECURSE _RES_FILES "${SRC_DIR}/${_RESDIR}/*")
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
# DLLs use custom link.ld scripts, so they share their own target dir
# (separate from user programs to avoid linker script conflicts).
set(DLL_TARGET_DIR "${CMAKE_BINARY_DIR}/dll-target")
function(add_dll NAME SRC_DIR)
  set(DLL_ELF "${DLL_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/${NAME}.elf")
  file(GLOB_RECURSE _DLL_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${DLL_ELF}
    COMMAND ${CMAKE_COMMAND} -E env "RUSTFLAGS=-Awarnings"
      ${CARGO_EXECUTABLE} build --release --quiet
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${USER_TARGET_JSON}
      --target-dir ${DLL_TARGET_DIR}
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${SRC_DIR}/build.rs
      ${_DLL_RS}
      ${SRC_DIR}/link.ld
      ${USER_TARGET_JSON}
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
  set(DRV_ELF "${CMAKE_BINARY_DIR}/drivers/${NAME}/${KERNEL_TARGET_TRIPLE}/release/${NAME}.elf")
  file(GLOB_RECURSE _DRV_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${DRV_ELF}
    COMMAND ${CMAKE_COMMAND} -E env "RUSTFLAGS=-Awarnings"
      ${CARGO_EXECUTABLE} build --release --quiet
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${KERNEL_TARGET_JSON}
      --target-dir ${CMAKE_BINARY_DIR}/drivers/${NAME}
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${_DRV_RS}
      ${KERNEL_TARGET_JSON}
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

# Shared libraries (.so) — built via Cargo -> .a -> anyld -> ET_DYN .so
# These use -Z build-std so they share a separate target dir from user programs.
set(SHLIB_TARGET_DIR "${CMAKE_BINARY_DIR}/shlib-target")
function(add_shared_lib NAME SRC_DIR)
  set(LIB_A "${SHLIB_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/lib${NAME}.a")
  set(LIB_SO "${CMAKE_BINARY_DIR}/shlib/${NAME}.so")
  file(GLOB_RECURSE _SL_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  # Step 1: Cargo -> static archive (.a)
  add_custom_command(
    OUTPUT ${LIB_A}
    COMMAND ${CMAKE_COMMAND} -E env "RUSTFLAGS=-Awarnings"
      ${CARGO_EXECUTABLE} build --release --quiet
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${USER_TARGET_JSON}
      --target-dir ${SHLIB_TARGET_DIR}
      -Z build-std=core,alloc
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${_SL_RS}
      ${USER_TARGET_JSON}
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building shared library: ${NAME} (Cargo)"
  )
  # Step 2: anyld -> .so (ET_DYN shared object, base=0 for dynamic loading)
  add_custom_command(
    OUTPUT ${LIB_SO}
    COMMAND ${ANYLD_EXECUTABLE} -q
      -o ${LIB_SO}
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

add_shared_lib(libanyui ${CMAKE_SOURCE_DIR}/libs/libanyui)
add_shared_lib(libfont ${CMAKE_SOURCE_DIR}/libs/libfont)
add_shared_lib(libdb ${CMAKE_SOURCE_DIR}/libs/libdb)
add_shared_lib(libzip ${CMAKE_SOURCE_DIR}/libs/libzip)
add_shared_lib(libsvg ${CMAKE_SOURCE_DIR}/libs/libsvg)
add_shared_lib(libgl ${CMAKE_SOURCE_DIR}/libs/libgl)
add_shared_lib(libm ${CMAKE_SOURCE_DIR}/libs/libm)
if(NOT ANYOS_ARCH STREQUAL "arm64")
  add_shared_lib(libcorevm ${CMAKE_SOURCE_DIR}/libs/libcorevm)
endif()

# --- libhttp (custom: links BearSSL for HTTPS support) ---
# BearSSL is x86_64-only; skip libhttp entirely on ARM64.
if(NOT ANYOS_ARCH STREQUAL "arm64")
  # libbearssl_x64.a already contains anyos_tls.o (the BearSSL TLS wrapper).
  # libhttp's Rust code (tls.rs) provides the callbacks (anyos_tcp_send, etc.)
  # that anyos_tls.o calls, using raw syscalls instead of anyos_std.
  set(_LIBHTTP_SRC "${CMAKE_SOURCE_DIR}/libs/libhttp")
  set(_LIBHTTP_A "${SHLIB_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/liblibhttp.a")
  set(_LIBHTTP_SO "${CMAKE_BINARY_DIR}/shlib/libhttp.so")
  set(_BEARSSL_X64_A "${CMAKE_SOURCE_DIR}/third_party/bearssl/build_x64/libbearssl_x64.a")
  file(GLOB_RECURSE _LIBHTTP_RS CONFIGURE_DEPENDS "${_LIBHTTP_SRC}/src/*.rs")

  # Step 1: Cargo → static archive (.a)
  add_custom_command(
    OUTPUT ${_LIBHTTP_A}
    COMMAND ${CMAKE_COMMAND} -E env "RUSTFLAGS=-Awarnings"
      ${CARGO_EXECUTABLE} build --release --quiet
      --manifest-path ${_LIBHTTP_SRC}/Cargo.toml
      --target ${USER_TARGET_JSON}
      --target-dir ${SHLIB_TARGET_DIR}
      -Z build-std=core,alloc
    DEPENDS
      ${_LIBHTTP_SRC}/Cargo.toml
      ${_LIBHTTP_RS}
      ${USER_TARGET_JSON}
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building shared library: libhttp (Cargo)"
  )

  # Step 2: anyld → .so (Rust .a + BearSSL .a including anyos_tls.o)
  add_custom_command(
    OUTPUT ${_LIBHTTP_SO}
    COMMAND ${ANYLD_EXECUTABLE} -q
      -o ${_LIBHTTP_SO}
      -e ${_LIBHTTP_SRC}/exports.def
      ${_LIBHTTP_A}
      ${_BEARSSL_X64_A}
    DEPENDS ${_LIBHTTP_A} ${_BEARSSL_X64_A}
      ${_LIBHTTP_SRC}/exports.def ${ANYLD_EXECUTABLE}
    COMMENT "Linking libhttp.so (anyld + BearSSL)"
  )

  # Step 3: Install to sysroot
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/Libraries/libhttp.so
    COMMAND ${CMAKE_COMMAND} -E copy ${_LIBHTTP_SO} ${SYSROOT_DIR}/Libraries/libhttp.so
    DEPENDS ${_LIBHTTP_SO}
    COMMENT "Installing libhttp.so to sysroot"
  )
  set(DLL_BINS ${DLL_BINS} ${SYSROOT_DIR}/Libraries/libhttp.so)
endif()

# ============================================================
# User programs (/System/bin/)
# ============================================================
set(RUST_USER_BINS "")
add_rust_user_program(ping)
add_rust_user_program(dhcp)
add_rust_user_program(dns)
add_rust_user_program(ls)
add_rust_user_program(cat)
add_rust_user_program(ifconfig)
add_rust_user_program(arp)
add_rust_user_program(sysinfo)
add_rust_user_program(dmesg)
add_rust_user_program(mkdir)
add_rust_user_program(rm)
add_rust_user_program(touch)
add_rust_user_program(cp)
add_rust_user_program(mv)
add_rust_user_program(date)
add_rust_user_program(sleep)
add_rust_user_program(hostname)
add_rust_user_program(ftp)
add_rust_user_program(wget)
add_rust_user_program(play)
add_rust_user_program(pipes)
add_rust_user_program(devlist)
add_rust_user_program(echo)
add_rust_user_program(ps)
add_rust_user_program(top)
add_rust_user_program(htop)
add_rust_user_program(kill)
add_rust_user_program(killall)
add_rust_user_program(nice)
add_rust_user_program(free)
add_rust_user_program(uptime)
add_rust_user_program(uname)
add_rust_user_program(pwd)
add_rust_user_program(wc)
add_rust_user_program(hexdump)
add_rust_user_program(head)
add_rust_user_program(tail)
add_rust_user_program(clear)
add_rust_user_program(env)
add_rust_user_program(grep)
add_rust_user_program(find)
add_rust_user_program(sort)
add_rust_user_program(uniq)
add_rust_user_program(rev)
add_rust_user_program(stat)
add_rust_user_program(ln)
add_rust_user_program(readlink)
add_rust_user_program(df)
add_rust_user_program(cal)
add_rust_user_program(seq)
add_rust_user_program(yes)
add_rust_user_program(whoami)
add_rust_user_program(which)
add_rust_user_program(strings)
add_rust_user_program(base64)
add_rust_user_program(xxd)
add_rust_user_program(set)
add_rust_user_program(export)
add_rust_user_program(mount)
add_rust_user_program(umount)
add_rust_user_program(open)
add_rust_user_program(listuser)
add_rust_user_program(listgroups)
add_rust_user_program(chmod)
add_rust_user_program(chown)
add_rust_user_program(su)
add_rust_user_program(echoserver)
add_rust_user_program(netstat)
add_rust_user_program(svc)
add_rust_user_program(logd)
add_rust_user_program(ami)
add_rust_user_program(vi)
add_rust_user_program(crond)
add_rust_user_program(crontab)
add_rust_user_program(sed)
add_rust_user_program(xargs)
add_rust_user_program(awk)
add_rust_user_program(nano)
add_rust_user_program(httpd)
add_rust_user_program(vncd)
add_rust_user_program(vmd)
add_rust_user_program(zip)
add_rust_user_program(unzip)
add_rust_user_program(gzip)
add_rust_user_program(tar)
add_rust_user_program(apkg)
# gunzip is a copy of gzip (detects via argv[0])
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/gunzip
  COMMAND ${CMAKE_COMMAND} -E copy ${SYSROOT_DIR}/System/bin/gzip ${SYSROOT_DIR}/System/bin/gunzip
  DEPENDS ${SYSROOT_DIR}/System/bin/gzip
  COMMENT "Creating gunzip (copy of gzip)"
)
set(RUST_USER_BINS ${RUST_USER_BINS} ${SYSROOT_DIR}/System/bin/gunzip)
add_rust_user_program(banner)
add_rust_user_program(jscript)
add_rust_user_program(jp2a)
add_rust_user_program(neofetch)
add_rust_user_program(nvi)
# Privileged sbin programs
add_rust_sbin_program(adduser)
add_rust_sbin_program(deluser)
add_rust_sbin_program(addgroup)
add_rust_sbin_program(delgroup)
add_rust_sbin_program(passwd)
add_rust_sbin_program(fdisk)

# true/false: package names are true_cmd/false_cmd (Rust keywords) but binaries named true/false
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/true
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${USER_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/true_cmd.elf
    ${SYSROOT_DIR}/System/bin/true
  DEPENDS ${WORKSPACE_STAMP} ${ANYELF_EXECUTABLE}
  COMMENT "Converting true ELF to flat binary"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/System/bin/true)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/false
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${USER_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/false_cmd.elf
    ${SYSROOT_DIR}/System/bin/false
  DEPENDS ${WORKSPACE_STAMP} ${ANYELF_EXECUTABLE}
  COMMENT "Converting false ELF to flat binary"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/System/bin/false)

# ============================================================
# System programs (/System/)
# ============================================================
set(SYSTEM_BINS "")
add_rust_system_program(init)
add_rust_system_program(audiomon)
add_rust_system_program(netmon)
add_rust_system_program(inputmon)
add_rust_system_program(login)
add_rust_system_program(permdialog)
add_rust_system_program(notifyd)
add_rust_user_program(amid)

# ============================================================
# Desktop GUI applications -> .app bundles in /Applications/
# ============================================================
set(APP_BINS "")
add_app(terminal    ${CMAKE_SOURCE_DIR}/system/terminal     "Terminal")
add_app(shell       ${CMAKE_SOURCE_DIR}/system/shell        "Shell")
add_app(taskmanager ${CMAKE_SOURCE_DIR}/system/taskmanager  "Activity Monitor")
add_app(settings    ${CMAKE_SOURCE_DIR}/system/settings     "Settings")
add_app(finder      ${CMAKE_SOURCE_DIR}/system/finder       "Finder")
add_app(diskutil   ${CMAKE_SOURCE_DIR}/system/diskutil     "Disk Utility")
add_app(eventviewer ${CMAKE_SOURCE_DIR}/system/eventviewer  "Event Viewer")
add_app(anybout    ${CMAKE_SOURCE_DIR}/system/anybout     "About anyOS")
add_app(anytrace    ${CMAKE_SOURCE_DIR}/system/anytrace     "anyTrace")
add_app(notepad     ${CMAKE_SOURCE_DIR}/apps/notepad        "Notepad")
add_app(imgview     ${CMAKE_SOURCE_DIR}/apps/imgview        "Image Viewer")
add_app(videoplayer ${CMAKE_SOURCE_DIR}/apps/videoplayer    "Video Player")
add_app(diagnostics ${CMAKE_SOURCE_DIR}/apps/diagnostics    "Diagnostics")
add_app(calc        ${CMAKE_SOURCE_DIR}/apps/calc           "Calculator")
add_app(fontviewer  ${CMAKE_SOURCE_DIR}/apps/fontviewer     "Font Viewer")
add_app(clock       ${CMAKE_SOURCE_DIR}/apps/clock          "Clock")
add_app(screenshot  ${CMAKE_SOURCE_DIR}/apps/screenshot     "Screenshot")
# surf depends on BearSSL (x86_64-only) — skip on ARM64
if(NOT ANYOS_ARCH STREQUAL "arm64")
  add_app(surf        ${CMAKE_SOURCE_DIR}/apps/surf           "Surf")
endif()
add_app(demo_anyui  ${CMAKE_SOURCE_DIR}/apps/demo_anyui     "anyUI Demo")
add_app(anycode     ${CMAKE_SOURCE_DIR}/apps/anycode        "anyOS Code")
add_app(paint       ${CMAKE_SOURCE_DIR}/apps/paint          "Paint")
add_app(minesweeper ${CMAKE_SOURCE_DIR}/apps/minesweeper   "Minesweeper")
add_app(webmanager  ${CMAKE_SOURCE_DIR}/apps/webmanager    "Web Manager")
add_app(diff        ${CMAKE_SOURCE_DIR}/apps/diff           "Diff")
add_app(mdview      ${CMAKE_SOURCE_DIR}/apps/mdview         "Markdown Viewer")
add_app(clipman     ${CMAKE_SOURCE_DIR}/apps/clipman        "Clipboard Manager")
add_app(vnc-settings ${CMAKE_SOURCE_DIR}/apps/vnc-settings "VNC Settings")
add_app(anybench    ${CMAKE_SOURCE_DIR}/apps/anybench      "anyBench")
add_app(gldemo      ${CMAKE_SOURCE_DIR}/apps/gldemo        "GL Demo")
add_app(iconview    ${CMAKE_SOURCE_DIR}/apps/iconview      "Icon Browser")
add_app(store       ${CMAKE_SOURCE_DIR}/apps/store         "App Store")
if(NOT ANYOS_ARCH STREQUAL "arm64")
  add_app(vmmanager   ${CMAKE_SOURCE_DIR}/apps/vmmanager    "VM Manager")
endif()

# ============================================================
# Compositor and Dock
# ============================================================
# Compositor (special output path: /System/compositor/compositor)
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/compositor/compositor
  COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/System/compositor
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${USER_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/compositor.elf
    ${SYSROOT_DIR}/System/compositor/compositor
  DEPENDS ${WORKSPACE_STAMP} ${ANYELF_EXECUTABLE}
  COMMENT "Converting compositor ELF to flat binary"
)
list(APPEND SYSTEM_BINS ${SYSROOT_DIR}/System/compositor/compositor)

# Dock (uses kernel target, built separately)
set(DOCK_SRC_DIR ${CMAKE_SOURCE_DIR}/system/compositor/dock)
set(DOCK_ELF "${CMAKE_BINARY_DIR}/kernel/${KERNEL_TARGET_TRIPLE}/release/dock.elf")
add_custom_command(
  OUTPUT ${DOCK_ELF}
  COMMAND ${CMAKE_COMMAND} -E env "RUSTFLAGS=-Awarnings"
    ${CARGO_EXECUTABLE} build --release --quiet
    --manifest-path ${DOCK_SRC_DIR}/Cargo.toml
    --target ${KERNEL_TARGET_JSON}
    --target-dir ${CMAKE_BINARY_DIR}/kernel
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

# ============================================================
# Sysroot overlay and user provisioning
# ============================================================
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
          ${CMAKE_SOURCE_DIR}/sysroot/System/media/icons/devices
          ${CMAKE_SOURCE_DIR}/sysroot/System/media/icons/devices/usb
          ${CMAKE_SOURCE_DIR}/sysroot/System/compositor/compositor.conf
          ${CMAKE_SOURCE_DIR}/sysroot/System/etc/inputmon.conf
  COMMENT "Populating sysroot from tools/sysroot"
)

# User provisioning from config/users.conf
set(PROVISION_DEPS "")
if(EXISTS "${CMAKE_SOURCE_DIR}/config/users.conf")
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/System/users/.provisioned
    COMMAND ${CMAKE_COMMAND}
      -DUSERS_CONF=${CMAKE_SOURCE_DIR}/config/users.conf
      -DCONFIG_DIR=${CMAKE_SOURCE_DIR}/config
      -DSYSROOT_DIR=${SYSROOT_DIR}
      -P ${CMAKE_SOURCE_DIR}/config/provision_users.cmake
    COMMAND ${CMAKE_COMMAND} -E touch ${SYSROOT_DIR}/System/users/.provisioned
    DEPENDS
      ${CMAKE_SOURCE_DIR}/config/users.conf
      ${CMAKE_SOURCE_DIR}/config/provision_users.cmake
      ${SYSROOT_DIR}/.stamp
    COMMENT "Provisioning users from config/users.conf"
  )
  set(PROVISION_DEPS ${SYSROOT_DIR}/System/users/.provisioned)
endif()

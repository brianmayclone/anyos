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
# By sharing one --target-dir, Cargo caches compiled dependencies (stdlib,
# libheap, etc.) once instead of rebuilding them per-program (~113×).
set(USER_TARGET_DIR "${CMAKE_BINARY_DIR}/user-target")

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
  set(ELF "${USER_TARGET_DIR}/x86_64-anyos-user/release/${NAME}.elf")
  file(GLOB_RECURSE _PROG_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${ELF}
    COMMAND ${CARGO_EXECUTABLE} build --release
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
      --target-dir ${USER_TARGET_DIR}
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
  set(ELF "${USER_TARGET_DIR}/x86_64-anyos-user/release/${NAME}.elf")
  file(GLOB_RECURSE _PROG_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${ELF}
    COMMAND ${CARGO_EXECUTABLE} build --release
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
      --target-dir ${USER_TARGET_DIR}
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
  set(ELF "${USER_TARGET_DIR}/x86_64-anyos-user/release/${NAME}.elf")
  file(GLOB_RECURSE _PROG_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${ELF}
    COMMAND ${CARGO_EXECUTABLE} build --release
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
      --target-dir ${USER_TARGET_DIR}
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
  cmake_parse_arguments(APP "" "FEATURES" "" ${ARGN})
  set(_APP_FEATURES_ARG "")
  if(APP_FEATURES)
    set(_APP_FEATURES_ARG "--features;${APP_FEATURES}")
  endif()
  set(APP_DIR "${SYSROOT_DIR}/Applications/${DISPLAY_NAME}.app")
  set(ELF "${USER_TARGET_DIR}/x86_64-anyos-user/release/${NAME}.elf")
  file(GLOB_RECURSE _PROG_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${ELF}
    COMMAND ${CARGO_EXECUTABLE} build --release
      ${_APP_FEATURES_ARG}
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
      --target-dir ${USER_TARGET_DIR}
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
# DLLs use custom link.ld scripts, so they share their own target dir
# (separate from user programs to avoid linker script conflicts).
set(DLL_TARGET_DIR "${CMAKE_BINARY_DIR}/dll-target")
function(add_dll NAME SRC_DIR)
  set(DLL_ELF "${DLL_TARGET_DIR}/x86_64-anyos-user/release/${NAME}.elf")
  file(GLOB_RECURSE _DLL_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  add_custom_command(
    OUTPUT ${DLL_ELF}
    COMMAND ${CARGO_EXECUTABLE} build --release
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
      --target-dir ${DLL_TARGET_DIR}
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
# These use -Z build-std so they share a separate target dir from user programs.
set(SHLIB_TARGET_DIR "${CMAKE_BINARY_DIR}/shlib-target")
function(add_shared_lib NAME SRC_DIR)
  set(LIB_A "${SHLIB_TARGET_DIR}/x86_64-anyos-user/release/lib${NAME}.a")
  set(LIB_SO "${CMAKE_BINARY_DIR}/shlib/${NAME}.so")
  file(GLOB_RECURSE _SL_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  # Step 1: Cargo → static archive (.a)
  add_custom_command(
    OUTPUT ${LIB_A}
    COMMAND ${CARGO_EXECUTABLE} build --release
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
      --target-dir ${SHLIB_TARGET_DIR}
      -Z build-std=core,alloc
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${_SL_RS}
      ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building shared library: ${NAME} (Cargo)"
  )
  # Step 2: anyld → .so (ET_DYN shared object, base=0 for dynamic loading)
  add_custom_command(
    OUTPUT ${LIB_SO}
    COMMAND ${ANYLD_EXECUTABLE}
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
add_rust_user_program(crond      ${CMAKE_SOURCE_DIR}/bin/crond)
add_rust_user_program(crontab    ${CMAKE_SOURCE_DIR}/bin/crontab)
add_rust_user_program(sed        ${CMAKE_SOURCE_DIR}/bin/sed)
add_rust_user_program(xargs      ${CMAKE_SOURCE_DIR}/bin/xargs)
add_rust_user_program(awk        ${CMAKE_SOURCE_DIR}/bin/awk)
add_rust_user_program(nano       ${CMAKE_SOURCE_DIR}/bin/nano)
add_rust_user_program(httpd      ${CMAKE_SOURCE_DIR}/bin/httpd)
add_rust_user_program(vncd       ${CMAKE_SOURCE_DIR}/bin/vncd)
add_rust_user_program(zip        ${CMAKE_SOURCE_DIR}/bin/zip)
add_rust_user_program(unzip      ${CMAKE_SOURCE_DIR}/bin/unzip)
add_rust_user_program(gzip       ${CMAKE_SOURCE_DIR}/bin/gzip)
add_rust_user_program(tar        ${CMAKE_SOURCE_DIR}/bin/tar)
# gunzip is a copy of gzip (detects via argv[0])
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/gunzip
  COMMAND ${CMAKE_COMMAND} -E copy ${SYSROOT_DIR}/System/bin/gzip ${SYSROOT_DIR}/System/bin/gunzip
  DEPENDS ${SYSROOT_DIR}/System/bin/gzip
  COMMENT "Creating gunzip (copy of gzip)"
)
set(RUST_USER_BINS ${RUST_USER_BINS} ${SYSROOT_DIR}/System/bin/gunzip)
add_rust_user_program(banner     ${CMAKE_SOURCE_DIR}/bin/banner)
add_rust_user_program(jp2a       ${CMAKE_SOURCE_DIR}/bin/jp2a)
add_rust_user_program(neofetch   ${CMAKE_SOURCE_DIR}/bin/neofetch)
add_rust_user_program(nvi        ${CMAKE_SOURCE_DIR}/bin/nvi)
# Privileged sbin programs
add_rust_sbin_program(adduser    ${CMAKE_SOURCE_DIR}/bin/adduser)
add_rust_sbin_program(deluser    ${CMAKE_SOURCE_DIR}/bin/deluser)
add_rust_sbin_program(addgroup   ${CMAKE_SOURCE_DIR}/bin/addgroup)
add_rust_sbin_program(delgroup   ${CMAKE_SOURCE_DIR}/bin/delgroup)
add_rust_sbin_program(passwd     ${CMAKE_SOURCE_DIR}/bin/passwd)
add_rust_sbin_program(fdisk      ${CMAKE_SOURCE_DIR}/bin/fdisk)

# true/false: package names are true_cmd/false_cmd (Rust keywords) but binaries named true/false
set(TRUE_ELF "${USER_TARGET_DIR}/x86_64-anyos-user/release/true_cmd.elf")
add_custom_command(
  OUTPUT ${TRUE_ELF}
  COMMAND ${CARGO_EXECUTABLE} build --release
    --manifest-path ${CMAKE_SOURCE_DIR}/bin/true/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
    --target-dir ${USER_TARGET_DIR}
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

set(FALSE_ELF "${USER_TARGET_DIR}/x86_64-anyos-user/release/false_cmd.elf")
add_custom_command(
  OUTPUT ${FALSE_ELF}
  COMMAND ${CARGO_EXECUTABLE} build --release
    --manifest-path ${CMAKE_SOURCE_DIR}/bin/false/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
    --target-dir ${USER_TARGET_DIR}
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
if(ANYOS_DEBUG_SURF)
  add_app(surf        ${CMAKE_SOURCE_DIR}/apps/surf            "Surf" FEATURES debug_surf)
else()
  add_app(surf        ${CMAKE_SOURCE_DIR}/apps/surf            "Surf")
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

# Build compositor binary (nested path: /System/compositor/compositor)
set(COMPOSITOR_SRC_DIR ${CMAKE_SOURCE_DIR}/system/compositor/compositor)
set(COMPOSITOR_ELF "${USER_TARGET_DIR}/x86_64-anyos-user/release/compositor.elf")
add_custom_command(
  OUTPUT ${COMPOSITOR_ELF}
  COMMAND ${CARGO_EXECUTABLE} build --release
    --manifest-path ${COMPOSITOR_SRC_DIR}/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/x86_64-anyos-user.json
    --target-dir ${USER_TARGET_DIR}
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
set(DOCK_ELF "${CMAKE_BINARY_DIR}/kernel/x86_64-anyos/release/dock.elf")
add_custom_command(
  OUTPUT ${DOCK_ELF}
  COMMAND ${CARGO_EXECUTABLE} build --release
    --manifest-path ${DOCK_SRC_DIR}/Cargo.toml
    --target ${CMAKE_SOURCE_DIR}/x86_64-anyos.json
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

# ============================================================
# 5. User Programs (flat binaries)
# ============================================================
set(SYSROOT_DIR "${CMAKE_BINARY_DIR}/sysroot")
set(FILTERED_COPY_SCRIPT "${CMAKE_SOURCE_DIR}/cmake/CopyTreeFiltered.cmake")

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
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/users/perm")
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
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Applications/Management")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/src")
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/system/src")
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

set(ANYOS_SELFHOST_ROOT "${SYSROOT_DIR}/Libraries/system/src/anyos")
file(GLOB_RECURSE ANYOS_SELFHOST_ANYRC_SRCS CONFIGURE_DEPENDS
  "${CMAKE_SOURCE_DIR}/libs/anyrc/*"
  "${CMAKE_SOURCE_DIR}/libs/stdlib/*"
  "${CMAKE_SOURCE_DIR}/libs/libstd/*"
  "${CMAKE_SOURCE_DIR}/libs/libheap/*"
  "${CMAKE_SOURCE_DIR}/libs/dynlink/*"
  "${CMAKE_SOURCE_DIR}/libs/libcorevm_client/*"
  "${CMAKE_SOURCE_DIR}/libs/libhttp_client/*"
  "${CMAKE_SOURCE_DIR}/libs/libzip_client/*"
  "${CMAKE_SOURCE_DIR}/bin/acargo/*"
  "${CMAKE_SOURCE_DIR}/bin/anyrc/*"
  "${CMAKE_SOURCE_DIR}/docs/crust-ccargo-api.md"
)

add_custom_command(
  OUTPUT ${ANYOS_SELFHOST_ROOT}/.stamp
  COMMAND ${CMAKE_COMMAND} -E make_directory ${ANYOS_SELFHOST_ROOT}
  COMMAND ${CMAKE_COMMAND} -E make_directory ${ANYOS_SELFHOST_ROOT}/bin
  COMMAND ${CMAKE_COMMAND} -E make_directory ${ANYOS_SELFHOST_ROOT}/libs
  COMMAND ${CMAKE_COMMAND} -E make_directory ${ANYOS_SELFHOST_ROOT}/docs
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
    ${CMAKE_SOURCE_DIR}/Cargo.toml
    ${ANYOS_SELFHOST_ROOT}/Cargo.toml
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
    ${CMAKE_SOURCE_DIR}/Cargo.lock
    ${ANYOS_SELFHOST_ROOT}/Cargo.lock
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
    ${CMAKE_SOURCE_DIR}/VERSION
    ${ANYOS_SELFHOST_ROOT}/VERSION
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/anyrc
    -DDST=${ANYOS_SELFHOST_ROOT}/libs/anyrc
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/stdlib
    -DDST=${ANYOS_SELFHOST_ROOT}/libs/stdlib
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/libstd
    -DDST=${ANYOS_SELFHOST_ROOT}/libs/libstd
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/libheap
    -DDST=${ANYOS_SELFHOST_ROOT}/libs/libheap
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/dynlink
    -DDST=${ANYOS_SELFHOST_ROOT}/libs/dynlink
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/libcorevm_client
    -DDST=${ANYOS_SELFHOST_ROOT}/libs/libcorevm_client
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/libhttp_client
    -DDST=${ANYOS_SELFHOST_ROOT}/libs/libhttp_client
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/libzip_client
    -DDST=${ANYOS_SELFHOST_ROOT}/libs/libzip_client
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/bin/acargo
    -DDST=${ANYOS_SELFHOST_ROOT}/bin/acargo
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/bin/anyrc
    -DDST=${ANYOS_SELFHOST_ROOT}/bin/anyrc
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND} -E rm -rf
    ${ANYOS_SELFHOST_ROOT}/bin/acargo/target
    ${ANYOS_SELFHOST_ROOT}/bin/anyrc/target
    ${ANYOS_SELFHOST_ROOT}/libs/anyrc/target
    ${ANYOS_SELFHOST_ROOT}/libs/stdlib/target
    ${ANYOS_SELFHOST_ROOT}/libs/libstd/target
    ${ANYOS_SELFHOST_ROOT}/libs/libheap/target
    ${ANYOS_SELFHOST_ROOT}/libs/dynlink/target
    ${ANYOS_SELFHOST_ROOT}/libs/libcorevm_client/target
    ${ANYOS_SELFHOST_ROOT}/libs/libhttp_client/target
    ${ANYOS_SELFHOST_ROOT}/libs/libzip_client/target
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
    ${CMAKE_SOURCE_DIR}/docs/crust-ccargo-api.md
    ${ANYOS_SELFHOST_ROOT}/docs/crust-ccargo-api.md
  COMMAND ${CMAKE_COMMAND} -E touch ${ANYOS_SELFHOST_ROOT}/.stamp
  DEPENDS
    ${CMAKE_SOURCE_DIR}/Cargo.toml
    ${CMAKE_SOURCE_DIR}/Cargo.lock
    ${CMAKE_SOURCE_DIR}/VERSION
    ${ANYOS_SELFHOST_ANYRC_SRCS}
  COMMENT "Installing Rust self-hosting sources to sysroot"
)
set(SELFHOST_SYSROOT_DEPS ${ANYOS_SELFHOST_ROOT}/.stamp)
set(ISO_SYSROOT_DEPS ${ANYOS_SELFHOST_ROOT}/.stamp)

set(ANYOS_SOURCE_MIRROR_ROOT "${SYSROOT_DIR}/Libraries/sources/anyos")
file(GLOB_RECURSE ANYOS_SOURCE_MIRROR_DEPS CONFIGURE_DEPENDS
  "${CMAKE_SOURCE_DIR}/bin/*/Cargo.toml"
  "${CMAKE_SOURCE_DIR}/bin/*/Cargo.lock"
  "${CMAKE_SOURCE_DIR}/bin/*/build.rs"
  "${CMAKE_SOURCE_DIR}/bin/*/src/*"
  "${CMAKE_SOURCE_DIR}/libs/anyrc/*"
  "${CMAKE_SOURCE_DIR}/libs/stdlib/*"
  "${CMAKE_SOURCE_DIR}/libs/libstd/Cargo.toml"
  "${CMAKE_SOURCE_DIR}/libs/libstd/Cargo.lock"
  "${CMAKE_SOURCE_DIR}/libs/libstd/src/*"
  "${CMAKE_SOURCE_DIR}/libs/libheap/*"
  "${CMAKE_SOURCE_DIR}/libs/dynlink/*"
  "${CMAKE_SOURCE_DIR}/libs/libcorevm_client/*"
  "${CMAKE_SOURCE_DIR}/libs/libhttp_client/*"
  "${CMAKE_SOURCE_DIR}/libs/libzip_client/*"
  "${CMAKE_SOURCE_DIR}/docs/crust-ccargo-api.md"
)

add_custom_command(
  OUTPUT ${ANYOS_SOURCE_MIRROR_ROOT}/.stamp
  COMMAND ${CMAKE_COMMAND} -E make_directory ${ANYOS_SOURCE_MIRROR_ROOT}
  COMMAND ${CMAKE_COMMAND} -E make_directory ${ANYOS_SOURCE_MIRROR_ROOT}/libs
  COMMAND ${CMAKE_COMMAND} -E make_directory ${ANYOS_SOURCE_MIRROR_ROOT}/docs
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
    ${CMAKE_SOURCE_DIR}/Cargo.toml
    ${ANYOS_SOURCE_MIRROR_ROOT}/Cargo.toml
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
    ${CMAKE_SOURCE_DIR}/Cargo.lock
    ${ANYOS_SOURCE_MIRROR_ROOT}/Cargo.lock
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
    ${CMAKE_SOURCE_DIR}/VERSION
    ${ANYOS_SOURCE_MIRROR_ROOT}/VERSION
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/bin
    -DDST=${ANYOS_SOURCE_MIRROR_ROOT}/bin
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/anyrc
    -DDST=${ANYOS_SOURCE_MIRROR_ROOT}/libs/anyrc
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/stdlib
    -DDST=${ANYOS_SOURCE_MIRROR_ROOT}/libs/stdlib
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/libstd
    -DDST=${ANYOS_SOURCE_MIRROR_ROOT}/libs/libstd
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/libheap
    -DDST=${ANYOS_SOURCE_MIRROR_ROOT}/libs/libheap
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/dynlink
    -DDST=${ANYOS_SOURCE_MIRROR_ROOT}/libs/dynlink
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/libcorevm_client
    -DDST=${ANYOS_SOURCE_MIRROR_ROOT}/libs/libcorevm_client
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/libhttp_client
    -DDST=${ANYOS_SOURCE_MIRROR_ROOT}/libs/libhttp_client
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND}
    -DSRC=${CMAKE_SOURCE_DIR}/libs/libzip_client
    -DDST=${ANYOS_SOURCE_MIRROR_ROOT}/libs/libzip_client
    -P ${FILTERED_COPY_SCRIPT}
  COMMAND ${CMAKE_COMMAND} -E rm -rf
    ${ANYOS_SOURCE_MIRROR_ROOT}/bin/acargo/target
    ${ANYOS_SOURCE_MIRROR_ROOT}/bin/anyrc/target
    ${ANYOS_SOURCE_MIRROR_ROOT}/libs/anyrc/target
    ${ANYOS_SOURCE_MIRROR_ROOT}/libs/stdlib/target
    ${ANYOS_SOURCE_MIRROR_ROOT}/libs/libstd/target
    ${ANYOS_SOURCE_MIRROR_ROOT}/libs/libheap/target
    ${ANYOS_SOURCE_MIRROR_ROOT}/libs/dynlink/target
    ${ANYOS_SOURCE_MIRROR_ROOT}/libs/libcorevm_client/target
    ${ANYOS_SOURCE_MIRROR_ROOT}/libs/libhttp_client/target
    ${ANYOS_SOURCE_MIRROR_ROOT}/libs/libzip_client/target
  COMMAND ${CMAKE_COMMAND} -E remove -f
    ${ANYOS_SOURCE_MIRROR_ROOT}/bin/git/bearssl_stream.o
    ${ANYOS_SOURCE_MIRROR_ROOT}/bin/git/main.o
    ${ANYOS_SOURCE_MIRROR_ROOT}/bin/git/git.elf
    ${ANYOS_SOURCE_MIRROR_ROOT}/bin/make/make.o
    ${ANYOS_SOURCE_MIRROR_ROOT}/bin/make/make.elf
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
    ${CMAKE_SOURCE_DIR}/docs/crust-ccargo-api.md
    ${ANYOS_SOURCE_MIRROR_ROOT}/docs/crust-ccargo-api.md
  COMMAND ${CMAKE_COMMAND} -E touch ${ANYOS_SOURCE_MIRROR_ROOT}/.stamp
  DEPENDS
    ${CMAKE_SOURCE_DIR}/Cargo.toml
    ${CMAKE_SOURCE_DIR}/Cargo.lock
    ${CMAKE_SOURCE_DIR}/VERSION
    ${ANYOS_SOURCE_MIRROR_DEPS}
  COMMENT "Installing anyOS sample sources to /Libraries/sources/anyos"
)
set(SELFHOST_SYSROOT_DEPS
  ${SELFHOST_SYSROOT_DEPS}
  ${ANYOS_SOURCE_MIRROR_ROOT}/.stamp
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
  "${CMAKE_SOURCE_DIR}/system/daemons/*/src/*.rs"
  "${CMAKE_SOURCE_DIR}/system/compositor/*/src/*.rs"
  "${CMAKE_SOURCE_DIR}/system/utilities/*/src/*.rs"
  "${CMAKE_SOURCE_DIR}/system/compositor/compositor/src/*.rs"
)
file(GLOB _WS_TOMLS CONFIGURE_DEPENDS
  "${CMAKE_SOURCE_DIR}/bin/*/Cargo.toml"
  "${CMAKE_SOURCE_DIR}/apps/*/Cargo.toml"
  "${CMAKE_SOURCE_DIR}/system/*/Cargo.toml"
  "${CMAKE_SOURCE_DIR}/system/daemons/*/Cargo.toml"
  "${CMAKE_SOURCE_DIR}/system/compositor/*/Cargo.toml"
  "${CMAKE_SOURCE_DIR}/system/utilities/*/Cargo.toml"
  "${CMAKE_SOURCE_DIR}/system/compositor/compositor/Cargo.toml"
)
file(GLOB _WS_BUILD_RS CONFIGURE_DEPENDS
  "${CMAKE_SOURCE_DIR}/bin/*/build.rs"
  "${CMAKE_SOURCE_DIR}/apps/*/build.rs"
  "${CMAKE_SOURCE_DIR}/system/*/build.rs"
  "${CMAKE_SOURCE_DIR}/system/daemons/*/build.rs"
  "${CMAKE_SOURCE_DIR}/system/compositor/*/build.rs"
  "${CMAKE_SOURCE_DIR}/system/utilities/*/build.rs"
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
  # surf/libhttp TLS now pure Rust (libtls), but libhttp .so build is x86_64-only for now
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

# .app bundles (placed in /Applications/{DISPLAY_NAME}.app/ or a subfolder)
# Uses mkappbundle for validated bundling with ELF auto-conversion via anyelf.
function(add_app_in_folder NAME SRC_DIR DISPLAY_NAME SUBDIR)
  if("${SUBDIR}" STREQUAL "")
    set(APP_DIR "${SYSROOT_DIR}/Applications/${DISPLAY_NAME}.app")
  else()
    set(APP_DIR "${SYSROOT_DIR}/Applications/${SUBDIR}/${DISPLAY_NAME}.app")
  endif()
  get_filename_component(APP_PARENT "${APP_DIR}" DIRECTORY)
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
    COMMAND ${CMAKE_COMMAND} -E make_directory "${APP_PARENT}"
    COMMAND ${MKAPPBUNDLE_EXECUTABLE} ${_BUNDLE_ARGS}
    DEPENDS ${_BUNDLE_DEPS}
    COMMENT "Packaging ${DISPLAY_NAME}.app (mkappbundle)"
    COMMAND_EXPAND_LISTS
    VERBATIM
  )
  set(APP_BINS ${APP_BINS} "${APP_DIR}/${DISPLAY_NAME}" PARENT_SCOPE)
endfunction()

function(add_app NAME SRC_DIR DISPLAY_NAME)
  add_app_in_folder(${NAME} ${SRC_DIR} "${DISPLAY_NAME}" "")
  set(APP_BINS ${APP_BINS} PARENT_SCOPE)
endfunction()

# System .app bundles (placed in /System/{DISPLAY_NAME}.app/ — no capability prompts)
function(add_system_app NAME SRC_DIR DISPLAY_NAME)
  set(APP_DIR "${SYSROOT_DIR}/System/${DISPLAY_NAME}.app")
  set(ELF "${USER_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/${NAME}.elf")
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
  if(EXISTS "${SRC_DIR}/build.conf")
    list(APPEND _BUNDLE_ARGS -r "${SRC_DIR}/build.conf")
    list(APPEND _BUNDLE_DEPS "${SRC_DIR}/build.conf")
  endif()
  add_custom_command(
    OUTPUT "${APP_DIR}/${DISPLAY_NAME}"
    COMMAND ${CMAKE_COMMAND} -E rm -rf "${APP_DIR}"
    COMMAND ${MKAPPBUNDLE_EXECUTABLE} ${_BUNDLE_ARGS}
    DEPENDS ${_BUNDLE_DEPS}
    COMMENT "Packaging ${DISPLAY_NAME}.app → /System/ (mkappbundle)"
    COMMAND_EXPAND_LISTS
    VERBATIM
  )
  set(SYSTEM_BINS ${SYSTEM_BINS} "${APP_DIR}/${DISPLAY_NAME}" PARENT_SCOPE)
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

# GPU drivers (.drv) — built like shared libs but installed to System/Drivers/gpu/
set(DRV_TARGET_DIR "${CMAKE_BINARY_DIR}/drv-target")
function(add_gpu_driver NAME SRC_DIR)
  set(LIB_A "${DRV_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/lib${NAME}.a")
  set(DRV_FILE "${CMAKE_BINARY_DIR}/drivers/${NAME}.drv")
  file(GLOB_RECURSE _DRV_RS CONFIGURE_DEPENDS "${SRC_DIR}/src/*.rs")
  # Step 1: Cargo -> static archive (.a)
  add_custom_command(
    OUTPUT ${LIB_A}
    COMMAND ${CMAKE_COMMAND} -E env "RUSTFLAGS=-Awarnings"
      ${CARGO_EXECUTABLE} build --release --quiet
      --manifest-path ${SRC_DIR}/Cargo.toml
      --target ${USER_TARGET_JSON}
      --target-dir ${DRV_TARGET_DIR}
      -Z build-std=core,alloc
    DEPENDS
      ${SRC_DIR}/Cargo.toml
      ${_DRV_RS}
      ${USER_TARGET_JSON}
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building GPU driver: ${NAME} (Cargo)"
  )
  # Step 2: anyld -> .drv (ET_DYN shared object)
  add_custom_command(
    OUTPUT ${DRV_FILE}
    COMMAND ${ANYLD_EXECUTABLE} -q
      -o ${DRV_FILE}
      -e ${SRC_DIR}/exports.def
      ${LIB_A}
    DEPENDS ${LIB_A} ${SRC_DIR}/exports.def ${ANYLD_EXECUTABLE}
    COMMENT "Linking ${NAME}.drv (anyld)"
  )
  # Step 3: Copy to sysroot
  add_custom_command(
    OUTPUT ${SYSROOT_DIR}/System/Drivers/gpu/${NAME}.drv
    COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/System/Drivers/gpu
    COMMAND ${CMAKE_COMMAND} -E copy ${DRV_FILE} ${SYSROOT_DIR}/System/Drivers/gpu/${NAME}.drv
    DEPENDS ${DRV_FILE}
    COMMENT "Installing ${NAME}.drv to sysroot"
  )
  set(DLL_BINS ${DLL_BINS} ${SYSROOT_DIR}/System/Drivers/gpu/${NAME}.drv PARENT_SCOPE)
endfunction()

# GPU drivers
add_gpu_driver(svga3d ${CMAKE_SOURCE_DIR}/drivers/gpu/svga3d)
add_gpu_driver(virgl ${CMAKE_SOURCE_DIR}/drivers/gpu/virgl)

add_shared_lib(libanyui ${CMAKE_SOURCE_DIR}/libs/libanyui)
add_shared_lib(libfont ${CMAKE_SOURCE_DIR}/libs/libfont)
add_shared_lib(libdb ${CMAKE_SOURCE_DIR}/libs/libdb)
add_shared_lib(libzip ${CMAKE_SOURCE_DIR}/libs/libzip)
add_shared_lib(libini ${CMAKE_SOURCE_DIR}/libs/libini)
add_shared_lib(libsvg ${CMAKE_SOURCE_DIR}/libs/libsvg)
add_shared_lib(libgl ${CMAKE_SOURCE_DIR}/libs/libgl)
add_shared_lib(libm ${CMAKE_SOURCE_DIR}/libs/libm)
add_shared_lib(libphysics ${CMAKE_SOURCE_DIR}/libs/libphysics)
# --- CoreVM (optional, loaded from git submodule at corevm/) ---
# libcorevm lives in its own repository. When the corevm submodule is checked
# out, the library and BIOS are built; otherwise they are silently skipped.
set(_COREVM_LIB_DIR "${CMAKE_SOURCE_DIR}/corevm/libs/libcorevm")
if(NOT ANYOS_ARCH STREQUAL "arm64" AND EXISTS "${_COREVM_LIB_DIR}/Cargo.toml")
  # Symlink the real anyOS crates into the corevm stubs directory so that
  # libcorevm's Cargo.toml resolves libheap/libsyscall to the real implementations.
  set(_STUBS_DIR "${CMAKE_SOURCE_DIR}/corevm/libs/stubs")
  foreach(_DEP libheap libsyscall)
    set(_STUB "${_STUBS_DIR}/${_DEP}")
    set(_REAL "${CMAKE_SOURCE_DIR}/libs/${_DEP}")
    if(EXISTS "${_REAL}" AND NOT IS_SYMLINK "${_STUB}")
      file(REMOVE_RECURSE "${_STUB}")
      execute_process(COMMAND ${CMAKE_COMMAND} -E create_symlink "${_REAL}" "${_STUB}")
    endif()
  endforeach()

  add_shared_lib(libcorevm ${_COREVM_LIB_DIR})

  # --- CoreVM BIOS (assembled with NASM, copied to sysroot) ---
  set(_COREVM_BIOS_DIR "${_COREVM_LIB_DIR}/bios")
  set(_COREVM_BIOS_BIN "${CMAKE_BINARY_DIR}/bios.bin")
  set(_COREVM_BIOS_SYSROOT "${SYSROOT_DIR}/Libraries/libcorevm/bios/bios.bin")
  file(GLOB _COREVM_BIOS_SRC "${_COREVM_BIOS_DIR}/*.asm")
  file(MAKE_DIRECTORY "${SYSROOT_DIR}/Libraries/libcorevm/bios")
  add_custom_command(
    OUTPUT ${_COREVM_BIOS_BIN}
    COMMAND nasm -f bin -I ${_COREVM_BIOS_DIR}/ -o ${_COREVM_BIOS_BIN} ${_COREVM_BIOS_DIR}/bios.asm
    DEPENDS ${_COREVM_BIOS_SRC}
    COMMENT "Assembling CoreVM BIOS"
  )
  add_custom_command(
    OUTPUT ${_COREVM_BIOS_SYSROOT}
    COMMAND ${CMAKE_COMMAND} -E copy ${_COREVM_BIOS_BIN} ${_COREVM_BIOS_SYSROOT}
    DEPENDS ${_COREVM_BIOS_BIN}
    COMMENT "Installing CoreVM BIOS to sysroot"
  )
  add_custom_target(corevm_bios ALL DEPENDS ${_COREVM_BIOS_SYSROOT})
elseif(NOT ANYOS_ARCH STREQUAL "arm64")
  message(STATUS "CoreVM submodule not found at corevm/ — skipping libcorevm and BIOS")
endif()

# --- libhttp (custom: TLS via libtls, pure Rust) ---
if(NOT ANYOS_ARCH STREQUAL "arm64")
  set(_LIBHTTP_SRC "${CMAKE_SOURCE_DIR}/libs/libhttp")
  set(_LIBHTTP_A "${SHLIB_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/liblibhttp.a")
  set(_LIBHTTP_SO "${CMAKE_BINARY_DIR}/shlib/libhttp.so")
  file(GLOB_RECURSE _LIBHTTP_RS CONFIGURE_DEPENDS "${_LIBHTTP_SRC}/src/*.rs")
  file(GLOB_RECURSE _LIBTLS_RS CONFIGURE_DEPENDS "${CMAKE_SOURCE_DIR}/libs/libtls/src/*.rs")

  # Step 1: Cargo → static archive (.a) — libtls is built as a Cargo dependency
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
      ${_LIBHTTP_SRC}/build.rs
      ${_LIBHTTP_RS}
      ${_LIBTLS_RS}
      ${USER_TARGET_JSON}
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
    COMMENT "Building shared library: libhttp (Cargo + libtls)"
  )

  # Step 2: anyld → .so (pure Rust, no external C libraries needed)
  add_custom_command(
    OUTPUT ${_LIBHTTP_SO}
    COMMAND ${ANYLD_EXECUTABLE} -q
      -o ${_LIBHTTP_SO}
      -e ${_LIBHTTP_SRC}/exports.def
      ${_LIBHTTP_A}
    DEPENDS ${_LIBHTTP_A}
      ${_LIBHTTP_SRC}/exports.def ${ANYLD_EXECUTABLE}
    COMMENT "Linking libhttp.so (anyld)"
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
add_rust_user_program(dd)
add_rust_user_program(mv)
add_rust_user_program(date)
add_rust_user_program(sleep)
add_rust_user_program(hostname)
add_rust_user_program(ftp)
add_rust_user_program(wget)
add_rust_user_program(speedtest)
add_rust_user_program(kstress)
add_rust_user_program(play)
add_rust_user_program(pipes)
add_rust_user_program(devlist)
add_rust_user_program(echo)
add_rust_user_program(ps)
add_rust_user_program(top)
add_rust_user_program(htop)
add_rust_user_program(uictl)
add_rust_user_program(kill)
add_rust_user_program(killall)
add_rust_user_program(reboot)
add_rust_user_program(nice)
add_rust_user_program(free)
add_rust_user_program(uptime)
add_rust_user_program(uname)
add_rust_user_program(pwd)
add_rust_user_program(wc)
add_rust_user_program(hexdump)
add_rust_user_program(head)
add_rust_user_program(tail)
add_rust_user_program(more)
add_rust_user_program(clear)
add_rust_user_program(env)
add_rust_user_program(grep)
add_rust_user_program(find)
add_rust_user_program(sort)
add_rust_user_program(sync)
add_rust_user_program(uniq)
add_rust_user_program(rev)
add_rust_user_program(stat)
add_rust_user_program(ln)
add_rust_user_program(readlink)
add_rust_user_program(df)
add_rust_user_program(du)
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
add_rust_user_program(sudo)
add_rust_user_program(echoserver)
add_rust_user_program(wifi)
add_rust_user_program(netstat)
add_rust_user_program(route)
add_rust_user_program(dcpdump)
add_rust_user_program(logd)
add_rust_user_program(ami)
add_rust_user_program(vi)
add_rust_user_program(crond)
add_rust_user_program(crontab)
add_rust_user_program(sed)
add_rust_user_program(xargs)
add_rust_user_program(awk)
add_rust_user_program(nano)
add_rust_user_program(ac)
add_rust_user_program(file)
add_rust_user_program(httpd)
add_rust_user_program(vncd)
add_rust_user_program(vdagent)
add_rust_user_program(vmd)
add_rust_user_program(vmctl)
add_rust_user_program(ftpd)
add_rust_user_program(zip)
add_rust_user_program(unzip)
add_rust_user_program(gzip)
add_rust_user_program(tar)
add_rust_user_program(apkg)
add_rust_user_program(mode)
add_rust_user_program(bcedit)
add_rust_user_program(sstore)
add_rust_user_program(sget)
add_rust_user_program(sdel)
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
add_rust_user_program(ntp)
add_rust_user_program(ntpd)
add_rust_user_program(neofetch)
add_rust_user_program(nvi)
add_rust_user_program(crust)
add_rust_user_program(ccargo)
add_rust_user_program(cgit)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/cargo
  COMMAND ${CMAKE_COMMAND} -E remove -f ${SYSROOT_DIR}/System/bin/cargo
  COMMAND ${CMAKE_COMMAND} -E create_symlink ccargo ${SYSROOT_DIR}/System/bin/cargo
  DEPENDS ${SYSROOT_DIR}/System/bin/ccargo
  COMMENT "Linking cargo -> ccargo"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/System/bin/cargo)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/rustc
  COMMAND ${CMAKE_COMMAND} -E remove -f ${SYSROOT_DIR}/System/bin/rustc
  COMMAND ${CMAKE_COMMAND} -E create_symlink crust ${SYSROOT_DIR}/System/bin/rustc
  DEPENDS ${SYSROOT_DIR}/System/bin/crust
  COMMENT "Linking rustc -> crust"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/System/bin/rustc)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/git
  COMMAND ${CMAKE_COMMAND} -E remove -f ${SYSROOT_DIR}/System/bin/git
  COMMAND ${CMAKE_COMMAND} -E create_symlink cgit ${SYSROOT_DIR}/System/bin/git
  DEPENDS ${SYSROOT_DIR}/System/bin/cgit
  COMMENT "Linking git -> cgit"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/System/bin/git)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/anyrc
  COMMAND ${CMAKE_COMMAND} -E remove -f ${SYSROOT_DIR}/System/bin/anyrc
  COMMAND ${CMAKE_COMMAND} -E create_symlink crust ${SYSROOT_DIR}/System/bin/anyrc
  DEPENDS ${SYSROOT_DIR}/System/bin/crust
  COMMENT "Linking anyrc -> crust"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/System/bin/anyrc)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/acargo
  COMMAND ${CMAKE_COMMAND} -E remove -f ${SYSROOT_DIR}/System/bin/acargo
  COMMAND ${CMAKE_COMMAND} -E create_symlink ccargo ${SYSROOT_DIR}/System/bin/acargo
  DEPENDS ${SYSROOT_DIR}/System/bin/ccargo
  COMMENT "Linking acargo -> ccargo"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/System/bin/acargo)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/bin/agit
  COMMAND ${CMAKE_COMMAND} -E remove -f ${SYSROOT_DIR}/System/bin/agit
  COMMAND ${CMAKE_COMMAND} -E create_symlink cgit ${SYSROOT_DIR}/System/bin/agit
  DEPENDS ${SYSROOT_DIR}/System/bin/cgit
  COMMENT "Linking agit -> cgit"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/System/bin/agit)
# Privileged sbin programs
add_rust_sbin_program(adduser)
add_rust_sbin_program(deluser)
add_rust_sbin_program(addgroup)
add_rust_sbin_program(delgroup)
add_rust_user_program(passwd)
add_rust_sbin_program(fdisk)
add_rust_sbin_program(install)

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
# CoreFS administrative tools (mkfs.corefs, fsck.corefs, corefs-*)
# Shipped under /System/sbin/ — require raw block-device access.
# Package names use underscores because Cargo rejects '.' in names;
# the sysroot binary keeps the canonical dotted name.
# ============================================================
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/sbin/mkfs.corefs
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${USER_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/mkfs_corefs.elf
    ${SYSROOT_DIR}/System/sbin/mkfs.corefs
  DEPENDS ${WORKSPACE_STAMP} ${ANYELF_EXECUTABLE}
  COMMENT "Converting mkfs.corefs ELF to flat binary"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/System/sbin/mkfs.corefs)

add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/sbin/fsck.corefs
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${USER_TARGET_DIR}/${USER_TARGET_TRIPLE}/release/fsck_corefs.elf
    ${SYSROOT_DIR}/System/sbin/fsck.corefs
  DEPENDS ${WORKSPACE_STAMP} ${ANYELF_EXECUTABLE}
  COMMENT "Converting fsck.corefs ELF to flat binary"
)
list(APPEND RUST_USER_BINS ${SYSROOT_DIR}/System/sbin/fsck.corefs)

add_rust_sbin_program(corefs-dump)
add_rust_sbin_program(corefs-scrub)
add_rust_sbin_program(corefs-defrag)
add_rust_sbin_program(corefs-snapshot)
add_rust_sbin_program(corefs-resize)
add_rust_sbin_program(corefs-tier)

# ============================================================
# System programs (/System/)
# ============================================================
set(SYSTEM_BINS "")
add_rust_system_program(svc)
add_rust_system_program(init)
add_rust_system_program(audiomon)
add_rust_system_program(wifimon)
add_rust_system_program(netmon)
add_rust_system_program(inputmon)
add_rust_system_program(login)
add_rust_system_program(permdialog)
add_rust_system_program(notifyd)
add_rust_user_program(amid)
add_rust_user_program(confd)
add_rust_user_program(searchd)
add_rust_user_program(dnsd)
add_rust_user_program(networkd)
add_rust_user_program(textmode_console)

# ============================================================
# Desktop GUI applications -> .app bundles in /Applications/
# ============================================================
set(APP_BINS "")
add_app(terminal    ${CMAKE_SOURCE_DIR}/system/utilities/terminal     "Terminal")
add_app(shell       ${CMAKE_SOURCE_DIR}/system/utilities/shell        "Shell")
add_app_in_folder(taskmanager ${CMAKE_SOURCE_DIR}/system/utilities/taskmanager  "Activity Monitor" "Management")
add_app(settings    ${CMAKE_SOURCE_DIR}/system/compositor/settings     "Settings")
add_app(finder      ${CMAKE_SOURCE_DIR}/system/compositor/finder       "Finder")
add_app(diskutil   ${CMAKE_SOURCE_DIR}/system/utilities/diskutil     "Disk Utility")
add_app(eventviewer ${CMAKE_SOURCE_DIR}/system/utilities/eventviewer  "Event Viewer")
add_app(anybout    ${CMAKE_SOURCE_DIR}/system/compositor/anybout     "About anyOS")
add_app(anytrace    ${CMAKE_SOURCE_DIR}/system/utilities/anytrace     "anyTrace")
add_app(notepad     ${CMAKE_SOURCE_DIR}/apps/notepad        "Notepad")
add_app(imgview     ${CMAKE_SOURCE_DIR}/apps/imgview        "Image Viewer")
add_app(videoplayer ${CMAKE_SOURCE_DIR}/apps/videoplayer    "Video Player")
add_app_in_folder(diagnostics ${CMAKE_SOURCE_DIR}/apps/diagnostics    "Diagnostics" "Management")
add_app(calc        ${CMAKE_SOURCE_DIR}/apps/calc           "Calculator")
add_app(fontviewer  ${CMAKE_SOURCE_DIR}/apps/fontviewer     "Font Viewer")
add_app(clock       ${CMAKE_SOURCE_DIR}/apps/clock          "Clock")
add_app(screenshot  ${CMAKE_SOURCE_DIR}/apps/screenshot     "Screenshot")
# surf TLS now pure Rust (libtls) — skip on ARM64 only because libhttp .so is x86_64
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
add_app(ftp-settings ${CMAKE_SOURCE_DIR}/apps/ftp-settings "FTP Settings")
add_app(anybench    ${CMAKE_SOURCE_DIR}/apps/anybench      "anyBench")
add_app(gldemo      ${CMAKE_SOURCE_DIR}/apps/gldemo        "GL Demo")
add_app(forger      ${CMAKE_SOURCE_DIR}/apps/forger        "Forger")
add_app(iconview    ${CMAKE_SOURCE_DIR}/apps/iconview      "Icon Browser")
add_app(store       ${CMAKE_SOURCE_DIR}/apps/store         "App Store")
if(NOT ANYOS_ARCH STREQUAL "arm64")
  add_app(vmmanager   ${CMAKE_SOURCE_DIR}/apps/vmmanager    "VM Manager")
endif()
add_app(anymail     ${CMAKE_SOURCE_DIR}/apps/anymail       "anyMail")
add_app(anyzilla    ${CMAKE_SOURCE_DIR}/apps/anyzilla      "anyzilla")
add_app_in_folder(amiconsole  ${CMAKE_SOURCE_DIR}/system/utilities/amiconsole  "AMI Console" "Management")
add_app_in_folder(backuprestore ${CMAKE_SOURCE_DIR}/system/utilities/backuprestore "Backup & Restore" "Management")
add_app_in_folder(configexplorer ${CMAKE_SOURCE_DIR}/system/utilities/configexplorer "Config Explorer" "Management")
add_app_in_folder(servicemanager ${CMAKE_SOURCE_DIR}/system/utilities/servicemanager "Service Manager" "Management")
add_app(notifications ${CMAKE_SOURCE_DIR}/apps/notifications "Notifications")
add_app(installer    ${CMAKE_SOURCE_DIR}/apps/installer     "Installer")
add_app(keyboard     ${CMAKE_SOURCE_DIR}/apps/keyboard      "Keyboard")
add_app(updater      ${CMAKE_SOURCE_DIR}/apps/updater       "Software Update")
add_app(diskusage    ${CMAKE_SOURCE_DIR}/apps/diskusage     "Disk Usage")
add_system_app(runner       ${CMAKE_SOURCE_DIR}/system/utilities/runner     "Runner")

# ============================================================
# Session host, Desktop shell, Crash dialog
# ============================================================

# fontd — Font server daemon (standalone workspace, kernel target)
set(FONTD_SRC_DIR ${CMAKE_SOURCE_DIR}/system/daemons/fontd)
set(FONTD_ELF "${CMAKE_BINARY_DIR}/kernel/${KERNEL_TARGET_TRIPLE}/release/fontd.elf")
add_custom_command(
  OUTPUT ${FONTD_ELF}
  COMMAND ${CMAKE_COMMAND} -E env "RUSTFLAGS=-Awarnings"
    ${CARGO_EXECUTABLE} build --release --quiet
    --manifest-path ${FONTD_SRC_DIR}/Cargo.toml
    --target ${KERNEL_TARGET_JSON}
    --target-dir ${CMAKE_BINARY_DIR}/kernel
  DEPENDS
    ${FONTD_SRC_DIR}/Cargo.toml
    ${FONTD_SRC_DIR}/build.rs
    ${FONTD_SRC_DIR}/src/main.rs
    ${FONTD_SRC_DIR}/src/cache.rs
    ${FONTD_SRC_DIR}/src/loader.rs
    ${FONTD_SRC_DIR}/src/protocol.rs
    ${STDLIB_DEPS}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building system program: fontd"
)
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/fontd
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${FONTD_ELF}
    ${SYSROOT_DIR}/System/fontd
  DEPENDS ${FONTD_ELF} ${ANYELF_EXECUTABLE}
  COMMENT "Converting fontd ELF to flat binary"
)
list(APPEND SYSTEM_BINS ${SYSROOT_DIR}/System/fontd)

# Sessionhost (standalone workspace, kernel target)
set(SESSIONHOST_SRC_DIR ${CMAKE_SOURCE_DIR}/system/compositor/sessionhost)
set(SESSIONHOST_ELF "${CMAKE_BINARY_DIR}/kernel/${KERNEL_TARGET_TRIPLE}/release/sessionhost.elf")
add_custom_command(
  OUTPUT ${SESSIONHOST_ELF}
  COMMAND ${CMAKE_COMMAND} -E env "RUSTFLAGS=-Awarnings"
    ${CARGO_EXECUTABLE} build --release --quiet
    --manifest-path ${SESSIONHOST_SRC_DIR}/Cargo.toml
    --target ${KERNEL_TARGET_JSON}
    --target-dir ${CMAKE_BINARY_DIR}/kernel
  DEPENDS
    ${SESSIONHOST_SRC_DIR}/Cargo.toml
    ${SESSIONHOST_SRC_DIR}/build.rs
    ${SESSIONHOST_SRC_DIR}/src/main.rs
    ${STDLIB_DEPS}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building system program: sessionhost"
)
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/Sessionhost
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${SESSIONHOST_ELF}
    ${SYSROOT_DIR}/System/Sessionhost
  DEPENDS ${SESSIONHOST_ELF} ${ANYELF_EXECUTABLE}
  COMMENT "Converting sessionhost ELF to flat binary"
)
list(APPEND SYSTEM_BINS ${SYSROOT_DIR}/System/Sessionhost)

# Desktop shell (standalone workspace, kernel target)
set(DESKTOPD_SRC_DIR ${CMAKE_SOURCE_DIR}/system/daemons/desktopd)
set(DESKTOPD_ELF "${CMAKE_BINARY_DIR}/kernel/${KERNEL_TARGET_TRIPLE}/release/desktopd.elf")
add_custom_command(
  OUTPUT ${DESKTOPD_ELF}
  COMMAND ${CMAKE_COMMAND} -E env "RUSTFLAGS=-Awarnings"
    ${CARGO_EXECUTABLE} build --release --quiet
    --manifest-path ${DESKTOPD_SRC_DIR}/Cargo.toml
    --target ${KERNEL_TARGET_JSON}
    --target-dir ${CMAKE_BINARY_DIR}/kernel
  DEPENDS
    ${DESKTOPD_SRC_DIR}/Cargo.toml
    ${DESKTOPD_SRC_DIR}/build.rs
    ${DESKTOPD_SRC_DIR}/src/main.rs
    ${DESKTOPD_SRC_DIR}/src/ipc.rs
    ${STDLIB_DEPS}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building system program: desktopd"
)
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/Desktop
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${DESKTOPD_ELF}
    ${SYSROOT_DIR}/System/Desktop
  DEPENDS ${DESKTOPD_ELF} ${ANYELF_EXECUTABLE}
  COMMENT "Converting desktopd ELF to flat binary"
)
list(APPEND SYSTEM_BINS ${SYSROOT_DIR}/System/Desktop)

# CrashDialog (standalone workspace, kernel target, .app bundle)
set(CRASHDIALOG_SRC_DIR ${CMAKE_SOURCE_DIR}/system/compositor/crashdialog)
set(CRASHDIALOG_ELF "${CMAKE_BINARY_DIR}/kernel/${KERNEL_TARGET_TRIPLE}/release/crashdialog.elf")
add_custom_command(
  OUTPUT ${CRASHDIALOG_ELF}
  COMMAND ${CMAKE_COMMAND} -E env "RUSTFLAGS=-Awarnings"
    ${CARGO_EXECUTABLE} build --release --quiet
    --manifest-path ${CRASHDIALOG_SRC_DIR}/Cargo.toml
    --target ${KERNEL_TARGET_JSON}
    --target-dir ${CMAKE_BINARY_DIR}/kernel
  DEPENDS
    ${CRASHDIALOG_SRC_DIR}/Cargo.toml
    ${CRASHDIALOG_SRC_DIR}/build.rs
    ${CRASHDIALOG_SRC_DIR}/src/main.rs
    ${STDLIB_DEPS}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building system program: crashdialog"
)
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/System/CrashDialog.app/CrashDialog
  COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/System/CrashDialog.app
  COMMAND ${ANYELF_EXECUTABLE} bin
    ${CRASHDIALOG_ELF}
    ${SYSROOT_DIR}/System/CrashDialog.app/CrashDialog
  DEPENDS ${CRASHDIALOG_ELF} ${ANYELF_EXECUTABLE}
  COMMENT "Converting crashdialog ELF to flat binary"
)
list(APPEND SYSTEM_BINS ${SYSROOT_DIR}/System/CrashDialog.app/CrashDialog)

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
  COMMENT "Populating sysroot from tools/sysroot"
)

# Copy bootloader binaries to sysroot so the installer can find them
add_custom_command(
  OUTPUT ${SYSROOT_DIR}/boot/stage1.bin ${SYSROOT_DIR}/boot/stage2.bin
  COMMAND ${CMAKE_COMMAND} -E make_directory ${SYSROOT_DIR}/boot
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
    ${CMAKE_BINARY_DIR}/stage1.bin ${SYSROOT_DIR}/boot/stage1.bin
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
    ${CMAKE_BINARY_DIR}/stage2.bin ${SYSROOT_DIR}/boot/stage2.bin
  DEPENDS ${CMAKE_BINARY_DIR}/stage1.bin ${CMAKE_BINARY_DIR}/stage2.bin
  COMMENT "Installing bootloader binaries to sysroot"
)
add_custom_target(bootloader_sysroot DEPENDS
  ${SYSROOT_DIR}/boot/stage1.bin
  ${SYSROOT_DIR}/boot/stage2.bin
)

if(ANYOS_ARCH STREQUAL "arm64")
  file(REMOVE "${SYSROOT_DIR}/System/etc/svc/sshd")
  file(REMOVE "${SYSROOT_DIR}/System/etc/httpd/httpd.conf")
  file(REMOVE "${SYSROOT_DIR}/System/etc/httpd/sites/default")
endif()
# ── Hardware firmware blobs (WiFi, Bluetooth) ──────────────────────────────
# Copy firmware files from sysroot/ source tree into the build sysroot.
# These are vendor-provided blobs required at runtime for WiFi/BT adapters.
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/Drivers/wifi")
file(GLOB _FW_WIFI "${CMAKE_SOURCE_DIR}/sysroot/System/Drivers/wifi/*")
foreach(_fw ${_FW_WIFI})
  get_filename_component(_fname "${_fw}" NAME)
  if(NOT EXISTS "${SYSROOT_DIR}/System/Drivers/wifi/${_fname}")
    file(COPY "${_fw}" DESTINATION "${SYSROOT_DIR}/System/Drivers/wifi")
  endif()
endforeach()
if(EXISTS "${CMAKE_SOURCE_DIR}/sysroot/System/Drivers/README")
  file(COPY "${CMAKE_SOURCE_DIR}/sysroot/System/Drivers/README"
       DESTINATION "${SYSROOT_DIR}/System/Drivers")
endif()

# WiFi credential storage directory
file(MAKE_DIRECTORY "${SYSROOT_DIR}/System/etc/wifi")

# FTP public share directory with a test file for download verification.
file(MAKE_DIRECTORY "${SYSROOT_DIR}/Users/shared/ftp")
if(NOT EXISTS "${SYSROOT_DIR}/Users/shared/ftp/README.txt")
  file(COPY "${CMAKE_SOURCE_DIR}/defaults/Users/shared/ftp/README.txt"
       DESTINATION "${SYSROOT_DIR}/Users/shared/ftp")
endif()

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
    COMMENT "Provisioning Users from config/users.conf"
  )
  set(PROVISION_DEPS ${SYSROOT_DIR}/System/users/.provisioned)
endif()

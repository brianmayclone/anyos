# ============================================================
# Build System Tools (anyelf, mkimage, anyld, mkappbundle)
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
  COMMAND cc -w -O2 -std=c99 -o ${ANYELF_EXECUTABLE}
    ${BUILDSYSTEM_DIR}/anyelf/src/anyelf.c
    ${BUILDSYSTEM_DIR}/anyelf/src/convert.c
  DEPENDS ${ANYELF_SRCS}
  COMMENT "Building buildsystem tool: anyelf"
)

# On Linux, -std=c99 hides POSIX functions like strdup(); macOS exposes them by default.
if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
  set(POSIX_FLAG "-D_POSIX_C_SOURCE=200809L")
else()
  set(POSIX_FLAG "")
endif()

# mkimage — disk image builder (replaces mkimage.py)
set(MKIMAGE_EXECUTABLE "${CMAKE_BINARY_DIR}/buildsystem/mkimage${CMAKE_EXECUTABLE_SUFFIX}")
file(GLOB MKIMAGE_SRCS "${BUILDSYSTEM_DIR}/mkimage/src/*.c" "${BUILDSYSTEM_DIR}/mkimage/src/*.h")
add_custom_command(
  OUTPUT ${MKIMAGE_EXECUTABLE}
  COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/buildsystem"
  COMMAND cc -w -O2 -std=c99 ${POSIX_FLAG} -o ${MKIMAGE_EXECUTABLE}
    ${BUILDSYSTEM_DIR}/mkimage/src/mkimage.c
    ${BUILDSYSTEM_DIR}/mkimage/src/elf.c
    ${BUILDSYSTEM_DIR}/mkimage/src/fat16.c
    ${BUILDSYSTEM_DIR}/mkimage/src/exfat.c
    ${BUILDSYSTEM_DIR}/mkimage/src/gpt.c
    ${BUILDSYSTEM_DIR}/mkimage/src/iso9660.c
  DEPENDS ${MKIMAGE_SRCS}
  COMMENT "Building buildsystem tool: mkimage"
)

# anyld — ELF64 shared object linker (for future .so builds)
set(ANYLD_EXECUTABLE "${CMAKE_BINARY_DIR}/buildsystem/anyld${CMAKE_EXECUTABLE_SUFFIX}")
file(GLOB ANYLD_SRCS "${BUILDSYSTEM_DIR}/anyld/src/*.c" "${BUILDSYSTEM_DIR}/anyld/src/*.h")
add_custom_command(
  OUTPUT ${ANYLD_EXECUTABLE}
  COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/buildsystem"
  COMMAND cc -w -O2 -std=c99 ${POSIX_FLAG} -o ${ANYLD_EXECUTABLE}
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
  COMMAND cc -w -O2 -std=c99 -o ${MKAPPBUNDLE_EXECUTABLE}
    ${BUILDSYSTEM_DIR}/mkappbundle/src/mkappbundle.c
  DEPENDS ${MKAPPBUNDLE_SRCS}
  COMMENT "Building buildsystem tool: mkappbundle"
)

# apkg-build — package archive creator
set(APKG_BUILD_EXECUTABLE "${CMAKE_BINARY_DIR}/buildsystem/apkg-build${CMAKE_EXECUTABLE_SUFFIX}")
file(GLOB APKG_BUILD_SRCS "${BUILDSYSTEM_DIR}/apkg-build/src/*.c" "${BUILDSYSTEM_DIR}/apkg-build/src/*.h")
add_custom_command(
  OUTPUT ${APKG_BUILD_EXECUTABLE}
  COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/buildsystem"
  COMMAND cc -w -O2 -std=c99 ${POSIX_FLAG} -o ${APKG_BUILD_EXECUTABLE}
    ${BUILDSYSTEM_DIR}/apkg-build/src/apkg_build.c
  DEPENDS ${APKG_BUILD_SRCS}
  COMMENT "Building buildsystem tool: apkg-build"
)

# apkg-index — repository index generator
set(APKG_INDEX_EXECUTABLE "${CMAKE_BINARY_DIR}/buildsystem/apkg-index${CMAKE_EXECUTABLE_SUFFIX}")
file(GLOB APKG_INDEX_SRCS "${BUILDSYSTEM_DIR}/apkg-index/src/*.c" "${BUILDSYSTEM_DIR}/apkg-index/src/*.h")
add_custom_command(
  OUTPUT ${APKG_INDEX_EXECUTABLE}
  COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_BINARY_DIR}/buildsystem"
  COMMAND cc -w -O2 -std=c99 ${POSIX_FLAG} -o ${APKG_INDEX_EXECUTABLE}
    ${BUILDSYSTEM_DIR}/apkg-index/src/apkg_index.c
  DEPENDS ${APKG_INDEX_SRCS}
  COMMENT "Building buildsystem tool: apkg-index"
)

# Target to build all buildsystem tools
add_custom_target(buildsystem-tools
  DEPENDS ${ANYELF_EXECUTABLE} ${MKIMAGE_EXECUTABLE} ${ANYLD_EXECUTABLE} ${MKAPPBUNDLE_EXECUTABLE}
          ${APKG_BUILD_EXECUTABLE} ${APKG_INDEX_EXECUTABLE}
)

# ============================================================
# add_cxx_program() — Build a 64-bit C++ program for anyOS
# ============================================================
# Usage:
#   add_cxx_program(myapp
#     SOURCES src/main.cpp src/util.cpp
#     [C_SOURCES src/helper.c]
#     [INSTALL_DIR System/bin]
#   )
#
# Compiles C/C++ sources with the anyOS cross-compiler, links against
# crt0 + libc64 + libcxx + libc++abi + libunwind, produces an ELF64
# binary, converts to anyOS flat format, and installs to the sysroot.
#
function(add_cxx_program NAME)
  cmake_parse_arguments(CXX "" "INSTALL_DIR" "SOURCES;C_SOURCES" ${ARGN})

  if(NOT CXX_INSTALL_DIR)
    set(CXX_INSTALL_DIR "System/bin")
  endif()

  set(CXX_OUT_DIR "${CMAKE_BINARY_DIR}/cxx_programs/${NAME}")
  set(ELF_OUTPUT "${CXX_OUT_DIR}/${NAME}.elf")
  set(BIN_OUTPUT "${CXX_OUT_DIR}/${NAME}")

  set(LIBC64_DIR_  "${CMAKE_SOURCE_DIR}/libs/libc64")
  set(LIBCXX_DIR_  "${CMAKE_SOURCE_DIR}/libs/libcxx")
  set(LIBUNWIND_DIR_ "${CMAKE_SOURCE_DIR}/libs/libunwind")
  set(LIBCXXABI_DIR_ "${CMAKE_SOURCE_DIR}/libs/libcxxabi")

  set(CXX_FLAGS
    --target=x86_64-unknown-none-elf
    -ffreestanding -nostdlib -fexceptions -frtti -std=c++20 -O2 -w
    -I${LIBCXX_DIR_}/include
    -I${LIBC64_DIR_}/include
    -I${LIBUNWIND_DIR_}/include
    -I${LIBCXXABI_DIR_}/include
  )

  set(C_FLAGS
    --target=x86_64-unknown-none-elf
    -ffreestanding -nostdlib -fno-builtin -O2 -w
    -I${LIBC64_DIR_}/include
  )

  # Compile C++ sources
  set(ALL_OBJECTS "")
  foreach(SRC ${CXX_SOURCES})
    get_filename_component(SRC_NAME ${SRC} NAME_WE)
    set(OBJ "${CXX_OUT_DIR}/${SRC_NAME}.cpp.o")
    add_custom_command(
      OUTPUT ${OBJ}
      COMMAND ${CMAKE_COMMAND} -E make_directory ${CXX_OUT_DIR}
      COMMAND ${CLANGXX_EXECUTABLE} ${CXX_FLAGS} -c ${CMAKE_CURRENT_SOURCE_DIR}/${SRC} -o ${OBJ}
      DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/${SRC} ${CXX_TOOLCHAIN_DEPS}
      COMMENT "Compiling C++ ${SRC} for ${NAME}"
    )
    list(APPEND ALL_OBJECTS ${OBJ})
  endforeach()

  # Compile C sources (if any)
  foreach(SRC ${CXX_C_SOURCES})
    get_filename_component(SRC_NAME ${SRC} NAME_WE)
    set(OBJ "${CXX_OUT_DIR}/${SRC_NAME}.c.o")
    add_custom_command(
      OUTPUT ${OBJ}
      COMMAND ${CMAKE_COMMAND} -E make_directory ${CXX_OUT_DIR}
      COMMAND ${CLANG_EXECUTABLE} ${C_FLAGS} -c ${CMAKE_CURRENT_SOURCE_DIR}/${SRC} -o ${OBJ}
      DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/${SRC} ${CXX_TOOLCHAIN_DEPS}
      COMMENT "Compiling C ${SRC} for ${NAME}"
    )
    list(APPEND ALL_OBJECTS ${OBJ})
  endforeach()

  # Link: crt0 + crti + objects + libcxx + libc++abi + libunwind + libc64 [+ libgcc] + crtn
  # Use clang as the linker driver (invokes ld.lld internally via -fuse-ld=lld).
  # If libgcc.a exists (from GCC cross-compiler), include it for runtime helpers
  # (__divti3, __udivti3, __int128 arithmetic, etc.).
  set(LIBGCC_LINK "")
  set(LIBGCC_DEP "")
  if(EXISTS "${LIBC64_DIR_}/libgcc.a")
    set(LIBGCC_LINK "${LIBC64_DIR_}/libgcc.a")
    set(LIBGCC_DEP "${LIBC64_DIR_}/libgcc.a")
  endif()

  add_custom_command(
    OUTPUT ${ELF_OUTPUT}
    COMMAND ${CLANGXX_EXECUTABLE}
      --target=x86_64-unknown-none-elf
      -nostdlib -static -fuse-ld=lld
      -T ${LIBC64_DIR_}/link.ld
      ${LIBC64_DIR_}/obj/crt0.o
      ${LIBC64_DIR_}/obj/crti.o
      ${ALL_OBJECTS}
      ${LIBCXX_DIR_}/libcxx.a
      ${LIBCXXABI_DIR_}/libc++abi.a
      ${LIBUNWIND_DIR_}/libunwind.a
      ${LIBC64_DIR_}/libc64.a
      ${LIBGCC_LINK}
      ${LIBC64_DIR_}/obj/crtn.o
      -o ${ELF_OUTPUT}
    DEPENDS ${ALL_OBJECTS}
      ${LIBC64_DIR_}/libc64.a
      ${LIBCXX_DIR_}/libcxx.a
      ${LIBCXXABI_DIR_}/libc++abi.a
      ${LIBUNWIND_DIR_}/libunwind.a
      ${LIBC64_DIR_}/link.ld
      ${LIBGCC_DEP}
    COMMENT "Linking ${NAME}.elf"
  )

  # Convert ELF → anyOS flat binary
  add_custom_command(
    OUTPUT ${BIN_OUTPUT}
    COMMAND ${ANYELF_EXECUTABLE} -f flat -o ${BIN_OUTPUT} ${ELF_OUTPUT}
    DEPENDS ${ELF_OUTPUT} ${ANYELF_EXECUTABLE}
    COMMENT "Converting ${NAME}.elf → ${NAME} (flat binary)"
  )

  # Install to sysroot
  set(SYSROOT_DEST "${SYSROOT_DIR}/${CXX_INSTALL_DIR}/${NAME}")
  add_custom_command(
    OUTPUT ${SYSROOT_DEST}
    COMMAND ${CMAKE_COMMAND} -E copy ${BIN_OUTPUT} ${SYSROOT_DEST}
    DEPENDS ${BIN_OUTPUT}
    COMMENT "Installing ${NAME} to ${CXX_INSTALL_DIR}"
  )

  add_custom_target(${NAME} ALL DEPENDS ${SYSROOT_DEST})
endfunction()

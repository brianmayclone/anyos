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

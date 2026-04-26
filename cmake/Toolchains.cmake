# ============================================================
# 64-bit C/C++ Toolchain (clang + libc64 + libcxx)
# ============================================================
# The 32-bit C world (libc, TCC, NASM, make, dash, ssh/sshd/scp, curl,
# libgit2, ClassiCube) was removed in the 32-bit cleanup. /System/bin no
# longer ships any i686-elf-built programs.

# ARM64 port: C toolchain is x86-only for now — skip entirely
if(ANYOS_ARCH STREQUAL "arm64")
  set(C_TOOLCHAIN_DEPS "")
  set(CXX_TOOLCHAIN_DEPS "")
  return()
endif()

# C_TOOLCHAIN_DEPS is unused after 32-bit removal; kept empty for callers
# that still reference the variable.
set(C_TOOLCHAIN_DEPS "")

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

  # Check for Stage 2 native compiler (built to run ON anyOS)
  set(NATIVE_TOOLCHAIN_DIR "$ENV{HOME}/build/anyos-toolchain/native-toolchain")
  set(NATIVE_TOOL_NAMES "")
  if(IS_DIRECTORY "${NATIVE_TOOLCHAIN_DIR}/bin")
    message(STATUS "Found native GCC toolchain at ${NATIVE_TOOLCHAIN_DIR}")
    file(GLOB NATIVE_BINS "${NATIVE_TOOLCHAIN_DIR}/bin/*")
    foreach(NATIVE_BIN ${NATIVE_BINS})
      get_filename_component(TOOL_NAME "${NATIVE_BIN}" NAME)
      list(APPEND NATIVE_TOOL_NAMES ${TOOL_NAME})
    endforeach()
  endif()

  # Install GCC toolchain binaries to /System/Toolchain/bin/ on disk
  # Skip cross-compiler binaries when native (ELF64) versions exist
  set(ANYOS_TOOLCHAIN_BINS gcc g++ as ld ar nm objdump objcopy ranlib strip)
  set(TOOLCHAIN_SYSROOT_DEPS "")
  foreach(TOOL ${ANYOS_TOOLCHAIN_BINS})
    if("${TOOL}" IN_LIST NATIVE_TOOL_NAMES)
      # Native version will be installed below — skip cross-compiler version
      continue()
    endif()
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

  # Install native (ELF64) binaries — these run ON anyOS
  if(IS_DIRECTORY "${NATIVE_TOOLCHAIN_DIR}/bin")
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

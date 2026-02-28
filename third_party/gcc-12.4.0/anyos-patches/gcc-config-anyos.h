/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * GCC target header for x86_64-anyos.
 * Installed to gcc/config/anyos.h in the GCC source tree.
 */

/* anyOS uses ELF. */
#define OBJECT_FORMAT_ELF

/* OS identification. */
#undef  TARGET_OS_CPP_BUILTINS
#define TARGET_OS_CPP_BUILTINS()        \
  do {                                  \
    builtin_define ("__anyos__");        \
    builtin_define ("__unix__");         \
    builtin_assert ("system=anyos");    \
    builtin_assert ("system=unix");     \
  } while (0)

/* Default to static linking — anyOS does not have a dynamic linker for
   user programs (DLLs are loaded by the compositor, not ld.so). */
#undef  LINK_SPEC
#define LINK_SPEC "-static"

/* Libraries to link: libcxx (C++ stdlib), libc++abi (C++ ABI/exceptions),
   libunwind (DWARF stack unwinder), libc64 (C library), libgcc (compiler rt). */
#undef  LIB_SPEC
#define LIB_SPEC "-lcxx -lc++abi -lunwind -lc64 -lgcc"

/* C-only programs don't need C++ libs. */
#undef  LIB_SPEC
#define LIB_SPEC "%{!lstdc++:-lc64 -lgcc} %{lstdc++:-lcxx -lc++abi -lunwind -lc64 -lgcc}"

/* Startup files provided by libc64. */
#undef  STARTFILE_SPEC
#define STARTFILE_SPEC "crt0.o%s crti.o%s crtbegin.o%s"

#undef  ENDFILE_SPEC
#define ENDFILE_SPEC "crtend.o%s crtn.o%s"

/* Search paths for libraries and startup files. */
#undef  STANDARD_STARTFILE_PREFIX
#define STANDARD_STARTFILE_PREFIX "/Libraries/libc64/lib/"

#undef  STANDARD_STARTFILE_PREFIX_1
#define STANDARD_STARTFILE_PREFIX_1 "/Libraries/libcxx/lib/"

/* No threading support yet (single-threaded model). */
#undef  THREAD_MODEL_SPEC
#define THREAD_MODEL_SPEC "single"

/* Default to 64-bit mode. */
#undef  CC1_SPEC
#define CC1_SPEC "%{!m32:-m64}"

/* No dynamic linker. */
#undef  DYNAMIC_LINKER
#define DYNAMIC_LINKER ""

/* Use cxa_atexit for static destructor registration. */
#undef  TARGET_LIBC_HAS_FUNCTION
#define TARGET_LIBC_HAS_FUNCTION no_c99_libc_has_function

/*
 * gcc/config/anyos.h — GCC target configuration for anyOS (x86_64)
 *
 * This file is applied as a patch to GCC source tree at:
 *   gcc/config/anyos.h
 *
 * It defines the OS-specific defaults for the x86_64-anyos target.
 */

/* System identification. */
#define TARGET_OS_CPP_BUILTINS()             \
  do {                                       \
    builtin_define ("__anyos__");             \
    builtin_define ("__anyOS__");             \
    builtin_define ("__unix__");              \
    builtin_assert ("system=anyos");          \
    builtin_assert ("system=unix");           \
  } while (0)

/* Default specs. */

/* Use ELF output — no dynamic linker on anyOS. */
#undef  LIB_SPEC
#define LIB_SPEC "-lcxx -lc++abi -lunwind -lc64"

/* Startup files: crt0.o, crti.o ... crtn.o */
#undef  STARTFILE_SPEC
#define STARTFILE_SPEC "crt0.o%s crti.o%s"

#undef  ENDFILE_SPEC
#define ENDFILE_SPEC "crtn.o%s"

/* Static linking only — no shared libraries yet. */
#undef  LINK_SPEC
#define LINK_SPEC "-static -T /Libraries/libc64/lib/link.ld"

/* Default linker script and library search paths on anyOS. */
#undef  STANDARD_STARTFILE_PREFIX
#define STANDARD_STARTFILE_PREFIX "/Libraries/libc64/lib/"

#undef  STANDARD_STARTFILE_PREFIX_1
#define STANDARD_STARTFILE_PREFIX_1 "/Libraries/libcxx/lib/"

/* No need for crtbegin/crtend (we provide our own crti/crtn). */
#undef  STARTFILE_SPEC
#define STARTFILE_SPEC "crt0.o%s crti.o%s"

/* Override C++ include paths. */
#undef  CPLUSPLUS_CPP_SPEC
#define CPLUSPLUS_CPP_SPEC ""

/* No thread model support yet (single-threaded default). */
#define THREAD_MODEL_SPEC "single"

/* Default assembler invocation. */
#undef  ASM_SPEC
#define ASM_SPEC ""

/* Use our assembler from /System/Toolchain/bin/as */
/* #undef  ASM_PROG
   #define ASM_PROG "as" */

/* Disable unwanted features. */
#undef  LINK_GCC_C_SEQUENCE_SPEC
#define LINK_GCC_C_SEQUENCE_SPEC "%G %{!nolibc:%L}"

/* Target CPU is generic x86_64. */
#define CC1_SPEC "%{!m32:-m64}"

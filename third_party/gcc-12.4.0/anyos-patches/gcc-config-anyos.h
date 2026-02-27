/*
 * gcc/config/anyos.h — GCC target configuration for anyOS (x86_64)
 *
 * This file is installed into the GCC source tree at:
 *   gcc/config/anyos.h
 *
 * It defines the OS-specific defaults for the x86_64-anyos target triplet.
 * anyOS is a 64-bit bare-metal OS with custom libc64, libcxx, libc++abi,
 * and libunwind.  All linking is static; no dynamic linker.
 */

/* System identification macros. */
#define TARGET_OS_CPP_BUILTINS()             \
  do {                                       \
    builtin_define ("__anyos__");             \
    builtin_define ("__anyOS__");             \
    builtin_define ("__unix__");              \
    builtin_assert ("system=anyos");          \
    builtin_assert ("system=unix");           \
  } while (0)

/* ── Library specs ──────────────────────────────────────────────────── */

/* Libraries linked by default.
 * Order: C++ stdlib → ABI → unwinder → C runtime → GCC builtins.
 * Users can pass -nolibc to suppress. */
#undef  LIB_SPEC
#define LIB_SPEC "-lcxx -lc++abi -lunwind -lc64 -lgcc"

/* ── Startup / shutdown files ───────────────────────────────────────── */

/* crt0.o: _start entry, crti.o: .init prologue */
#undef  STARTFILE_SPEC
#define STARTFILE_SPEC "crt0.o%s crti.o%s crtbegin.o%s"

/* crtend.o: .fini epilogue, crtn.o: .fini return */
#undef  ENDFILE_SPEC
#define ENDFILE_SPEC "crtend.o%s crtn.o%s"

/* ── Linker configuration ───────────────────────────────────────────── */

/* Static linking only — no shared libraries on anyOS yet. */
#undef  LINK_SPEC
#define LINK_SPEC "-static"

/* Library search paths on the target system. */
#undef  STANDARD_STARTFILE_PREFIX
#define STANDARD_STARTFILE_PREFIX "/Libraries/libc64/lib/"

#undef  STANDARD_STARTFILE_PREFIX_1
#define STANDARD_STARTFILE_PREFIX_1 "/Libraries/libcxx/lib/"

/* Link sequence: GCC builtins (%G) + default libs (%L). */
#undef  LINK_GCC_C_SEQUENCE_SPEC
#define LINK_GCC_C_SEQUENCE_SPEC "%G %{!nolibc:%L}"

/* ── Thread model ───────────────────────────────────────────────────── */

/* anyOS has pthread support; use "single" for now since GCC's libgcc
 * thread primitives are not yet wired up. */
#define THREAD_MODEL_SPEC "single"

/* ── Compiler driver options ────────────────────────────────────────── */

/* Default to 64-bit mode. */
#define CC1_SPEC "%{!m32:-m64}"

/* No fixincludes needed. */
#undef  CPLUSPLUS_CPP_SPEC
#define CPLUSPLUS_CPP_SPEC ""

/* Default assembler invocation (no special flags needed). */
#undef  ASM_SPEC
#define ASM_SPEC ""

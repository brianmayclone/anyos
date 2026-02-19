/* TCC configuration for anyOS */
#define TCC_VERSION "0.9.27"

/* Target: i386 ELF */
#define TCC_TARGET_I386

/* Static build — no dlopen/dlsym/dlfcn */
#define CONFIG_TCC_STATIC

/* No bounds checking */
/* #undef CONFIG_TCC_BCHECK */

/* No backtrace */
/* #undef CONFIG_TCC_BACKTRACE */

/* No SELinux */
/* #undef HAVE_SELINUX */

/* Default paths for anyOS */
#define CONFIG_TCCDIR "/Libraries/libc/lib/tcc"
#define CONFIG_TCC_SYSINCLUDEPATHS "/Libraries/libc/include"
#define CONFIG_TCC_LIBPATHS "/Libraries/libc/lib"
#define CONFIG_TCC_CRTPREFIX "/Libraries/libc/lib"
#define CONFIG_TCC_ELFINTERP ""

/* Use ONE_SOURCE build */
#define ONE_SOURCE 1

/* Platform defines */
#define _GNU_SOURCE

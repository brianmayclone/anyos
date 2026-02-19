/* config.h -- Pre-generated configuration for anyOS cross-compilation.
 * Equivalent to what autoconf would generate with:
 *   --enable-static --disable-fnmatch --disable-glob --without-libedit
 */

/* Define for anyOS */
#define _PATH_BSHELL "/bin/sh"
#define _PATH_DEVNULL "/dev/null"
#define _PATH_TTY "/dev/tty"

/* Size checks */
#define SIZEOF_INTMAX_T 4
#define SIZEOF_LONG_LONG_INT 8

/* printf format for intmax_t */
#define PRIdMAX "ld"

/* Functions available in anyOS libc */
#define HAVE_BSEARCH 1
#define HAVE_FACCESSAT 1
#define HAVE_GETPWNAM 1
#define HAVE_GETRLIMIT 1
#define HAVE_ISALPHA 1
#define HAVE_KILLPG 1
#define HAVE_MEMPCPY 1
#define HAVE_SIGNAL 1
#define HAVE_STPCPY 1
#define HAVE_STRCHRNUL 1
#define HAVE_STRSIGNAL 1
#define HAVE_STRTOD 1
#define HAVE_STRTOIMAX 0
#define HAVE_STRTOUMAX 0
#define HAVE_SYSCONF 1

/* isblank() available in anyOS libc */
#define HAVE_DECL_ISBLANK 1

/* No sigsetmask */
/* #undef HAVE_SIGSETMASK */

/* No fnmatch or glob */
/* #undef HAVE_FNMATCH */
/* #undef HAVE_GLOB */

/* No 64-bit file ops -- use 32-bit */
#define fstat64 fstat
#define lstat64 lstat
#define stat64 stat
#define open64 open
#define readdir64 readdir
#define dirent64 dirent
#define glob64_t glob_t
#define glob64 glob
#define globfree64 globfree

/* No libedit (line editing) -- build SMALL */
#define SMALL 1

/* Enable LINENO support */
#define WITH_LINENO 1

/* Disable job control for initial port (no real tty/process groups) */
#define JOBS 0

/* Alias attribute supported by GCC cross compiler */
#define HAVE_ALIAS_ATTRIBUTE 1

/* We have alloca.h */
#define HAVE_ALLOCA_H 1

/* No paths.h */
/* #undef HAVE_PATHS_H */

/* No st_mtim in struct stat */
/* #undef HAVE_ST_MTIM */

/* Package info */
#define PACKAGE_NAME "dash"
#define PACKAGE_VERSION "0.5.12"
#define PACKAGE_STRING "dash 0.5.12"

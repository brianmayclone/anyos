# anyOS C Library (libc) Reference

The anyOS **libc** is a minimal POSIX-compatible C standard library for 32-bit `i686-elf` user programs. It provides the standard headers and runtime needed by the on-disk **TCC** compiler and cross-compiled C programs (curl, git, NASM, DOOM, Quake).

**Location:** `libs/libc/`
**Compiler:** `i686-elf-gcc -m32`
**Syscall ABI:** `INT 0x80` (EAX=num, EBX–EDI=args)

---

## Table of Contents

- [Runtime](#runtime)
- [stdio.h](#stdioh)
- [stdlib.h](#stdlibh)
- [string.h / strings.h](#stringh--stringsh)
- [unistd.h](#unistdh)
- [fcntl.h](#fcntlh)
- [dirent.h](#direnth)
- [sys/stat.h](#sysstath)
- [time.h](#timeh)
- [math.h](#mathh)
- [signal.h](#signalh)
- [setjmp.h](#setjmph)
- [errno.h](#errnoh)
- [ctype.h](#ctypeh)
- [getopt.h](#getopth)
- [Networking Headers](#networking-headers)
- [Other Headers](#other-headers)
- [libc64](#libc64)

---

## Runtime

### Entry Point (`crt0.S`)
- `_start`: Sets up stack, calls `main(argc, argv)`, calls `exit()` with return value
- Arguments parsed from `SYS_GETARGS` syscall result

### Syscall Layer (`syscall.S`)
- `_syscall0` through `_syscall5`: INT 0x80 wrappers for 0–5 arguments
- Preserves EBX (callee-saved) via push/pop

### CRT Files
- `crt0.o` (= `crt1.o`): Entry point
- `crti.o`, `crtn.o`: Empty stubs (no init/fini arrays)
- `libtcc1.a`: Empty (TCC runtime stub)

---

## stdio.h

### Types
- `FILE` — File stream with fd, buffer, position, flags, ungetc support
- `fpos_t` — `long` (file position)

### Predefined Streams
- `stdin` (fd 0), `stdout` (fd 1), `stderr` (fd 2)

### File Operations
```c
FILE *fopen(const char *path, const char *mode);
FILE *freopen(const char *path, const char *mode, FILE *stream);
FILE *fdopen(int fd, const char *mode);
int   fclose(FILE *stream);
int   fflush(FILE *stream);
int   fileno(FILE *stream);
```

### Reading & Writing
```c
size_t fread(void *ptr, size_t size, size_t nmemb, FILE *stream);
size_t fwrite(const void *ptr, size_t size, size_t nmemb, FILE *stream);
int    fgetc(FILE *stream);
int    fputc(int c, FILE *stream);
char  *fgets(char *s, int size, FILE *stream);
int    fputs(const char *s, FILE *stream);
int    getc(FILE *stream);    // macro → fgetc
int    putc(int c, FILE *stream); // macro → fputc
int    getchar(void);         // macro → fgetc(stdin)
int    putchar(int c);        // macro → fputc(c, stdout)
int    puts(const char *s);
int    ungetc(int c, FILE *stream);
```

### Formatted I/O
```c
int printf(const char *fmt, ...);
int fprintf(FILE *stream, const char *fmt, ...);
int sprintf(char *str, const char *fmt, ...);
int snprintf(char *str, size_t size, const char *fmt, ...);
int vprintf(const char *fmt, va_list ap);
int vfprintf(FILE *stream, const char *fmt, va_list ap);
int vsprintf(char *str, const char *fmt, va_list ap);
int vsnprintf(char *str, size_t size, const char *fmt, va_list ap);
int sscanf(const char *str, const char *fmt, ...);
int fscanf(FILE *stream, const char *fmt, ...);
```

**Supported format specifiers:** `%d`, `%i`, `%u`, `%x`, `%X`, `%o`, `%p`, `%s`, `%c`, `%f`, `%e`, `%g`, `%ld`, `%lu`, `%lx`, `%lld`, `%llu`, `%llx`, `%zu`, `%zd`, `%%`, `%n`. Width, precision, zero-padding, left-align supported.

### Positioning
```c
int   fseek(FILE *stream, long offset, int whence);
long  ftell(FILE *stream);
void  rewind(FILE *stream);
int   feof(FILE *stream);
int   ferror(FILE *stream);
void  clearerr(FILE *stream);
```

### Other
```c
int    remove(const char *path);
int    rename(const char *old, const char *new);
FILE  *tmpfile(void);
void   perror(const char *s);
void   setvbuf(FILE *stream, char *buf, int mode, size_t size);
void   setbuf(FILE *stream, char *buf);
void   setlinebuf(FILE *stream);
```

---

## stdlib.h

### Memory
```c
void *malloc(size_t size);
void *calloc(size_t nmemb, size_t size);
void *realloc(void *ptr, size_t size);
void  free(void *ptr);
```
Implementation uses `sbrk()` with a free-list allocator (first-fit with coalescing).

### Process
```c
void exit(int status);
void abort(void);
int  system(const char *command);   // stub, returns -1
int  atexit(void (*func)(void));    // registers up to 32 handlers
```

### String Conversion
```c
int    atoi(const char *s);
long   atol(const char *s);
double atof(const char *s);
long   strtol(const char *s, char **endptr, int base);
unsigned long strtoul(const char *s, char **endptr, int base);
long long strtoll(const char *s, char **endptr, int base);
unsigned long long strtoull(const char *s, char **endptr, int base);
double strtod(const char *s, char **endptr);
float  strtof(const char *s, char **endptr);
```

### Sorting & Searching
```c
void  qsort(void *base, size_t nmemb, size_t size, int (*compar)(const void *, const void *));
void *bsearch(const void *key, const void *base, size_t nmemb, size_t size, int (*compar)(const void *, const void *));
```

### Random
```c
int  rand(void);
void srand(unsigned int seed);
```

### Environment
```c
char *getenv(const char *name);
int   setenv(const char *name, const char *value, int overwrite);
int   unsetenv(const char *name);
```

### Other
```c
int  abs(int j);
long labs(long j);
int  mkstemp(char *template);  // stub
char *mktemp(char *template);  // stub
```

---

## string.h / strings.h

### Memory Operations
```c
void *memcpy(void *dest, const void *src, size_t n);
void *memmove(void *dest, const void *src, size_t n);
void *memset(void *s, int c, size_t n);
int   memcmp(const void *s1, const void *s2, size_t n);
void *memchr(const void *s, int c, size_t n);
void *memrchr(const void *s, int c, size_t n);
```

### String Operations
```c
size_t strlen(const char *s);
size_t strnlen(const char *s, size_t maxlen);
char  *strcpy(char *dest, const char *src);
char  *strncpy(char *dest, const char *src, size_t n);
char  *strcat(char *dest, const char *src);
char  *strncat(char *dest, const char *src, size_t n);
int    strcmp(const char *s1, const char *s2);
int    strncmp(const char *s1, const char *s2, size_t n);
char  *strchr(const char *s, int c);
char  *strrchr(const char *s, int c);
char  *strstr(const char *haystack, const char *needle);
char  *strdup(const char *s);
char  *strndup(const char *s, size_t n);
char  *strerror(int errnum);
size_t strspn(const char *s, const char *accept);
size_t strcspn(const char *s, const char *reject);
char  *strtok(char *str, const char *delim);
char  *strpbrk(const char *s, const char *accept);
```

### strings.h Extensions
```c
int   strcasecmp(const char *s1, const char *s2);
int   strncasecmp(const char *s1, const char *s2, size_t n);
char *strcasestr(const char *haystack, const char *needle);
char *strchrnul(const char *s, int c);
```

---

## unistd.h

```c
ssize_t read(int fd, void *buf, size_t count);
ssize_t write(int fd, const void *buf, size_t count);
int     close(int fd);
int     lseek(int fd, int offset, int whence);
int     isatty(int fd);
char   *getcwd(char *buf, size_t size);
int     chdir(const char *path);
void    _exit(int status);
void   *sbrk(int increment);
int     unlink(const char *path);
int     access(const char *path, int mode);
int     ftruncate(int fd, unsigned int length);
pid_t   fork(void);
pid_t   waitpid(pid_t pid, int *status, int options);
int     execv(const char *path, char *const argv[]);
int     execvp(const char *file, char *const argv[]);
int     execve(const char *path, char *const argv[], char *const envp[]);
int     dup(int oldfd);                          // stub
int     dup2(int oldfd, int newfd);              // stub
int     pipe(int pipefd[2]);                     // stub
int     gethostname(char *name, size_t len);     // stub: "anyos"
char   *realpath(const char *path, char *resolved_path); // stub
int     rmdir(const char *path);                 // delegates to unlink
int     symlink(const char *target, const char *linkpath);
ssize_t readlink(const char *path, char *buf, size_t bufsiz);
int     link(const char *old, const char *new);  // stub
int     chmod(const char *path, mode_t mode);
int     chown(const char *path, uid_t owner, gid_t group);
unsigned int sleep(unsigned int seconds);
pid_t   getpid(void);
uid_t   getuid(void);
gid_t   getgid(void);
uid_t   geteuid(void);        // returns getuid()
gid_t   getegid(void);        // returns getgid()
pid_t   getppid(void);
pid_t   getpgid(pid_t pid);
int     setpgid(pid_t pid, pid_t pgid);
pid_t   getpgrp(void);
pid_t   setpgrp(void);
pid_t   setsid(void);
pid_t   getsid(pid_t pid);
pid_t   vfork(void);
unsigned int alarm(unsigned int seconds);
ssize_t pread(int fd, void *buf, size_t count, off_t offset);
ssize_t pwrite(int fd, const void *buf, size_t count, off_t offset);
int     ioctl(int fd, unsigned long request, ...);
int     lstat(const char *path, struct stat *buf);
int     fchmod(int fd, mode_t mode);
int     fsync(int fd);
int     fdatasync(int fd);
mode_t  umask(mode_t cmask);
long    sysconf(int name);
int     faccessat(int dirfd, const char *path, int mode, int flags);
int     unlinkat(int dirfd, const char *path, int flags);
```

---

## fcntl.h

### Flags
```c
#define O_RDONLY    0x0000
#define O_WRONLY    0x0001
#define O_RDWR      0x0002
#define O_CREAT     0x04
#define O_TRUNC     0x08
#define O_APPEND    0x0400
#define O_NONBLOCK  0x0800
#define O_EXCL      0x0080
#define O_CLOEXEC   0x80000
#define O_DIRECTORY 0x10000
```

### Functions
```c
int open(const char *path, int flags, ...);
int fcntl(int fd, int cmd, ...);    // stub
```

Note: POSIX flags are translated to anyOS flags in the `open()` implementation.

---

## dirent.h

```c
struct dirent {
    unsigned long d_ino;
    unsigned char d_type;   // DT_REG=8, DT_DIR=4, DT_UNKNOWN=0
    char          d_name[256];
};

DIR *opendir(const char *name);
struct dirent *readdir(DIR *dirp);
int    closedir(DIR *dirp);
void   rewinddir(DIR *dirp);
int    alphasort(const struct dirent **a, const struct dirent **b);
int    scandir(const char *dirp, struct dirent ***namelist,
               int (*filter)(const struct dirent *),
               int (*compar)(const struct dirent **, const struct dirent **));
int    dirfd(DIR *dirp);
```

---

## sys/stat.h

```c
struct stat {
    dev_t     st_dev;
    ino_t     st_ino;
    mode_t    st_mode;
    nlink_t   st_nlink;
    uid_t     st_uid;
    gid_t     st_gid;
    dev_t     st_rdev;
    off_t     st_size;
    blksize_t st_blksize;
    blkcnt_t  st_blocks;
    time_t    st_atime, st_mtime, st_ctime;
};

int stat(const char *path, struct stat *buf);
int fstat(int fd, struct stat *buf);
int fstatat(int dirfd, const char *path, struct stat *buf, int flags);
int mkdir(const char *path, mode_t mode);
```

### Mode Constants
`S_IFMT`, `S_IFREG`, `S_IFDIR`, `S_IFCHR`, `S_IFLNK`, `S_IFIFO`, `S_IFBLK`, `S_IFSOCK`
`S_ISREG()`, `S_ISDIR()`, `S_ISCHR()`, `S_ISLNK()`, `S_ISFIFO()`, `S_ISBLK()`, `S_ISSOCK()`
`S_IRWXU`, `S_IRUSR`, `S_IWUSR`, `S_IXUSR`, `S_IRWXG`, `S_IRGRP`, `S_IWGRP`, `S_IXGRP`, `S_IRWXO`, `S_IROTH`, `S_IWOTH`, `S_IXOTH`

---

## time.h

```c
typedef long time_t;
typedef long clock_t;

struct tm {
    int tm_sec, tm_min, tm_hour, tm_mday, tm_mon, tm_year;
    int tm_wday, tm_yday, tm_isdst;
};

struct timespec { time_t tv_sec; long tv_nsec; };

time_t     time(time_t *t);
clock_t    clock(void);
time_t     mktime(struct tm *tm);
double     difftime(time_t t1, time_t t0);
struct tm *localtime(const time_t *timep);
struct tm *gmtime(const time_t *timep);
struct tm *localtime_r(const time_t *timep, struct tm *result);
struct tm *gmtime_r(const time_t *timep, struct tm *result);
size_t     strftime(char *s, size_t max, const char *fmt, const struct tm *tm);
int        nanosleep(const struct timespec *req, struct timespec *rem);
int        gettimeofday(struct timeval *tv, struct timezone *tz);
```

---

## math.h

```c
double ldexp(double x, int exp);
double frexp(double x, int *exp);
double modf(double x, double *iptr);
double fabs(double x);
double floor(double x);
double ceil(double x);
double sqrt(double x);
double pow(double base, double exp);
double log(double x);
double log10(double x);
double log2(double x);
double exp(double x);
double sin(double x);
double cos(double x);
double tan(double x);
double atan(double x);
double atan2(double y, double x);
double asin(double x);
double acos(double x);
double fmod(double x, double y);
// float variants: fabsf, sqrtf, sinf, cosf, atan2f, fmodf, floorf, ceilf, powf, logf, expf
// Additional: log2f, log10f
```

Constants: `M_PI`, `M_PI_2`, `M_PI_4`, `M_E`, `M_LN2`, `HUGE_VAL`, `INFINITY`, `NAN`

---

## signal.h

```c
typedef void (*sighandler_t)(int);

sighandler_t signal(int signum, sighandler_t handler);
int          raise(int sig);
int          kill(int pid, int sig);
int          sigprocmask(int how, const sigset_t *set, sigset_t *oldset);
int          sigaction(int signum, const struct sigaction *act, struct sigaction *oldact);
int          sigsuspend(const sigset_t *mask);
int          sigpending(sigset_t *set);
int          siginterrupt(int sig, int flag);
```

Signals: `SIGHUP`(1), `SIGINT`(2), `SIGQUIT`(3), `SIGILL`(4), `SIGTRAP`(5), `SIGABRT`(6), `SIGBUS`(7), `SIGFPE`(8), `SIGKILL`(9), `SIGUSR1`(10), `SIGSEGV`(11), `SIGUSR2`(12), `SIGPIPE`(13), `SIGALRM`(14), `SIGTERM`(15), `SIGCHLD`(17), `SIGCONT`(18), `SIGSTOP`(19), `SIGTSTP`(20), `SIGTTIN`(21), `SIGTTOU`(22)
Special handlers: `SIG_DFL`, `SIG_IGN`, `SIG_ERR`

---

## setjmp.h

```c
typedef int jmp_buf[6];  // ebx, esi, edi, ebp, esp, eip

int  setjmp(jmp_buf env);
void longjmp(jmp_buf env, int val);
```

Implemented in assembly for i686.

---

## errno.h

Global `errno` variable with 65+ POSIX error constants:

`EPERM`(1), `ENOENT`(2), `ESRCH`(3), `EINTR`(4), `EIO`(5), `ENXIO`(6), `E2BIG`(7), `ENOEXEC`(8), `EBADF`(9), `ECHILD`(10), `EAGAIN`(11), `ENOMEM`(12), `EACCES`(13), `EFAULT`(14), `EBUSY`(16), `EEXIST`(17), `EXDEV`(18), `ENODEV`(19), `ENOTDIR`(20), `EISDIR`(21), `EINVAL`(22), `ENFILE`(23), `EMFILE`(24), `ENOTTY`(25), `EFBIG`(27), `ENOSPC`(28), `ESPIPE`(29), `EROFS`(30), `EPIPE`(32), `EDOM`(33), `ERANGE`(34), `ENOSYS`(38), `ELOOP`(40), `ENAMETOOLONG`(36), `ENOTEMPTY`(39), `ECONNREFUSED`(111), `ECONNRESET`(104), `ETIMEDOUT`(110), `EHOSTUNREACH`(113), `ENETUNREACH`(101), `EADDRINUSE`(98), `EADDRNOTAVAIL`(99), `EAFNOSUPPORT`(97), `EALREADY`(114), `EISCONN`(106), `ENOTCONN`(107), `ENOTSOCK`(88), `EMSGSIZE`(90), `EOPNOTSUPP`(95), `EWOULDBLOCK`(=EAGAIN)

---

## ctype.h

```c
int isalpha(int c);   int isdigit(int c);   int isalnum(int c);
int isspace(int c);   int isupper(int c);   int islower(int c);
int isprint(int c);   int ispunct(int c);   int isxdigit(int c);
int iscntrl(int c);   int isgraph(int c);   int isascii(int c);
int toupper(int c);   int tolower(int c);
```

---

## getopt.h

```c
extern char *optarg;
extern int   optind, opterr, optopt;

struct option {
    const char *name;
    int has_arg;    // no_argument=0, required_argument=1, optional_argument=2
    int *flag;
    int val;
};

int getopt(int argc, char *const argv[], const char *optstring);
int getopt_long(int argc, char *const argv[], const char *optstring,
                const struct option *longopts, int *longindex);
```

---

## Networking Headers

### sys/socket.h
Socket types: `SOCK_STREAM`, `SOCK_DGRAM`, `SOCK_RAW`
Address families: `AF_UNSPEC`, `AF_INET`, `AF_INET6`
Functions: `socket`, `connect`, `bind`, `listen`, `accept`, `send`, `recv`, `sendto`, `recvfrom`, `setsockopt`, `getsockopt`, `shutdown`, `getpeername`, `getsockname`

### netinet/in.h
Types: `in_addr`, `sockaddr_in`, `in6_addr`, `sockaddr_in6`
Constants: `INADDR_ANY`, `INADDR_BROADCAST`, `INADDR_LOOPBACK`
Byte-order: `htons`, `ntohs`, `htonl`, `ntohl`

### arpa/inet.h
```c
int         inet_aton(const char *cp, struct in_addr *inp);
in_addr_t   inet_addr(const char *cp);
char       *inet_ntoa(struct in_addr in);
int         inet_pton(int af, const char *src, void *dst);
const char *inet_ntop(int af, const void *src, char *dst, socklen_t size);
```

### netdb.h
```c
struct hostent *gethostbyname(const char *name);
int getaddrinfo(const char *node, const char *service,
                const struct addrinfo *hints, struct addrinfo **res);
void freeaddrinfo(struct addrinfo *res);
const char *gai_strerror(int errcode);
int getnameinfo(const struct sockaddr *sa, socklen_t salen,
                char *host, socklen_t hostlen,
                char *serv, socklen_t servlen, int flags);
```

---

## Other Headers

| Header | Contents |
|--------|----------|
| `stdint.h` | Fixed-width integer types (int8_t–int64_t, uint8_t–uint64_t, intptr_t, etc.) |
| `stddef.h` | `size_t`, `ssize_t`, `ptrdiff_t`, `wchar_t`, `NULL`, `offsetof` |
| `stdbool.h` | `bool`, `true`, `false` |
| `stdarg.h` | `va_list`, `va_start`, `va_end`, `va_arg`, `va_copy` |
| `limits.h` | `INT_MIN`, `INT_MAX`, `UINT_MAX`, `LONG_MIN`, `LONG_MAX`, `PATH_MAX` (4096) |
| `assert.h` | `assert(expr)` macro (calls `abort()` on failure) |
| `spawn.h` | `posix_spawn`, `posix_spawnp` with attribute init/destroy functions |
| `sys/time.h` | `timeval`, `timezone`, `gettimeofday`, `utimes` |
| `sys/select.h` | `fd_set`, `FD_ZERO/SET/CLR/ISSET`, `select`, `pselect` |
| `poll.h` | `pollfd`, `POLLIN/POLLOUT/POLLERR/POLLHUP/POLLNVAL/POLLPRI/POLLRDNORM/POLLWRNORM`, `poll` |
| `pwd.h` | `passwd`, `getpwuid`, `getpwnam`, `getpwuid_r` |
| `sys/mman.h` | `mmap`, `munmap`, `mprotect` (stubs) |
| `sys/utsname.h` | `utsname`, `uname` (stub) |
| `termios.h` | `tcgetattr`, `tcsetattr`, `cfgetispeed`, `cfgetospeed`, `tcgetpgrp`, `tcsetpgrp` |
| `regex.h` | `regcomp`, `regexec`, `regfree`, `regerror` |
| `sys/resource.h` | `getrlimit`, `setrlimit`, `rlimit` |
| `inttypes.h` | `strtoimax`, `strtoumax`, PRId64/PRIu64 format macros |

---

## libc64

**Location:** `libs/libc64/`

Minimal 64-bit freestanding headers for `x86_64-unknown-none-elf` compilation targets (used by BearSSL x64 build). NOT a full C library — only provides type definitions and a few string functions.

### Headers
- `stdlib.h`, `string.h`, `stdint.h`, `stddef.h`, `stdbool.h`, `stdarg.h`, `limits.h`
- `cpuid.h`, `immintrin.h`, `x86intrin.h`, `intrin.h` (all stubs returning 0/no-op)

### Source
- `string.c`: `memcpy`, `memmove`, `memset`, `memcmp`, `strlen`, `strcmp`, `strncmp`, `strcpy`, `strncpy`

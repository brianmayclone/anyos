# anyOS C++ Standard Library & libc64 Reference

This document covers the 64-bit C and C++ standard libraries for anyOS. **libcxx** (C++20) depends on **libc64** (64-bit C library), and both target the `x86_64-unknown-none-elf` freestanding environment. Together they provide the complete runtime needed to build and run C and C++ programs in 64-bit long mode on anyOS.

---

## Table of Contents

- [libc64 (64-bit C Standard Library)](#libc64-64-bit-c-standard-library)
  - [Overview](#overview)
  - [Build System](#build-system)
  - [Runtime](#runtime)
  - [Syscall Layer](#syscall-layer)
  - [Headers](#headers)
  - [Source Modules](#source-modules)
  - [Stub Intrinsic Headers](#stub-intrinsic-headers)
  - [Key Differences from libc (32-bit)](#key-differences-from-libc-32-bit)
- [libcxx (C++20 Standard Library)](#libcxx-c20-standard-library)
  - [Overview](#overview-1)
  - [Build System](#build-system-1)
  - [Containers](#containers)
    - [vector\<T\>](#vectort)
    - [string](#string)
    - [string_view](#string_view)
    - [array\<T, N\>](#arrayt-n)
    - [map\<K, V\>](#mapk-v)
    - [set\<K\>](#setk)
    - [unordered_map\<K, V\>](#unordered_mapk-v)
    - [span\<T\>](#spant)
  - [Smart Pointers](#smart-pointers)
    - [unique_ptr\<T\>](#unique_ptrt)
    - [shared_ptr\<T\>](#shared_ptrt)
  - [Utility Types](#utility-types)
    - [pair\<T1, T2\>](#pairt1-t2)
    - [tuple\<Ts...\>](#tuplets)
    - [optional\<T\>](#optionalt)
  - [Memory Management](#memory-management)
  - [Algorithms](#algorithms)
  - [Numeric Algorithms](#numeric-algorithms)
  - [Iterators](#iterators)
  - [Type Traits](#type-traits)
  - [Functional Objects](#functional-objects)
  - [Numeric Limits](#numeric-limits)
  - [I/O Streams](#io-streams)
  - [String Streams](#string-streams)
  - [C++ Wrapper Headers](#c-wrapper-headers)
  - [Linking a C++ Program](#linking-a-c-program)
- [Limitations and Known Differences](#limitations-and-known-differences)

---

# libc64 (64-bit C Standard Library)

## Overview

**Location:** `libs/libc64/`
**Compiler:** `clang --target=x86_64-unknown-none-elf -ffreestanding -nostdlib -nostdinc -O2`
**Output:** `libc64.a` + `crt0.o` + `crti.o` + `crtn.o`
**Syscall ABI:** `SYSCALL` instruction (RAX=num, RBX=a1, R10=a2, RDX=a3, RSI=a4, RDI=a5; return in RAX)
**Installed to:** `/System/Libraries/libc64/`

libc64 is the full 64-bit freestanding C standard library for x86_64 user programs running in long mode on anyOS. It provides the same API surface as the 32-bit libc (see [libc-api.md](libc-api.md) for the complete C library API reference) but compiled for the x86_64 target.

## Build System

```makefile
CC = clang
AR = ar
AS = clang

CFLAGS = --target=x86_64-unknown-none-elf -ffreestanding -nostdlib -nostdinc \
         -fno-builtin -fno-stack-protector -Wall -Wextra -O2 -I include
ASFLAGS = --target=x86_64-unknown-none-elf -c
```

Built via `make` in `libs/libc64/`. The CMake build system invokes this automatically and installs artifacts to the sysroot.

## Runtime

### Entry Point (`crt0.S`)
- `_start`: Clears frame pointer (`xor %rbp, %rbp`), calls `__libc_start_main()`
- `__libc_start_main` (in `start.c`): Parses arguments from `SYS_GETARGS` syscall, invokes `main(argc, argv)`, calls `exit()` with return value

### CRT Files
| File | Purpose |
|------|---------|
| `crt0.o` (= `crt1.o`) | Entry point, calls `__libc_start_main` |
| `crti.o` | Init section prologue (`.init` / `.fini` function prologues) |
| `crtn.o` | Init section epilogue (`.init` / `.fini` function epilogues) |

### Startup Sequence
```
_start (crt0.S)
  └─ __libc_start_main (start.c)
       ├─ SYS_GETARGS syscall → parse argc/argv
       ├─ main(argc, argv)
       └─ exit(return_value)
```

## Syscall Layer

### `_syscall()` (`syscall.S`)

Maps System V AMD64 ABI calling convention to the anyOS kernel SYSCALL convention:

| C ABI (caller) | Kernel Register | Role |
|----------------|-----------------|------|
| `rdi` | `rax` | Syscall number |
| `rsi` | `rbx` | Argument 1 |
| `rdx` | `r10` | Argument 2 |
| `rcx` | `rdx` | Argument 3 |
| `r8` | `rsi` | Argument 4 |
| `r9` | `rdi` | Argument 5 |

```c
long _syscall(long num, long a1, long a2, long a3, long a4, long a5);
```

RBX is saved/restored because LLVM reserves it on x86_64. The `SYSCALL` instruction clobbers RCX and R11.

## Headers

libc64 provides a complete set of C standard and POSIX headers (under `libs/libc64/include/`):

| Header | Description |
|--------|-------------|
| `stdio.h` | File I/O, printf, scanf, FILE streams |
| `stdlib.h` | malloc/free, atoi, strtol, qsort, exit, atexit, environment |
| `string.h` / `strings.h` | Memory/string operations, case-insensitive comparisons |
| `unistd.h` | read, write, close, lseek, getcwd, sbrk, fork stubs |
| `fcntl.h` | open, O_RDONLY/O_WRONLY/O_RDWR/O_CREAT/O_TRUNC/O_APPEND |
| `dirent.h` | opendir, readdir, closedir, scandir |
| `sys/stat.h` | stat, fstat, mkdir, mode constants |
| `time.h` | time, clock, localtime, gmtime, strftime, nanosleep |
| `math.h` | sqrt, sin, cos, pow, log, floor, ceil, fabs, etc. |
| `signal.h` | signal, raise, POSIX signal constants |
| `setjmp.h` | setjmp, longjmp (x86_64 assembly implementation) |
| `errno.h` | errno variable, POSIX error constants (ENOENT, EINVAL, ...) |
| `ctype.h` | isalpha, isdigit, toupper, tolower, etc. |
| `getopt.h` | getopt, getopt_long |
| `stdint.h` | Fixed-width types (int8_t--int64_t, uint8_t--uint64_t) |
| `stddef.h` | size_t, ssize_t, ptrdiff_t, NULL, offsetof |
| `stdbool.h` | bool, true, false |
| `stdarg.h` | va_list, va_start, va_end, va_arg, va_copy |
| `limits.h` | INT_MIN, INT_MAX, LONG_MIN, LONG_MAX, PATH_MAX |
| `assert.h` | assert() macro |
| `inttypes.h` | Printf/scanf format macros (PRId64, PRIu32, etc.) |
| `locale.h` | Locale stubs |
| `regex.h` | Regex stubs |
| `termios.h` | Terminal I/O stubs |
| `endian.h` | Byte order macros |
| `iconv.h` | Character conversion stubs |
| `zlib.h` | zlib compatibility stubs |
| `sys/socket.h` | socket, connect, bind, listen, accept, send, recv |
| `sys/mman.h` | mmap, munmap stubs |
| `sys/select.h` | select, fd_set stubs |
| `sys/time.h` | gettimeofday, timeval |
| `sys/utsname.h` | uname stub |
| `netinet/in.h` | sockaddr_in, in_addr, htons/ntohs |
| `arpa/inet.h` | inet_aton, inet_ntoa, inet_pton, inet_ntop |
| `netdb.h` | gethostbyname, getaddrinfo |
| `net/if.h` | Network interface stubs |
| `poll.h` | poll stubs |
| `pwd.h` | getpwuid, getpwnam |
| `spawn.h` | posix_spawn stubs |
| `memory.h` | Alias for string.h |

See [libc-api.md](libc-api.md) for the detailed function-level API documentation. The libc64 API is identical to the 32-bit libc, except for the differences noted below.

## Source Modules

| File | Contents |
|------|----------|
| `crt0.S` | Entry point (`_start`) |
| `crti.S` | `.init` / `.fini` section prologues |
| `crtn.S` | `.init` / `.fini` section epilogues |
| `syscall.S` | SYSCALL wrapper (`_syscall`) |
| `setjmp.S` | setjmp/longjmp (x86_64 register save/restore) |
| `start.c` | `__libc_start_main` (arg parsing, calls main) |
| `stdio.c` | FILE streams, printf, fprintf, fopen, fread, fwrite, etc. |
| `stdlib.c` | malloc/free (arena-based with 64 KiB sbrk chunks), atoi, qsort, exit, atexit |
| `string.c` | memcpy, memmove, memset, memcmp, strlen, strcmp, strcpy, strcat, strstr, strdup, etc. |
| `ctype.c` | Character classification functions |
| `math.c` | Mathematical functions (software implementations) |
| `time.c` | Time functions |
| `unistd.c` | POSIX wrappers (read, write, close, lseek, getcwd, sbrk) |
| `stat.c` | stat, fstat, mkdir |
| `signal.c` | signal, raise |
| `socket.c` | Socket functions (socket, connect, bind, send, recv) |
| `mman.c` | mmap/munmap stubs |
| `stubs.c` | Miscellaneous stubs for unimplemented functions |

### Memory Allocator

The libc64 malloc implementation uses an arena-based strategy with free-list reuse:

- **Arena allocation**: Requests 64 KiB chunks from `sbrk()` at a time, suballocates locally
- **16-byte alignment**: All allocations are aligned to 16 bytes (x86_64 ABI requirement)
- **Free list**: Freed blocks are coalesced and reused via first-fit search
- **Block splitting**: Large free blocks are split when only a portion is needed
- Functions: `malloc()`, `calloc()`, `realloc()`, `free()`

## Stub Intrinsic Headers

For compatibility with code that includes compiler intrinsic headers, libc64 provides stub versions that define no-op or zero-returning macros:

| Header | Purpose |
|--------|---------|
| `cpuid.h` | `__get_cpuid()` stub (returns 0) |
| `immintrin.h` | Empty (SSE/AVX intrinsics not available) |
| `x86intrin.h` | Empty |
| `intrin.h` | Empty |

## Key Differences from libc (32-bit)

| Feature | libc (32-bit) | libc64 (64-bit) |
|---------|---------------|-----------------|
| Compiler | `i686-elf-gcc -m32` | `clang --target=x86_64-unknown-none-elf` |
| Syscall ABI | `INT 0x80` (EAX/EBX/ECX/EDX/ESI/EDI) | `SYSCALL` (RAX/RBX/R10/RDX/RSI/RDI) |
| Pointer size | 4 bytes | 8 bytes |
| Alignment | 8-byte | 16-byte |
| setjmp buffer | 6 x 32-bit registers | 8 x 64-bit registers |
| CRT files | `crt0.o` only (`crti.o`/`crtn.o` are empty stubs) | `crt0.o` + `crti.o` + `crtn.o` (real `.init`/`.fini` support) |
| Intrinsic headers | Not provided | `cpuid.h`, `immintrin.h`, `x86intrin.h`, `intrin.h` (stubs) |
| sbrk chunk size | Per-allocation | 64 KiB arena batching |
| Installed to | `/System/Libraries/libc/` | `/System/Libraries/libc64/` |

---

# libcxx (C++20 Standard Library)

## Overview

**Location:** `libs/libcxx/`
**Compiler:** `clang++ --target=x86_64-unknown-none-elf -std=c++20 -fno-exceptions -fno-rtti -O2`
**Output:** `libcxx.a` (depends on `libc64.a`)
**Installed to:** `/System/Libraries/libcxx/`

libcxx is a minimal but functional C++20 standard library for anyOS, providing the core containers, smart pointers, algorithms, type traits, and I/O streams needed for 64-bit C++ programs. It operates in a freestanding environment with no exceptions (`-fno-exceptions`) and no RTTI (`-fno-rtti`).

### Design Principles

- **Header-only where possible**: Most components are header-only templates. Only `new.cpp` (operator new/delete) and `iostream.cpp` (global stream objects) require compilation.
- **No exceptions**: Functions that would throw in standard C++ call `std::abort()` instead (e.g., `at()` on invalid index, `optional::value()` when empty, allocation failure).
- **No RTTI**: No `dynamic_cast`, no `typeid`. Virtual destructors in `shared_ptr`'s control block use explicit virtual dispatch.
- **Depends on libc64**: Uses libc64's `malloc`/`free`/`printf`/`memcpy` etc. for all memory and I/O operations.

## Build System

```makefile
CXX      = clang++
AR       = ar
CXXFLAGS = --target=x86_64-unknown-none-elf -ffreestanding -nostdlib \
           -fno-exceptions -fno-rtti -std=c++20 -O2 \
           -I include -I ../libc64/include -Wall -Wextra -Wno-unused-parameter
```

Built via `make` in `libs/libcxx/`. The CMake build system invokes this automatically and installs the library and headers to the sysroot.

### Source Files

| File | Contents |
|------|----------|
| `new.cpp` | `operator new`/`delete` implementations (routes to libc64 malloc/free) |
| `iostream.cpp` | Global `std::cout`, `std::cerr`, `std::cin` stream object definitions |

---

## Containers

### vector\<T\>

**Header:** `<vector>`

Dynamic array with amortized O(1) push_back and O(1) random access. Growth factor is 2x.

```cpp
#include <vector>

std::vector<int> v;                    // default
std::vector<int> v(10);               // 10 value-initialized elements
std::vector<int> v(5, 42);            // 5 copies of 42
std::vector<int> v = {1, 2, 3};       // initializer list
std::vector<int> v(first, last);      // iterator range
```

**Element Access:**
- `operator[]`, `at()`, `front()`, `back()`, `data()`

**Capacity:**
- `empty()`, `size()`, `max_size()`, `capacity()`, `reserve()`, `shrink_to_fit()`

**Modifiers:**
- `clear()`, `push_back()`, `emplace_back()`, `pop_back()`
- `insert()` (by value, by move), `erase()` (single, range)
- `resize()` (default or fill), `swap()`, `assign()`

**Iterators:**
- `begin()`/`end()`, `cbegin()`/`cend()`, `rbegin()`/`rend()`
- Iterator type: raw pointer (`T*`)

**Comparison:** `==`, `!=`, `<` (lexicographic)

---

### string

**Header:** `<string>`

Mutable string class with Small String Optimization (SSO). Strings of 22 bytes or fewer are stored inline without heap allocation.

```cpp
#include <string>

std::string s;                         // empty
std::string s("hello");               // from C string
std::string s("hello", 3);            // from buffer + length → "hel"
std::string s(5, 'x');                // fill → "xxxxx"
std::string s(sv);                    // from string_view
```

**SSO Details:**
- Threshold: 22 characters (+ NUL = 23 bytes stored inline)
- Total object size: 32 bytes on x86_64
- No heap allocation for short strings

**Element Access:**
- `operator[]`, `at()`, `front()`, `back()`, `data()`, `c_str()`
- Implicit conversion to `string_view`

**Capacity:**
- `empty()`, `size()`, `length()`, `max_size()`, `capacity()`
- `reserve()`, `shrink_to_fit()` (may transition between SSO and heap)

**Modifiers:**
- `clear()`, `push_back()`, `pop_back()`
- `append()` (string, C string, count+char)
- `insert()`, `erase()`, `replace()`, `resize()`, `swap()`
- `operator+=` (string, C string, char)

**Search:**
- `find()`, `rfind()`, `find_first_of()`, `find_last_of()`, `find_first_not_of()`
- `substr()`, `compare()`, `starts_with()`, `ends_with()`, `contains()` (C++20)

**Conversions:**
- `std::to_string(int)`, `std::to_string(long)`, `std::to_string(unsigned long)`, etc.

**Comparison:** `==`, `!=`, `<`, `>`, `<=`, `>=` (with `string`, `const char*`)
**Concatenation:** `+` operator (string+string, string+cstr, cstr+string, string+char, char+string)
**Hashing:** `std::hash<string>` specialization (FNV-1a via string_view)

---

### string_view

**Header:** `<string_view>`

Non-owning, read-only reference to a contiguous character sequence. Zero-cost abstraction for passing string data without copying.

```cpp
#include <string_view>

std::string_view sv;                   // empty
std::string_view sv("hello");         // from C string (computes length)
std::string_view sv("hello", 3);      // from pointer + length → "hel"
std::string_view sv = str;            // from std::string (implicit)
```

**Element Access:**
- `operator[]`, `at()`, `front()`, `back()`, `data()`

**Capacity:**
- `empty()`, `size()`, `length()`, `max_size()`

**Modifiers:**
- `remove_prefix()`, `remove_suffix()`, `swap()`

**Operations:**
- `copy()`, `substr()`, `compare()`
- `find()`, `rfind()`, `find_first_of()`, `find_last_of()`, `find_first_not_of()`
- `starts_with()`, `ends_with()`, `contains()` (C++20)

**Literal:** `"hello"_sv` (via `using namespace std::string_view_literals`)
**Hashing:** `std::hash<string_view>` specialization (FNV-1a)

---

### array\<T, N\>

**Header:** `<array>`

Fixed-size aggregate container. No heap allocation, size known at compile time.

```cpp
#include <array>

std::array<int, 4> a = {1, 2, 3, 4};  // aggregate initialization
```

**Element Access:** `operator[]`, `at()`, `front()`, `back()`, `data()`
**Capacity:** `empty()`, `size()`, `max_size()` (all constexpr, equal to N)
**Operations:** `fill()`, `swap()`
**Iterators:** `begin()`/`end()`, `cbegin()`/`cend()`, `rbegin()`/`rend()`
**Structured bindings:** `std::get<I>(array)` supported
**Comparison:** `==`, `!=`, `<`
**Zero-size:** `array<T, 0>` specialization provided

---

### map\<K, V\>

**Header:** `<map>`

Ordered associative container backed by a Red-Black tree. Keys are sorted by `Compare` (default: `std::less<K>`). O(log n) lookup, insertion, and deletion.

```cpp
#include <map>

std::map<std::string, int> m;
m["key"] = 42;                         // insert or update via operator[]
m.insert({"key2", 100});              // insert pair
auto it = m.find("key");             // O(log n) lookup
```

**Element Access:**
- `operator[]` (inserts default if missing), `at()` (aborts if missing)

**Lookup:**
- `find()`, `count()`, `contains()` (C++20), `lower_bound()`

**Modifiers:**
- `insert()`, `emplace()`, `erase()` (by iterator or key), `clear()`, `swap()`

**Iterators:** Bidirectional iterators over `std::pair<const K, V>`. In-order traversal.

**Capacity:** `empty()`, `size()`
**Comparison:** `==`, `!=`

---

### set\<K\>

**Header:** `<set>`

Ordered set container backed by a dedicated Red-Black tree (key-only nodes, no mapped value). O(log n) operations.

```cpp
#include <set>

std::set<int> s = {3, 1, 4, 1, 5};    // duplicates ignored → {1, 3, 4, 5}
s.insert(2);
bool has = s.contains(3);             // true
```

**Lookup:** `find()`, `count()`, `contains()`, `lower_bound()`
**Modifiers:** `insert()`, `emplace()`, `erase()` (by iterator or key), `clear()`, `swap()`
**Iterators:** Bidirectional, always const (set values are immutable)
**Capacity:** `empty()`, `size()`
**Comparison:** `==`, `!=`

---

### unordered_map\<K, V\>

**Header:** `<unordered_map>`

Hash table based associative container. O(1) average lookup, insertion, and deletion. Uses separate chaining (linked lists per bucket).

```cpp
#include <unordered_map>

std::unordered_map<std::string, int> m;
m["key"] = 42;
auto it = m.find("key");              // O(1) average
```

**Template Parameters:** `<K, V, Hash = hash<K>, KeyEqual = equal_to<K>>`

**Default Configuration:**
- Initial bucket count: 16 (minimum 4)
- Max load factor: 1.0
- Rehash: Doubles bucket count when load factor exceeded

**Element Access:**
- `operator[]` (inserts default if missing), `at()` (aborts if missing)

**Lookup:** `find()`, `count()`, `contains()`

**Modifiers:**
- `insert()`, `emplace()`, `erase()` (by iterator or key), `clear()`, `reserve()`, `swap()`

**Capacity:** `empty()`, `size()`, `bucket_count()`, `load_factor()`, `max_load_factor()`
**Iterators:** Forward iterators over `std::pair<const K, V>`
**Comparison:** `==`, `!=`

---

### span\<T\>

**Header:** `<span>`

Non-owning view over a contiguous sequence of objects (C++20). Both dynamic extent and static (compile-time) extent are supported.

```cpp
#include <span>

int arr[] = {1, 2, 3, 4, 5};
std::span<int> s(arr);                 // dynamic extent
std::span<int, 5> s(arr);             // static extent

std::array<int, 3> a = {1, 2, 3};
std::span<int> s(a);                   // from array

auto sub = s.subspan(1, 3);           // {2, 3, 4}
auto first = s.first(2);              // {1, 2}
auto last = s.last(2);                // {4, 5}
```

**Element Access:** `operator[]`, `front()`, `back()`, `data()`
**Capacity:** `size()`, `size_bytes()`, `empty()`, `extent` (constexpr)
**Subviews:** `first()`, `last()`, `subspan()`
**Iterators:** `begin()`/`end()`, `cbegin()`/`cend()`, `rbegin()`/`rend()`
**Conversion:** `as_bytes()`, `as_writable_bytes()`
**Sentinel:** `std::dynamic_extent` constant

---

## Smart Pointers

### unique_ptr\<T\>

**Header:** `<memory>`

Exclusive-ownership smart pointer. Move-only, no copy. Automatically deletes the managed object when the unique_ptr goes out of scope.

```cpp
#include <memory>

auto p = std::make_unique<int>(42);    // preferred creation
auto p = std::unique_ptr<int>(new int(42));
int& val = *p;
p.reset();                             // releases ownership
int* raw = p.release();               // returns raw pointer, releases ownership
```

**Specializations:**
- Single object: `unique_ptr<T>` -- `operator*`, `operator->`
- Array: `unique_ptr<T[]>` -- `operator[]`, `delete[]` on destruction

**Deleter:** Custom deleter via second template parameter (default: `default_delete<T>`)

**Modifiers:** `reset()`, `release()`, `swap()`
**Observers:** `get()`, `get_deleter()`, `operator bool`
**Comparison:** `==`, `!=` (with other unique_ptr or nullptr)

---

### shared_ptr\<T\>

**Header:** `<memory>`

Reference-counted shared-ownership smart pointer. Multiple shared_ptrs can own the same object. The managed object is deleted when the last shared_ptr is destroyed.

```cpp
#include <memory>

auto p = std::make_shared<int>(42);    // single allocation (inplace control block)
auto p = std::shared_ptr<int>(new int(42));  // separate allocation
auto q = p;                            // reference count → 2
long count = p.use_count();           // 2
p.reset();                            // count → 1
```

**Control Block:**
- `make_shared`: Object and control block in a single allocation (optimal)
- Direct construction: Separate control block allocation

**Modifiers:** `reset()`, `swap()`
**Observers:** `get()`, `use_count()`, `operator bool`, `operator*`, `operator->`
**Comparison:** `==`, `!=` (with other shared_ptr or nullptr)

Note: `weak_ptr` is not currently implemented. The weak_count field in the control block is reserved for future use.

---

## Utility Types

### pair\<T1, T2\>

**Header:** `<utility>`

```cpp
#include <utility>

std::pair<int, std::string> p(1, "hello");
auto p = std::make_pair(1, "hello");
auto [key, value] = p;                // structured bindings
```

**Members:** `first`, `second`
**Comparison:** `==`, `!=`, `<`, `>`, `<=`, `>=` (lexicographic)

---

### tuple\<Ts...\>

**Header:** `<tuple>`

Heterogeneous fixed-size container. Supports arbitrary numbers of elements.

```cpp
#include <tuple>

auto t = std::make_tuple(1, 3.14, "hello");
int x = std::get<0>(t);               // 1
auto [a, b, c] = t;                   // structured bindings

auto t = std::tie(x, y);              // tuple of references
```

**Access:** `std::get<I>(tuple)`
**Introspection:** `std::tuple_size<T>`, `std::tuple_element<I, T>`
**Factories:** `make_tuple()`, `tie()`, `forward_as_tuple()`
**Comparison:** `==`, `!=`, `<` (element-wise)
**Special:** `std::ignore` for discarding tie elements

---

### optional\<T\>

**Header:** `<optional>`

A container that may or may not hold a value. No heap allocation; uses aligned in-place storage.

```cpp
#include <optional>

std::optional<int> opt;                // empty
std::optional<int> opt = 42;          // contains value
opt = std::nullopt;                    // reset to empty

if (opt) { int x = *opt; }           // check and access
int x = opt.value_or(0);              // default value
opt.emplace(100);                      // construct in-place
```

**Observers:**
- `has_value()`, `operator bool`, `value()` (aborts if empty), `operator*`, `operator->`
- `value_or(default)` -- returns value or default

**Modifiers:** `reset()`, `emplace()`, `swap()`
**Comparison:** `==`, `!=` (with other optional, nullopt, or value)
**Factory:** `std::make_optional()`

---

## Memory Management

**Header:** `<new>`

```cpp
void* operator new(std::size_t size);              // aborts on failure
void* operator new[](std::size_t size);            // aborts on failure
void  operator delete(void* ptr) noexcept;
void  operator delete[](void* ptr) noexcept;
void  operator delete(void* ptr, std::size_t) noexcept;   // sized delete
void  operator delete[](void* ptr, std::size_t) noexcept;

// Nothrow variants — return nullptr on failure
void* operator new(std::size_t size, const std::nothrow_t&) noexcept;
void* operator new[](std::size_t size, const std::nothrow_t&) noexcept;

// Placement new — no allocation
void* operator new(std::size_t, void* ptr) noexcept;    // returns ptr
void* operator new[](std::size_t, void* ptr) noexcept;  // returns ptr
```

All new/delete implementations route to libc64's `malloc()`/`free()`. Since there are no exceptions, regular `operator new` calls `abort()` on allocation failure instead of throwing `std::bad_alloc`.

**Types:**
- `std::nothrow_t`, `std::nothrow` -- tag for nothrow overloads
- `std::align_val_t` -- alignment tag (enum class)

**Uninitialized Memory Operations** (in `<memory>`):
- `std::uninitialized_default_construct()`, `std::uninitialized_fill()`
- `std::uninitialized_copy()`, `std::uninitialized_move()`
- `std::destroy()`, `std::destroy_at()`
- `std::addressof()`

---

## Algorithms

**Header:** `<algorithm>`

### Non-Modifying Sequence Operations
```cpp
find(first, last, value)                    // linear search
find_if(first, last, predicate)             // search by predicate
find_if_not(first, last, predicate)
all_of(first, last, pred)                   // true if all match
any_of(first, last, pred)                   // true if any matches
none_of(first, last, pred)                  // true if none match
for_each(first, last, fn)                   // apply function
count(first, last, value)                   // count occurrences
count_if(first, last, pred)
equal(first1, last1, first2)               // element-wise equality
mismatch(first1, last1, first2)            // find first difference
lexicographical_compare(f1, l1, f2, l2)    // dictionary ordering
```

### Modifying Sequence Operations
```cpp
copy(first, last, dest)
copy_backward(first, last, d_last)
copy_if(first, last, dest, pred)
copy_n(first, n, dest)
move(first, last, dest)                     // move elements
move_backward(first, last, d_last)
fill(first, last, value)
fill_n(first, n, value)
generate(first, last, gen)
transform(first, last, dest, unary_op)      // apply and store
replace(first, last, old_val, new_val)
unique(first, last)                         // remove consecutive duplicates
remove(first, last, value)                  // shift-remove (returns new end)
remove_if(first, last, pred)
reverse(first, last)
rotate(first, middle, last)
swap_ranges(first1, last1, first2)
```

### Sorting & Ordering
```cpp
sort(first, last)                           // introsort (quicksort + insertion sort)
sort(first, last, comparator)               // custom comparator
stable_sort(first, last)                    // insertion sort (stable)
nth_element(first, nth, last)               // partial ordering
partition(first, last, pred)
is_sorted(first, last)
```

The default `sort()` uses introsort: quicksort with median-of-three pivot selection, falling back to insertion sort for runs of 16 or fewer elements and when recursion depth exceeds 2*log(n). The custom-comparator overload uses insertion sort.

### Binary Search (sorted ranges)
```cpp
lower_bound(first, last, value)
upper_bound(first, last, value)
binary_search(first, last, value)           // true if found
equal_range(first, last, value)             // returns pair<lower, upper>
```

### Min / Max
```cpp
min(a, b)                  max(a, b)
min(a, b, comp)            max(a, b, comp)
min(initializer_list)      max(initializer_list)
min_element(first, last)   max_element(first, last)
clamp(value, lo, hi)
```

### Heap Operations
```cpp
make_heap(first, last)
push_heap(first, last)
pop_heap(first, last)
sort_heap(first, last)
```

### Merge
```cpp
merge(f1, l1, f2, l2, dest)               // merge two sorted ranges
```

---

## Numeric Algorithms

**Header:** `<numeric>`

```cpp
accumulate(first, last, init)               // sum with initial value
accumulate(first, last, init, binary_op)    // custom operation
reduce(first, last, init)                   // C++17 (same as accumulate)
reduce(first, last, init, binary_op)
inner_product(f1, l1, f2, init)            // dot product
inner_product(f1, l1, f2, init, op1, op2)  // generalized
partial_sum(first, last, dest)             // running sum
adjacent_difference(first, last, dest)
iota(first, last, start_value)             // fill with incrementing values
gcd(a, b)                                  // greatest common divisor (C++17)
lcm(a, b)                                  // least common multiple (C++17)
midpoint(a, b)                             // midpoint (C++20, integer and floating)
```

---

## Iterators

**Header:** `<iterator>`

### Iterator Category Tags
```cpp
struct input_iterator_tag {};
struct output_iterator_tag {};
struct forward_iterator_tag       : input_iterator_tag {};
struct bidirectional_iterator_tag : forward_iterator_tag {};
struct random_access_iterator_tag : bidirectional_iterator_tag {};
struct contiguous_iterator_tag    : random_access_iterator_tag {};  // C++20
```

### iterator_traits\<Iter\>
```cpp
iterator_traits<Iter>::value_type
iterator_traits<Iter>::difference_type
iterator_traits<Iter>::pointer
iterator_traits<Iter>::reference
iterator_traits<Iter>::iterator_category
```
Pointer specialization: `iterator_traits<T*>` maps to `random_access_iterator_tag`.

### reverse_iterator\<Iter\>
Wraps a bidirectional or random-access iterator to iterate in reverse. Full arithmetic and comparison support.

```cpp
std::reverse_iterator<It> rit(it);
auto rit = std::make_reverse_iterator(it);
```

### Iterator Operations
```cpp
distance(first, last)     // O(1) for random-access, O(n) for input
advance(it, n)            // bidirectional-aware
next(it, n = 1)           // returns advanced copy
prev(it, n = 1)           // returns retreated copy
```

### Range Access (C++17)
```cpp
std::begin(container)     std::end(container)
std::cbegin(container)    std::cend(container)
std::begin(array)         std::end(array)
std::size(container)      std::size(array)
std::data(container)      std::data(array)
std::empty(container)     std::empty(array)
```

---

## Type Traits

**Header:** `<type_traits>`

### Primary Type Categories
```cpp
is_void<T>              is_null_pointer<T>       is_integral<T>
is_floating_point<T>    is_array<T>              is_pointer<T>
is_lvalue_reference<T>  is_rvalue_reference<T>   is_reference<T>
is_function<T>          is_enum<T>               is_union<T>
is_class<T>
```

### Composite Type Categories
```cpp
is_arithmetic<T>        is_fundamental<T>        is_scalar<T>
is_object<T>
```

### Type Properties
```cpp
is_const<T>             is_volatile<T>
is_signed<T>            is_unsigned<T>
is_trivially_copyable<T>  is_trivially_destructible<T>
is_trivial<T>           is_standard_layout<T>    is_pod<T>
is_empty<T>             is_abstract<T>           is_polymorphic<T>
is_final<T>
is_constructible<T, Args...>   is_default_constructible<T>
is_copy_constructible<T>       is_move_constructible<T>
is_assignable<T, U>            is_destructible<T>
```

### Type Relationships
```cpp
is_same<T, U>           is_base_of<Base, Derived>
is_convertible<From, To>
```

### Type Transformations
```cpp
remove_const<T>         add_const<T>
remove_volatile<T>      add_volatile<T>
remove_cv<T>            add_cv<T>
remove_reference<T>     add_lvalue_reference<T>   add_rvalue_reference<T>
remove_pointer<T>       add_pointer<T>
remove_extent<T>
decay<T>
conditional<B, T, F>    enable_if<B, T>
make_signed<T>          make_unsigned<T>
common_type<T, U>
aligned_storage<Len, Align>
alignment_of<T>
```

All traits have `_t` alias templates and `_v` variable templates where applicable.

### Meta-Functions (C++17)
```cpp
void_t<Ts...>           // always void, used for SFINAE
conjunction<Bs...>       disjunction<Bs...>       negation<B>
```

### Helpers
```cpp
integral_constant<T, v>
true_type / false_type
declval<T>()            // unevaluated rvalue reference
```

---

## Functional Objects

**Header:** `<functional>`

### Comparison Function Objects
```cpp
less<T>           greater<T>         equal_to<T>
less_equal<T>     greater_equal<T>   not_equal_to<T>
```
Transparent (void) specializations for `less<void>`, `greater<void>`, `equal_to<void>`, `plus<void>`, `minus<void>`.

### Arithmetic Function Objects
```cpp
plus<T>           minus<T>           multiplies<T>
divides<T>        modulus<T>         negate<T>
```

### Logical Function Objects
```cpp
logical_and<T>    logical_or<T>      logical_not<T>
```

### Bitwise Function Objects
```cpp
bit_and<T>        bit_or<T>          bit_xor<T>          bit_not<T>
```

### Hash
```cpp
std::hash<T>      // specializations for all integral types, float, double,
                  // pointers, string, string_view
```
Integer hashing uses multiply-shift (`* 2654435761`). Float/string hashing uses FNV-1a.

### reference_wrapper
```cpp
std::reference_wrapper<T>
std::ref(t)       std::cref(t)
```
Wrapper that makes references copyable and callable.

---

## Numeric Limits

**Header:** `<limits>`

Full `std::numeric_limits<T>` specializations for all fundamental types:

| Type | `min()` | `max()` | `digits` |
|------|---------|---------|----------|
| `bool` | `false` | `true` | 1 |
| `char` | `CHAR_MIN` | `CHAR_MAX` | 7 |
| `short` | `SHRT_MIN` | `SHRT_MAX` | 15 |
| `int` | `INT_MIN` | `INT_MAX` | 31 |
| `long` | `LONG_MIN` | `LONG_MAX` | 63 |
| `long long` | `LLONG_MIN` | `LLONG_MAX` | 63 |
| `float` | `1.17549e-38` | `3.40282e+38` | 24 |
| `double` | `2.22507e-308` | `1.79769e+308` | 53 |
| `long double` | `3.36210e-4932` | `1.18973e+4932` | 64 |

Plus all unsigned variants and `signed char`/`unsigned char`.

**Float-specific members:** `epsilon()`, `infinity()`, `quiet_NaN()`, `denorm_min()`, `is_iec559`, `round_to_nearest`

---

## I/O Streams

**Header:** `<iostream>`

Minimal I/O streams that route to libc64's `printf`/`fwrite`/`fgetc`. Supports format flags, width padding, and manipulators.

### Global Streams
```cpp
std::cout    // ostream writing to stdout (fd 1)
std::cerr    // ostream writing to stderr (fd 2)
std::cin     // istream reading from stdin (fd 0)
```

### ostream
```cpp
std::cout << "text" << 42 << 3.14 << std::endl;
std::cout << std::hex << 255;          // "ff"
std::cout << std::setw(10) << "right"; // right-padded
```

**Supported insertion types:** `const char*`, `char`, `bool`, `int`, `unsigned int`, `long`, `unsigned long`, `long long`, `unsigned long long`, `float`, `double`, `const void*`, `string`, `string_view`

**Methods:** `put()`, `write()`, `flush()`
**State:** `good()`, `eof()`, `fail()`, `bad()`, `operator bool`

### istream
```cpp
int n;
std::string s;
std::cin >> n >> s;                    // whitespace-delimited input
char c = std::cin.get();
std::cin.getline(buf, size);
std::cin.ignore(count, delim);
int c = std::cin.peek();
```

**Supported extraction types:** `char`, `int`, `long`, `string`

### Free Functions
```cpp
std::getline(istream, string, delim = '\n')
```

### Manipulators
```cpp
std::endl        std::flush
std::hex         std::dec         std::oct
std::boolalpha   std::noboolalpha
std::fixed       std::scientific
std::left        std::right
std::setw(n)     // width for next insertion
```

### ios_base Format Flags
```cpp
ios_base::dec, hex, oct, basefield
ios_base::left, right, internal, adjustfield
ios_base::boolalpha, showbase, showpos, uppercase
ios_base::fixed, scientific, floatfield
```

---

## String Streams

**Header:** `<sstream>`

Self-contained string-based streams (not derived from iostream).

### ostringstream
```cpp
std::ostringstream oss;
oss << "Value: " << 42 << " pi=" << 3.14;
std::string result = oss.str();        // "Value: 42 pi=3.14"
oss.str("");                           // reset
```

**Supported types:** `const char*`, `char`, `bool`, `int`, `unsigned int`, `long`, `unsigned long`, `long long`, `unsigned long long`, `float`, `double`, `const void*`, `string`, `string_view`

### istringstream
```cpp
std::istringstream iss("42 hello 3.14");
int n; std::string s; double d;
iss >> n >> s >> d;                     // n=42, s="hello", d=3.14
```

**Supported types:** `char`, `int`, `long`, `double`, `string`
**Methods:** `str()`, `good()`, `eof()`, `fail()`, `operator bool`
**Free function:** `std::getline(istringstream&, string&, delim = '\n')`

---

## C++ Wrapper Headers

Standard C++ wrapper headers that include and re-export the corresponding libc64 C headers into the `std` namespace:

| Header | Wraps | Key Contents |
|--------|-------|-------------|
| `<cctype>` | `ctype.h` | `std::isalpha`, `std::isdigit`, `std::toupper`, `std::tolower`, ... |
| `<cerrno>` | `errno.h` | `errno`, `ENOENT`, `EINVAL`, ... |
| `<climits>` | `limits.h` | `INT_MIN`, `INT_MAX`, `LONG_MIN`, `LONG_MAX`, ... |
| `<cmath>` | `math.h` | `std::sqrt`, `std::sin`, `std::cos`, `std::pow`, `std::log`, ... |
| `<cstddef>` | `stddef.h` | `std::size_t`, `std::ptrdiff_t`, `std::nullptr_t`, `std::byte` |
| `<cstdint>` | `stdint.h` | `std::int8_t`--`std::int64_t`, `std::uint8_t`--`std::uint64_t`, ... |
| `<cstdio>` | `stdio.h` | `std::printf`, `std::fprintf`, `std::fopen`, `std::FILE`, ... |
| `<cstdlib>` | `stdlib.h` | `std::malloc`, `std::free`, `std::abort`, `std::exit`, `std::atoi`, ... |
| `<cstring>` | `string.h` | `std::memcpy`, `std::strlen`, `std::strcmp`, `std::strcpy`, ... |
| `<ctime>` | `time.h` | `std::time`, `std::localtime`, `std::strftime`, ... |

---

## Linking a C++ Program

To link a C++ program for anyOS, you need both libcxx and libc64. The typical link command:

```bash
clang++ --target=x86_64-unknown-none-elf -std=c++20 \
    -fno-exceptions -fno-rtti -ffreestanding -nostdlib \
    -I /System/Libraries/libcxx/include \
    -I /System/Libraries/libc64/include \
    -o myprogram main.cpp \
    /System/Libraries/libc64/lib/crt0.o \
    /System/Libraries/libc64/lib/crti.o \
    -L /System/Libraries/libcxx/lib -lcxx \
    -L /System/Libraries/libc64/lib -lc64 \
    /System/Libraries/libc64/lib/crtn.o
```

**Link order matters:** `crt0.o` and `crti.o` first, then your object files, then `-lcxx`, then `-lc64`, then `crtn.o` last.

### CMakeLists.txt Integration

C++ programs are registered in the build system alongside the library install targets. The `CXX_TOOLCHAIN_DEPS` custom target ensures both libraries and their headers are installed to the sysroot before any dependent C++ program is built.

### Quick Start: Hello World (C)

```c
/* hello.c */
#include <stdio.h>

int main(int argc, char **argv) {
    printf("Hello from 64-bit anyOS! argc=%d\n", argc);
    for (int i = 0; i < argc; i++)
        printf("  argv[%d] = %s\n", i, argv[i]);
    return 0;
}
```

### Quick Start: Hello World (C++)

```cpp
/* hello.cpp */
#include <iostream>
#include <vector>
#include <string>

int main() {
    std::vector<std::string> names = {"anyOS", "libcxx", "C++20"};
    for (const auto& name : names) {
        std::cout << "Hello from " << name << std::endl;
    }
    return 0;
}
```

---

## Limitations and Known Differences

### No Exceptions

Both libc64 and libcxx are compiled with `-fno-exceptions`. Functions that would normally throw in standard C++ call `std::abort()` instead:
- `operator new` aborts on allocation failure (does not throw `std::bad_alloc`)
- `std::optional::value()` aborts when empty
- `std::map::at()` / `std::unordered_map::at()` abort when key not found
- All container allocations abort on failure

### No RTTI

libcxx is compiled with `-fno-rtti`. The following features are unavailable:
- `dynamic_cast`
- `typeid` / `std::type_info`
- Virtual destructors in `shared_ptr` use explicit virtual dispatch rather than RTTI

### No Threading Support

There are no threading primitives: no `<thread>`, `<mutex>`, `<atomic>`, `<condition_variable>`, or `<future>`. The standard library is not thread-safe. If the kernel supports multiple threads in a process, external synchronization is required.

### No Filesystem Library

There is no `<filesystem>`. Use the POSIX-layer functions in `<unistd.h>`, `<dirent.h>`, and `<sys/stat.h>` directly.

### No Locales or Wide Characters

- `setlocale()` always returns `"C"`
- No wide character support (`<wchar.h>`, `<cwchar>`)
- No locale-aware formatting
- `iconv_open()` / `iconv()` are stubs that return error

### No Regular Expressions

The `<regex.h>` functions (`regcomp`, `regexec`, `regfree`) are stubs that always return failure. Use command-line `grep` or implement pattern matching manually.

### No std::function

The `<functional>` header provides function objects (`less`, `greater`, `plus`, etc.), `hash`, and `reference_wrapper`, but does not include `std::function`. Use function pointers or templates for callable abstractions.

### No Dynamic Linking

Both `libc64.a` and `libcxx.a` are static archives only. There are no shared library (`.so`) variants.

### Smart Pointer Limitations

- `weak_ptr` is not implemented (the weak_count field in `shared_ptr`'s control block is reserved for future use)
- `shared_ptr` aliasing constructor is not implemented
- `enable_shared_from_this` is not implemented

### printf Limitations

The `vsnprintf` implementation does not support floating-point format specifiers (`%f`, `%e`, `%g`). For floating-point output, use `std::cout` (via libcxx iostream) or `std::to_string()`.

### Memory Allocator Characteristics

- Arena-based: requests 64 KiB chunks from `sbrk()`, suballocates with a free-list
- `free()` does not return memory to the kernel -- freed blocks are added to an internal free list for reuse
- All allocations are 16-byte aligned (x86_64 ABI requirement)
- Block splitting and first-fit search for freed memory

### Stub Functions (Link-Compatible Only)

Several POSIX functions are provided as stubs for link compatibility but do not perform their standard operation:

| Function | Behavior |
|----------|----------|
| `mmap()` / `munmap()` / `mprotect()` | Return `MAP_FAILED` / `ENOSYS` |
| `link()` | Returns `ENOSYS` (hard links not supported) |
| `tmpfile()` | Returns `NULL` |
| `tcgetattr()` / `tcsetattr()` | Return `-1` |
| `alarm()` | No-op, returns 0 |
| `poll()` / `select()` | Stubs |
| `iconv_open()` / `iconv()` | Return error |
| `regcomp()` / `regexec()` | Return failure |
| `fchmod()` / `chown()` | No-op, return 0 |

### std::string SSO Threshold

The Small String Optimization threshold is 22 characters (+ NUL = 23 bytes stored inline). Strings longer than 22 characters require heap allocation. The `std::string` object is always 32 bytes on x86_64.

### Container Behavior on Allocation Failure

All containers call `std::abort()` on allocation failure. There is no exception-based error recovery path. Use the nothrow `operator new` variants if you need to detect allocation failure before constructing objects.

### Missing Standard Library Components

The following standard C++ library features are **not** provided:

- `<thread>`, `<mutex>`, `<atomic>`, `<condition_variable>`, `<future>` -- no threading
- `<filesystem>` -- use POSIX APIs directly
- `<regex>` -- stubs only
- `<chrono>` -- use `<ctime>` / `<time.h>` directly
- `<exception>`, `<stdexcept>` -- no exceptions
- `<typeinfo>`, `<typeindex>` -- no RTTI
- `<variant>` -- not implemented (use `optional` or manual tagged unions)
- `<any>` -- not implemented
- `<charconv>` -- not implemented (use `snprintf` / `strtol`)
- `<format>` -- not implemented (use `ostringstream` or `snprintf`)
- `<ranges>` -- not implemented
- `<coroutine>` -- not implemented
- `<source_location>` -- not implemented
- `<concepts>` -- not implemented (use SFINAE / `enable_if` / `type_traits`)

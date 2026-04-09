# anyrc — Self-Hosted Rust Compiler for anyOS

anyrc is a native Rust subset compiler that runs within anyOS, enabling self-hosted development. It compiles Rust source code directly to x86_64 machine code without external dependencies like LLVM or Cranelift.

## Overview

- **Language:** Rust subset (see [Supported Features](#supported-rust-subset))
- **Backend:** Custom x86_64 machine code emitter
- **Output:** ELF executables, object files, rlib archives
- **Borrow Checker:** Full NLL-style analysis
- **Targets:** `x86_64-anyos`, `x86_64-anyos-user`, `x86_64-linux`
- **Lines of Code:** ~14,800 (compiler library) + 250 (CLI)

## Project Structure

```
libs/anyrc/           Compiler library (14,812 lines)
├── src/
│   ├── lib.rs        Crate root
│   ├── lexer.rs      Tokenizer with symbol interning (531 lines)
│   ├── parser.rs     Recursive descent + Pratt expression parsing (2,471 lines)
│   ├── ast.rs        AST data structures (394 lines)
│   ├── hir.rs        High-Level IR (desugared AST) (336 lines)
│   ├── hir_lower.rs  AST → HIR lowering (593 lines)
│   ├── macros.rs     macro_rules! expansion + built-in macros (800+ lines)
│   ├── cfg.rs        Conditional compilation (#[cfg()] evaluation) (NEW)
│   ├── resolve.rs    Name resolution (type, value, macro namespaces) (1,132 lines)
│   ├── typeck.rs     Hindley-Milner type inference + trait resolution (1,435 lines)
│   ├── borrowck.rs   NLL borrow checker (235 lines)
│   ├── mir.rs        Mid-Level IR (CFG-based) (162 lines)
│   ├── mir_build.rs  HIR → MIR construction (2,004 lines)
│   ├── mir_opt.rs    MIR optimization passes (273 lines)
│   ├── mono.rs       Monomorphization (294 lines)
│   ├── codegen/
│   │   ├── emit.rs       x86-64 instruction emission (1,609 lines)
│   │   ├── x86asm.rs     Instruction encoding (473 lines)
│   │   └── regalloc.rs   Linear scan register allocator (111 lines)
│   ├── linker/
│   │   ├── elf.rs        ELF64 object/executable generation (641 lines)
│   │   └── link.rs       Symbol resolution, relocation, linker script support
│   ├── driver.rs     Compilation orchestration
│   ├── loader.rs     Module loading + rlib pack/unpack
│   ├── runtime.rs    Runtime stubs (alloc, Vec, memcpy, memset, etc.)
│   ├── diagnostics.rs Error reporting with source spans
│   └── intern.rs     Symbol interning
bin/anyrc/            CLI frontend
└── src/main.rs
libs/anyrc_tests/     Test suite (34 test modules)
```

## Compiler Pipeline

```
Source (.rs)
  → Lexer (tokenization, symbol interning)
  → Parser (recursive descent + Pratt parsing)
  → Macro Expansion (macro_rules!, #[derive], built-in macros)
  → Conditional Compilation (#[cfg()] stripping)
  → Module Resolution (mod foo; → file loading)
  → HIR Lowering (desugar for-loops, if let, ?, +=)
  → Name Resolution (3 namespaces, import fixpoint)
  → Type Checking (HM unification, trait resolution)
  → MIR Construction (CFG, basic blocks, explicit drops)
  → Borrow Checker (NLL lifetime inference, move checking)
  → MIR Optimizations (const prop, DCE, inlining, copy prop)
  → Monomorphization (worklist from main)
  → Codegen (linear scan regalloc, System V ABI)
  → ELF Generation (object files + linking)
```

## CLI Usage

```
anyrc [OPTIONS] <INPUT>

  -o <OUTPUT>              Output file (default: a.out)
  --emit <TYPE>            exe | obj | rlib | mir | hir | asm
  --opt-level <N>          Optimization level (0-3)
  --crate-type <TYPE>      bin | lib | staticlib
  --crate-name <NAME>      Crate name
  --src-dir <DIR>          Source directory for module resolution
  --extern <NAME=PATH>     Link against extern crate .rlib
  --cfg <SPEC>             Conditional compilation flag
  -T <SCRIPT>              Linker script path
  --link-arg <ARG>         Additional linker argument (.o file)
  --env <KEY=VALUE>        Set compile-time environment variable
  --feature <NAME>         Enable feature gate
  --version                Print version
  -h, --help               Print help
```

### Conditional Compilation

anyrc supports `#[cfg(...)]` attributes with the following predicates:

- `#[cfg(name)]` — true if `name` flag is set
- `#[cfg(name = "value")]` — true if `name` equals `value`
- `#[cfg(not(pred))]` — negation
- `#[cfg(any(pred, ...))]` — disjunction
- `#[cfg(all(pred, ...))]` — conjunction

Cfg flags are passed via `--cfg`:

```bash
anyrc --cfg 'target_arch="x86_64"' --cfg 'feature="kunit"' src/main.rs
```

### Linker Script Support

For kernel-level linking with custom memory layouts:

```bash
anyrc -T kernel/link.ld --link-arg kernel/asm/boot.o src/lib.rs -o kernel.elf
```

The linker script parser extracts:
- `ENTRY(symbol)` — entry point symbol
- `. = 0xFFFFFFFF80000000;` — base address

### Built-in Macros

| Macro | Description |
|-------|-------------|
| `println!()` / `eprintln!()` | Console output via SYS_WRITE |
| `format!()` | String formatting |
| `vec![]` | Vector construction |
| `assert!()` / `assert_eq!()` | Assertions (no-op in release) |
| `env!("VAR")` | Compile-time environment variable |
| `option_env!("VAR")` | Optional compile-time env var |
| `include_bytes!("path")` | Compile-time file inclusion (bytes) |
| `include_str!("path")` | Compile-time file inclusion (string) |
| `cfg!(pred)` | Compile-time cfg predicate check |
| `concat!("a", "b")` | Compile-time string concatenation |
| `stringify!(tokens)` | Token stringification |
| `line!()` / `column!()` / `file!()` | Source location |
| `module_path!()` | Module path |
| `compile_error!("msg")` | Compile-time error |

### Runtime Intrinsics

anyrc links the following runtime stubs into executables:

| Symbol | Purpose |
|--------|---------|
| `__anyrc_alloc` | Heap allocation (sbrk-based) |
| `__anyrc_dealloc` | Deallocation (no-op for sbrk) |
| `__anyrc_realloc` | Reallocation (copy-based) |
| `__anyrc_vec_push` | Vector growth + push |
| `__anyrc_vec_pop` | Vector pop |
| `__anyrc_vec_free` | Vector memory reclamation |
| `__anyrc_string_*` | String operations via Vec |
| `__anyrc_println` | Console output |
| `memcpy` | Memory copy (compiler_builtins) |
| `memmove` | Overlapping memory copy |
| `memset` | Memory fill |
| `memcmp` | Memory comparison |
| `bcmp` | Byte comparison |
| `strlen` | String length |

## Supported Rust Subset

**Included:**
- Functions, structs, enums (with data), impl blocks, traits
- Generics with trait bounds and where clauses
- Lifetimes (`'a`, `'static`, elision)
- Pattern matching (`match`, `if let`, `while let`)
- Closures (non-capturing)
- `unsafe` blocks, raw pointers, `as` casts
- `macro_rules!` declarative macros
- `#[derive(...)]` for built-in traits (Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)
- `#[repr(C)]`, `#[repr(C, packed)]`, `#[repr(transparent)]`
- `#[cfg(...)]` conditional compilation with `not`, `any`, `all`
- `#[no_mangle]`, `#![no_std]`, `#![no_main]`
- `#![feature(...)]` (accepted for compatibility)
- Inline assembly (`asm!`)
- Module system (`mod`, `use`, `pub`, `pub(crate)`, `crate`, `super`)
- `const`, `static`, `const fn`
- Operator overloading via traits
- `for` loops, ranges, labeled loops
- Associated constants, array indexing, `.len()`
- `dyn Trait` (trait objects with virtual dispatch)
- Atomic types (`AtomicU32`, `AtomicU64`, `AtomicBool`)
- `extern "C"` blocks and functions

**Not included:**
- `async`/`await`
- Proc macros
- GATs (Generic Associated Types)
- Const generics

## Output Formats

| Format | Flag | Description |
|--------|------|-------------|
| Executable | `--emit exe` | Linked ELF64 binary (default) |
| Object | `--emit obj` | Relocatable ELF64 .o file |
| Rlib | `--emit rlib` | Rust library (ELF + metadata) |
| MIR dump | `--emit mir` | Textual MIR for debugging |
| HIR dump | `--emit hir` | HIR text (stub) |
| Assembly | `--emit asm` | x86-64 assembly (stub) |

### Rlib Format

`.rlib` files are binary archives: `[obj_size:4][obj_bytes][metadata_bytes]`

Metadata contains: crate name, version, exported symbols, dependency list.
Format header: `ARCM` (anyrc metadata).

---

# acargo — Rust Build System for anyOS

acargo is a Cargo-compatible build tool for anyOS that wraps the anyrc compiler, providing dependency resolution, build scripts, feature management, incremental compilation, and project scaffolding.

## Project Structure

```
bin/acargo/src/
├── main.rs           CLI entry point and command dispatch
├── build.rs          Build engine with build script + fingerprint integration
├── manifest.rs       Cargo.toml parsing and manifest data model
├── toml.rs           Custom TOML parser (no external dependencies)
├── resolve.rs        Dependency resolution and topological sorting
├── build_script.rs   Build script (build.rs) execution and output parsing
├── workspace.rs      Workspace discovery and member resolution
├── fingerprint.rs    Incremental compilation via mtime fingerprinting
├── jobs.rs           Parallel job scheduling framework
├── scaffold.rs       Project scaffolding (new/init)
└── fs.rs             File system operations wrapper
```

## CLI Usage

```
acargo <COMMAND> [OPTIONS]

Commands:
  build, b              Compile the current package
  run                   Build and run the binary
  check, c              Check for errors without producing output
  test, t               Build and run tests
  bench                 Build and run benchmarks
  new <name>            Create a new package
  init [dir]            Initialize package in existing directory
  clean                 Remove build artifacts
  fetch                 Download registry dependencies
  update                Update registry dependencies to latest
  search <query>        Search crates.io
  tree                  Display dependency tree
  metadata              Output package metadata as JSON
  doc                   Generate documentation
  help                  Print help

Options:
  --release, -r          Build with optimizations
  --verbose, -v          Show detailed output
  --features, -F <LIST>  Comma-separated features to enable
  --all-features         Enable all features
  --no-default-features  Disable default features
  --target <SPEC>        Target specification
  --jobs, -j <N>         Number of parallel jobs
  --lib                  Create a library project (with new/init)
  --                     Pass remaining args to the binary (with run)
```

## Package Registry (crates.io)

acargo downloads dependencies from crates.io when no `path` is specified:

```toml
[dependencies]
serde = "1.0"                          # Fetched from crates.io
serde_json = { version = "1.0" }       # Also from crates.io
anyos_std = { path = "../../libs/stdlib" }  # Local path (preferred)
```

### How it works

1. **Sparse Index**: acargo fetches crate metadata from `https://index.crates.io/` using the RFC 2789 sparse index protocol
2. **SemVer Resolution**: Finds the newest non-yanked version matching the requirement (`^1.0` → latest `1.x.y`)
3. **Download**: Fetches `.crate` files from `https://static.crates.io/crates/{name}/{name}-{version}.crate`
4. **Extract**: Decompresses gzip tar archives using libzip
5. **Cache**: All files cached in `/System/var/acargo/registry/`
6. **Lock**: Resolved versions written to `Cargo.lock` for reproducible builds

### Version Requirements (Cargo-compatible)

| Syntax | Meaning |
|--------|---------|
| `^1.2.3` or `1.2.3` | >=1.2.3, <2.0.0 (caret, default) |
| `^0.2.3` | >=0.2.3, <0.3.0 |
| `~1.2.3` | >=1.2.3, <1.3.0 (tilde) |
| `=1.2.3` | Exactly 1.2.3 |
| `>=1.0, <2.0` | Explicit range |
| `*` | Any version |

### Commands

```bash
acargo fetch          # Download all registry dependencies
acargo update         # Re-resolve to latest compatible versions
acargo search serde   # Search crates.io
```

### Cache Layout

```
/System/var/acargo/registry/
├── index/            # Sparse index cache (NDJSON per crate)
│   ├── se/rd/serde
│   └── to/ki/tokio
├── cache/            # Downloaded .crate archives
│   ├── serde-1.0.193.crate
│   └── tokio-1.35.0.crate
└── src/              # Extracted source directories
    ├── serde-1.0.193/
    │   ├── Cargo.toml
    │   └── src/
    └── tokio-1.35.0/
```

### Cargo.lock

acargo generates and respects `Cargo.lock` for version pinning:

```toml
version = 3

[[package]]
name = "serde"
version = "1.0.193"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abcdef..."
```

Use `acargo update` to refresh locked versions.

## Feature Resolution

acargo resolves features from `[features]` in Cargo.toml:

```toml
[features]
default = ["logging"]
logging = []
kunit = []
debug_verbose = []
```

```bash
acargo build --features kunit,debug_verbose
acargo build --all-features
acargo build --no-default-features --features kunit
```

Features are translated to `--cfg feature="name"` flags for anyrc.

## Build Scripts

acargo executes `build.rs` files, parsing `cargo:` directives:

| Directive | Effect |
|-----------|--------|
| `cargo:rustc-cfg=FLAG` | Add cfg flag to compilation |
| `cargo:rustc-link-arg=ARG` | Add linker argument |
| `cargo:rustc-link-lib=LIB` | Link against library |
| `cargo:rustc-link-search=PATH` | Add library search path |
| `cargo:rustc-env=KEY=VALUE` | Set compile-time environment variable |
| `cargo:rerun-if-changed=PATH` | Rebuild if file changes |
| `cargo:rerun-if-env-changed=VAR` | Rebuild if env var changes |
| `cargo:warning=MESSAGE` | Print warning during build |

Build scripts receive these environment variables:
- `CARGO_MANIFEST_DIR` — directory containing Cargo.toml
- `OUT_DIR` — output directory for build script artifacts
- `TARGET` — target triple (e.g. `x86_64-anyos`)
- `HOST` — host triple
- `PROFILE` — `debug` or `release`
- `OPT_LEVEL` — optimization level

## Incremental Compilation

acargo uses mtime-based fingerprinting to skip unchanged crates:

1. Before compiling, checks if output artifact exists and is newer than all source files
2. Computes a hash of compile options (opt level, cfg flags, features)
3. Compares against stored fingerprint
4. If fresh, skips compilation and reports "Fresh" status

Fingerprint files are stored in `target/{profile}/.fingerprint/`.

## Conditional Compilation

acargo automatically generates cfg flags:
- `target_arch="x86_64"` (or `"aarch64"` with `--target`)
- `target_pointer_width="64"`
- `target_endian="little"`
- `target_os="anyos"`
- `feature="name"` for each active feature

## Workspace Support

acargo discovers workspace roots by walking up from the current directory:

```toml
[workspace]
members = ["kernel", "libs/*", "bin/*"]
exclude = ["third_party"]
```

Member directories are resolved via glob patterns (`*` expansion).

## Dependency Tree

```bash
acargo tree
```

Output:
```
my_project v0.1.0
├── anyos_std v0.1.0
│   └── libheap v0.1.0
└── libanyui_client v0.1.0
    └── dynlink v0.1.0
```

## Building the Kernel

To build the anyOS kernel with acargo from within anyOS:

```bash
cd /System/src/kernel
acargo build --release --target x86_64-anyos \
  --features debug_verbose
```

The kernel build process:
1. acargo resolves the kernel's Cargo.toml (no external dependencies)
2. Executes `build.rs` which emits:
   - `cargo:rustc-link-arg=<asm_object>` for assembly objects
   - `cargo:rustc-link-arg=-T<linker_script>` for the linker script
   - `cargo:rustc-env=ANYOS_VERSION=<version>`
3. Compiles all kernel modules with cfg flags for target architecture
4. Links with the linker script and assembly objects

## Self-Hosting

anyrc is designed to eventually compile itself and the anyOS kernel within anyOS:

| Milestone | Description | Status |
|-----------|-------------|--------|
| M1 | Compile "Hello World" to ELF | Done |
| M2 | Structs, enums, generics, traits | Done |
| M3 | Compile `core` library | In progress |
| M4 | Compile `alloc` (Vec, Box, String) | In progress |
| M5 | Self-hosting on Linux | Planned |
| M6 | Self-hosting on anyOS | Planned |
| M7 | Compile anyOS kernel on anyOS | Planned |

## Syntax Highlighting

anyrc source files use the `.rs` extension. The anyCode editor provides full Rust syntax highlighting via `syntax/rust.syn`, including keywords, types, builtins, comments, strings, and number literals.

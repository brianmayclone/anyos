# anyrc — Rust Subset Compiler for anyOS

**Date:** 2026-03-09
**Status:** Design Approved

## Goal

A self-hosting Rust subset compiler (`anyrc`) that can compile the anyOS kernel and userspace within anyOS itself. Native x86_64 codegen, full borrow checker, dual-target (Linux host + anyOS).

## Decisions

| Decision | Choice |
|----------|--------|
| Approach | Rust subset compiler (not full rustc port) |
| Backend | Own x86_64 machine code emitter (no LLVM/Cranelift/TCC) |
| Borrow Checker | Full NLL-style borrow checker on MIR |
| Target | Dual: Linux (dev/test) + anyOS (self-hosting) |
| core/alloc | Compile official Rust source with anyrc |
| Architecture | Multi-pass pipeline: Lexer → Parser → AST → HIR → MIR → Codegen → ELF |

## Supported Rust Subset

**In scope:**
- `fn`, `struct`, `enum`, `impl`, `trait`, `type` aliases
- Generics with trait bounds, `where` clauses
- Lifetimes (`'a`, `'static`, elision rules)
- Pattern matching (`match`, `if let`, `while let`)
- Closures (`|x| x + 1`, `move ||`)
- `unsafe` blocks + raw pointers
- `macro_rules!` declarative macros
- `#[derive(...)]` for built-in traits (Copy, Clone, Debug, etc.)
- `#[cfg(...)]`, `#[allow(...)]`, `#[repr(...)]`
- Inline assembly (`asm!`, `global_asm!`)
- `core` + `alloc` (compiled from official Rust source)
- Module system (`mod`, `use`, `pub`, `crate`)
- Operator overloading via traits
- `const`, `static`, `const fn` (basic)

**Out of scope:**
- `async`/`await`
- Proc macros (only `macro_rules!` + built-in derives)
- GATs (Generic Associated Types)
- `dyn Trait` (only static dispatch via generics / `impl Trait`)
- Trait objects with vtables

## Project Structure

```
libs/anyrc/                  ← Compiler library
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── lexer/               ← Tokenizer with symbol interning
    ├── parser/              ← Recursive descent + Pratt parsing for expressions
    ├── ast/                 ← AST data structures
    ├── hir/                 ← High-Level IR (desugared AST)
    ├── resolve/             ← Name resolution (3 namespaces: type, value, macro)
    ├── typeck/              ← Hindley-Milner + traits type inference/checking
    ├── borrowck/            ← NLL borrow checker on MIR
    ├── mir/                 ← Mid-Level IR (CFG-based)
    ├── mir_opt/             ← MIR optimization passes
    ├── codegen/             ← x86_64 machine code generation
    ├── linker/              ← ELF object + executable generation
    ├── driver/              ← Compilation orchestration + I/O abstraction
    └── diagnostics/         ← Error reporting with source spans
bin/anyrc/                   ← CLI frontend
├── Cargo.toml
└── src/main.rs
```

## Compiler Pipeline

```
Source (.rs)
  ↓
Lexer → Token stream (symbol interning for identifiers)
  ↓
Parser → AST (recursive descent + Pratt parsing)
  ↓
Macro Expansion (macro_rules! pattern matching, iterative until fixpoint)
  ↓
AST → HIR (desugar: for→loop, ?→match, if let→match, +=→trait call, #[derive])
  ↓
Name Resolution (3 namespaces, fixpoint for imports, DefId assignment)
  ↓
Type Inference + Checking (HM unification, trait resolution, method resolution)
  ↓
HIR → MIR (control flow graph, basic blocks, explicit drops)
  ↓
Borrow Checker (NLL: lifetime inference, borrow checking, move checking)
  ↓
MIR Optimizations (const prop, simplify CFG, DCE, inlining, copy prop, inst combine)
  ↓
Monomorphization (worklist from main(), instantiate generics per concrete type)
  ↓
Codegen: MIR → x86_64 (linear scan register alloc, System V ABI)
  ↓
ELF Object (.o) with .anyrc_meta section for crate metadata
  ↓
Linker → ELF executable (symbol resolution, section merging, relocations)
```

## Phase Details

### Lexer

- Single-pass tokenizer, stateless
- Symbol interning: all identifiers/lifetimes deduplicated in intern table, compared by integer
- Token carries `Span` (byte offset start + end) for diagnostics
- Handles raw strings (`r#"..."#`), byte strings, char/string escape sequences

### Parser

- Recursive descent for items, statements, types, patterns
- Pratt parsing for expressions (operator precedence)
- Macro calls recognized and stored as token streams for later expansion
- Produces full AST with spans on every node

### HIR Desugaring

| AST | HIR |
|-----|-----|
| `for x in iter { ... }` | `loop { match iter.next() { Some(x) => ..., None => break } }` |
| `x?` | `match x { Ok(v) => v, Err(e) => return Err(From::from(e)) }` |
| `if let P = e { ... }` | `match e { P => { ... }, _ => {} }` |
| `x += 1` | `x = Add::add(x, 1)` |
| `#[derive(Clone)]` | Generated `impl Clone for T { ... }` |

Each HIR node gets a unique `HirId`.

### Name Resolution

- Three namespaces: Type (structs, enums, traits), Value (fns, consts, locals), Macro
- Fixpoint algorithm for import resolution (glob imports, re-exports, cycles)
- Every `Path` resolved to a `DefId`

### Type Inference + Checking

- Hindley-Milner with extensions for traits and lifetimes
- Constraint collection → unification → trait resolution
- Method resolution: self type → direct impls → trait impls → auto-deref chain
- Type representation covers all Rust primitives, ADTs, references, raw pointers, fn pointers, tuples, arrays, slices

### MIR

CFG-based IR with:
- `BasicBlock`: statements + terminator
- `Statement`: `Assign(Place, Rvalue)`, `StorageLive`, `StorageDead`
- `Rvalue`: `Use`, `Ref`, `BinaryOp`, `UnaryOp`, `Cast`, `Aggregate`, `Discriminant`, `Len`
- `Operand`: `Copy(Place)`, `Move(Place)`, `Constant`
- `Place`: local + projections (Field, Index, Deref, Downcast)
- `Terminator`: `Goto`, `SwitchInt`, `Call`, `Return`, `Unreachable`, `Drop`

### Borrow Checker (NLL)

**Phase 1 — Lifetime Inference:**
- Each reference gets a `RegionVar` (set of CFG locations where it's live)
- Constraints: `'a ⊆ 'b` at specific locations
- Propagation over CFG until fixpoint

**Phase 2 — Borrow Checking:**
At each CFG point, check active borrows:
- Move of borrowed place → error
- Write to shared-borrowed place → error
- `&mut` while any other borrow active → error
- `&` while mutable borrow active → error

**Phase 3 — Move Checking:**
Dataflow analysis tracking initialized vs. moved state:
- Use after move → error
- Partial moves tracked per field
- Conditional moves → maybe-uninit → error on use

### Codegen

- MIR block-by-block translation to x86_64
- Linear scan register allocation
- System V AMD64 ABI (args: RDI, RSI, RDX, RCX, R8, R9; return: RAX)
- Own x86_64 assembler module (REX prefix, ModR/M, SIB encoding)
- SSE instructions for float operations
- Monomorphization via worklist algorithm from entry point

### ELF Generation

**Object files (.o):**
- Sections: `.text`, `.rodata`, `.data`, `.bss`, `.symtab`, `.strtab`, `.rela.text`, `.anyrc_meta`
- Standard ELF64 relocations: `R_X86_64_PC32`, `R_X86_64_PLT32`, `R_X86_64_64`

**Linker:**
1. Symbol resolution (match undefined with definitions)
2. Section merging
3. Relocation patching
4. Program headers (PT_LOAD with RX, RW, R permissions)
5. Entry point: `_start` → `main()`
6. Supports linker scripts for kernel (higher-half addresses)

**Crate metadata** (`.anyrc_meta` section):
- Exported item signatures, struct/enum/trait definitions
- Generic item HIR (for monomorphization in consumers)
- Impl blocks, macro definitions, lang items
- Binary format (compact, fast to parse)

## core/alloc Bootstrapping

1. Ship official Rust `library/core/` and `library/alloc/` source in sysroot (`/usr/lib/anyrc/src/`)
2. anyrc compiles them with `--edition 2021` and appropriate `#[cfg]` flags
3. `#[rustc_...]` attributes ignored or treated as no-op
4. `#[lang = "..."]` items recognized explicitly (sized, copy, drop, fn, etc.)
5. `compiler_builtins` intrinsics (`__muloti4`, `memcpy`, `memset`) provided by `compiler_rt.rs`
6. Built-in derives hardcoded (not proc-macro based)
7. Minimal const evaluator for `const fn` in core (arithmetic, struct construction, simple branches)

## Dual-Target I/O

```rust
pub trait FileSystem {
    fn read_file(&self, path: &str) -> Result<Vec<u8>, Error>;
    fn write_file(&self, path: &str, data: &[u8]) -> Result<(), Error>;
    fn file_exists(&self, path: &str) -> bool;
}
// Linux: std::fs
// anyOS: anyos_std::fs (syscalls)
// Selected via #[cfg(target_os)]
```

## CLI Interface

```
anyrc [OPTIONS] <INPUT>

  -o <OUTPUT>           Output file (default: a.out)
  --target <TARGET>     x86_64-anyos | x86_64-anyos-user | x86_64-linux
  --emit <TYPE>         asm | mir | hir | obj | exe (default: exe)
  --edition <YEAR>      2021 | 2024
  --opt-level <N>       0 | 1 | 2
  --cfg <SPEC>          Conditional compilation flag
  --crate-type <TYPE>   bin | lib | staticlib
  --crate-name <NAME>   Crate name
  --extern <NAME=PATH>  External crate dependency (.o file)
  --sysroot <PATH>      Path to core/alloc libraries
  -L <PATH>             Library search path
  --color <WHEN>        auto | always | never
  --error-format <FMT>  human | json
  --dump-mir            Dump MIR to stdout
  --dump-hir            Dump HIR to stdout
```

## Self-Hosting Milestones

| # | Milestone | Description |
|---|-----------|-------------|
| M1 | First ELF | anyrc compiles "Hello World" (fn main, syscall write) |
| M2 | Type System | Compiles programs with structs, enums, generics, traits |
| M3 | core subset | Compiles core library (lang items, intrinsics) |
| M4 | alloc | Compiles alloc (Vec, Box, String) |
| M5 | Self-hosting on Linux | anyrc compiles itself on Linux host |
| M6 | Self-hosting on anyOS | anyrc runs on anyOS and compiles itself there |
| M7 | Full circle | anyrc compiles the anyOS kernel on anyOS |

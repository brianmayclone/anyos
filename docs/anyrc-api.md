# anyrc — Self-Hosted Rust Compiler for anyOS

anyrc is a native Rust subset compiler that runs within anyOS, enabling self-hosted development. It compiles Rust source code directly to x86_64 machine code without external dependencies like LLVM or Cranelift.

## Overview

- **Language:** Rust subset (see [Supported Features](#supported-rust-subset))
- **Backend:** Custom x86_64 machine code emitter
- **Output:** ELF executables and object files
- **Borrow Checker:** Full NLL-style analysis
- **Targets:** `x86_64-anyos`, `x86_64-anyos-user`, `x86_64-linux`

## Project Structure

```
libs/anyrc/           Compiler library
├── src/
│   ├── lib.rs        Crate root
│   ├── lexer.rs      Tokenizer with symbol interning
│   ├── parser.rs     Recursive descent + Pratt expression parsing
│   ├── ast.rs        AST data structures
│   ├── hir.rs        High-Level IR (desugared AST)
│   ├── hir_lower.rs  AST → HIR lowering
│   ├── macros.rs     macro_rules! expansion
│   ├── resolve.rs    Name resolution (type, value, macro namespaces)
│   ├── typeck.rs     Hindley-Milner type inference + trait resolution
│   ├── borrowck.rs   NLL borrow checker
│   ├── mir.rs        Mid-Level IR (CFG-based)
│   ├── mir_build.rs  HIR → MIR construction
│   ├── mir_opt.rs    MIR optimization passes
│   ├── mono.rs       Monomorphization
│   ├── codegen/      x86_64 code generation + register allocator
│   ├── linker/       ELF object/executable generation + linking
│   ├── driver.rs     Compilation orchestration
│   ├── diagnostics.rs Error reporting with source spans
│   └── intern.rs     Symbol interning
bin/anyrc/            CLI frontend
└── src/main.rs
libs/anyrc_tests/     Test suite (28 test modules)
```

## Compiler Pipeline

```
Source (.rs)
  → Lexer (tokenization, symbol interning)
  → Parser (recursive descent + Pratt parsing)
  → Macro Expansion (macro_rules!, #[derive])
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
  --dump-mir            Dump MIR to stdout
  --dump-hir            Dump HIR to stdout
```

## Supported Rust Subset

**Included:**
- Functions, structs, enums (with data), impl blocks, traits
- Generics with trait bounds and where clauses
- Lifetimes (`'a`, `'static`, elision)
- Pattern matching (`match`, `if let`, `while let`)
- Closures (non-capturing)
- `unsafe` blocks, raw pointers, `as` casts
- `macro_rules!` declarative macros
- `#[derive(...)]` for built-in traits (Copy, Clone, Debug, etc.)
- `#[repr(...)]`, `#[cfg(...)]`, `#[no_mangle]`, `#![no_std]`
- Inline assembly (`asm!`)
- Module system (`mod`, `use`, `pub`, `crate`, `super`)
- `const`, `static`, `const fn`
- Operator overloading via traits
- `for` loops, ranges
- Associated constants, array indexing, `.len()`
- `dyn Trait` (trait objects with virtual dispatch)
- Atomic types

**Not included:**
- `async`/`await`
- Proc macros
- GATs (Generic Associated Types)

## Self-Hosting

anyrc is designed to eventually compile itself and the anyOS kernel within anyOS:

| Milestone | Description |
|-----------|-------------|
| M1 | Compile "Hello World" to ELF |
| M2 | Structs, enums, generics, traits |
| M3 | Compile `core` library |
| M4 | Compile `alloc` (Vec, Box, String) |
| M5 | Self-hosting on Linux |
| M6 | Self-hosting on anyOS |
| M7 | Compile anyOS kernel on anyOS |

## Syntax Highlighting

anyrc source files use the `.rs` extension. The anyCode editor provides full Rust syntax highlighting via `syntax/rust.syn`, including keywords, types, builtins, comments, strings, and number literals.

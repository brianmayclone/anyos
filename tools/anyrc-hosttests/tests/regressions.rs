use anyrc::diagnostics::SourceMap;
use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind};
use anyrc::cfg::CfgContext;
use anyrc::loader;
use anyrc::hir::HirStmt;
use anyrc::hir::HirExprKind;
use anyrc::hir_lower::LoweringContext;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::parser::Parser;
use std::fs;
use std::path::{Path, PathBuf};

fn compile_ok(name: &str, src: &str) {
    let output = format!("/tmp/{}_anyrc_test.o", name);
    let opts = CompileOptions {
        input: format!("{}.rs", name),
        output,
        emit: EmitKind::Obj,
        opt_level: 0,
        crate_type: CrateType::Lib,
        crate_name: Some(name.to_string()),
        src_dir: None,
        extern_crates: vec![],
        cfg_flags: vec![],
        linker_script: None,
        link_args: vec![],
        env_vars: vec![],
        features: vec![],
    };

    if let Err(errors) = compile(src, &opts.input, &opts) {
        let source_map = SourceMap::new(opts.input.clone(), src.to_string());
        let rendered: Vec<String> = errors.iter().map(|e| e.render(&source_map)).collect();
        panic!(
            "expected snippet `{}` to compile, but got errors:\n{}",
            name,
            rendered.join("\n")
        );
    }
}

fn parse_file_ok(path: &str) {
    let repo_root = repo_root();
    let full_path = repo_root.join(path);
    let src = fs::read_to_string(&full_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", full_path.display(), e));
    let mut interner = Interner::new();
    let mut parser = Parser::new(&src, &mut interner);
    let _ = parser.parse_crate();
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf()
}

fn anyos_cfg_flags() -> Vec<String> {
    vec![
        String::from("target_arch=\"x86_64\""),
        String::from("target_pointer_width=\"64\""),
        String::from("target_endian=\"little\""),
        String::from("target_os=\"anyos\""),
    ]
}

fn compile_repo_rlib(
    crate_name: &str,
    rel_src: &str,
    src_dir: &str,
    extern_crates: Vec<anyrc::driver::ExternCrateSpec>,
) -> loader::CrateMetadata {
    let repo_root = repo_root();
    let input_path = repo_root.join(rel_src);
    let src = fs::read_to_string(&input_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", input_path.display(), e));
    let output = format!("/tmp/{}_anyrc_test.rlib", crate_name);
    let opts = CompileOptions {
        input: input_path.display().to_string(),
        output: output.clone(),
        emit: EmitKind::Rlib,
        opt_level: 0,
        crate_type: CrateType::Lib,
        crate_name: Some(crate_name.to_string()),
        src_dir: Some(repo_root.join(src_dir).display().to_string()),
        extern_crates,
        cfg_flags: anyos_cfg_flags(),
        linker_script: None,
        link_args: vec![],
        env_vars: vec![],
        features: vec![],
    };

    let bytes = match compile(&src, &opts.input, &opts) {
        Ok(bytes) => bytes,
        Err(errors) => {
            let source_map = SourceMap::new(opts.input.clone(), src);
            let rendered: Vec<String> = errors.iter().map(|e| e.render(&source_map)).collect();
            panic!(
                "expected repo crate `{}` to compile, but got errors:\n{}",
                crate_name,
                rendered.join("\n")
            );
        }
    };

    fs::write(&output, &bytes).expect("write rlib output");
    let (_, meta) = loader::unpack_rlib(&bytes).expect("unpack rlib metadata");
    meta
}

fn load_repo_hir(rel_src: &str, src_dir: &str) -> (anyrc::hir::HirCrate, Interner) {
    let repo_root = repo_root();
    let input_path = repo_root.join(rel_src);
    let src = fs::read_to_string(&input_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", input_path.display(), e));
    let mut interner = Interner::new();
    let mut parser = Parser::new(&src, &mut interner);
    let mut krate = parser.parse_crate();
    let cfg_ctx = CfgContext::from_flags(&anyos_cfg_flags());
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    expand_macros(&mut krate, &mut interner);
    let loader = anyrc::loader::OsFileLoader;
    let src_dir = repo_root.join(src_dir);
    let _ = anyrc::loader::resolve_modules(
        &mut krate,
        src_dir.to_str().expect("src dir"),
        &mut interner,
        &loader,
    );
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    let mut lower = LoweringContext::new(&mut interner);
    let hir = lower.lower_crate(&krate);
    (hir, interner)
}

#[test]
fn raw_ptr_field_access_after_deref_compiles() {
    compile_ok(
        "raw_ptr_field_access_after_deref",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn step(curr: *mut FreeBlock) -> *mut FreeBlock {
            (*curr).next
        }
        "#,
    );
}

#[test]
fn anyos_std_public_interface_includes_runtime_modules() {
    let libheap_meta = compile_repo_rlib("libheap", "libs/libheap/src/lib.rs", "libs/libheap/src", vec![]);
    assert!(
        libheap_meta.interface_source.contains("use core::alloc::Layout;"),
        "libheap interface lost private imports needed by exported signatures:\n{}",
        libheap_meta.interface_source
    );

    let libheap_rlib = String::from("/tmp/libheap_anyrc_test.rlib");
    let anyos_std_meta = compile_repo_rlib(
        "anyos_std",
        "libs/stdlib/src/lib.rs",
        "libs/stdlib/src",
        vec![anyrc::driver::ExternCrateSpec {
            name: String::from("libheap"),
            rlib_path: libheap_rlib,
        }],
    );

    for needle in [
        "pub mod fs {",
        "pub fn open(",
        "pub const O_WRITE: u32",
        "pub mod env {",
        "pub fn set(",
        "pub fn get(",
        "pub mod shell {",
        "pub struct Redirect",
        "pub fn parse_redirects(",
    ] {
        assert!(
            anyos_std_meta.interface_source.contains(needle),
            "anyos_std interface missing `{}`:\n{}",
            needle,
            anyos_std_meta.interface_source
        );
    }
}

#[test]
fn libfont_client_compiles_with_builtin_dll_exports_and_public_interface() {
    let dynlink_rlib = String::from("/tmp/dynlink_anyrc_test.rlib");
    let _dynlink_meta = compile_repo_rlib("dynlink", "libs/dynlink/src/lib.rs", "libs/dynlink/src", vec![]);
    let meta = compile_repo_rlib(
        "libfont_client",
        "libs/libfont_client/src/lib.rs",
        "libs/libfont_client/src",
        vec![anyrc::driver::ExternCrateSpec {
            name: String::from("dynlink"),
            rlib_path: dynlink_rlib,
        }],
    );

    assert!(
        !meta.interface_source.contains("Elf64Sym"),
        "libfont_client interface leaked dynlink internals:\n{}",
        meta.interface_source
    );
    for needle in [
        "pub fn init() -> bool;",
        "pub fn load(arg0: &str) -> Option<u32>;",
        "pub fn measure(arg0: u32, arg1: u16, arg2: &str) -> (u32, u32);",
    ] {
        assert!(
            meta.interface_source.contains(needle),
            "libfont_client interface missing `{}`:\n{}",
            needle,
            meta.interface_source
        );
    }
}

#[test]
fn parser_accepts_wrapped_libheap_interface_source() {
    let meta = compile_repo_rlib("libheap", "libs/libheap/src/lib.rs", "libs/libheap/src", vec![]);
    let src = format!("mod libheap {{\n{}\n}}", meta.interface_source);
    let mut interner = Interner::new();
    let mut parser = Parser::new(&src, &mut interner);
    let _ = parser.parse_crate();
}

#[test]
fn parser_accepts_stdlib_source_files() {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("read stdlib dir") {
            let entry = entry.expect("stdlib dir entry");
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().and_then(|name| name.to_str()) == Some("host") {
                    continue;
                }
                walk(&path, files);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    walk(&repo_root().join("libs/stdlib/src"), &mut files);
    files.sort();

    for path in files {
        let src = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
        let mut interner = Interner::new();
        let mut parser = Parser::new(&src, &mut interner);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parser.parse_crate()))
            .unwrap_or_else(|_| panic!("parser failed for {}", path.display()));
    }
}

#[test]
fn anyos_std_process_module_hir_contains_runtime_api() {
    let (hir, interner) = load_repo_hir("libs/stdlib/src/lib.rs", "libs/stdlib/src");
    let process_mod = hir.items.iter().find_map(|item| match &item.kind {
        anyrc::hir::HirItemKind::Mod(m) if interner.resolve(m.name) == "process" => Some(m),
        _ => None,
    }).expect("process module");
    let items = process_mod.items.as_ref().expect("process module items");
    let item_names: Vec<String> = items.iter().map(|item| match &item.kind {
        anyrc::hir::HirItemKind::Fn(f) => format!("fn {}", interner.resolve(f.name)),
        anyrc::hir::HirItemKind::Use(_) => String::from("use"),
        anyrc::hir::HirItemKind::ExternBlock(_) => String::from("extern"),
        anyrc::hir::HirItemKind::Struct(s) => format!("struct {}", interner.resolve(s.name)),
        anyrc::hir::HirItemKind::Const(c) => format!("const {}", interner.resolve(c.name)),
        anyrc::hir::HirItemKind::Static(s) => format!("static {}", interner.resolve(s.name)),
        anyrc::hir::HirItemKind::TypeAlias(t) => format!("type {}", interner.resolve(t.name)),
        anyrc::hir::HirItemKind::Trait(t) => format!("trait {}", interner.resolve(t.name)),
        anyrc::hir::HirItemKind::Enum(e) => format!("enum {}", interner.resolve(e.name)),
        anyrc::hir::HirItemKind::Impl(_) => String::from("impl"),
        anyrc::hir::HirItemKind::Mod(m) => format!("mod {}", interner.resolve(m.name)),
    }).collect();
    for required in ["exit", "yield_cpu", "sbrk", "mmap", "munmap"] {
        assert!(
            items.iter().any(|item| matches!(&item.kind, anyrc::hir::HirItemKind::Fn(f) if interner.resolve(f.name) == required)),
            "process module missing `{}` in HIR; items: {:?}",
            required,
            item_names,
        );
    }
}

#[test]
fn byte_char_literals_compile_as_u8_scalars() {
    compile_ok(
        "byte_char_literals_compile_as_u8_scalars",
        r#"
        fn slash() -> u8 {
            b'/'
        }

        fn newline() -> u8 {
            b'\n'
        }
        "#,
    );
}

#[test]
fn byte_char_literals_in_conditions_compile() {
    compile_ok(
        "byte_char_literals_in_conditions_compile",
        r#"
        fn is_digit(b: u8) -> bool {
            b >= b'0' && b <= b'9'
        }
        "#,
    );
}

#[test]
fn implicit_core_prelude_types_and_variants_compile() {
    compile_ok(
        "implicit_core_prelude_types_and_variants_compile",
        r#"
        fn maybe(flag: bool) -> Option<u8> {
            if flag { Some(1) } else { None }
        }

        fn resulty(flag: bool) -> Result<u8, u8> {
            if flag { Ok(7) } else { Err(9) }
        }
        "#,
    );
}

#[test]
fn println_and_exit_intrinsics_compile() {
    compile_ok(
        "println_and_exit_intrinsics_compile",
        r#"
        fn main() {
            __anyrc_println("hello");
            exit(0);
        }
        "#,
    );
}

#[test]
fn raw_ptr_is_null_in_while_condition_compiles() {
    compile_ok(
        "raw_ptr_is_null_in_while_condition",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn walk(mut curr: *mut FreeBlock) {
            while !curr.is_null() {
                curr = (*curr).next;
            }
        }
        "#,
    );
}

#[test]
fn raw_ptr_deref_to_raw_ptr_binding_compiles() {
    compile_ok(
        "raw_ptr_deref_to_raw_ptr_binding",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn bind(free_list: *mut *mut FreeBlock) {
            let curr = *free_list;
            if !curr.is_null() {}
        }
        "#,
    );
}

#[test]
fn raw_ptr_deref_binding_with_compound_condition_compiles() {
    compile_ok(
        "raw_ptr_deref_binding_with_compound_condition",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn bind(free_list: *mut *mut FreeBlock, block: *mut FreeBlock) {
            let curr = *free_list;
            if !curr.is_null() && (curr as usize) < (block as usize) {}
        }
        "#,
    );
}

#[test]
fn libheap_free_list_insert_loop_compiles() {
    compile_ok(
        "libheap_free_list_insert_loop",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn free_list_dealloc(free_list: *mut *mut FreeBlock, ptr: *mut u8, size: usize) {
            if ptr.is_null() { return; }

            let block = ptr as *mut FreeBlock;
            (*block).size = size;

            let mut prev: *mut FreeBlock = core::ptr::null_mut();
            let mut curr = *free_list;
            while !curr.is_null() && (curr as usize) < (block as usize) {
                prev = curr;
                curr = (*curr).next;
            }

            (*block).next = curr;
            if prev.is_null() { *free_list = block; } else { (*prev).next = block; }
        }
        "#,
    );
}

#[test]
fn loop_with_body_assignment_compiles() {
    compile_ok(
        "loop_with_body_assignment",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn run(free_list: *mut *mut FreeBlock, block: *mut FreeBlock) {
            let mut curr = *free_list;
            while !curr.is_null() && (curr as usize) < (block as usize) {
                curr = (*curr).next;
            }
        }
        "#,
    );
}

#[test]
fn loop_with_prev_and_post_loop_store_compiles() {
    compile_ok(
        "loop_with_prev_and_post_loop_store",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn run(free_list: *mut *mut FreeBlock, block: *mut FreeBlock) {
            let mut prev: *mut FreeBlock = core::ptr::null_mut();
            let mut curr = *free_list;
            while !curr.is_null() && (curr as usize) < (block as usize) {
                prev = curr;
                curr = (*curr).next;
            }
            (*block).next = curr;
            if prev.is_null() { *free_list = block; } else { (*prev).next = block; }
        }
        "#,
    );
}

#[test]
fn anyos_std_args_parse_fields_and_methods_compile() {
    compile_ok(
        "anyos_std_args_parse_fields_and_methods",
        r#"
        fn main() {
            let mut buf = [0u8; 256];
            let raw = anyos_std::process::args(&mut buf);
            let args = anyos_std::args::parse(raw, b"n");

            if args.pos_count > 0 {
                let _first = args.positional[0];
            }

            let _ = args.has(b'h');
            let _ = args.opt(b'n');
            let _ = args.opt_u32(b'n', 10);
            let _ = args.first_or("");
            let _ = args.pos(0);
        }
        "#,
    );
}

#[test]
fn integer_indexing_and_from_le_bytes_compile() {
    compile_ok(
        "integer_indexing_and_from_le_bytes",
        r#"
        fn read_u32(buf: &[u8; 4]) -> u32 {
            u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]])
        }

        fn pick(slice: &[u8]) -> u8 {
            slice[0]
        }
        "#,
    );
}

#[test]
fn parser_accepts_struct_literal_shorthand_and_match_arms_without_commas() {
    compile_ok(
        "parser_accepts_struct_literal_shorthand_and_match_arms_without_commas",
        r#"
        struct Pair { left: u32, right: u32 }

        fn build(left: u32, right: u32) -> Pair {
            Pair { left, right }
        }

        fn label(x: u32) -> &'static str {
            match x {
                0 => { "zero" }
                _ => { "other" }
            }
        }
        "#,
    );
}

#[test]
fn parser_accepts_open_ranges_and_integer_suffixes() {
    compile_ok(
        "parser_accepts_open_ranges_and_integer_suffixes",
        r#"
        fn main() {
            let x = 0usize;
            let s = "hello";
            let _ = &s[x..];
            let _ = &s[..x];
        }
        "#,
    );
}

#[test]
fn str_and_slice_methods_compile() {
    compile_ok(
        "str_and_slice_methods_compile",
        r#"
        fn main() {
            let s = "hello world";
            let bytes = s.as_bytes();
            let _ = bytes[0];
            let _ = s.contains("world");
            let _ = s.starts_with("he");
            let _ = s.ends_with("ld");
            let _ = s.find('o');
            let _ = s.is_empty();

            let mut buf = [0u8; 16];
            buf[..5].copy_from_slice(&bytes[..5]);

            for part in s.split(' ') {
                let _ = part.as_bytes()[0];
            }
            for b in s.bytes() {
                let _ = b;
            }
        }
        "#,
    );
}

#[test]
fn parser_accepts_or_patterns_ref_patterns_and_labels() {
    compile_ok(
        "parser_accepts_or_patterns_ref_patterns_and_labels",
        r#"
        fn main(xs: &[u8]) {
            'outer: loop {
                for &b in xs {
                    match b {
                        b'a' | b'b' => break 'outer,
                        _ => {}
                    }
                }
                break;
            }
        }
        "#,
    );
}

#[test]
fn match_arm_guard_with_andand_compiles() {
    compile_ok(
        "match_arm_guard_with_andand",
        r#"
        enum Key { Char(u8), Escape }

        fn handle(key: Key) -> bool {
            match key {
                Key::Char(c) if c >= 32 && c < 127 => true,
                _ => false,
            }
        }
        "#,
    );
}

#[test]
fn matches_macro_in_if_condition_compiles() {
    compile_ok(
        "matches_macro_in_if_condition",
        r#"
        fn main() {
            let first = "ccargo";
            if matches!(first, "ccargo" | "cargo" | "acargo") {
                exit(0);
            }
        }
        "#,
    );
}

#[test]
fn typed_vec_collect_from_split_whitespace_compiles() {
    compile_ok(
        "typed_vec_collect_from_split_whitespace",
        r#"
        fn main() {
            let raw = "a b";
            let _args: Vec<&str> = raw.split_whitespace().collect();
        }
        "#,
    );
}

#[test]
fn parser_accepts_acargo_main_file() {
    parse_file_ok("bin/acargo/src/main.rs");
}

#[test]
fn parser_accepts_ac_main_file() {
    parse_file_ok("bin/ac/src/main.rs");
}

#[test]
fn parser_accepts_open_main_file() {
    parse_file_ok("bin/open/src/main.rs");
}

#[test]
fn string_range_indexing_inside_split_loop_compiles() {
    compile_ok(
        "string_range_indexing_inside_split_loop",
        r#"
        fn parse_u16(_: &str) -> Option<u16> { Some(0) }

        fn main() {
            let data = "12:group\n";
            for line in data.split('\n') {
                if line.is_empty() {
                    continue;
                }
                if let Some(colon) = line.find(':') {
                    let _a = &line[..colon];
                    let _b = &line[colon + 1..];
                    let _ = parse_u16(&line[..colon]);
                }
            }
        }
        "#,
    );
}

#[test]
fn primitive_char_and_float_items_compile() {
    compile_ok(
        "primitive_char_and_float_items",
        r#"
        fn decode(n: u32) -> Option<char> {
            char::from_u32(n)
        }

        fn clamp_hi(x: f64) -> f64 {
            if x > 700.0 { f64::INFINITY } else { f64::NEG_INFINITY }
        }

        fn nan32() -> f32 {
            f32::NAN
        }
        "#,
    );
}

#[test]
fn closure_ref_pattern_and_tuple_field_body_parse() {
    compile_ok(
        "closure_ref_pattern_and_tuple_field_body",
        r#"
        fn find_zero(xs: &[u8]) -> bool {
            xs.iter().position(|&b| b == 0).is_some()
        }
        "#,
    );

    let src = r#"
        fn has_pair(xs: &[(u32, u32)], needle: u32) -> bool {
            xs.iter().any(|e| e.0 == needle)
        }
    "#;
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let _ = parser.parse_crate();
}

#[test]
fn unit_structs_impl_trait_and_extern_fn_types_parse() {
    let src = r#"
        struct Marker;

        type Callback = extern "C" fn(u32) -> u32;

        fn run(mut f: impl FnMut(&str)) {
            let _m = Marker;
        }
    "#;

    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let _ = parser.parse_crate();
}

#[test]
fn callable_trait_bounds_parse() {
    let src = r#"
        fn cmd_send_recv<F: FnMut(&str) -> bool>(mut on_line: F) {
        }
    "#;
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let _ = parser.parse_crate();
}

#[test]
fn string_and_vec_coercions_compile() {
    compile_ok(
        "string_and_vec_coercions",
        r#"
        use alloc::string::String;
        use alloc::vec::Vec;

        fn takes_str(s: &str) -> usize { s.len() }
        fn takes_slice(xs: &[u8]) -> usize { xs.len() }
        fn takes_words(xs: &[&str]) -> usize { xs.len() }

        fn run(name: String, bytes: Vec<u8>, words: Vec<&str>) {
            let _ = takes_str(&name);
            let _ = &name[1..];
            let _ = takes_slice(&bytes);
            let _ = &bytes[1..];
            let _ = takes_words(&words);
            let _ = words[0];
        }
        "#,
    );
}

#[test]
fn raw_ptr_prev_assignment_then_field_store_compiles() {
    compile_ok(
        "raw_ptr_prev_assignment_then_field_store",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn run(free_list: *mut *mut FreeBlock, block: *mut FreeBlock) {
            let mut prev: *mut FreeBlock = core::ptr::null_mut();
            let curr = *free_list;
            prev = curr;
            (*prev).next = block;
        }
        "#,
    );
}

#[test]
fn raw_ptr_prev_null_check_then_field_store_compiles() {
    compile_ok(
        "raw_ptr_prev_null_check_then_field_store",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn run(free_list: *mut *mut FreeBlock, block: *mut FreeBlock) {
            let mut prev: *mut FreeBlock = core::ptr::null_mut();
            let curr = *free_list;
            prev = curr;
            if !prev.is_null() {
                (*prev).next = block;
            }
        }
        "#,
    );
}

#[test]
fn raw_ptr_loop_then_prev_field_store_without_else_compiles() {
    compile_ok(
        "raw_ptr_loop_then_prev_field_store_without_else",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn run(free_list: *mut *mut FreeBlock, block: *mut FreeBlock) {
            let mut prev: *mut FreeBlock = core::ptr::null_mut();
            let mut curr = *free_list;
            while !curr.is_null() && (curr as usize) < (block as usize) {
                prev = curr;
                curr = (*curr).next;
            }
            if !prev.is_null() {
                (*prev).next = block;
            }
        }
        "#,
    );
}

#[test]
fn raw_ptr_loop_then_block_next_store_compiles() {
    compile_ok(
        "raw_ptr_loop_then_block_next_store",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn run(free_list: *mut *mut FreeBlock, block: *mut FreeBlock) {
            let mut prev: *mut FreeBlock = core::ptr::null_mut();
            let mut curr = *free_list;
            while !curr.is_null() && (curr as usize) < (block as usize) {
                prev = curr;
                curr = (*curr).next;
            }
            (*block).next = curr;
        }
        "#,
    );
}

#[test]
fn raw_ptr_loop_then_free_list_store_in_if_compiles() {
    compile_ok(
        "raw_ptr_loop_then_free_list_store_in_if",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn run(free_list: *mut *mut FreeBlock, block: *mut FreeBlock) {
            let mut prev: *mut FreeBlock = core::ptr::null_mut();
            let mut curr = *free_list;
            while !curr.is_null() && (curr as usize) < (block as usize) {
                prev = curr;
                curr = (*curr).next;
            }
            if prev.is_null() {
                *free_list = block;
            }
        }
        "#,
    );
}

#[test]
fn raw_ptr_loop_then_full_if_else_without_block_store_compiles() {
    compile_ok(
        "raw_ptr_loop_then_full_if_else_without_block_store",
        r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn run(free_list: *mut *mut FreeBlock, block: *mut FreeBlock) {
            let mut prev: *mut FreeBlock = core::ptr::null_mut();
            let mut curr = *free_list;
            while !curr.is_null() && (curr as usize) < (block as usize) {
                prev = curr;
                curr = (*curr).next;
            }
            if prev.is_null() {
                *free_list = block;
            } else {
                (*prev).next = block;
            }
        }
        "#,
    );
}

#[test]
fn parser_splits_while_and_following_assignment_statement() {
    let src = r#"
        struct FreeBlock {
            size: usize,
            next: *mut FreeBlock,
        }

        unsafe fn run(free_list: *mut *mut FreeBlock, block: *mut FreeBlock) {
            let mut prev: *mut FreeBlock = core::ptr::null_mut();
            let mut curr = *free_list;
            while !curr.is_null() && (curr as usize) < (block as usize) {
                prev = curr;
                curr = (*curr).next;
            }
            (*block).next = curr;
        }
    "#;

    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let krate = parser.parse_crate();
    let mut lower = LoweringContext::new(&mut interner);
    let hir = lower.lower_crate(&krate);

    let run = hir.items.iter().find_map(|item| match &item.kind {
        anyrc::hir::HirItemKind::Fn(f) => Some(f),
        _ => None,
    }).expect("run fn");
    let body = run.body.as_ref().expect("run body");

    assert_eq!(body.stmts.len(), 4);
    assert!(matches!(body.stmts[2], HirStmt::Semi(ref expr, _) if matches!(expr.kind, HirExprKind::Loop(_, _))));
    assert!(matches!(body.stmts[3], HirStmt::Semi(ref expr, _) if matches!(expr.kind, HirExprKind::Assign(_, _))));
}

#[test]
fn let_else_result_binding_compiles() {
    compile_ok(
        "let_else_result_binding",
        r#"
        fn unwrap_or_zero(x: Result<i32, i32>) -> i32 {
            let Ok(value) = x else { return 0; };
            value
        }
        "#,
    );
}

#[test]
fn array_slice_copy_and_sort_by_closure_compiles() {
    compile_ok(
        "array_slice_copy_and_sort_by_closure",
        r#"
        struct DirEntry {
            name: [u8; 56],
            name_len: u8,
            entry_type: u8,
            size: u32,
        }

        struct Vec<T>(T);
        impl<T> Vec<T> {
            fn sort_by<F>(&mut self, _f: F) {}
        }

        fn demo(state: &mut Vec<DirEntry>) {
            let mut buf = [0u8; 64];
            let name_len = 4usize;
            let mut name = [0u8; 56];
            name[..name_len].copy_from_slice(&buf[8..8 + name_len]);
            state.sort_by(|a, b| a.name[..a.name_len as usize].cmp(&b.name[..b.name_len as usize]));
            let _entry = DirEntry {
                name,
                name_len: name_len as u8,
                entry_type: 0,
                size: 0,
            };
        }
        "#,
    );
}

#[test]
fn array_slice_copy_into_fixed_buffer_compiles() {
    compile_ok(
        "array_slice_copy_into_fixed_buffer",
        r#"
        struct DirEntry {
            name: [u8; 56],
            name_len: u8,
            entry_type: u8,
            size: u32,
        }

        fn demo() {
            let mut buf = [0u8; 64];
            let name_len = 4usize;
            let mut name = [0u8; 56];
            name[..name_len].copy_from_slice(&buf[8..8 + name_len]);
            let _entry = DirEntry {
                name,
                name_len: name_len as u8,
                entry_type: 0,
                size: 0,
            };
        }
        "#,
    );
}

#[test]
fn intrinsic_vec_sort_by_closure_compiles() {
    compile_ok(
        "intrinsic_vec_sort_by_closure",
        r#"
        struct DirEntry {
            name: [u8; 56],
            name_len: u8,
            entry_type: u8,
            size: u32,
        }

        struct DialogState {
            entries: Vec<DirEntry>,
        }

        fn demo(state: &mut DialogState) {
            state.entries.sort_by(|a, b| {
                a.name[..a.name_len as usize].cmp(&b.name[..b.name_len as usize])
            });
        }
        "#,
    );
}

#[test]
fn intrinsic_vec_sort_by_branching_ordering_closure_compiles() {
    compile_ok(
        "intrinsic_vec_sort_by_branching_ordering_closure",
        r#"
        struct DirEntry {
            entry_type: u8,
        }

        struct DialogState {
            entries: Vec<DirEntry>,
        }

        fn demo(state: &mut DialogState) {
            state.entries.sort_by(|a, b| {
                if a.entry_type == 2 && b.entry_type != 2 {
                    core::cmp::Ordering::Less
                } else if a.entry_type != 2 && b.entry_type == 2 {
                    core::cmp::Ordering::Greater
                } else {
                    core::cmp::Ordering::Equal
                }
            });
        }
        "#,
    );
}

#[test]
fn fixed_array_field_slice_to_utf8_compiles() {
    compile_ok(
        "fixed_array_field_slice_to_utf8",
        r#"
        struct DirEntry {
            name: [u8; 56],
            name_len: u8,
        }

        fn demo(entry: &DirEntry) -> String {
            let slice = &entry.name[..entry.name_len as usize];
            String::from(core::str::from_utf8(slice).unwrap_or("?"))
        }
        "#,
    );
}

#[test]
fn filedialog_like_module_patterns_compile_together() {
    compile_ok(
        "filedialog_like_module_patterns_compile_together",
        r#"
        struct DirEntry {
            name: [u8; 56],
            name_len: u8,
            entry_type: u8,
            size: u32,
        }

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Mode {
            OpenFile,
            OpenFolder,
            SaveFile,
        }

        struct DialogState {
            mode: Mode,
            current_path: [u8; 257],
            path_len: usize,
            entries: Vec<DirEntry>,
            filename_buf: [u8; 128],
            filename_len: usize,
        }

        fn load_entries(state: &mut DialogState, buf: &[u8; 4096], count: usize) {
            state.entries.clear();
            for i in 0..count {
                let off = i * 64;
                let entry_type = buf[off];
                let name_len = buf[off + 1] as usize;
                let size = u32::from_le_bytes([buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]]);
                if name_len == 0 || name_len > 56 { continue; }
                if state.mode == Mode::OpenFolder && entry_type != 2 { continue; }
                let mut name = [0u8; 56];
                name[..name_len].copy_from_slice(&buf[off + 8..off + 8 + name_len]);
                state.entries.push(DirEntry { name, name_len: name_len as u8, entry_type, size });
            }
            state.entries.sort_by(|a, b| {
                if a.entry_type == 2 && b.entry_type != 2 {
                    core::cmp::Ordering::Less
                } else if a.entry_type != 2 && b.entry_type == 2 {
                    core::cmp::Ordering::Greater
                } else {
                    a.name[..a.name_len as usize].cmp(&b.name[..b.name_len as usize])
                }
            });
        }

        fn entry_name(entry: &DirEntry) -> String {
            let slice = &entry.name[..entry.name_len as usize];
            String::from(core::str::from_utf8(slice).unwrap_or("?"))
        }

        fn current_path(state: &DialogState) -> &str {
            core::str::from_utf8(&state.current_path[..state.path_len]).unwrap_or("/")
        }

        fn selected_name(state: &DialogState) -> String {
            let path = current_path(state);
            let mut full = String::from(path);
            if !full.ends_with('/') {
                full.push('/');
            }
            if !state.entries.is_empty() {
                full.push_str(&entry_name(&state.entries[0]));
            }
            full
        }

        fn save_name(state: &DialogState) -> String {
            let text = core::str::from_utf8(&state.filename_buf[..state.filename_len]).unwrap_or("");
            String::from(text)
        }
        "#,
    );
}

#[test]
fn ordering_variants_with_imported_and_qualified_paths_compile() {
    compile_ok(
        "ordering_variants_with_imported_and_qualified_paths",
        r#"
        use core::cmp::Ordering;

        fn choose(left: u8, right: u8) -> core::cmp::Ordering {
            if left < right {
                Ordering::Less
            } else if left > right {
                core::cmp::Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        "#,
    );
}

#[test]
fn iterator_find_and_option_map_over_boxed_items_compile() {
    compile_ok(
        "iterator_find_and_option_map_over_boxed_items",
        r#"
        use alloc::boxed::Box;

        struct WinInfo {
            ext_id: u32,
        }

        fn find_ref(windows: Vec<Box<WinInfo>>, ext_id: u32) -> Option<&WinInfo> {
            windows
                .iter()
                .find(|w| w.ext_id == ext_id)
                .map(|b| &**b)
        }
        "#,
    );
}

#[test]
fn filedialog_state_helpers_compile_together() {
    compile_ok(
        "filedialog_state_helpers_compile_together",
        r#"
        struct DirEntry {
            name: [u8; 56],
            name_len: u8,
            entry_type: u8,
            size: u32,
        }

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Mode {
            OpenFile,
            OpenFolder,
            SaveFile,
        }

        enum FileDialogResult {
            Selected(String),
        }

        struct DialogState {
            mode: Mode,
            current_path: [u8; 257],
            path_len: usize,
            entries: Vec<DirEntry>,
            selected: Option<usize>,
            scroll_offset: u32,
            filename_buf: [u8; 128],
            filename_len: usize,
        }

        mod fs {
            pub fn readdir(_path: &str, _buf: &mut [u8; 4096]) -> u32 { 0 }
        }

        fn set_path(state: &mut DialogState, path: &str) {
            let bytes = path.as_bytes();
            let len = bytes.len().min(256);
            state.current_path[..len].copy_from_slice(&bytes[..len]);
            state.path_len = len;
        }

        fn load_entries(state: &mut DialogState) {
            state.entries.clear();
            let path = core::str::from_utf8(&state.current_path[..state.path_len]).unwrap_or("/");

            let mut buf = [0u8; 64 * 64];
            let count = fs::readdir(path, &mut buf);
            if count == u32::MAX {
                return;
            }

            for i in 0..count as usize {
                let off = i * 64;
                if off + 64 > buf.len() { break; }
                let entry_type = buf[off];
                let name_len = buf[off + 1] as usize;
                let size = u32::from_le_bytes([buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]]);

                if name_len == 0 || name_len > 56 { continue; }
                if name_len == 1 && buf[off + 8] == b'.' { continue; }
                if name_len == 2 && buf[off + 8] == b'.' && buf[off + 9] == b'.' { continue; }
                if state.mode == Mode::OpenFolder && entry_type != 2 { continue; }

                let mut name = [0u8; 56];
                name[..name_len].copy_from_slice(&buf[off + 8..off + 8 + name_len]);

                state.entries.push(DirEntry {
                    name,
                    name_len: name_len as u8,
                    entry_type,
                    size,
                });
            }

            state.entries.sort_by(|a, b| {
                if a.entry_type == 2 && b.entry_type != 2 {
                    core::cmp::Ordering::Less
                } else if a.entry_type != 2 && b.entry_type == 2 {
                    core::cmp::Ordering::Greater
                } else {
                    a.name[..a.name_len as usize].cmp(&b.name[..b.name_len as usize])
                }
            });
        }

        fn navigate_parent(state: &mut DialogState) {
            let path = core::str::from_utf8(&state.current_path[..state.path_len]).unwrap_or("/");
            if path == "/" { return; }

            let trimmed = path.trim_end_matches('/');
            let parent = match trimmed.rfind('/') {
                Some(0) => "/",
                Some(pos) => &trimmed[..pos],
                None => "/",
            };
            let parent_str = String::from(parent);
            set_path(state, &parent_str);
            load_entries(state);
            state.selected = None;
            state.scroll_offset = 0;
        }

        fn confirm_action(state: &DialogState) -> Option<FileDialogResult> {
            match state.mode {
                Mode::OpenFile => {
                    if let Some(idx) = state.selected {
                        if idx < state.entries.len() && state.entries[idx].entry_type == 1 {
                            let name = entry_name(&state.entries[idx]);
                            let full = build_full_path(state, &name);
                            return Some(FileDialogResult::Selected(full));
                        }
                    }
                    None
                }
                Mode::OpenFolder => {
                    if let Some(idx) = state.selected {
                        if idx < state.entries.len() && state.entries[idx].entry_type == 2 {
                            let name = entry_name(&state.entries[idx]);
                            let full = build_full_path(state, &name);
                            return Some(FileDialogResult::Selected(full));
                        }
                    }
                    let path = core::str::from_utf8(&state.current_path[..state.path_len]).unwrap_or("/");
                    Some(FileDialogResult::Selected(String::from(path)))
                }
                Mode::SaveFile => {
                    if state.filename_len > 0 {
                        let name = core::str::from_utf8(&state.filename_buf[..state.filename_len]).unwrap_or("");
                        let full = build_full_path(state, name);
                        return Some(FileDialogResult::Selected(full));
                    }
                    None
                }
            }
        }

        fn entry_name(entry: &DirEntry) -> String {
            let slice = &entry.name[..entry.name_len as usize];
            String::from(core::str::from_utf8(slice).unwrap_or("?"))
        }

        fn build_full_path(state: &DialogState, name: &str) -> String {
            let path = core::str::from_utf8(&state.current_path[..state.path_len]).unwrap_or("/");
            let mut full = String::from(path);
            if !full.ends_with('/') {
                full.push('/');
            }
            full.push_str(name);
            full
        }
        "#,
    );
}

#[test]
fn if_let_ok_binds_array_payload_for_indexing() {
    compile_ok(
        "if_let_ok_binds_array_payload_for_indexing",
        r#"
        enum Result<T, E> {
            Ok(T),
            Err(E),
        }

        fn meta() -> Result<[u32; 4], u32> {
            Result::Ok([1, 2, 3, 4])
        }

        fn main() {
            if let Result::Ok(meta) = meta() {
                let _ = meta[1] as usize;
            }
        }
        "#,
    );
}

#[test]
fn type_alias_result_payload_binds_inside_if_let() {
    compile_ok(
        "type_alias_result_payload_binds_inside_if_let",
        r#"
        enum Result<T, E> {
            Ok(T),
            Err(E),
        }

        type MyResult<T> = Result<T, u32>;

        fn meta() -> MyResult<[u32; 4]> {
            Result::Ok([1, 2, 3, 4])
        }

        fn main() {
            if let Result::Ok(meta) = meta() {
                let _ = meta[1] as usize;
            }
        }
        "#,
    );
}

#[test]
fn bare_ok_pattern_binds_alias_result_payload() {
    compile_ok(
        "bare_ok_pattern_binds_alias_result_payload",
        r#"
        enum Result<T, E> {
            Ok(T),
            Err(E),
        }

        type MyResult<T> = Result<T, u32>;

        fn meta() -> MyResult<[u32; 4]> {
            Result::Ok([1, 2, 3, 4])
        }

        fn main() {
            if let Ok(meta) = meta() {
                let _ = meta[1] as usize;
            }
        }
        "#,
    );
}

#[test]
fn impl_method_returning_alias_result_binds_ok_payload() {
    compile_ok(
        "impl_method_returning_alias_result_binds_ok_payload",
        r#"
        enum Result<T, E> {
            Ok(T),
            Err(E),
        }

        type MyResult<T> = Result<T, u32>;

        struct File;

        impl File {
            fn metadata(&self) -> MyResult<[u32; 4]> {
                Result::Ok([1, 2, 3, 4])
            }
        }

        fn main() {
            let file = File;
            if let Ok(meta) = file.metadata() {
                let _ = meta[1] as usize;
            }
        }
        "#,
    );
}

#[test]
fn namespaced_alias_result_from_impl_method_binds_ok_payload() {
    compile_ok(
        "namespaced_alias_result_from_impl_method_binds_ok_payload",
        r#"
        mod core_result {
            pub enum Result<T, E> {
                Ok(T),
                Err(E),
            }
        }

        mod error {
            pub type Result<T> = crate::core_result::Result<T, u32>;
        }

        struct File;

        impl File {
            fn metadata(&self) -> error::Result<[u32; 4]> {
                crate::core_result::Result::Ok([1, 2, 3, 4])
            }
        }

        fn main() {
            let file = File;
            if let Ok(meta) = file.metadata() {
                let _ = meta[1] as usize;
            }
        }
        "#,
    );
}

#[test]
fn match_tuple_variant_binds_generic_payload_fields() {
    compile_ok(
        "match_tuple_variant_binds_generic_payload_fields",
        r#"
        struct Vec<T> {
            items: [T; 1],
        }

        impl<T> Vec<T> {
            fn set(&mut self, idx: usize, value: T) {
                self.items[idx] = value;
            }
        }

        struct OccupiedEntry<K, V> {
            key: K,
            value: V,
        }

        struct VacantEntry<K, V> {
            key: K,
            value: V,
        }

        enum Entry<K, V> {
            Occupied(OccupiedEntry<K, V>),
            Vacant(VacantEntry<K, V>),
        }

        impl<K, V> Entry<K, V> {
            fn touch(self) {
                match self {
                    Entry::Occupied(e) => {
                        let _ = e.key;
                        let _ = e.value;
                    }
                    Entry::Vacant(e) => {
                        let _ = e.key;
                        let _ = e.value;
                    }
                }
            }
        }
        "#,
    );
}

#[test]
fn hash_map_entry_like_match_binds_lifetime_generic_payload_fields() {
    compile_ok(
        "hash_map_entry_like_match_binds_lifetime_generic_payload_fields",
        r#"
        struct HashMap<K, V> {
            buckets: [Option<(K, V)>; 1],
            len: usize,
        }

        struct OccupiedEntry<'a, K, V> {
            map: &'a mut HashMap<K, V>,
            idx: usize,
        }

        struct VacantEntry<'a, K, V> {
            map: &'a mut HashMap<K, V>,
            key: K,
            idx: usize,
        }

        enum Entry<'a, K, V> {
            Occupied(OccupiedEntry<'a, K, V>),
            Vacant(VacantEntry<'a, K, V>),
        }

        impl<'a, K, V> Entry<'a, K, V> {
            fn touch(self, default: V) {
                match self {
                    Entry::Occupied(e) => {
                        let _ = e.map.buckets[e.idx];
                    }
                    Entry::Vacant(e) => {
                        e.map.buckets[e.idx] = Some((e.key, default));
                        e.map.len += 1;
                    }
                }
            }
        }
        "#,
    );
}

#[test]
fn hash_map_entry_like_as_mut_unwrap_chain_compiles() {
    compile_ok(
        "hash_map_entry_like_as_mut_unwrap_chain_compiles",
        r#"
        enum Option<T> {
            Some(T),
            None,
        }

        impl<T> Option<T> {
            fn as_mut(&mut self) -> Option<&mut T> {
                match self {
                    Option::Some(v) => Option::Some(v),
                    Option::None => Option::None,
                }
            }

            fn unwrap(self) -> T {
                match self {
                    Option::Some(v) => v,
                    Option::None => loop {},
                }
            }
        }

        struct HashMap<K, V> {
            buckets: [Option<(K, V)>; 1],
            len: usize,
        }

        struct OccupiedEntry<'a, K, V> {
            map: &'a mut HashMap<K, V>,
            idx: usize,
        }

        struct VacantEntry<'a, K, V> {
            map: &'a mut HashMap<K, V>,
            key: K,
            idx: usize,
        }

        enum Entry<'a, K, V> {
            Occupied(OccupiedEntry<'a, K, V>),
            Vacant(VacantEntry<'a, K, V>),
        }

        impl<'a, K, V> Entry<'a, K, V> {
            fn touch(self, default: V) -> &'a mut V {
                match self {
                    Entry::Occupied(e) => {
                        let (_, v) = e.map.buckets[e.idx].as_mut().unwrap();
                        v
                    }
                    Entry::Vacant(e) => {
                        e.map.buckets[e.idx] = Option::Some((e.key, default));
                        e.map.len += 1;
                        let (_, v) = e.map.buckets[e.idx].as_mut().unwrap();
                        v
                    }
                }
            }
        }
        "#,
    );
}

#[test]
fn struct_literal_prefers_locally_resolved_type_over_global_name_collision() {
    compile_ok(
        "struct_literal_prefers_locally_resolved_type_over_global_name_collision",
        r#"
        mod fs {
            pub struct DirEntry {
                pub name: String,
                pub file_type: u8,
                pub size: u32,
            }
        }

        mod ui {
            pub struct DirEntry {
                pub name: [u8; 56],
                pub name_len: u8,
                pub entry_type: u8,
                pub size: u32,
            }

            pub fn make() -> DirEntry {
                let mut name = [0u8; 56];
                name[0] = b'a';
                DirEntry {
                    name,
                    name_len: 1,
                    entry_type: 2,
                    size: 0,
                }
            }
        }
        "#,
    );
}

#[test]
fn shell_glob_byte_char_comparisons_compile() {
    compile_ok(
        "shell_glob_byte_char_comparisons_compile",
        r#"
        fn has_glob_chars(s: &str) -> bool {
            let b = s.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'\\' && i + 1 < b.len() { i += 2; }
                else if b[i] == b'*' || b[i] == b'?' || b[i] == b'[' { return true; }
                else { i += 1; }
            }
            false
        }
        "#,
    );
}

#[test]
fn intrinsic_option_as_mut_unwrap_tuple_pattern_compiles() {
    compile_ok(
        "intrinsic_option_as_mut_unwrap_tuple_pattern_compiles",
        r#"
        fn value(opt: &mut Option<(u32, u32)>) -> &mut u32 {
            let (_, v) = opt.as_mut().unwrap();
            v
        }
        "#,
    );
}

#[test]
fn fs_like_read_write_traits_and_metadata_flow_compile() {
    compile_ok(
        "fs_like_read_write_traits_and_metadata_flow_compile",
        r#"
        enum Result<T, E> {
            Ok(T),
            Err(E),
        }

        mod error {
            pub enum Error { NotFound, BrokenPipe }
            pub type Result<T> = crate::Result<T, Error>;
        }

        trait Read {
            fn read(&mut self, buf: &mut [u8]) -> error::Result<usize>;

            fn read_to_end(&mut self, out: &mut Vec<u8>) -> error::Result<usize> {
                let mut total = 0;
                let mut tmp = [0u8; 1024];
                loop {
                    let n = self.read(&mut tmp)?;
                    if n == 0 { break; }
                    out.extend_from_slice(&tmp[..n]);
                    total += n;
                }
                Ok(total)
            }
        }

        trait Write {
            fn write(&mut self, buf: &[u8]) -> error::Result<usize>;
        }

        struct File {
            fd: u32,
        }

        fn fstat(_fd: u32, stat_buf: &mut [u32; 4]) -> u32 {
            stat_buf[1] = 7;
            0
        }

        fn read(_fd: u32, _buf: &mut [u8]) -> u32 { 0 }
        fn write(_fd: u32, _buf: &[u8]) -> u32 { 1 }

        impl File {
            fn metadata(&self) -> error::Result<[u32; 4]> {
                let mut stat_buf = [0u32; 4];
                let ret = fstat(self.fd, &mut stat_buf);
                if ret == u32::MAX {
                    return Err(error::Error::NotFound);
                }
                Ok(stat_buf)
            }
        }

        impl Read for File {
            fn read(&mut self, buf: &mut [u8]) -> error::Result<usize> {
                let ret = read(self.fd, buf);
                if ret == u32::MAX {
                    return Err(error::Error::NotFound);
                }
                Ok(ret as usize)
            }
        }

        impl Write for File {
            fn write(&mut self, buf: &[u8]) -> error::Result<usize> {
                let ret = write(self.fd, buf);
                if ret == u32::MAX {
                    return Err(error::Error::BrokenPipe);
                }
                Ok(ret as usize)
            }
        }

        fn read_to_vec(file: &mut File) -> error::Result<Vec<u8>> {
            let mut v = Vec::new();
            if let Ok(meta) = file.metadata() {
                let size = meta[1] as usize;
                if size > 0 {
                    v.reserve(size);
                }
            }
            file.read_to_end(&mut v)?;
            Ok(v)
        }
        "#,
    );
}

#[test]
fn window_utf8_and_slice_writer_patterns_compile() {
    compile_ok(
        "window_utf8_and_slice_writer_patterns_compile",
        r#"
        struct MenuBuilder {
            buf: [u8; 128],
            pos: usize,
            num_menus_offset: usize,
            num_menus: usize,
        }

        impl MenuBuilder {
            pub fn build(&mut self) -> &[u8] {
                let nm = self.num_menus as u32;
                self.buf[self.num_menus_offset..self.num_menus_offset + 4]
                    .copy_from_slice(&nm.to_le_bytes());
                &self.buf[..self.pos]
            }

            fn write_bytes(&mut self, data: &[u8]) {
                let end = (self.pos + data.len()).min(self.buf.len());
                let count = end - self.pos;
                self.buf[self.pos..self.pos + count].copy_from_slice(&data[..count]);
                self.pos += count;
            }
        }

        fn clipboard_string(buf: &[u8; 4096], actual: usize) -> Option<String> {
            core::str::from_utf8(&buf[..actual]).ok().map(|s| String::from(s))
        }
        "#,
    );
}

#[test]
fn vec_repeat_macro_supports_indexing_and_slices() {
    compile_ok(
        "vec_repeat_macro_supports_indexing_and_slices",
        r#"
        fn take_bytes(buf: &mut [u8]) -> usize {
            buf[3] = 9;
            buf[3] as usize
        }

        fn build() -> usize {
            let mut buf = vec![0u8; 16];
            let n = take_bytes(&mut buf);
            n + buf[3] as usize
        }
        "#,
    );
}

#[test]
fn vec_list_macro_supports_to_vec_expansion() {
    compile_ok(
        "vec_list_macro_supports_to_vec_expansion",
        r#"
        fn sum_tail() -> usize {
            let buf = vec![1u8, 2u8, 3u8, 4u8];
            let tail = &buf[1..];
            tail[1] as usize
        }
        "#,
    );
}

#[test]
fn generic_impl_methods_substitute_receiver_type_arguments() {
    compile_ok(
        "generic_impl_methods_substitute_receiver_type_arguments",
        r#"
        struct Wrapper<T> {
            value: T,
        }

        impl<T> Wrapper<T> {
            fn get(&self) -> &T {
                &self.value
            }

            fn replace(&mut self, value: T) -> T {
                let old = self.value;
                self.value = value;
                old
            }
        }

        fn use_wrapper(w: &mut Wrapper<u8>) -> u8 {
            let old = w.replace(7u8);
            old + *w.get()
        }
        "#,
    );
}

#[test]
fn option_as_mut_expect_preserves_inner_reference_type() {
    compile_ok(
        "option_as_mut_expect_preserves_inner_reference_type",
        r#"
        enum Option<T> {
            Some(T),
            None,
        }

        impl<T> Option<T> {
            fn as_mut(&mut self) -> Option<&mut T> {
                match self {
                    Option::Some(v) => Option::Some(v),
                    Option::None => Option::None,
                }
            }

            fn expect(self, _msg: &str) -> T {
                match self {
                    Option::Some(v) => v,
                    Option::None => loop {},
                }
            }
        }

        static mut APP: Option<u32> = Option::None;

        fn app() -> &'static mut u32 {
            unsafe { APP.as_mut().expect("not initialized") }
        }

        fn touch() -> u32 {
            *app()
        }
        "#,
    );
}

#[test]
fn core_str_from_utf8_unwrap_or_supports_string_slicing() {
    compile_ok(
        "core_str_from_utf8_unwrap_or_supports_string_slicing",
        r#"
        enum Result<T, E> {
            Ok(T),
            Err(E),
        }

        impl<T, E> Result<T, E> {
            fn unwrap_or(self, default: T) -> T {
                match self {
                    Result::Ok(v) => v,
                    Result::Err(_) => default,
                }
            }
        }

        mod core {
            pub mod str {
                pub fn from_utf8(_bytes: &[u8]) -> crate::Result<&str, ()> {
                    crate::Result::Ok("hello world")
                }
            }
        }

        fn args(buf: &[u8; 256], len: usize) -> &str {
            let all = core::str::from_utf8(&buf[..len]).unwrap_or("");
            if all.starts_with('"') {
                match all[1..].find('"') {
                    Some(close) => all[close + 2..].trim_start(),
                    None => "",
                }
            } else {
                match all.find(' ') {
                    Some(idx) => all[idx + 1..].trim_start(),
                    None => "",
                }
            }
        }
        "#,
    );
}

#[test]
fn slice_try_into_unwrap_supports_from_le_bytes_patterns() {
    compile_ok(
        "slice_try_into_unwrap_supports_from_le_bytes_patterns",
        r#"
        enum Result<T, E> {
            Ok(T),
            Err(E),
        }

        impl<T, E> Result<T, E> {
            fn unwrap(self) -> T {
                match self {
                    Result::Ok(v) => v,
                    Result::Err(_) => loop {},
                }
            }
        }

        fn statfs_like(out: &[u8; 24]) -> u64 {
            u64::from_le_bytes(out[0..8].try_into().unwrap())
        }
        "#,
    );
}

#[test]
fn vec_slice_after_mut_slice_coercion_keeps_byte_element_type() {
    compile_ok(
        "vec_slice_after_mut_slice_coercion_keeps_byte_element_type",
        r#"
        enum Result<T, E> {
            Ok(T),
            Err(E),
        }

        impl<T, E> Result<T, E> {
            fn unwrap_or(self, default: T) -> T {
                match self {
                    Result::Ok(v) => v,
                    Result::Err(_) => default,
                }
            }
        }

        mod core {
            pub mod str {
                pub fn from_utf8(_bytes: &[u8]) -> crate::Result<&str, ()> {
                    crate::Result::Ok("ok")
                }
            }
        }

        fn fill(_buf: &mut [u8]) {}

        fn read_name() -> usize {
            let mut buf = vec![0u8; 32];
            fill(&mut buf);
            core::str::from_utf8(&buf[4..12]).unwrap_or("").len()
        }
        "#,
    );
}

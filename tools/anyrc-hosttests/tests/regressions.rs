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

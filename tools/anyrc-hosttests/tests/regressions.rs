use anyrc::diagnostics::SourceMap;
use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind};
use anyrc::hir::HirStmt;
use anyrc::hir::HirExprKind;
use anyrc::hir_lower::LoweringContext;
use anyrc::intern::Interner;
use anyrc::parser::Parser;

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

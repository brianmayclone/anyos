use anyrc::diagnostics::SourceMap;
use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind};

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


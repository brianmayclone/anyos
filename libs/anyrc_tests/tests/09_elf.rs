use anyrc::codegen::emit::CodeEmitter;
use anyrc::codegen::regalloc;
use anyrc::hir_lower::LoweringContext;
use anyrc::intern::Interner;
use anyrc::linker::elf::{self, ElfRelocation, ElfSymbol, ObjectFile, Section};
use anyrc::linker::link::{self, LinkErrorKind, TargetAbi};
use anyrc::macros::expand_macros;
use anyrc::mir_build::MirBuilder;
use anyrc::parser::Parser;
use anyrc::resolve::Resolver;
use anyrc::typeck::TypeChecker;

fn compile_to_object(src: &str) -> Vec<u8> {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&mut interner);
    let resolve_result = resolver.resolve_crate(&hir);
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let typeck_result = checker.check_crate(&hir);
    let bodies = MirBuilder::build_crate(&mut interner, &resolve_result, &typeck_result, &hir);

    let mut text_data = Vec::new();
    let mut symbols = Vec::new();

    let struct_sizes = regalloc::StructSizes::new();
    let field_offsets = regalloc::StructFieldOffsets::new();
    let field_types = regalloc::StructFieldTypes::new();
    for body in &bodies {
        let alloc = regalloc::allocate(body, &struct_sizes);
        let (code, _relocs) = CodeEmitter::emit_fn(
            body,
            &alloc,
            &interner,
            &struct_sizes,
            &field_offsets,
            &field_types,
        );
        let fn_offset = text_data.len() as u64;
        let fn_size = code.len() as u64;
        let fn_name = interner.resolve(body.name).to_string();
        symbols.push(ElfSymbol::global_func(&fn_name, 0, fn_offset, fn_size));
        text_data.extend_from_slice(&code);
    }

    let obj = ObjectFile {
        sections: vec![Section {
            name: ".text".to_string(),
            data: text_data,
            sh_type: 1,
            sh_flags: 0x6,
            sh_addralign: 16,
        }],
        symbols,
        relocations: vec![],
    };
    elf::write_object(&obj)
}

fn compile_and_link(src: &str) -> Vec<u8> {
    use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind};
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
        ..CompileOptions::default()
    };
    compile(src, "test.rs", &options).unwrap()
}

fn compile_and_link_anyos(src: &str) -> Vec<u8> {
    use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind};
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
        cfg_flags: vec![
            "target_os=\"anyos\"".to_string(),
            "target_arch=\"x86_64\"".to_string(),
        ],
        ..CompileOptions::default()
    };
    compile(src, "test.rs", &options).unwrap()
}

#[test]
fn generate_valid_elf_object() {
    let obj = compile_to_object("fn foo() -> i32 { 42 }");
    assert_eq!(&obj[0..4], &[0x7f, b'E', b'L', b'F']);
    assert_eq!(obj[4], 2); // ELFCLASS64
    assert_eq!(obj[5], 1); // ELFDATA2LSB
}

#[test]
fn elf_has_text_section() {
    let obj = compile_to_object("fn foo() -> i32 { 42 }");
    let sections = elf::parse_section_names(&obj);
    assert!(sections.contains(&".text".to_string()));
}

#[test]
fn elf_has_symbol() {
    let obj = compile_to_object("fn foo() -> i32 { 42 }");
    let symbols = elf::parse_symbol_names(&obj);
    assert!(symbols.iter().any(|s| s.contains("foo")));
}

#[test]
fn link_produces_executable() {
    let exe = compile_and_link("fn main() -> i32 { 0 }");
    assert_eq!(&exe[0..4], &[0x7f, b'E', b'L', b'F']);
    // ET_EXEC
    assert_eq!(exe[16], 2);
}

#[test]
fn linked_executable_has_entry() {
    let exe = compile_and_link("fn main() -> i32 { 42 }");
    let entry = u64::from_le_bytes(exe[24..32].try_into().unwrap());
    assert_ne!(entry, 0);
}

#[test]
fn anyos_executable_loads_above_identity_map_boundary() {
    let exe = compile_and_link_anyos("fn main() -> i32 { 0 }");
    let entry = u64::from_le_bytes(exe[24..32].try_into().unwrap());
    let phoff = u64::from_le_bytes(exe[32..40].try_into().unwrap()) as usize;
    let vaddr = u64::from_le_bytes(exe[phoff + 16..phoff + 24].try_into().unwrap());
    assert_eq!(vaddr, 0x08000000);
    assert!(entry >= 0x08000000);
}

#[test]
fn checked_linker_rejects_unresolved_relocations() {
    let obj = ObjectFile {
        sections: vec![Section {
            name: ".text".to_string(),
            data: vec![0xE8, 0, 0, 0, 0, 0xC3],
            sh_type: 1,
            sh_flags: 0x6,
            sh_addralign: 16,
        }],
        symbols: vec![ElfSymbol {
            name: "missing_symbol".to_string(),
            section: None,
            offset: 0,
            size: 0,
            binding: 1,
            sym_type: 0,
        }],
        relocations: vec![ElfRelocation::pc32(1, 0, -4)],
    };

    let bytes = elf::write_object(&obj);
    let errors = link::link_for_target_checked(&[bytes], "bad", true, TargetAbi::AnyOs)
        .expect_err("unresolved relocation must be a hard linker error");

    assert!(errors.iter().any(|err| {
        err.kind == LinkErrorKind::UnresolvedSymbol && err.symbol == "missing_symbol"
    }));
}

#[test]
fn enum_variant_function_items_get_constructor_symbols() {
    let exe = compile_and_link_anyos(
        r#"
        enum Value {
            Null,
            String(i32),
        }

        fn accept(_mapper: fn(i32) -> Value) -> i32 {
            7
        }

        fn main() -> i32 {
            accept(Value::String)
        }
        "#,
    );
    assert_eq!(&exe[0..4], &[0x7f, b'E', b'L', b'F']);
}

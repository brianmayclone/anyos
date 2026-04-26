use crate::prelude::*;
use crate::codegen::emit::CodeEmitter;
use crate::codegen::regalloc;
use crate::codegen::x86asm::RelocKind;
use crate::diagnostics::{Diagnostic, Level, Span};
use crate::hir_lower::LoweringContext;
use crate::intern::Interner;
use crate::linker::elf::{self, ElfRelocation, ElfSymbol, ObjectFile, Section};
use crate::linker::link;
use crate::macros::expand_macros;
use crate::mir::MirBody;
use crate::mir_build::MirBuilder;
use crate::mir_opt::optimize;
use crate::mono::monomorphize;
use crate::borrowck::check_borrows;
use crate::parser::Parser;
use crate::resolve::Resolver;
use crate::typeck::TypeChecker;
use anyos_std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub input: String,
    pub output: String,
    pub emit: EmitKind,
    pub opt_level: u32,
    pub crate_type: CrateType,
    pub crate_name: Option<String>,
    /// Base directory for module file resolution (parent of src/main.rs or src/lib.rs).
    /// If None, derived from input file path.
    pub src_dir: Option<String>,
    /// Paths to .rlib files to link against (extern crate dependencies).
    pub extern_crates: Vec<ExternCrateSpec>,
    /// Cfg flags for conditional compilation (e.g. "target_arch=\"x86_64\"", "feature=\"kunit\"").
    pub cfg_flags: Vec<String>,
    /// Linker script path (e.g. "-T kernel/link.ld").
    pub linker_script: Option<String>,
    /// Additional linker arguments (object files, libraries, flags).
    pub link_args: Vec<String>,
    /// Environment variables available to env!() macro (key=value pairs).
    pub env_vars: Vec<(String, String)>,
    /// Feature gates enabled via #![feature(...)].
    pub features: Vec<String>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            input: String::new(),
            output: String::from("a.out"),
            emit: EmitKind::Exe,
            opt_level: 0,
            crate_type: CrateType::Bin,
            crate_name: None,
            src_dir: None,
            extern_crates: Vec::new(),
            cfg_flags: vec![String::from("target_os=\"linux\"")],
            linker_script: None,
            link_args: Vec::new(),
            env_vars: Vec::new(),
            features: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExternCrateSpec {
    pub name: String,
    pub rlib_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitKind {
    Exe,
    Obj,
    Rlib,
    Mir,
    Hir,
    Asm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrateType {
    Bin,
    Lib,
    StaticLib,
}

pub fn compile(source: &str, _filename: &str, options: &CompileOptions) -> Result<Vec<u8>, Vec<Diagnostic>> {
    // 1. Lex + Parse
    let mut interner = Interner::new();
    let mut parser = Parser::new(source, &mut interner);
    let mut krate = parser.parse_crate();

    // Check crate-level attributes
    let mut no_main = krate.attrs.iter().any(|a| {
        a.path.segments.len() == 1 && interner.resolve(a.path.segments[0].ident) == "no_main"
    });
    let _no_std = krate.attrs.iter().any(|a| {
        a.path.segments.len() == 1 && interner.resolve(a.path.segments[0].ident) == "no_std"
    });

    // Recognize #![feature(...)] attributes — accepted silently for compatibility.
    // anyrc doesn't have unstable features like rustc, but we need to accept
    // these attributes without erroring so that kernel code compiles.
    let _feature_gates: Vec<String> = krate.attrs.iter().filter_map(|a| {
        if a.path.segments.len() == 1 && interner.resolve(a.path.segments[0].ident) == "feature" {
            // Extract feature names from the token tree
            if let crate::ast::AttrArgs::Delimited(tokens) = &a.args {
                let names: Vec<String> = tokens.iter().filter_map(|tt| {
                    if let crate::ast::TokenTree::Token(t) = tt {
                        if let crate::lexer::TokenKind::Ident(sym) = t.kind {
                            return Some(interner.resolve(sym).to_string());
                        }
                    }
                    None
                }).collect();
                return Some(names.join(","));
            }
            None
        } else {
            None
        }
    }).collect();

    // 1b. Build cfg context from options
    let cfg_ctx = crate::cfg::CfgContext::from_flags(&options.cfg_flags);
    no_main = no_main
        || krate
            .attrs
            .iter()
            .any(|attr| crate::cfg::cfg_attr_applies_attr(attr, &cfg_ctx, &interner, "no_main"));
    let src_dir = if let Some(ref dir) = options.src_dir {
        dir.clone()
    } else {
        // Derive from input path: "src/main.rs" -> "src/"
        let input = &options.input;
        if let Some(pos) = input.rfind('/') {
            input[..pos].to_string()
        } else {
            ".".to_string()
        }
    };
    let loader = crate::loader::OsFileLoader;

    // 2. Expand macros and load files to a fixed point. Real crates often have
    // macro-generated `mod foo;` items whose loaded files contain more macros.
    for _ in 0..16 {
        crate::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
        expand_macros(&mut krate, &mut interner);
        let included_sources =
            crate::loader::resolve_includes_with_env(
                &mut krate,
                &src_dir,
                &mut interner,
                &loader,
                &options.env_vars,
            );
        let loaded_modules =
            crate::loader::resolve_modules_with_env(
                &mut krate,
                &src_dir,
                &mut interner,
                &loader,
                &options.env_vars,
            );
        if included_sources.is_empty() && loaded_modules.is_empty() {
            break;
        }
    }
    crate::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    let (public_interface_source, public_interface) = {
        let mut interface_lower_ctx = LoweringContext::new(&mut interner);
        let interface_hir = interface_lower_ctx.lower_crate(&krate);
        (
            build_public_interface_source(&interface_hir, &interner),
            build_public_interface(&interface_hir, &interner),
        )
    };
    inject_extern_crate_interfaces(&mut krate, &options.extern_crates, &mut interner, &loader);

    // 3. Lower to HIR
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    drop(lower_ctx);

    // 4. Resolve names
    let mut resolver = Resolver::new(&mut interner);
    let resolve_result = resolver.resolve_crate(&hir);
    if !resolve_result.errors.is_empty() {
        return Err(resolve_result.errors);
    }

    // 5. Type check
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let typeck_result = checker.check_crate(&hir);
    if !typeck_result.errors.is_empty() {
        return Err(typeck_result.errors);
    }

    // 6. Build MIR
    let mut mir_bodies = MirBuilder::build_crate(&mut interner, &resolve_result, &typeck_result, &hir);

    // 6b. Monomorphize generic functions
    let mut mir_bodies = monomorphize(mir_bodies, &typeck_result, &mut interner, &hir, &resolve_result);

    // 7. Borrow check
    for body in &mir_bodies {
        let result = check_borrows(
            body,
            &interner,
            &typeck_result.struct_defs,
            &typeck_result.enum_variants,
            &typeck_result.copy_types,
        );
        if !result.errors.is_empty() {
            let fn_name = interner.resolve(body.name);
            let errors = result
                .errors
                .into_iter()
                .map(|mut err| {
                    err.message = format!("in {}: {}", fn_name, err.message);
                    err
                })
                .collect();
            return Err(errors);
        }
    }

    // 8. Optimize MIR
    if options.opt_level > 0 {
        for body in &mut mir_bodies {
            optimize(body);
        }
    }

    // 9. Build struct size map: DefId -> stack storage size in bytes
    //    Uses typeck struct_defs for field types to compute accurate sizes.
    let struct_sizes = {
        use crate::hir::{HirItemKind, HirVariantFields, HirItem};
        use crate::codegen::regalloc::ty_size;

        // First pass: collect field storage as fallback for structs not in typeck
        let mut map: anyos_std::collections::HashMap<crate::hir::DefId, usize> = anyos_std::collections::HashMap::new();
        fn collect_struct_counts(items: &[HirItem], map: &mut anyos_std::collections::HashMap<crate::hir::DefId, usize>) {
            for item in items {
                match &item.kind {
                    HirItemKind::Struct(s) => { map.insert(s.def_id, s.fields.len().max(1) * 8); }
                    HirItemKind::Enum(e) => {
                        let max_fields = e.variants.iter().map(|v| match &v.fields {
                            HirVariantFields::Unit => 0,
                            HirVariantFields::Tuple(tys) => tys.len(),
                            HirVariantFields::Struct(fields) => fields.len(),
                        }).max().unwrap_or(0);
                        if max_fields > 0 { map.insert(e.def_id, (1 + max_fields) * 8); }
                    }
                    HirItemKind::Mod(m) => {
                        if let Some(sub_items) = &m.items { collect_struct_counts(sub_items, map); }
                    }
                    _ => {}
                }
            }
        }
        collect_struct_counts(&hir.items, &mut map);

        // Second pass: compute accurate storage sizes from field types.
        for (def_id, fields) in &typeck_result.struct_defs {
            let total_bytes: i32 = fields.iter()
                .map(|(_, ty)| ty_size(ty, &map))
                .sum();
            map.insert(*def_id, total_bytes.max(8) as usize);
        }
        map
    };

    // 9a. Build struct field offset map
    let field_offsets: regalloc::StructFieldOffsets = {
        use crate::codegen::regalloc::ty_size;
        let mut offsets = anyos_std::collections::HashMap::new();
        for (def_id, fields) in &typeck_result.struct_defs {
            let mut field_offsets_vec = Vec::new();
            let mut offset = 0i32;
            for (_, ty) in fields {
                field_offsets_vec.push(offset);
                offset += ty_size(ty, &struct_sizes);
            }
            offsets.insert(*def_id, field_offsets_vec);
        }
        offsets
    };

    let field_types: regalloc::StructFieldTypes = typeck_result
        .struct_defs
        .iter()
        .map(|(def_id, fields)| {
            (
                *def_id,
                fields.iter().map(|(_, ty)| ty.clone()).collect::<Vec<_>>(),
            )
        })
        .collect();

    // 9b. Collect static data from typeck results
    let static_data: Vec<StaticData> = typeck_result.static_defs.values().map(|(name, _ty, val, _is_mut)| {
        let name_str = interner.resolve(*name).to_string();
        let bytes = match val {
            crate::typeck::ConstVal::Int(v) => (*v as i64).to_le_bytes().to_vec(),
            crate::typeck::ConstVal::Bool(v) => (*v as i64).to_le_bytes().to_vec(),
            crate::typeck::ConstVal::Char(v) => (*v as i64).to_le_bytes().to_vec(),
        };
        StaticData { name: name_str, data: bytes }
    }).collect();

    // 10. Emit based on type
    match options.emit {
        EmitKind::Obj => {
            let obj = codegen_to_object_with_statics(&mir_bodies, &interner, &struct_sizes, &field_offsets, &field_types, &static_data, &typeck_result.enum_variants, &typeck_result.type_def_to_name);
            Ok(elf::write_object(&obj))
        }
        EmitKind::Exe => {
            let obj = codegen_to_object_with_statics(&mir_bodies, &interner, &struct_sizes, &field_offsets, &field_types, &static_data, &typeck_result.enum_variants, &typeck_result.type_def_to_name);
            let obj_bytes = elf::write_object(&obj);
            // Collect all object files: our code + extern crate .rlib objects + runtime stubs
            let mut all_objects = vec![obj_bytes];
            // Add runtime support stubs (__anyrc_alloc, __anyrc_vec_push, etc.)
            let rt_obj = build_runtime_object();
            all_objects.push(elf::write_object(&rt_obj));
            for ext in &options.extern_crates {
                if let Some(rlib_data) = crate::loader::OsFileLoader::read_bytes(&ext.rlib_path) {
                    if let Some((obj_data, _meta)) = crate::loader::unpack_rlib(&rlib_data) {
                        all_objects.push(obj_data);
                    }
                }
            }
            let no_main_flag = no_main || options.crate_type == CrateType::StaticLib;
            // Use extended linker if linker script or link args are provided
            let target_abi = target_abi_from_cfg(&options.cfg_flags);
            if options.linker_script.is_some() || !options.link_args.is_empty() {
                let mut extra_objects = Vec::new();
                for arg in &options.link_args {
                    // If link arg is an object file, read it
                    if arg.ends_with(".o") || arg.ends_with(".a") {
                        if let Some(data) = crate::loader::OsFileLoader::read_bytes(arg) {
                            extra_objects.push(data);
                        }
                    }
                }
                let link_opts = link::LinkOptions {
                    linker_script: options.linker_script.clone(),
                    extra_objects,
                    base_address: None,
                    entry_symbol: None,
                    target_abi,
                };
                let exe = link::link_ext_checked(&all_objects, &options.output, no_main_flag, &link_opts)
                    .map_err(link_errors_to_diagnostics)?;
                validate_anyos_user_exe_if_needed(&exe, target_abi, no_main_flag, options)?;
                Ok(exe)
            } else {
                let exe = link::link_for_target_checked(&all_objects, &options.output, no_main_flag, target_abi)
                    .map_err(link_errors_to_diagnostics)?;
                validate_anyos_user_exe_if_needed(&exe, target_abi, no_main_flag, options)?;
                Ok(exe)
            }
        }
        EmitKind::Rlib => {
            let obj = codegen_to_object_with_statics(&mir_bodies, &interner, &struct_sizes, &field_offsets, &field_types, &static_data, &typeck_result.enum_variants, &typeck_result.type_def_to_name);
            let obj_bytes = elf::write_object(&obj);
            let crate_name = options.crate_name.as_deref().unwrap_or("unknown");
            // Collect exported public symbols
            let mut exports = Vec::new();
            for body in &mir_bodies {
                let name = interner.resolve(body.name).to_string();
                exports.push(crate::loader::ExportedSymbol {
                    name,
                    kind: crate::loader::ExportKind::Function,
                });
            }
            let meta = crate::loader::CrateMetadata {
                name: crate_name.to_string(),
                version: "0.1.0".to_string(),
                exports,
                deps: options.extern_crates.iter().map(|e| e.name.clone()).collect(),
                interface_source: public_interface_source,
                interface: public_interface,
            };
            Ok(crate::loader::pack_rlib(&obj_bytes, &meta))
        }
        EmitKind::Mir => {
            let output = mir_to_string(&mir_bodies, &interner);
            Ok(output.into_bytes())
        }
        EmitKind::Hir | EmitKind::Asm => {
            // Stub
            Ok(vec![])
        }
    }
}

fn target_abi_from_cfg(cfg_flags: &[String]) -> link::TargetAbi {
    if cfg_flags.iter().any(|flag| flag == "target_os=\"linux\"" || flag == "target_os=linux") {
        link::TargetAbi::Linux
    } else {
        link::TargetAbi::AnyOs
    }
}

fn validate_anyos_user_exe_if_needed(
    exe: &[u8],
    target_abi: link::TargetAbi,
    no_main: bool,
    options: &CompileOptions,
) -> Result<(), Vec<Diagnostic>> {
    if target_abi != link::TargetAbi::AnyOs
        || is_kernel_linker_script(options.linker_script.as_deref())
    {
        return Ok(());
    }

    crate::linker::anyos::validate_user_elf(exe)
        .map_err(|err| vec![link_diagnostic(format!("invalid anyOS user ELF: {}", err))])?;

    if !no_main {
        crate::linker::anyos::validate_generated_start_stub(exe)
            .map_err(|err| vec![link_diagnostic(format!("invalid anyOS entry ABI: {}", err))])?;
    }

    Ok(())
}

fn is_kernel_linker_script(script: Option<&str>) -> bool {
    script
        .map(|path| path.ends_with("kernel/link.ld") || path.contains("/kernel/link.ld"))
        .unwrap_or(false)
}

fn link_diagnostic(message: String) -> Diagnostic {
    Diagnostic::new(Level::Error, &message, Span::dummy())
}

fn link_errors_to_diagnostics(errors: Vec<link::LinkError>) -> Vec<Diagnostic> {
    errors
        .into_iter()
        .map(|err| {
            let kind = match err.kind {
                link::LinkErrorKind::UnresolvedSymbol => "unresolved symbol",
                link::LinkErrorKind::RelocationOutOfBounds => "relocation out of bounds",
                link::LinkErrorKind::RelocationOutOfRange => "relocation out of range",
                link::LinkErrorKind::UnsupportedRelocation => "unsupported relocation",
            };
            link_diagnostic(format!(
                "link error: {} '{}' at .text+0x{:x} (reloc type {})",
                kind, err.symbol, err.offset, err.rela_type
            ))
        })
        .collect()
}

fn codegen_to_object(
    bodies: &[MirBody],
    interner: &Interner,
    struct_sizes: &regalloc::StructSizes,
    field_offsets: &regalloc::StructFieldOffsets,
    field_types: &regalloc::StructFieldTypes,
) -> ObjectFile {
    codegen_to_object_with_statics(
        bodies,
        interner,
        struct_sizes,
        field_offsets,
        field_types,
        &[],
        &anyos_std::collections::HashMap::new(),
        &anyos_std::collections::HashMap::new(),
    )
}

/// Static data entry: (symbol_name, initial_bytes)
pub struct StaticData {
    pub name: String,
    pub data: Vec<u8>,
}

fn codegen_to_object_with_statics(
    bodies: &[MirBody],
    interner: &Interner,
    struct_sizes: &regalloc::StructSizes,
    field_offsets: &regalloc::StructFieldOffsets,
    field_types: &regalloc::StructFieldTypes,
    statics: &[StaticData],
    enum_variants: &anyos_std::collections::HashMap<crate::hir::DefId, Vec<(crate::intern::Symbol, Vec<crate::typeck::TyKind>)>>,
    type_def_to_name: &anyos_std::collections::HashMap<crate::hir::DefId, crate::intern::Symbol>,
) -> ObjectFile {
    let mut text_data = Vec::new();
    let mut symbols = Vec::new();
    let mut relocations = Vec::new();

    for body in bodies {
        let alloc = regalloc::allocate(body, struct_sizes);
        let (code, relocs) =
            CodeEmitter::emit_fn(body, &alloc, interner, struct_sizes, field_offsets, field_types);

        let fn_offset = text_data.len() as u64;
        let fn_size = code.len() as u64;
        let fn_name = interner.resolve(body.name).to_string();

        // Convert assembler relocations to ELF relocations
        for rel in &relocs {
            let sym_idx = symbols.len() + 1; // will be added after this symbol? No, we need to find existing
            // For now, create a symbol for the target if it doesn't exist
            // The relocation target might be another function in this compilation unit
            let target_sym = symbols.iter().position(|s: &ElfSymbol| s.name == rel.symbol);
            let sym_idx = match target_sym {
                Some(i) => i,
                None => {
                    // Create undefined symbol
                    let idx = symbols.len();
                    symbols.push(ElfSymbol {
                        name: rel.symbol.clone(),
                        section: None,
                        offset: 0,
                        size: 0,
                        binding: 1, // STB_GLOBAL
                        sym_type: 0, // STT_NOTYPE
                    });
                    idx
                }
            };
            let rela_type = match rel.kind {
                RelocKind::PcRel32 => 2, // R_X86_64_PC32
                RelocKind::Abs64 => 1,   // R_X86_64_64
            };
            relocations.push(ElfRelocation {
                offset: fn_offset + rel.offset as u64,
                symbol: sym_idx,
                rela_type,
                addend: -4, // PC-relative needs -4 addend
            });
        }

        // Add the function symbol (might replace an undefined one)
        let existing = symbols.iter().position(|s| s.name == fn_name);
        if let Some(idx) = existing {
            // Update undefined symbol to defined
            symbols[idx].section = Some(0); // .text
            symbols[idx].offset = fn_offset;
            symbols[idx].size = fn_size;
            symbols[idx].sym_type = 2; // STT_FUNC
        } else {
            symbols.push(ElfSymbol::global_func(&fn_name, 0, fn_offset, fn_size));
        }

        text_data.extend_from_slice(&code);
    }

    append_enum_variant_constructor_symbols(
        &mut text_data,
        &mut symbols,
        enum_variants,
        type_def_to_name,
        interner,
    );

    let mut sections = vec![
        Section {
            name: ".text".to_string(),
            data: text_data,
            sh_type: 1, // SHT_PROGBITS
            sh_flags: 0x2 | 0x4, // SHF_ALLOC | SHF_EXECINSTR
            sh_addralign: 16,
        },
    ];

    // Add .data section if there are statics
    if !statics.is_empty() {
        let data_section_idx = sections.len(); // index 1
        let mut data_bytes = Vec::new();
        for s in statics {
            let offset = data_bytes.len() as u64;
            data_bytes.extend_from_slice(&s.data);
            // Add symbol for this static in .data section
            let existing = symbols.iter().position(|sym| sym.name == s.name);
            if let Some(idx) = existing {
                symbols[idx].section = Some(data_section_idx);
                symbols[idx].offset = offset;
                symbols[idx].size = s.data.len() as u64;
                symbols[idx].sym_type = 1; // STT_OBJECT
            } else {
                symbols.push(ElfSymbol {
                    name: s.name.clone(),
                    section: Some(data_section_idx),
                    offset,
                    size: s.data.len() as u64,
                    binding: 1, // STB_GLOBAL
                    sym_type: 1, // STT_OBJECT
                });
            }
        }
        sections.push(Section {
            name: ".data".to_string(),
            data: data_bytes,
            sh_type: 1, // SHT_PROGBITS
            sh_flags: 0x2 | 0x1, // SHF_ALLOC | SHF_WRITE
            sh_addralign: 8,
        });
    }

    ObjectFile {
        sections,
        symbols,
        relocations,
    }
}

fn append_enum_variant_constructor_symbols(
    text_data: &mut Vec<u8>,
    symbols: &mut Vec<ElfSymbol>,
    enum_variants: &anyos_std::collections::HashMap<crate::hir::DefId, Vec<(crate::intern::Symbol, Vec<crate::typeck::TyKind>)>>,
    type_def_to_name: &anyos_std::collections::HashMap<crate::hir::DefId, crate::intern::Symbol>,
    interner: &Interner,
) {
    for (enum_def_id, variants) in enum_variants {
        let enum_name = type_def_to_name
            .get(enum_def_id)
            .map(|sym| interner.resolve(*sym).to_string());

        for (idx, (variant_sym, fields)) in variants.iter().enumerate() {
            if fields.is_empty() {
                continue;
            }

            let variant_name = interner.resolve(*variant_sym).to_string();
            let mut names = vec![variant_name.clone()];
            if let Some(enum_name) = &enum_name {
                names.push(format!("{}::{}", enum_name, variant_name));
            }

            for name in names {
                if symbols
                    .iter()
                    .any(|sym| sym.name == name && sym.section.is_some())
                {
                    continue;
                }

                let fn_offset = text_data.len() as u64;
                let code = enum_variant_constructor_code(idx as u64);
                let fn_size = code.len() as u64;
                text_data.extend_from_slice(&code);

                if let Some(existing) = symbols.iter().position(|sym| sym.name == name) {
                    symbols[existing].section = Some(0);
                    symbols[existing].offset = fn_offset;
                    symbols[existing].size = fn_size;
                    symbols[existing].sym_type = 2;
                } else {
                    symbols.push(ElfSymbol::global_func(&name, 0, fn_offset, fn_size));
                }
            }
        }
    }
}

fn enum_variant_constructor_code(discriminant: u64) -> Vec<u8> {
    let mut code = Vec::new();
    code.extend_from_slice(&[0x48, 0xB8]); // mov rax, imm64
    code.extend_from_slice(&discriminant.to_le_bytes());
    code.push(0xC3); // ret
    code
}

fn mir_to_string(bodies: &[MirBody], interner: &Interner) -> String {
    let mut out = String::new();
    for body in bodies {
        out.push_str(&format!("fn {}() {{\n", interner.resolve(body.name)));
        for (i, bb) in body.basic_blocks.iter().enumerate() {
            out.push_str(&format!("  bb{}: {{\n", i));
            out.push_str(&format!("    // {} statements\n", bb.statements.len()));
            out.push_str(&format!("    // terminator: {:?}\n", terminator_kind(&bb.terminator)));
            out.push_str("  }\n");
        }
        out.push_str("}\n\n");
    }
    out
}

fn terminator_kind(t: &crate::mir::Terminator) -> &'static str {
    match t {
        crate::mir::Terminator::Goto(_) => "goto",
        crate::mir::Terminator::SwitchInt { .. } => "switchInt",
        crate::mir::Terminator::Call { .. } => "call",
        crate::mir::Terminator::Return => "return",
        crate::mir::Terminator::Unreachable => "unreachable",
    }
}

/// Build an ELF object file containing the runtime support stubs.
fn build_runtime_object() -> elf::ObjectFile {
    let stubs = crate::runtime::runtime_stubs();
    let mut text_data = Vec::new();
    let mut symbols = Vec::new();
    let mut relocations = Vec::new();
    let mut stub_offsets = Vec::new();

    for (name, code) in &stubs {
        let offset = text_data.len() as u64;
        let size = code.len() as u64;
        stub_offsets.push((name.as_str(), offset));
        text_data.extend_from_slice(code);
        symbols.push(elf::ElfSymbol {
            name: name.clone(),
            section: Some(0), // .text
            offset,
            size,
            binding: 1, // STB_GLOBAL
            sym_type: 2, // STT_FUNC
        });
    }

    for ((name, code), (_, stub_offset)) in stubs.iter().zip(stub_offsets.iter()) {
        let target = match name.as_str() {
            "__anyrc_realloc" => Some("__anyrc_alloc"),
            "__anyrc_vec_push" => Some("__anyrc_realloc"),
            _ => None,
        };
        let Some(target) = target else {
            continue;
        };
        let Some(target_idx) = symbols.iter().position(|sym| sym.name == target) else {
            continue;
        };
        for (idx, window) in code.windows(5).enumerate() {
            if window == [0xE8, 0x00, 0x00, 0x00, 0x00] {
                relocations.push(elf::ElfRelocation {
                    offset: *stub_offset + idx as u64 + 1,
                    symbol: target_idx,
                    rela_type: 2,
                    addend: -4,
                });
            }
        }
    }

    elf::ObjectFile {
        sections: vec![elf::Section {
            name: ".text".to_string(),
            data: text_data,
            sh_type: 1,         // SHT_PROGBITS
            sh_flags: 0x2 | 0x4, // SHF_ALLOC | SHF_EXECINSTR
            sh_addralign: 16,
        }],
        symbols,
        relocations,
    }
}

fn inject_extern_crate_interfaces(
    krate: &mut crate::ast::Crate,
    extern_crates: &[ExternCrateSpec],
    interner: &mut Interner,
    loader: &dyn crate::loader::FileLoader,
) {
    for ext in extern_crates {
        let Some(rlib_data) = loader.read_file_bytes(&ext.rlib_path) else {
            continue;
        };
        let Some((_, meta)) = crate::loader::unpack_rlib(&rlib_data) else {
            continue;
        };
        if meta.interface_source.trim().is_empty() {
            continue;
        }

        let interface_source = relativize_extern_interface_source(&meta.interface_source, &ext.name);
        let wrapper_src = format!("mod {} {{\n{}\n}}", ext.name, interface_source);
        let mut parser = Parser::new(&wrapper_src, interner);
        let mut iface_krate = parser.parse_crate();
        krate.items.append(&mut iface_krate.items);
    }
}

fn relativize_extern_interface_source(source: &str, crate_name: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut pos = 0;

    while let Some(rel) = source[pos..].find("crate::") {
        let start = pos + rel;
        if start > 0 && is_ident_byte(bytes[start - 1]) {
            out.push_str(&source[pos..start + "crate::".len()]);
            pos = start + "crate::".len();
            continue;
        }

        out.push_str(&source[pos..start]);
        let after_prefix = start + "crate::".len();
        let mut seg_end = after_prefix;
        while seg_end < bytes.len() && is_ident_byte(bytes[seg_end]) {
            seg_end += 1;
        }
        let first_segment = &source[after_prefix..seg_end];
        if is_compiler_known_external_crate(first_segment) {
            out.push_str("crate::");
        } else {
            out.push_str("crate::");
            out.push_str(crate_name);
            out.push_str("::");
        }
        pos = after_prefix;
    }

    out.push_str(&source[pos..]);
    out
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_compiler_known_external_crate(name: &str) -> bool {
    matches!(
        name,
        "core"
            | "alloc"
            | "std"
            | "anyos_std"
            | "proc_macro"
            | "proc_macro2"
            | "quote"
            | "syn"
            | "serde"
            | "dynlink"
    )
}

fn build_public_interface_source(hir: &crate::hir::HirCrate, interner: &Interner) -> String {
    let mut out = String::new();
    render_items(&mut out, &hir.items, interner, 0, false);
    out
}

fn build_public_interface(
    hir: &crate::hir::HirCrate,
    interner: &Interner,
) -> crate::loader::CrateInterface {
    let mut items = Vec::new();
    collect_interface_items(&mut items, &hir.items, interner, 0, false);
    crate::loader::CrateInterface { items }
}

fn collect_interface_items(
    out: &mut Vec<crate::loader::InterfaceItem>,
    items: &[crate::hir::HirItem],
    interner: &Interner,
    indent: usize,
    in_trait: bool,
) {
    let local_names = local_item_names(items);
    let public_type_names = local_public_type_names(items);
    for item in items {
        if !item_is_exported(item, in_trait, &local_names, &public_type_names, interner) {
            continue;
        }
        let mut signature = String::new();
        render_item(&mut signature, item, interner, indent, in_trait);
        if let Some((name, kind)) = interface_item_name_and_kind(item, interner) {
            out.push(crate::loader::InterfaceItem {
                name,
                kind,
                signature,
            });
        }
    }
}

fn interface_item_name_and_kind(
    item: &crate::hir::HirItem,
    interner: &Interner,
) -> Option<(String, crate::loader::InterfaceItemKind)> {
    use crate::hir::HirItemKind;
    use crate::loader::InterfaceItemKind;

    match &item.kind {
        HirItemKind::Fn(f) => Some((
            interner.resolve(f.name).to_string(),
            InterfaceItemKind::Function,
        )),
        HirItemKind::Struct(s) => Some((
            interner.resolve(s.name).to_string(),
            InterfaceItemKind::Struct,
        )),
        HirItemKind::Enum(e) => Some((
            interner.resolve(e.name).to_string(),
            InterfaceItemKind::Enum,
        )),
        HirItemKind::Trait(t) => Some((
            interner.resolve(t.name).to_string(),
            InterfaceItemKind::Trait,
        )),
        HirItemKind::TypeAlias(ta) => Some((
            interner.resolve(ta.name).to_string(),
            InterfaceItemKind::TypeAlias,
        )),
        HirItemKind::Const(c) => Some((
            interner.resolve(c.name).to_string(),
            InterfaceItemKind::Const,
        )),
        HirItemKind::Static(s) => Some((
            interner.resolve(s.name).to_string(),
            InterfaceItemKind::Static,
        )),
        HirItemKind::Impl(ib) => Some((
            format!("impl {}", render_ty(&ib.self_ty, interner)),
            InterfaceItemKind::Impl,
        )),
        HirItemKind::Use(u) => {
            let mut rendered = String::new();
            render_use_tree(&mut rendered, u, interner);
            Some((rendered, InterfaceItemKind::Use))
        }
        HirItemKind::Mod(m) => Some((
            interner.resolve(m.name).to_string(),
            InterfaceItemKind::Module,
        )),
        HirItemKind::ExternBlock(_) => Some((
            "extern".to_string(),
            InterfaceItemKind::ExternBlock,
        )),
    }
}

fn render_items(
    out: &mut String,
    items: &[crate::hir::HirItem],
    interner: &Interner,
    indent: usize,
    in_trait: bool,
) {
    let local_names = local_item_names(items);
    let public_type_names = local_public_type_names(items);
    let needed_private_consts =
        needed_private_const_names(items, &local_names, &public_type_names, interner, in_trait);
    let needed_private_types =
        needed_private_type_names(items, &local_names, &public_type_names, interner, in_trait);
    for item in items {
        if !item_is_exported(item, in_trait, &local_names, &public_type_names, interner)
            && !private_const_is_needed(item, &needed_private_consts)
            && !private_type_is_needed(item, &needed_private_types)
        {
            continue;
        }
        render_item(out, item, interner, indent, in_trait);
    }
}

fn render_item(
    out: &mut String,
    item: &crate::hir::HirItem,
    interner: &Interner,
    indent: usize,
    in_trait: bool,
) {
    use crate::hir::{HirItemKind, HirUseTreeKind, HirVariantFields};

    let ind = "    ".repeat(indent);
    match &item.kind {
        HirItemKind::Fn(f) => {
            out.push_str(&ind);
            out.push_str(&render_visibility(f.vis));
            if f.is_const {
                out.push_str("const ");
            }
            if f.is_unsafe {
                out.push_str("unsafe ");
            }
            if let Some(abi) = &f.abi {
                out.push_str("extern \"");
                out.push_str(abi);
                out.push_str("\" ");
            }
            out.push_str("fn ");
            out.push_str(interner.resolve(f.name));
            out.push_str(&render_generics(&f.generics, interner));
            out.push('(');
            for (idx, param) in f.params.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("arg{}: {}", idx, render_ty(&param.ty, interner)));
            }
            out.push(')');
            if let Some(ret) = &f.ret_ty {
                out.push_str(" -> ");
                out.push_str(&render_ty(ret, interner));
            }
            if in_trait {
                if f.body.is_some() {
                    out.push_str(" { loop {} }");
                } else {
                    out.push(';');
                }
            } else {
                out.push(';');
            }
            out.push('\n');
        }
        HirItemKind::Struct(s) => {
            out.push_str(&ind);
            out.push_str(&render_visibility(s.vis));
            out.push_str("struct ");
            out.push_str(interner.resolve(s.name));
            out.push_str(&render_generics(&s.generics, interner));
            if s.fields.is_empty() {
                out.push_str(";\n");
            } else if is_tuple_fields(&s.fields, interner) {
                let public_fields: Vec<_> = s.fields.iter()
                    .filter(|field| is_public_vis(field.vis))
                    .collect();
                out.push('(');
                for (idx, field) in public_fields.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&render_visibility(field.vis));
                    out.push_str(&render_ty(&field.ty, interner));
                }
                if public_fields.len() != s.fields.len() {
                    if !public_fields.is_empty() {
                        out.push_str(", ");
                    }
                    out.push_str("()");
                }
                out.push_str(");\n");
            } else {
                let public_fields: Vec<_> = s.fields.iter()
                    .filter(|field| is_public_vis(field.vis))
                    .collect();
                out.push_str(" {\n");
                for field in public_fields {
                    out.push_str(&"    ".repeat(indent + 1));
                    out.push_str(&render_visibility(field.vis));
                    out.push_str(interner.resolve(field.name));
                    out.push_str(": ");
                    out.push_str(&render_ty(&field.ty, interner));
                    out.push_str(",\n");
                }
                if s.fields.iter().any(|field| !is_public_vis(field.vis)) {
                    out.push_str(&"    ".repeat(indent + 1));
                    out.push_str("__private: (),\n");
                }
                out.push_str(&ind);
                out.push_str("}\n");
            }
        }
        HirItemKind::Enum(e) => {
            out.push_str(&ind);
            out.push_str(&render_visibility(e.vis));
            out.push_str("enum ");
            out.push_str(interner.resolve(e.name));
            out.push_str(&render_generics(&e.generics, interner));
            out.push_str(" {\n");
            for variant in &e.variants {
                out.push_str(&"    ".repeat(indent + 1));
                out.push_str(interner.resolve(variant.name));
                match &variant.fields {
                    HirVariantFields::Unit => {}
                    HirVariantFields::Tuple(tys) => {
                        out.push('(');
                        for (idx, ty) in tys.iter().enumerate() {
                            if idx > 0 {
                                out.push_str(", ");
                            }
                            out.push_str(&render_ty(ty, interner));
                        }
                        out.push(')');
                    }
                    HirVariantFields::Struct(fields) => {
                        out.push_str(" {\n");
                        for field in fields {
                            out.push_str(&"    ".repeat(indent + 2));
                            out.push_str(interner.resolve(field.name));
                            out.push_str(": ");
                            out.push_str(&render_ty(&field.ty, interner));
                            out.push_str(",\n");
                        }
                        out.push_str(&"    ".repeat(indent + 1));
                        out.push('}');
                    }
                }
                if let Some(discriminant) = &variant.discriminant {
                    out.push_str(" = ");
                    out.push_str(&render_expr(discriminant, interner));
                }
                out.push_str(",\n");
            }
            out.push_str(&ind);
            out.push_str("}\n");
        }
        HirItemKind::Impl(ib) => {
            let impl_local_names = local_item_names(&ib.items);
            let impl_public_type_names = local_public_type_names(&ib.items);
            let exported_items: Vec<&crate::hir::HirItem> = if trait_impl_is_interface_relevant(ib, interner) {
                ib.items.iter()
                    .filter(|item| {
                        matches!(
                            item.kind,
                            HirItemKind::Fn(_)
                                | HirItemKind::TypeAlias(_)
                                | HirItemKind::Const(_)
                                | HirItemKind::Static(_)
                        )
                    })
                    .collect()
            } else {
                ib.items.iter()
                    .filter(|item| {
                        item_is_exported(
                            item,
                            false,
                            &impl_local_names,
                            &impl_public_type_names,
                            interner,
                        )
                    })
                    .collect()
            };
            if exported_items.is_empty() {
                return;
            }
            out.push_str(&ind);
            if ib.is_unsafe {
                out.push_str("unsafe ");
            }
            out.push_str("impl");
            out.push_str(&render_generics(&ib.generics, interner));
            out.push(' ');
            if let Some(trait_ref) = &ib.trait_ref {
                out.push_str(&render_path(trait_ref, interner));
                out.push_str(" for ");
            }
            out.push_str(&render_ty(&ib.self_ty, interner));
            out.push_str(" {\n");
            for item in exported_items {
                render_item(out, item, interner, indent + 1, false);
            }
            out.push_str(&ind);
            out.push_str("}\n");
        }
        HirItemKind::Trait(t) => {
            out.push_str(&ind);
            out.push_str(&render_visibility(t.vis));
            if t.is_unsafe {
                out.push_str("unsafe ");
            }
            out.push_str("trait ");
            out.push_str(interner.resolve(t.name));
            out.push_str(&render_generics(&t.generics, interner));
            if !t.supertraits.is_empty() {
                out.push_str(": ");
                for (idx, bound) in t.supertraits.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(" + ");
                    }
                    out.push_str(&render_path(&bound.path, interner));
                }
            }
            out.push_str(" {\n");
            render_items(out, &t.items, interner, indent + 1, true);
            out.push_str(&ind);
            out.push_str("}\n");
        }
        HirItemKind::TypeAlias(ta) => {
            out.push_str(&ind);
            out.push_str(&render_visibility(ta.vis));
            out.push_str("type ");
            out.push_str(interner.resolve(ta.name));
            out.push_str(&render_generics(&ta.generics, interner));
            if let Some(ty) = &ta.ty {
                out.push_str(" = ");
                out.push_str(&render_ty(ty, interner));
            }
            out.push_str(";\n");
        }
        HirItemKind::Const(c) => {
            out.push_str(&ind);
            out.push_str(&render_visibility(c.vis));
            out.push_str("const ");
            out.push_str(interner.resolve(c.name));
            out.push_str(": ");
            out.push_str(&render_ty(&c.ty, interner));
            if let Some(value) = &c.value {
                out.push_str(" = ");
                out.push_str(&render_expr(value, interner));
            }
            out.push_str(";\n");
        }
        HirItemKind::Static(s) => {
            out.push_str(&ind);
            out.push_str(&render_visibility(s.vis));
            out.push_str("static ");
            if s.is_mut {
                out.push_str("mut ");
            }
            out.push_str(interner.resolve(s.name));
            out.push_str(": ");
            out.push_str(&render_ty(&s.ty, interner));
            out.push_str(";\n");
        }
        HirItemKind::Use(u) => {
            out.push_str(&ind);
            out.push_str(&render_visibility(u.vis));
            out.push_str("use ");
            render_use_tree(out, u, interner);
            out.push_str(";\n");
        }
        HirItemKind::Mod(m) => {
            out.push_str(&ind);
            out.push_str(&render_visibility(m.vis));
            out.push_str("mod ");
            out.push_str(interner.resolve(m.name));
            if let Some(items) = &m.items {
                out.push_str(" {\n");
                render_items(out, items, interner, indent + 1, false);
                out.push_str(&ind);
                out.push_str("}\n");
            } else {
                out.push_str(" {}\n");
            }
        }
        HirItemKind::ExternBlock(eb) => {
            let extern_local_names = local_item_names(&eb.items);
            let extern_public_type_names = local_public_type_names(&eb.items);
            let exported_items: Vec<&crate::hir::HirItem> = eb.items.iter()
                .filter(|item| {
                    item_is_exported(
                        item,
                        false,
                        &extern_local_names,
                        &extern_public_type_names,
                        interner,
                    )
                })
                .collect();
            if exported_items.is_empty() {
                return;
            }
            out.push_str(&ind);
            out.push_str("extern ");
            if let Some(abi) = &eb.abi {
                out.push('"');
                out.push_str(abi);
                out.push('"');
                out.push(' ');
            }
            out.push_str("{\n");
            for sub in exported_items {
                match &sub.kind {
                    HirItemKind::Fn(_) | HirItemKind::Static(_) => render_item(out, sub, interner, indent + 1, false),
                    _ => {}
                }
            }
            out.push_str(&ind);
            out.push_str("}\n");
        }
    }

    if !matches!(item.kind, HirItemKind::Use(_)) {
        out.push('\n');
    }

    let _ = HirUseTreeKind::Glob;
}

fn render_visibility(vis: crate::ast::Visibility) -> &'static str {
    match vis {
        crate::ast::Visibility::Private => "",
        crate::ast::Visibility::Public => "pub ",
        crate::ast::Visibility::PubCrate => "pub(crate) ",
        crate::ast::Visibility::PubSuper
        | crate::ast::Visibility::PubSelf
        | crate::ast::Visibility::PubIn => "pub(crate) ",
    }
}

fn is_public_vis(vis: crate::ast::Visibility) -> bool {
    vis == crate::ast::Visibility::Public
}

fn item_is_exported(
    item: &crate::hir::HirItem,
    in_trait: bool,
    local_names: &[crate::intern::Symbol],
    public_type_names: &[crate::intern::Symbol],
    interner: &Interner,
) -> bool {
    match &item.kind {
        crate::hir::HirItemKind::Fn(f) => in_trait || is_public_vis(f.vis),
        crate::hir::HirItemKind::Struct(s) => is_public_vis(s.vis),
        crate::hir::HirItemKind::Enum(e) => is_public_vis(e.vis),
        crate::hir::HirItemKind::Trait(t) => is_public_vis(t.vis),
        crate::hir::HirItemKind::TypeAlias(ta) => is_public_vis(ta.vis),
        crate::hir::HirItemKind::Const(c) => is_public_vis(c.vis),
        crate::hir::HirItemKind::Static(s) => is_public_vis(s.vis),
        crate::hir::HirItemKind::Use(u) => use_item_supports_interface(u, local_names, interner),
        crate::hir::HirItemKind::Mod(m) => {
            if is_public_vis(m.vis) {
                return true;
            }
            let Some(items) = &m.items else {
                return false;
            };
            let nested_local_names = local_item_names(items);
            let nested_public_type_names = local_public_type_names(items);
            items.iter().any(|item| {
                item_is_exported(
                    item,
                    false,
                    &nested_local_names,
                    &nested_public_type_names,
                    interner,
                )
            })
        }
        crate::hir::HirItemKind::Impl(ib) => {
            if ib.trait_ref.is_none()
                && inherent_impl_self_is_private_local(ib, local_names, public_type_names)
            {
                return false;
            }
            if trait_impl_is_interface_relevant(ib, interner)
                && ib.items.iter().any(|item| {
                    matches!(
                        item.kind,
                        crate::hir::HirItemKind::Fn(_)
                            | crate::hir::HirItemKind::TypeAlias(_)
                            | crate::hir::HirItemKind::Const(_)
                            | crate::hir::HirItemKind::Static(_)
                    )
                })
            {
                return true;
            }
            let nested_local_names = local_item_names(&ib.items);
            let nested_public_type_names = local_public_type_names(&ib.items);
            ib.items.iter().any(|item| {
                item_is_exported(
                    item,
                    false,
                    &nested_local_names,
                    &nested_public_type_names,
                    interner,
                )
            })
        }
        crate::hir::HirItemKind::ExternBlock(eb) => {
            let nested_local_names = local_item_names(&eb.items);
            let nested_public_type_names = local_public_type_names(&eb.items);
            eb.items.iter().any(|item| {
                item_is_exported(
                    item,
                    false,
                    &nested_local_names,
                    &nested_public_type_names,
                    interner,
                )
            })
        }
    }
}

fn trait_impl_is_interface_relevant(
    ib: &crate::hir::HirImplBlock,
    interner: &Interner,
) -> bool {
    let Some(trait_ref) = &ib.trait_ref else {
        return false;
    };
    let Some(last) = trait_ref.segments.last() else {
        return false;
    };
    matches!(
        interner.resolve(last.ident),
        "Deref" | "DerefMut" | "Index" | "From" | "PartialEq" | "Iterator" | "IntoIterator"
    )
}

fn local_item_names(items: &[crate::hir::HirItem]) -> Vec<crate::intern::Symbol> {
    items.iter().filter_map(item_declared_name).collect()
}

fn local_public_type_names(items: &[crate::hir::HirItem]) -> Vec<crate::intern::Symbol> {
    items
        .iter()
        .filter_map(|item| match &item.kind {
            crate::hir::HirItemKind::Struct(s) if is_public_vis(s.vis) => Some(s.name),
            crate::hir::HirItemKind::Enum(e) if is_public_vis(e.vis) => Some(e.name),
            crate::hir::HirItemKind::Trait(t) if is_public_vis(t.vis) => Some(t.name),
            crate::hir::HirItemKind::TypeAlias(t) if is_public_vis(t.vis) => Some(t.name),
            _ => None,
        })
        .collect()
}

fn item_declared_name(item: &crate::hir::HirItem) -> Option<crate::intern::Symbol> {
    match &item.kind {
        crate::hir::HirItemKind::Fn(f) => Some(f.name),
        crate::hir::HirItemKind::Struct(s) => Some(s.name),
        crate::hir::HirItemKind::Enum(e) => Some(e.name),
        crate::hir::HirItemKind::Trait(t) => Some(t.name),
        crate::hir::HirItemKind::TypeAlias(ta) => Some(ta.name),
        crate::hir::HirItemKind::Const(c) => Some(c.name),
        crate::hir::HirItemKind::Static(s) => Some(s.name),
        crate::hir::HirItemKind::Mod(m) => Some(m.name),
        _ => None,
    }
}

fn inherent_impl_self_is_private_local(
    ib: &crate::hir::HirImplBlock,
    local_names: &[crate::intern::Symbol],
    public_type_names: &[crate::intern::Symbol],
) -> bool {
    let crate::hir::HirTy::Path(path) = &ib.self_ty else {
        return false;
    };
    let Some(first) = path.segments.first() else {
        return false;
    };
    local_names.contains(&first.ident) && !public_type_names.contains(&first.ident)
}

fn private_const_is_needed(
    item: &crate::hir::HirItem,
    needed: &HashSet<crate::intern::Symbol>,
) -> bool {
    match &item.kind {
        crate::hir::HirItemKind::Const(c) => {
            !is_public_vis(c.vis) && needed.contains(&c.name)
        }
        _ => false,
    }
}

fn private_type_is_needed(
    item: &crate::hir::HirItem,
    needed: &HashSet<crate::intern::Symbol>,
) -> bool {
    match &item.kind {
        crate::hir::HirItemKind::Struct(s) => {
            !is_public_vis(s.vis) && needed.contains(&s.name)
        }
        crate::hir::HirItemKind::Enum(e) => {
            !is_public_vis(e.vis) && needed.contains(&e.name)
        }
        crate::hir::HirItemKind::Trait(t) => {
            !is_public_vis(t.vis) && needed.contains(&t.name)
        }
        crate::hir::HirItemKind::TypeAlias(t) => {
            !is_public_vis(t.vis) && needed.contains(&t.name)
        }
        _ => false,
    }
}

fn needed_private_type_names(
    items: &[crate::hir::HirItem],
    local_names: &[crate::intern::Symbol],
    public_type_names: &[crate::intern::Symbol],
    interner: &Interner,
    in_trait: bool,
) -> HashSet<crate::intern::Symbol> {
    let mut needed = HashSet::new();
    for item in items {
        if item_is_exported(item, in_trait, local_names, public_type_names, interner) {
            collect_signature_type_refs(item, &mut needed);
        }
    }

    needed = filter_local_type_refs(&needed, local_names);

    loop {
        let before = needed.len();
        for item in items {
            if !private_type_is_needed(item, &needed) {
                continue;
            }
            collect_signature_type_refs(item, &mut needed);
        }
        needed = filter_local_type_refs(&needed, local_names);
        if needed.len() == before {
            break;
        }
    }

    needed
}

fn filter_local_type_refs(
    names: &HashSet<crate::intern::Symbol>,
    local_names: &[crate::intern::Symbol],
) -> HashSet<crate::intern::Symbol> {
    let mut filtered = HashSet::new();
    for name in names.iter() {
        if local_names.contains(name) {
            filtered.insert(*name);
        }
    }
    filtered
}

fn collect_signature_type_refs(
    item: &crate::hir::HirItem,
    out: &mut HashSet<crate::intern::Symbol>,
) {
    use crate::hir::{HirItemKind, HirVariantFields};

    match &item.kind {
        HirItemKind::Fn(f) => {
            collect_type_refs_in_generics(&f.generics, out);
            for param in &f.params {
                collect_type_refs_in_ty(&param.ty, out);
            }
            if let Some(ret) = &f.ret_ty {
                collect_type_refs_in_ty(ret, out);
            }
        }
        HirItemKind::Struct(s) => {
            collect_type_refs_in_generics(&s.generics, out);
            for field in &s.fields {
                collect_type_refs_in_ty(&field.ty, out);
            }
        }
        HirItemKind::Enum(e) => {
            collect_type_refs_in_generics(&e.generics, out);
            for variant in &e.variants {
                match &variant.fields {
                    HirVariantFields::Tuple(tys) => {
                        for ty in tys {
                            collect_type_refs_in_ty(ty, out);
                        }
                    }
                    HirVariantFields::Struct(fields) => {
                        for field in fields {
                            collect_type_refs_in_ty(&field.ty, out);
                        }
                    }
                    HirVariantFields::Unit => {}
                }
            }
        }
        HirItemKind::Trait(t) => {
            collect_type_refs_in_generics(&t.generics, out);
            for bound in &t.supertraits {
                collect_type_refs_in_path(&bound.path, out);
            }
            for sub in &t.items {
                collect_signature_type_refs(sub, out);
            }
        }
        HirItemKind::TypeAlias(ta) => {
            collect_type_refs_in_generics(&ta.generics, out);
            if let Some(ty) = &ta.ty {
                collect_type_refs_in_ty(ty, out);
            }
        }
        HirItemKind::Const(c) => collect_type_refs_in_ty(&c.ty, out),
        HirItemKind::Static(s) => collect_type_refs_in_ty(&s.ty, out),
        HirItemKind::Impl(ib) => {
            collect_type_refs_in_generics(&ib.generics, out);
            if let Some(trait_ref) = &ib.trait_ref {
                collect_type_refs_in_path(trait_ref, out);
            }
            collect_type_refs_in_ty(&ib.self_ty, out);
            for sub in &ib.items {
                collect_signature_type_refs(sub, out);
            }
        }
        HirItemKind::Mod(_) | HirItemKind::Use(_) | HirItemKind::ExternBlock(_) => {}
    }
}

fn collect_type_refs_in_generics(
    generics: &crate::hir::HirGenerics,
    out: &mut HashSet<crate::intern::Symbol>,
) {
    for param in &generics.params {
        match param {
            crate::hir::HirGenericParam::Type(_, bounds, default, _) => {
                for bound in bounds {
                    collect_type_refs_in_path(&bound.path, out);
                }
                if let Some(default) = default {
                    collect_type_refs_in_ty(default, out);
                }
            }
            crate::hir::HirGenericParam::Const(_, ty, _) => collect_type_refs_in_ty(ty, out),
            crate::hir::HirGenericParam::Lifetime(_, _, _) => {}
        }
    }
}

fn collect_type_refs_in_ty(
    ty: &crate::hir::HirTy,
    out: &mut HashSet<crate::intern::Symbol>,
) {
    match ty {
        crate::hir::HirTy::Path(path) => collect_type_refs_in_path(path, out),
        crate::hir::HirTy::QualifiedPath(qpath) => {
            collect_type_refs_in_ty(&qpath.self_ty, out);
            if let Some(trait_path) = &qpath.trait_path {
                collect_type_refs_in_path(trait_path, out);
            }
            collect_type_refs_in_path(&qpath.path, out);
        }
        crate::hir::HirTy::Reference(_, inner, _, _)
        | crate::hir::HirTy::RawPtr(inner, _, _)
        | crate::hir::HirTy::Slice(inner, _) => collect_type_refs_in_ty(inner, out),
        crate::hir::HirTy::Tuple(tys, _) => {
            for ty in tys {
                collect_type_refs_in_ty(ty, out);
            }
        }
        crate::hir::HirTy::Array(inner, _, _) => collect_type_refs_in_ty(inner, out),
        crate::hir::HirTy::FnPtr(params, ret, _) => {
            for param in params {
                collect_type_refs_in_ty(param, out);
            }
            if let Some(ret) = ret {
                collect_type_refs_in_ty(ret, out);
            }
        }
        crate::hir::HirTy::DynTrait(bounds, _) => {
            for bound in bounds {
                collect_type_refs_in_path(&bound.path, out);
            }
        }
        crate::hir::HirTy::MacroCall(_, _)
        | crate::hir::HirTy::Infer(_)
        | crate::hir::HirTy::Never(_) => {}
    }
}

fn collect_type_refs_in_path(
    path: &crate::hir::HirPath,
    out: &mut HashSet<crate::intern::Symbol>,
) {
    if path.segments.len() == 1 {
        out.insert(path.segments[0].ident);
    }
    for segment in &path.segments {
        if let Some(args) = &segment.args {
            for arg in &args.args {
                match arg {
                    crate::hir::HirGenericArg::Type(ty)
                    | crate::hir::HirGenericArg::AssocTypeBinding(_, ty) => {
                        collect_type_refs_in_ty(ty, out);
                    }
                    crate::hir::HirGenericArg::Const(_)
                    | crate::hir::HirGenericArg::Lifetime(_) => {}
                }
            }
        }
    }
}

fn needed_private_const_names(
    items: &[crate::hir::HirItem],
    local_names: &[crate::intern::Symbol],
    public_type_names: &[crate::intern::Symbol],
    interner: &Interner,
    in_trait: bool,
) -> HashSet<crate::intern::Symbol> {
    let mut needed = HashSet::new();
    for item in items {
        if item_is_exported(item, in_trait, local_names, public_type_names, interner) {
            collect_signature_const_refs(item, &mut needed);
        }
    }

    loop {
        let before = needed.len();
        for item in items {
            let crate::hir::HirItemKind::Const(c) = &item.kind else {
                continue;
            };
            if is_public_vis(c.vis) || !needed.contains(&c.name) {
                continue;
            }
            collect_const_refs_in_ty(&c.ty, &mut needed);
            if let Some(value) = &c.value {
                collect_const_refs_in_expr(value, &mut needed);
            }
        }
        if needed.len() == before {
            break;
        }
    }

    needed
}

fn collect_signature_const_refs(
    item: &crate::hir::HirItem,
    out: &mut HashSet<crate::intern::Symbol>,
) {
    use crate::hir::{HirItemKind, HirVariantFields};

    match &item.kind {
        HirItemKind::Fn(f) => {
            collect_const_refs_in_generics(&f.generics, out);
            for param in &f.params {
                collect_const_refs_in_ty(&param.ty, out);
            }
            if let Some(ret) = &f.ret_ty {
                collect_const_refs_in_ty(ret, out);
            }
        }
        HirItemKind::Struct(s) => {
            collect_const_refs_in_generics(&s.generics, out);
            for field in &s.fields {
                if is_public_vis(field.vis) {
                    collect_const_refs_in_ty(&field.ty, out);
                }
            }
        }
        HirItemKind::Enum(e) => {
            collect_const_refs_in_generics(&e.generics, out);
            for variant in &e.variants {
                match &variant.fields {
                    HirVariantFields::Tuple(tys) => {
                        for ty in tys {
                            collect_const_refs_in_ty(ty, out);
                        }
                    }
                    HirVariantFields::Struct(fields) => {
                        for field in fields {
                            collect_const_refs_in_ty(&field.ty, out);
                        }
                    }
                    HirVariantFields::Unit => {}
                }
                if let Some(discriminant) = &variant.discriminant {
                    collect_const_refs_in_expr(discriminant, out);
                }
            }
        }
        HirItemKind::Trait(t) => {
            collect_const_refs_in_generics(&t.generics, out);
            for bound in &t.supertraits {
                collect_const_refs_in_path(&bound.path, out);
            }
            for sub in &t.items {
                collect_signature_const_refs(sub, out);
            }
        }
        HirItemKind::TypeAlias(ta) => {
            collect_const_refs_in_generics(&ta.generics, out);
            if let Some(ty) = &ta.ty {
                collect_const_refs_in_ty(ty, out);
            }
        }
        HirItemKind::Const(c) => {
            collect_const_refs_in_ty(&c.ty, out);
            if let Some(value) = &c.value {
                collect_const_refs_in_expr(value, out);
            }
        }
        HirItemKind::Static(s) => {
            collect_const_refs_in_ty(&s.ty, out);
        }
        HirItemKind::Impl(ib) => {
            collect_const_refs_in_generics(&ib.generics, out);
            if let Some(trait_ref) = &ib.trait_ref {
                collect_const_refs_in_path(trait_ref, out);
            }
            collect_const_refs_in_ty(&ib.self_ty, out);
            for sub in &ib.items {
                collect_signature_const_refs(sub, out);
            }
        }
        HirItemKind::Mod(_) | HirItemKind::Use(_) | HirItemKind::ExternBlock(_) => {}
    }
}

fn collect_const_refs_in_generics(
    generics: &crate::hir::HirGenerics,
    out: &mut HashSet<crate::intern::Symbol>,
) {
    for param in &generics.params {
        match param {
            crate::hir::HirGenericParam::Type(_, bounds, default, _) => {
                for bound in bounds {
                    collect_const_refs_in_path(&bound.path, out);
                }
                if let Some(default) = default {
                    collect_const_refs_in_ty(default, out);
                }
            }
            crate::hir::HirGenericParam::Const(_, ty, _) => collect_const_refs_in_ty(ty, out),
            crate::hir::HirGenericParam::Lifetime(_, _, _) => {}
        }
    }
}

fn collect_const_refs_in_ty(
    ty: &crate::hir::HirTy,
    out: &mut HashSet<crate::intern::Symbol>,
) {
    match ty {
        crate::hir::HirTy::Path(path) => collect_const_refs_in_path(path, out),
        crate::hir::HirTy::QualifiedPath(qpath) => {
            collect_const_refs_in_ty(&qpath.self_ty, out);
            if let Some(trait_path) = &qpath.trait_path {
                collect_const_refs_in_path(trait_path, out);
            }
            collect_const_refs_in_path(&qpath.path, out);
        }
        crate::hir::HirTy::Reference(_, inner, _, _)
        | crate::hir::HirTy::RawPtr(inner, _, _)
        | crate::hir::HirTy::Slice(inner, _) => collect_const_refs_in_ty(inner, out),
        crate::hir::HirTy::Tuple(tys, _) => {
            for ty in tys {
                collect_const_refs_in_ty(ty, out);
            }
        }
        crate::hir::HirTy::Array(inner, len, _) => {
            collect_const_refs_in_ty(inner, out);
            collect_const_refs_in_expr(len, out);
        }
        crate::hir::HirTy::FnPtr(params, ret, _) => {
            for param in params {
                collect_const_refs_in_ty(param, out);
            }
            if let Some(ret) = ret {
                collect_const_refs_in_ty(ret, out);
            }
        }
        crate::hir::HirTy::DynTrait(bounds, _) => {
            for bound in bounds {
                collect_const_refs_in_path(&bound.path, out);
            }
        }
        crate::hir::HirTy::MacroCall(_, _)
        | crate::hir::HirTy::Infer(_)
        | crate::hir::HirTy::Never(_) => {}
    }
}

fn collect_const_refs_in_path(
    path: &crate::hir::HirPath,
    out: &mut HashSet<crate::intern::Symbol>,
) {
    if path.segments.len() == 1 {
        out.insert(path.segments[0].ident);
    }
    for segment in &path.segments {
        if let Some(args) = &segment.args {
            for arg in &args.args {
                match arg {
                    crate::hir::HirGenericArg::Type(ty) => collect_const_refs_in_ty(ty, out),
                    crate::hir::HirGenericArg::AssocTypeBinding(_, ty) => collect_const_refs_in_ty(ty, out),
                    crate::hir::HirGenericArg::Const(expr) => collect_const_refs_in_expr(expr, out),
                    crate::hir::HirGenericArg::Lifetime(_) => {}
                }
            }
        }
    }
}

fn collect_const_refs_in_expr(
    expr: &crate::hir::HirExpr,
    out: &mut HashSet<crate::intern::Symbol>,
) {
    use crate::hir::HirExprKind;

    match &expr.kind {
        HirExprKind::Path(path) => {
            if path.segments.len() == 1 {
                out.insert(path.segments[0].ident);
            }
            collect_const_refs_in_path(path, out);
        }
        HirExprKind::QualifiedPath(qpath) => {
            collect_const_refs_in_ty(&qpath.self_ty, out);
            if let Some(trait_path) = &qpath.trait_path {
                collect_const_refs_in_path(trait_path, out);
            }
            collect_const_refs_in_path(&qpath.path, out);
        }
        HirExprKind::Binary(_, lhs, rhs)
        | HirExprKind::Assign(lhs, rhs)
        | HirExprKind::AssignOp(_, lhs, rhs)
        | HirExprKind::Index(lhs, rhs) => {
            collect_const_refs_in_expr(lhs, out);
            collect_const_refs_in_expr(rhs, out);
        }
        HirExprKind::Unary(_, inner)
        | HirExprKind::Field(inner, _)
        | HirExprKind::Return(Some(inner))
        | HirExprKind::Break(_, Some(inner))
        | HirExprKind::Ref(inner, _)
        | HirExprKind::RawRef(inner, _)
        | HirExprKind::Deref(inner)
        | HirExprKind::Paren(inner)
        | HirExprKind::Try(inner) => collect_const_refs_in_expr(inner, out),
        HirExprKind::Call(callee, args) => {
            collect_const_refs_in_expr(callee, out);
            for arg in args {
                collect_const_refs_in_expr(arg, out);
            }
        }
        HirExprKind::MethodCall(receiver, _, tys, args) => {
            collect_const_refs_in_expr(receiver, out);
            for ty in tys {
                collect_const_refs_in_ty(ty, out);
            }
            for arg in args {
                collect_const_refs_in_expr(arg, out);
            }
        }
        HirExprKind::If(cond, then_block, else_expr) => {
            collect_const_refs_in_expr(cond, out);
            collect_const_refs_in_block(then_block, out);
            if let Some(else_expr) = else_expr {
                collect_const_refs_in_expr(else_expr, out);
            }
        }
        HirExprKind::Match(scrutinee, arms) => {
            collect_const_refs_in_expr(scrutinee, out);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_const_refs_in_expr(guard, out);
                }
                collect_const_refs_in_expr(&arm.body, out);
            }
        }
        HirExprKind::Closure(params, ret, body, _) => {
            for param in params {
                collect_const_refs_in_ty(&param.ty, out);
            }
            if let Some(ret) = ret {
                collect_const_refs_in_ty(ret, out);
            }
            collect_const_refs_in_expr(body, out);
        }
        HirExprKind::Cast(inner, ty) => {
            collect_const_refs_in_expr(inner, out);
            collect_const_refs_in_ty(ty, out);
        }
        HirExprKind::Struct(path, fields, base) => {
            collect_const_refs_in_path(path, out);
            for field in fields {
                collect_const_refs_in_expr(&field.value, out);
            }
            if let Some(base) = base {
                collect_const_refs_in_expr(base, out);
            }
        }
        HirExprKind::Tuple(items) | HirExprKind::Array(items) => {
            for item in items {
                collect_const_refs_in_expr(item, out);
            }
        }
        HirExprKind::ArrayRepeat(value, count) => {
            collect_const_refs_in_expr(value, out);
            collect_const_refs_in_expr(count, out);
        }
        HirExprKind::Range(start, end, _) => {
            if let Some(start) = start {
                collect_const_refs_in_expr(start, out);
            }
            if let Some(end) = end {
                collect_const_refs_in_expr(end, out);
            }
        }
        HirExprKind::Block(block) | HirExprKind::Loop(block, _) | HirExprKind::Unsafe(block) => {
            collect_const_refs_in_block(block, out);
        }
        HirExprKind::For(_, iter, body, _) => {
            collect_const_refs_in_expr(iter, out);
            collect_const_refs_in_block(body, out);
        }
        HirExprKind::InlineAsm(asm) => {
            for operand in &asm.operands {
                match operand {
                    crate::hir::HirAsmOperand::In { expr, .. }
                    | crate::hir::HirAsmOperand::InOut { expr, .. } => {
                        collect_const_refs_in_expr(expr, out);
                    }
                    crate::hir::HirAsmOperand::Out { expr, .. } => {
                        if let Some(expr) = expr {
                            collect_const_refs_in_expr(expr, out);
                        }
                    }
                }
            }
        }
        HirExprKind::Lit(_)
        | HirExprKind::Return(None)
        | HirExprKind::Break(_, None)
        | HirExprKind::Continue(_) => {}
    }
}

fn collect_const_refs_in_block(
    block: &crate::hir::HirBlock,
    out: &mut HashSet<crate::intern::Symbol>,
) {
    for stmt in &block.stmts {
        match stmt {
            crate::hir::HirStmt::Let(_, _, ty, init, _) => {
                if let Some(ty) = ty {
                    collect_const_refs_in_ty(ty, out);
                }
                if let Some(init) = init {
                    collect_const_refs_in_expr(init, out);
                }
            }
            crate::hir::HirStmt::Expr(expr) | crate::hir::HirStmt::Semi(expr, _) => {
                collect_const_refs_in_expr(expr, out);
            }
            crate::hir::HirStmt::Item(item) => collect_signature_const_refs(item, out),
        }
    }
}

fn use_item_supports_interface(
    use_tree: &crate::hir::HirUseTree,
    local_names: &[crate::intern::Symbol],
    interner: &Interner,
) -> bool {
    if is_public_vis(use_tree.vis) {
        return true;
    }
    let Some(&root) = use_tree.path.first() else {
        return false;
    };
    let root_name = interner.resolve(root);
    if root_name == "core" || root_name == "alloc" {
        return true;
    }
    if root_name == "crate" || root_name == "self" || root_name == "super" {
        return true;
    }
    if !local_names.iter().any(|name| *name == root) {
        return true;
    }
    local_names.iter().any(|name| *name == root)
}

fn render_generics(generics: &crate::hir::HirGenerics, interner: &Interner) -> String {
    if generics.params.is_empty() {
        return String::new();
    }
    let mut out = String::from("<");
    for (idx, param) in generics.params.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        out.push_str(&render_generic_param(param, interner));
    }
    out.push('>');
    out
}

fn render_generic_param(param: &crate::hir::HirGenericParam, interner: &Interner) -> String {
    match param {
        crate::hir::HirGenericParam::Type(name, bounds, default, _) => {
            let mut out = interner.resolve(*name).to_string();
            if !bounds.is_empty() {
                out.push_str(": ");
                for (idx, bound) in bounds.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(" + ");
                    }
                    out.push_str(&render_path(&bound.path, interner));
                }
            }
            if let Some(default) = default {
                out.push_str(" = ");
                out.push_str(&render_ty(default, interner));
            }
            out
        }
        crate::hir::HirGenericParam::Lifetime(name, bounds, _) => {
            let mut out = render_lifetime(*name, interner);
            if !bounds.is_empty() {
                out.push_str(": ");
                for (idx, bound) in bounds.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(" + ");
                    }
                    out.push_str(&render_lifetime(*bound, interner));
                }
            }
            out
        }
        crate::hir::HirGenericParam::Const(name, ty, _) => {
            format!("const {}: {}", interner.resolve(*name), render_ty(ty, interner))
        }
    }
}

fn render_ty(ty: &crate::hir::HirTy, interner: &Interner) -> String {
    match ty {
        crate::hir::HirTy::Path(path) => render_path(path, interner),
        crate::hir::HirTy::QualifiedPath(qpath) => {
            let mut out = String::from("<");
            out.push_str(&render_ty(&qpath.self_ty, interner));
            if let Some(trait_path) = &qpath.trait_path {
                out.push_str(" as ");
                out.push_str(&render_path(trait_path, interner));
            }
            out.push_str(">::");
            out.push_str(&render_path(&qpath.path, interner));
            out
        }
        crate::hir::HirTy::Reference(lifetime, inner, mutability, _) => {
            let mut out = String::from("&");
            if let Some(lifetime) = lifetime {
                out.push_str(&render_lifetime(*lifetime, interner));
                out.push(' ');
            }
            if *mutability == crate::ast::Mutability::Mut {
                out.push_str("mut ");
            }
            out.push_str(&render_ty(inner, interner));
            out
        }
        crate::hir::HirTy::RawPtr(inner, mutability, _) => {
            let mut out = String::from("*");
            out.push_str(if *mutability == crate::ast::Mutability::Mut { "mut " } else { "const " });
            out.push_str(&render_ty(inner, interner));
            out
        }
        crate::hir::HirTy::Tuple(tys, _) => {
            let mut out = String::from("(");
            for (idx, ty) in tys.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                out.push_str(&render_ty(ty, interner));
            }
            if tys.len() == 1 {
                out.push(',');
            }
            out.push(')');
            out
        }
        crate::hir::HirTy::Array(inner, len, _) => {
            format!("[{}; {}]", render_ty(inner, interner), render_expr(len, interner))
        }
        crate::hir::HirTy::Slice(inner, _) => format!("[{}]", render_ty(inner, interner)),
        crate::hir::HirTy::FnPtr(params, ret, _) => {
            let mut out = String::from("fn(");
            for (idx, param) in params.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                out.push_str(&render_ty(param, interner));
            }
            out.push(')');
            if let Some(ret) = ret {
                out.push_str(" -> ");
                out.push_str(&render_ty(ret, interner));
            }
            out
        }
        crate::hir::HirTy::DynTrait(bounds, _) => {
            let rendered: Vec<String> = bounds.iter()
                .map(|b| render_path(&b.path, interner))
                .collect();
            format!("dyn {}", rendered.join(" + "))
        }
        crate::hir::HirTy::MacroCall(name, _) => {
            let mut out = String::new();
            out.push_str(interner.resolve(*name));
            out.push_str("![..]");
            out
        }
        crate::hir::HirTy::Infer(_) => "_".to_string(),
        crate::hir::HirTy::Never(_) => "!".to_string(),
    }
}

fn render_path(path: &crate::hir::HirPath, interner: &Interner) -> String {
    let mut out = String::new();
    for (idx, segment) in path.segments.iter().enumerate() {
        if idx > 0 {
            out.push_str("::");
        }
        out.push_str(interner.resolve(segment.ident));
        if let Some(args) = &segment.args {
            out.push('<');
            for (arg_idx, arg) in args.args.iter().enumerate() {
                if arg_idx > 0 {
                    out.push_str(", ");
                }
                match arg {
                    crate::hir::HirGenericArg::Type(ty) => out.push_str(&render_ty(ty, interner)),
                    crate::hir::HirGenericArg::AssocTypeBinding(name, ty) => {
                        out.push_str(interner.resolve(*name));
                        out.push_str(" = ");
                        out.push_str(&render_ty(ty, interner));
                    }
                    crate::hir::HirGenericArg::Lifetime(sym) => out.push_str(&render_lifetime(*sym, interner)),
                    crate::hir::HirGenericArg::Const(expr) => out.push_str(&render_expr(expr, interner)),
                }
            }
            out.push('>');
        }
    }
    out
}

fn render_lifetime(sym: crate::intern::Symbol, interner: &Interner) -> String {
    let name = interner.resolve(sym);
    if name.starts_with('\'') {
        name.to_string()
    } else {
        format!("'{}", name)
    }
}

fn render_use_tree(out: &mut String, use_tree: &crate::hir::HirUseTree, interner: &Interner) {
    for (idx, seg) in use_tree.path.iter().enumerate() {
        if idx > 0 {
            out.push_str("::");
        }
        out.push_str(interner.resolve(*seg));
    }
    match &use_tree.kind {
        crate::hir::HirUseTreeKind::Simple(alias) => {
            if let Some(alias) = alias {
                out.push_str(" as ");
                out.push_str(interner.resolve(*alias));
            }
        }
        crate::hir::HirUseTreeKind::Glob => out.push_str("::*"),
        crate::hir::HirUseTreeKind::Nested(trees) => {
            if !use_tree.path.is_empty() {
                out.push_str("::");
            }
            out.push('{');
            for (idx, tree) in trees.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                render_use_tree(out, tree, interner);
            }
            out.push('}');
        }
    }
}

fn render_expr(expr: &crate::hir::HirExpr, interner: &Interner) -> String {
    match &expr.kind {
        crate::hir::HirExprKind::Lit(lit) => match lit {
            crate::ast::Literal::Int(v) => v.to_string(),
            crate::ast::Literal::Float(v) => format!("{}", v),
            crate::ast::Literal::String(s) => format!("{:?}", s),
            crate::ast::Literal::Char(c) => format!("{:?}", c),
            crate::ast::Literal::Bool(v) => v.to_string(),
            crate::ast::Literal::ByteString(bytes) => {
                let parts: Vec<String> = bytes.iter().map(|b| b.to_string()).collect();
                format!("b\"{}\"", parts.join(""))
            }
        },
        crate::hir::HirExprKind::Path(path) => render_path(path, interner),
        crate::hir::HirExprKind::Unary(op, inner) => {
            let op_str = match op {
                crate::ast::UnOp::Neg => "-",
                crate::ast::UnOp::Not => "!",
                crate::ast::UnOp::Deref => "*",
            };
            format!("{}{}", op_str, render_expr(inner, interner))
        }
        crate::hir::HirExprKind::Binary(op, lhs, rhs) => {
            let op_str = match op {
                crate::ast::BinOp::Add => "+",
                crate::ast::BinOp::Sub => "-",
                crate::ast::BinOp::Mul => "*",
                crate::ast::BinOp::Div => "/",
                crate::ast::BinOp::Rem => "%",
                crate::ast::BinOp::BitAnd => "&",
                crate::ast::BinOp::BitOr => "|",
                crate::ast::BinOp::BitXor => "^",
                crate::ast::BinOp::Shl => "<<",
                crate::ast::BinOp::Shr => ">>",
                crate::ast::BinOp::Eq => "==",
                crate::ast::BinOp::Ne => "!=",
                crate::ast::BinOp::Lt => "<",
                crate::ast::BinOp::Le => "<=",
                crate::ast::BinOp::Gt => ">",
                crate::ast::BinOp::Ge => ">=",
                crate::ast::BinOp::And => "&&",
                crate::ast::BinOp::Or => "||",
            };
            format!("({} {} {})", render_expr(lhs, interner), op_str, render_expr(rhs, interner))
        }
        crate::hir::HirExprKind::Cast(inner, ty) => {
            format!("{} as {}", render_expr(inner, interner), render_ty(ty, interner))
        }
        crate::hir::HirExprKind::Paren(inner) => format!("({})", render_expr(inner, interner)),
        crate::hir::HirExprKind::Tuple(items) => {
            let mut out = String::from("(");
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                out.push_str(&render_expr(item, interner));
            }
            if items.len() == 1 {
                out.push(',');
            }
            out.push(')');
            out
        }
        crate::hir::HirExprKind::Array(items) => {
            let mut out = String::from("[");
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                out.push_str(&render_expr(item, interner));
            }
            out.push(']');
            out
        }
        crate::hir::HirExprKind::ArrayRepeat(value, count) => {
            format!("[{}; {}]", render_expr(value, interner), render_expr(count, interner))
        }
        crate::hir::HirExprKind::Ref(inner, mutability) => {
            if *mutability == crate::ast::Mutability::Mut {
                format!("&mut {}", render_expr(inner, interner))
            } else {
                format!("&{}", render_expr(inner, interner))
            }
        }
        crate::hir::HirExprKind::RawRef(inner, mutability) => {
            if *mutability == crate::ast::Mutability::Mut {
                format!("&raw mut {}", render_expr(inner, interner))
            } else {
                format!("&raw const {}", render_expr(inner, interner))
            }
        }
        crate::hir::HirExprKind::Deref(inner) => format!("*{}", render_expr(inner, interner)),
        _ => "0".to_string(),
    }
}

fn is_tuple_fields(fields: &[crate::hir::HirFieldDef], interner: &Interner) -> bool {
    for (idx, field) in fields.iter().enumerate() {
        if interner.resolve(field.name) != idx.to_string() {
            return false;
        }
    }
    true
}

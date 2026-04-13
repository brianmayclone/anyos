use crate::prelude::*;
use crate::codegen::emit::CodeEmitter;
use crate::codegen::regalloc;
use crate::codegen::x86asm::RelocKind;
use crate::diagnostics::Diagnostic;
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
    let no_main = krate.attrs.iter().any(|a| {
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

    // 1c. Strip items that don't match cfg predicates
    crate::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);

    // 2. Expand macros (pass env_vars and cfg_ctx for built-in macros)
    expand_macros(&mut krate, &mut interner);

    // 2b. Resolve multi-file modules (mod foo;)
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
    let _loaded_modules = crate::loader::resolve_modules(&mut krate, &src_dir, &mut interner, &loader);
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
        let result = check_borrows(body, &interner, &typeck_result.struct_defs);
        if !result.errors.is_empty() {
            return Err(result.errors);
        }
    }

    // 8. Optimize MIR
    if options.opt_level > 0 {
        for body in &mut mir_bodies {
            optimize(body);
        }
    }

    // 9. Build struct size map: DefId → size in 8-byte slots
    //    Uses typeck struct_defs for field types to compute accurate sizes.
    let struct_sizes = {
        use crate::hir::{HirItemKind, HirVariantFields, HirItem};
        use crate::codegen::regalloc::ty_size;

        // First pass: collect field counts as fallback for structs not in typeck
        let mut map: anyos_std::collections::HashMap<crate::hir::DefId, usize> = anyos_std::collections::HashMap::new();
        fn collect_struct_counts(items: &[HirItem], map: &mut anyos_std::collections::HashMap<crate::hir::DefId, usize>) {
            for item in items {
                match &item.kind {
                    HirItemKind::Struct(s) => { map.insert(s.def_id, s.fields.len()); }
                    HirItemKind::Enum(e) => {
                        let max_fields = e.variants.iter().map(|v| match &v.fields {
                            HirVariantFields::Unit => 0,
                            HirVariantFields::Tuple(tys) => tys.len(),
                            HirVariantFields::Struct(fields) => fields.len(),
                        }).max().unwrap_or(0);
                        if max_fields > 0 { map.insert(e.def_id, 1 + max_fields); }
                    }
                    HirItemKind::Mod(m) => {
                        if let Some(sub_items) = &m.items { collect_struct_counts(sub_items, map); }
                    }
                    _ => {}
                }
            }
        }
        collect_struct_counts(&hir.items, &mut map);

        // Second pass: compute accurate sizes from field types
        // We compute size as sum of field sizes (in 8-byte slots)
        for (def_id, fields) in &typeck_result.struct_defs {
            let total_bytes: i32 = fields.iter()
                .map(|(_, ty)| ty_size(ty, &map))
                .sum();
            let slots = (total_bytes / 8).max(1) as usize;
            map.insert(*def_id, slots);
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
            let obj = codegen_to_object_with_statics(&mir_bodies, &interner, &struct_sizes, &field_offsets, &static_data);
            Ok(elf::write_object(&obj))
        }
        EmitKind::Exe => {
            let obj = codegen_to_object_with_statics(&mir_bodies, &interner, &struct_sizes, &field_offsets, &static_data);
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
                };
                let exe = link::link_ext(&all_objects, &options.output, no_main_flag, &link_opts);
                Ok(exe)
            } else {
                let exe = link::link(&all_objects, &options.output, no_main_flag);
                Ok(exe)
            }
        }
        EmitKind::Rlib => {
            let obj = codegen_to_object_with_statics(&mir_bodies, &interner, &struct_sizes, &field_offsets, &static_data);
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
                interface_source: build_public_interface_source(&hir, &interner),
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

fn codegen_to_object(bodies: &[MirBody], interner: &Interner, struct_sizes: &regalloc::StructSizes, field_offsets: &regalloc::StructFieldOffsets) -> ObjectFile {
    codegen_to_object_with_statics(bodies, interner, struct_sizes, field_offsets, &[])
}

/// Static data entry: (symbol_name, initial_bytes)
pub struct StaticData {
    pub name: String,
    pub data: Vec<u8>,
}

fn codegen_to_object_with_statics(bodies: &[MirBody], interner: &Interner, struct_sizes: &regalloc::StructSizes, field_offsets: &regalloc::StructFieldOffsets, statics: &[StaticData]) -> ObjectFile {
    let mut text_data = Vec::new();
    let mut symbols = Vec::new();
    let mut relocations = Vec::new();

    for body in bodies {
        let alloc = regalloc::allocate(body, struct_sizes);
        let (code, relocs) = CodeEmitter::emit_fn(body, &alloc, interner, field_offsets);

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

    for (name, code) in &stubs {
        let offset = text_data.len() as u64;
        let size = code.len() as u64;
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

    elf::ObjectFile {
        sections: vec![elf::Section {
            name: ".text".to_string(),
            data: text_data,
            sh_type: 1,         // SHT_PROGBITS
            sh_flags: 0x2 | 0x4, // SHF_ALLOC | SHF_EXECINSTR
            sh_addralign: 16,
        }],
        symbols,
        relocations: Vec::new(),
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

        let wrapper_src = format!("mod {} {{\n{}\n}}", ext.name, meta.interface_source);
        let mut parser = Parser::new(&wrapper_src, interner);
        let mut iface_krate = parser.parse_crate();
        krate.items.append(&mut iface_krate.items);
    }
}

fn build_public_interface_source(hir: &crate::hir::HirCrate, interner: &Interner) -> String {
    let mut out = String::new();
    render_items(&mut out, &hir.items, interner, 0, false);
    out
}

fn render_items(
    out: &mut String,
    items: &[crate::hir::HirItem],
    interner: &Interner,
    indent: usize,
    in_trait: bool,
) {
    for item in items {
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
                out.push('(');
                for (idx, field) in s.fields.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&render_visibility(field.vis));
                    out.push_str(&render_ty(&field.ty, interner));
                }
                out.push_str(");\n");
            } else {
                out.push_str(" {\n");
                for field in &s.fields {
                    out.push_str(&"    ".repeat(indent + 1));
                    out.push_str(&render_visibility(field.vis));
                    out.push_str(interner.resolve(field.name));
                    out.push_str(": ");
                    out.push_str(&render_ty(&field.ty, interner));
                    out.push_str(",\n");
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
            render_items(out, &ib.items, interner, indent + 1, false);
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
            out.push_str(&ind);
            out.push_str("extern ");
            if let Some(abi) = &eb.abi {
                out.push('"');
                out.push_str(abi);
                out.push('"');
                out.push(' ');
            }
            out.push_str("{\n");
            for sub in &eb.items {
                match &sub.kind {
                    HirItemKind::Fn(_) | HirItemKind::Static(_) => {
                        render_item(out, sub, interner, indent + 1, false);
                    }
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
    }
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
            let mut out = interner.resolve(*name).to_string();
            if !bounds.is_empty() {
                out.push_str(": ");
                for (idx, bound) in bounds.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(" + ");
                    }
                    out.push_str(interner.resolve(*bound));
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
        crate::hir::HirTy::Reference(lifetime, inner, mutability, _) => {
            let mut out = String::from("&");
            if let Some(lifetime) = lifetime {
                out.push_str(interner.resolve(*lifetime));
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
        crate::hir::HirTy::DynTrait(path, _) => format!("dyn {}", render_path(path, interner)),
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
                    crate::hir::HirGenericArg::Lifetime(sym) => out.push_str(interner.resolve(*sym)),
                    crate::hir::HirGenericArg::Const(expr) => out.push_str(&render_expr(expr, interner)),
                }
            }
            out.push('>');
        }
    }
    out
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
            out.push_str("::{");
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

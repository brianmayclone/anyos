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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitKind {
    Exe,
    Obj,
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

    // 2. Expand macros
    expand_macros(&mut krate, &mut interner);

    // 3. Lower to HIR
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    drop(lower_ctx);

    // 4. Resolve names
    let mut resolver = Resolver::new(&interner);
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
    let mut mir_bodies = MirBuilder::build_crate(&interner, &resolve_result, &typeck_result, &hir);

    // 6b. Monomorphize generic functions
    let mut mir_bodies = monomorphize(mir_bodies, &typeck_result, &mut interner, &hir, &resolve_result);

    // 7. Borrow check
    for body in &mir_bodies {
        let result = check_borrows(body, &interner);
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

    // 9. Build struct size map from HIR (includes enum tagged union sizes)
    let struct_sizes = {
        use crate::hir::{HirItemKind, HirVariantFields, HirItem};
        fn collect_struct_sizes(items: &[HirItem], map: &mut std::collections::HashMap<crate::hir::DefId, usize>) {
            for item in items {
                match &item.kind {
                    HirItemKind::Struct(s) => {
                        map.insert(s.def_id, s.fields.len());
                    }
                    HirItemKind::Enum(e) => {
                        let max_fields = e.variants.iter().map(|v| match &v.fields {
                            HirVariantFields::Unit => 0,
                            HirVariantFields::Tuple(tys) => tys.len(),
                            HirVariantFields::Struct(fields) => fields.len(),
                        }).max().unwrap_or(0);
                        if max_fields > 0 {
                            map.insert(e.def_id, 1 + max_fields);
                        }
                    }
                    HirItemKind::Mod(m) => {
                        if let Some(sub_items) = &m.items {
                            collect_struct_sizes(sub_items, map);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut map = std::collections::HashMap::new();
        collect_struct_sizes(&hir.items, &mut map);
        map
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
            let obj = codegen_to_object_with_statics(&mir_bodies, &interner, &struct_sizes, &static_data);
            Ok(elf::write_object(&obj))
        }
        EmitKind::Exe => {
            let obj = codegen_to_object_with_statics(&mir_bodies, &interner, &struct_sizes, &static_data);
            let obj_bytes = elf::write_object(&obj);
            let no_main_flag = no_main || options.crate_type == CrateType::StaticLib;
            let exe = link::link(&[obj_bytes], &options.output, no_main_flag);
            Ok(exe)
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

fn codegen_to_object(bodies: &[MirBody], interner: &Interner, struct_sizes: &regalloc::StructSizes) -> ObjectFile {
    codegen_to_object_with_statics(bodies, interner, struct_sizes, &[])
}

/// Static data entry: (symbol_name, initial_bytes)
pub struct StaticData {
    pub name: String,
    pub data: Vec<u8>,
}

fn codegen_to_object_with_statics(bodies: &[MirBody], interner: &Interner, struct_sizes: &regalloc::StructSizes, statics: &[StaticData]) -> ObjectFile {
    let mut text_data = Vec::new();
    let mut symbols = Vec::new();
    let mut relocations = Vec::new();

    for body in bodies {
        let alloc = regalloc::allocate(body, struct_sizes);
        let (code, relocs) = CodeEmitter::emit_fn(body, &alloc, interner);

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

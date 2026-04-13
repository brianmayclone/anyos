use anyrc::cfg::CfgContext;
use anyrc::diagnostics::{Diagnostic, SourceMap};
use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind, ExternCrateSpec};
use anyrc::hir_lower::LoweringContext;
use anyrc::intern::Interner;
use anyrc::loader::{self, FileLoader};
use anyrc::macros::expand_macros;
use anyrc::parser::Parser;
use anyrc::resolve::Resolver;
use anyrc::typeck::TypeChecker;
use std::fs;
use std::path::{Path, PathBuf};

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
    extern_crates: Vec<ExternCrateSpec>,
) -> loader::CrateMetadata {
    let repo_root = repo_root();
    let input_path = repo_root.join(rel_src);
    let src = fs::read_to_string(&input_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", input_path.display(), e));
    let output = format!("/tmp/{}_anyrc_debug.rlib", crate_name);
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

fn inject_extern_crate_interfaces(
    krate: &mut anyrc::ast::Crate,
    extern_crates: &[ExternCrateSpec],
    interner: &mut Interner,
    loader: &dyn FileLoader,
) {
    for ext in extern_crates {
        let Some(rlib_data) = loader.read_file_bytes(&ext.rlib_path) else {
            continue;
        };
        let Some((_, meta)) = loader::unpack_rlib(&rlib_data) else {
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

fn collect_sources(
    rel_src: &str,
    src_dir: &str,
    extern_crates: &[ExternCrateSpec],
) -> Vec<(String, String)> {
    let repo_root = repo_root();
    let input_path = repo_root.join(rel_src);
    let root_src = fs::read_to_string(&input_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", input_path.display(), e));

    let mut interner = Interner::new();
    let mut parser = Parser::new(&root_src, &mut interner);
    let mut krate = parser.parse_crate();
    let cfg_ctx = CfgContext::from_flags(&anyos_cfg_flags());
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    expand_macros(&mut krate, &mut interner);

    let loader = anyrc::loader::OsFileLoader;
    let src_dir = repo_root.join(src_dir);
    let loaded = anyrc::loader::resolve_modules(
        &mut krate,
        src_dir.to_str().expect("src dir"),
        &mut interner,
        &loader,
    );
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    inject_extern_crate_interfaces(&mut krate, extern_crates, &mut interner, &loader);

    let mut lower = LoweringContext::new(&mut interner);
    let _hir = lower.lower_crate(&krate);

    let mut sources = vec![(input_path.display().to_string(), root_src)];
    sources.extend(loaded.into_iter().map(|m| (m.path, m.source)));
    sources
}

fn debug_type_names(rel_src: &str, src_dir: &str, extern_crates: &[ExternCrateSpec]) {
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
    anyrc::loader::resolve_modules(
        &mut krate,
        src_dir.to_str().expect("src dir"),
        &mut interner,
        &loader,
    );
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    inject_extern_crate_interfaces(&mut krate, extern_crates, &mut interner, &loader);

    let mut lower = LoweringContext::new(&mut interner);
    let hir = lower.lower_crate(&krate);

    let mut resolver = Resolver::new(&mut interner);
    let resolve_result = resolver.resolve_crate(&hir);
    if !resolve_result.errors.is_empty() {
        println!("resolve failed with {} diagnostics", resolve_result.errors.len());
        return;
    }

    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let typeck_result = checker.check_crate(&hir);
    println!("type-name map for selected DefIds:");
    for def_id in [622u32, 729, 1190, 10124, 10709, 11086, 11563] {
        let key = anyrc::hir::DefId(def_id);
        let name = typeck_result
            .type_def_to_name
            .get(&key)
            .map(|sym| interner.resolve(*sym))
            .unwrap_or("<unknown>");
        println!("  DefId({def_id}) -> {name}");
    }
}

fn render_candidate(
    err: &Diagnostic,
    path: &str,
    source: &str,
) -> Option<String> {
    let start = err.span.start() as usize;
    let end = err.span.end() as usize;
    if end > source.len() || start > end {
        return None;
    }
    let sm = SourceMap::new(path.to_string(), source.to_string());
    let rendered = err.render(&sm);
    let snippet = source.get(start..end)?.replace('\n', "\\n");
    let line_text = sm.line_text(sm.line_col(err.span).0).trim();
    if !(snippet.contains('[')
        || snippet.contains("copy_from_slice")
        || snippet.contains("cmp")
        || line_text.contains('[')
        || line_text.contains("copy_from_slice")
        || line_text.contains("cmp")
        || line_text.contains("try_into"))
    {
        return None;
    }
    Some(format!("{rendered}\n   = snippet: {snippet}"))
}

fn main() {
    let libheap_meta = compile_repo_rlib("libheap", "libs/libheap/src/lib.rs", "libs/libheap/src", vec![]);
    println!("libheap interface bytes: {}", libheap_meta.interface_source.len());

    let libheap_rlib = String::from("/tmp/libheap_anyrc_debug.rlib");
    let repo_root = repo_root();
    let input_path = repo_root.join("libs/stdlib/src/lib.rs");
    let src = fs::read_to_string(&input_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", input_path.display(), e));
    let opts = CompileOptions {
        input: input_path.display().to_string(),
        output: String::from("/tmp/anyos_std_anyrc_debug.rlib"),
        emit: EmitKind::Rlib,
        opt_level: 0,
        crate_type: CrateType::Lib,
        crate_name: Some(String::from("anyos_std")),
        src_dir: Some(repo_root.join("libs/stdlib/src").display().to_string()),
        extern_crates: vec![ExternCrateSpec {
            name: String::from("libheap"),
            rlib_path: libheap_rlib,
        }],
        cfg_flags: anyos_cfg_flags(),
        linker_script: None,
        link_args: vec![],
        env_vars: vec![],
        features: vec![],
    };

    match compile(&src, &opts.input, &opts) {
        Ok(_) => {
            println!("anyos_std compiled successfully");
        }
        Err(errors) => {
            debug_type_names(
                "libs/stdlib/src/lib.rs",
                "libs/stdlib/src",
                &opts.extern_crates,
            );
            println!("compile failed with {} diagnostics", errors.len());
            for (idx, err) in errors.iter().enumerate() {
                println!("\n== diagnostic {} ==", idx + 1);
                println!("raw span: {}..{}", err.span.start(), err.span.end());
                println!("message: {}", err.message);
                let root_sm = SourceMap::new(opts.input.clone(), src.clone());
                println!("root render:\n{}", err.render(&root_sm));

                let sources = collect_sources(
                    "libs/stdlib/src/lib.rs",
                    "libs/stdlib/src",
                    &opts.extern_crates,
                );
                let mut matched = 0usize;
                for (path, source) in &sources {
                    if let Some(candidate) = render_candidate(err, path, source) {
                        println!("\n[candidate {}]\n{}", path, candidate);
                        matched += 1;
                        if matched >= 8 {
                            break;
                        }
                    }
                }
                if matched == 0 {
                    println!("no likely source candidates found");
                }
            }
        }
    }
}

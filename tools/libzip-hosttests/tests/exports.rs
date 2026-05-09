use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[test]
fn exports_def_contains_all_c_abi_symbols() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let source =
        fs::read_to_string(repo.join("libs/libzip/src/lib.rs")).expect("read libzip source");
    let exports =
        fs::read_to_string(repo.join("libs/libzip/exports.def")).expect("read libzip exports.def");

    let source_symbols = c_abi_symbols(&source);
    let exported_symbols = exported_symbols(&exports);
    let missing: Vec<_> = source_symbols
        .difference(&exported_symbols)
        .cloned()
        .collect();

    assert!(
        missing.is_empty(),
        "exports.def is missing C ABI symbols: {missing:?}"
    );
}

fn c_abi_symbols(source: &str) -> BTreeSet<String> {
    let mut symbols = BTreeSet::new();
    for line in source.lines() {
        let Some(pos) = line.find("pub extern \"C\" fn ") else {
            continue;
        };
        let rest = &line[pos + "pub extern \"C\" fn ".len()..];
        let name_end = rest.find('(').unwrap_or(rest.len());
        let name = rest[..name_end].trim();
        if name.starts_with("libzip_") {
            symbols.insert(name.to_string());
        }
    }
    symbols
}

fn exported_symbols(exports: &str) -> BTreeSet<String> {
    exports
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("libzip_"))
        .map(str::to_string)
        .collect()
}

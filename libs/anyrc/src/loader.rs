//! Multi-file module loader for anyrc.
//!
//! Resolves `mod foo;` declarations by loading source files from disk.
//! Supports both `foo.rs` and `foo/mod.rs` layout conventions.

use crate::prelude::*;
use crate::ast::{Crate, Item, ModDef};
use crate::intern::{Interner, Symbol};
use crate::parser::Parser;
use crate::macros::expand_macros;

/// Loaded module source: (module_path, source_code)
pub struct ModuleSource {
    pub path: String,
    pub source: String,
}

/// File system abstraction for loading module files.
/// On anyOS: uses anyos_std::fs, on tests: can be mocked.
pub trait FileLoader {
    fn read_file(&self, path: &str) -> Option<String>;
    fn read_file_bytes(&self, path: &str) -> Option<Vec<u8>>;
    fn file_exists(&self, path: &str) -> bool;
}

/// Default file loader using anyos_std::fs
pub struct OsFileLoader;

impl OsFileLoader {
    pub fn read_bytes(path: &str) -> Option<Vec<u8>> {
        let fd = anyos_std::fs::open(path, 0);
        if fd == u32::MAX {
            return None;
        }
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = anyos_std::fs::read(fd, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            data.extend_from_slice(&buf[..n as usize]);
        }
        anyos_std::fs::close(fd);
        Some(data)
    }
}

impl FileLoader for OsFileLoader {
    fn read_file(&self, path: &str) -> Option<String> {
        let data = OsFileLoader::read_bytes(path)?;
        alloc::string::String::from_utf8(data).ok()
    }

    fn read_file_bytes(&self, path: &str) -> Option<Vec<u8>> {
        OsFileLoader::read_bytes(path)
    }

    fn file_exists(&self, path: &str) -> bool {
        let fd = anyos_std::fs::open(path, 0);
        if fd == u32::MAX {
            false
        } else {
            anyos_std::fs::close(fd);
            true
        }
    }
}

/// Resolve all `mod foo;` declarations in a crate by loading files from disk.
/// `base_dir` is the directory containing the root source file (e.g., "src/").
///
/// Returns list of loaded module sources (for source map / diagnostics).
pub fn resolve_modules(
    krate: &mut Crate,
    base_dir: &str,
    interner: &mut Interner,
    loader: &dyn FileLoader,
) -> Vec<ModuleSource> {
    let mut loaded = Vec::new();
    resolve_items(&mut krate.items, base_dir, interner, loader, &mut loaded);
    loaded
}

fn resolve_items(
    items: &mut Vec<Item>,
    dir: &str,
    interner: &mut Interner,
    loader: &dyn FileLoader,
    loaded: &mut Vec<ModuleSource>,
) {
    for item in items.iter_mut() {
        if let Item::Mod(mod_def) = item {
            if mod_def.items.is_none() {
                // `mod foo;` — need to load from file
                let mod_name = interner.resolve(mod_def.name);

                // Try foo.rs first, then foo/mod.rs
                let path_rs = format!("{}/{}.rs", dir, mod_name);
                let path_mod = format!("{}/{}/mod.rs", dir, mod_name);
                let sub_dir;

                let source = if let Some(src) = loader.read_file(&path_rs) {
                    sub_dir = format!("{}/{}", dir, mod_name);
                    loaded.push(ModuleSource {
                        path: path_rs,
                        source: src.clone(),
                    });
                    src
                } else if let Some(src) = loader.read_file(&path_mod) {
                    sub_dir = format!("{}/{}", dir, mod_name);
                    loaded.push(ModuleSource {
                        path: path_mod,
                        source: src.clone(),
                    });
                    src
                } else {
                    // Module file not found — leave items as None, resolver will error
                    continue;
                };

                // Parse the loaded source
                let mut parser = Parser::new(&source, interner);
                let mut sub_crate = parser.parse_crate();
                expand_macros(&mut sub_crate, interner);

                // Recursively resolve sub-modules
                resolve_items(&mut sub_crate.items, &sub_dir, interner, loader, loaded);

                // Set the module's items
                mod_def.items = Some(sub_crate.items);
            } else {
                // Inline module `mod foo { ... }` — recurse into its items
                if let Some(ref mut sub_items) = mod_def.items {
                    let sub_dir = format!("{}/{}", dir, interner.resolve(mod_def.name));
                    resolve_items(sub_items, &sub_dir, interner, loader, loaded);
                }
            }
        }
    }
}

/// Metadata for a compiled crate (.rlib).
/// Stored alongside the object code so that downstream crates can resolve symbols.
#[derive(Clone)]
pub struct CrateMetadata {
    pub name: String,
    pub version: String,
    /// Exported public symbols: (name, kind)
    pub exports: Vec<ExportedSymbol>,
    /// Dependencies this crate requires
    pub deps: Vec<String>,
}

#[derive(Clone)]
pub struct ExportedSymbol {
    pub name: String,
    pub kind: ExportKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Function,
    Static,
    Type,
    Const,
}

/// Serialize crate metadata to bytes (simple binary format).
/// Format: [magic:4][name_len:2][name][version_len:2][version]
///         [dep_count:2][deps...][export_count:2][exports...]
pub fn serialize_metadata(meta: &CrateMetadata) -> Vec<u8> {
    let mut buf = Vec::new();
    // Magic: "ARCM" (anyrc metadata)
    buf.extend_from_slice(b"ARCM");
    // Name
    let name_bytes = meta.name.as_bytes();
    buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(name_bytes);
    // Version
    let ver_bytes = meta.version.as_bytes();
    buf.extend_from_slice(&(ver_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(ver_bytes);
    // Dependencies
    buf.extend_from_slice(&(meta.deps.len() as u16).to_le_bytes());
    for dep in &meta.deps {
        let dep_bytes = dep.as_bytes();
        buf.extend_from_slice(&(dep_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(dep_bytes);
    }
    // Exports
    buf.extend_from_slice(&(meta.exports.len() as u16).to_le_bytes());
    for exp in &meta.exports {
        let exp_bytes = exp.name.as_bytes();
        buf.extend_from_slice(&(exp_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(exp_bytes);
        buf.push(exp.kind as u8);
    }
    buf
}

/// Deserialize crate metadata from bytes.
pub fn deserialize_metadata(data: &[u8]) -> Option<CrateMetadata> {
    if data.len() < 4 || &data[0..4] != b"ARCM" {
        return None;
    }
    let mut pos = 4;

    let read_u16 = |data: &[u8], pos: &mut usize| -> Option<u16> {
        if *pos + 2 > data.len() { return None; }
        let v = u16::from_le_bytes(data[*pos..*pos + 2].try_into().ok()?);
        *pos += 2;
        Some(v)
    };

    let read_str = |data: &[u8], pos: &mut usize| -> Option<String> {
        let len = read_u16(data, pos)? as usize;
        if *pos + len > data.len() { return None; }
        let s = core::str::from_utf8(&data[*pos..*pos + len]).ok()?.to_string();
        *pos += len;
        Some(s)
    };

    let name = read_str(data, &mut pos)?;
    let version = read_str(data, &mut pos)?;

    let dep_count = read_u16(data, &mut pos)? as usize;
    let mut deps = Vec::new();
    for _ in 0..dep_count {
        deps.push(read_str(data, &mut pos)?);
    }

    let export_count = read_u16(data, &mut pos)? as usize;
    let mut exports = Vec::new();
    for _ in 0..export_count {
        let exp_name = read_str(data, &mut pos)?;
        if pos >= data.len() { return None; }
        let kind = match data[pos] {
            0 => ExportKind::Function,
            1 => ExportKind::Static,
            2 => ExportKind::Type,
            3 => ExportKind::Const,
            _ => return None,
        };
        pos += 1;
        exports.push(ExportedSymbol { name: exp_name, kind });
    }

    Some(CrateMetadata { name, version, exports, deps })
}

/// An .rlib file: object code + metadata packed together.
/// Layout: [obj_size:4][obj_bytes][meta_bytes]
pub fn pack_rlib(obj: &[u8], meta: &CrateMetadata) -> Vec<u8> {
    let meta_bytes = serialize_metadata(meta);
    let mut buf = Vec::with_capacity(4 + obj.len() + meta_bytes.len());
    buf.extend_from_slice(&(obj.len() as u32).to_le_bytes());
    buf.extend_from_slice(obj);
    buf.extend_from_slice(&meta_bytes);
    buf
}

/// Unpack an .rlib file into (object_bytes, metadata).
pub fn unpack_rlib(data: &[u8]) -> Option<(Vec<u8>, CrateMetadata)> {
    if data.len() < 4 { return None; }
    let obj_size = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    if 4 + obj_size > data.len() { return None; }
    let obj = data[4..4 + obj_size].to_vec();
    let meta = deserialize_metadata(&data[4 + obj_size..])?;
    Some((obj, meta))
}

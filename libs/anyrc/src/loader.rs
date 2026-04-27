//! Multi-file module loader for anyrc.
//!
//! Resolves `mod foo;` declarations by loading source files from disk.
//! Supports both `foo.rs` and `foo/mod.rs` layout conventions.

use crate::ast::{Crate, Delimiter, Item, ModDef, TokenTree};
use crate::intern::Interner;
use crate::lexer::TokenKind;
use crate::parser::Parser;
use crate::prelude::*;

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
    resolve_modules_with_env(krate, base_dir, interner, loader, &[])
}

pub fn resolve_modules_with_env(
    krate: &mut Crate,
    base_dir: &str,
    interner: &mut Interner,
    loader: &dyn FileLoader,
    env_vars: &[(String, String)],
) -> Vec<ModuleSource> {
    let mut loaded = Vec::new();
    resolve_items(
        &mut krate.items,
        base_dir,
        interner,
        loader,
        env_vars,
        &mut loaded,
    );
    loaded
}

/// Resolve item-position `include!("file.rs")` macros by parsing the included
/// file and splicing its items into the including module.
pub fn resolve_includes(
    krate: &mut Crate,
    base_dir: &str,
    interner: &mut Interner,
    loader: &dyn FileLoader,
) -> Vec<ModuleSource> {
    resolve_includes_with_env(krate, base_dir, interner, loader, &[])
}

pub fn resolve_includes_with_env(
    krate: &mut Crate,
    base_dir: &str,
    interner: &mut Interner,
    loader: &dyn FileLoader,
    env_vars: &[(String, String)],
) -> Vec<ModuleSource> {
    let mut loaded = Vec::new();
    resolve_includes_in_items(
        &mut krate.items,
        base_dir,
        interner,
        loader,
        env_vars,
        &mut loaded,
    );
    loaded
}

fn resolve_includes_in_items(
    items: &mut Vec<Item>,
    dir: &str,
    interner: &mut Interner,
    loader: &dyn FileLoader,
    env_vars: &[(String, String)],
    loaded: &mut Vec<ModuleSource>,
) {
    let mut i = 0;
    while i < items.len() {
        let include_path = match &items[i] {
            Item::MacroCall(path, args, _, _)
                if path.segments.len() == 1
                    && interner.resolve(path.segments[0].ident) == "include" =>
            {
                include_arg_path(args, interner, env_vars)
            }
            _ => None,
        };

        if let Some(path) = include_path {
            let full_path = join_path(dir, &path);
            if let Some(source) = loader.read_file(&full_path) {
                let include_dir = parent_dir(&full_path);
                let mut parser = Parser::new(&source, interner);
                let mut sub_crate = parser.parse_crate();
                resolve_includes_in_items(
                    &mut sub_crate.items,
                    &include_dir,
                    interner,
                    loader,
                    env_vars,
                    loaded,
                );
                loaded.push(ModuleSource {
                    path: full_path,
                    source,
                });
                items.splice(i..=i, sub_crate.items);
                continue;
            }
        }

        match &mut items[i] {
            Item::Mod(mod_def) => {
                if let Some(sub_items) = &mut mod_def.items {
                    let sub_dir = format!("{}/{}", dir, interner.resolve(mod_def.name));
                    resolve_includes_in_items(
                        sub_items, &sub_dir, interner, loader, env_vars, loaded,
                    );
                }
            }
            Item::Impl(ib) => {
                resolve_includes_in_items(&mut ib.items, dir, interner, loader, env_vars, loaded);
            }
            Item::Trait(td) => {
                resolve_includes_in_items(&mut td.items, dir, interner, loader, env_vars, loaded);
            }
            Item::ExternBlock(eb) => {
                resolve_includes_in_items(&mut eb.items, dir, interner, loader, env_vars, loaded);
            }
            _ => {}
        }
        i += 1;
    }
}

fn include_arg_path(
    args: &[TokenTree],
    interner: &Interner,
    env_vars: &[(String, String)],
) -> Option<String> {
    match args.first() {
        Some(TokenTree::Token(tok)) => match &tok.kind {
            TokenKind::StringLit(path) => Some(path.clone()),
            TokenKind::Ident(sym) if interner.resolve(*sym) == "concat" => {
                eval_string_macro(args, interner, env_vars)
            }
            TokenKind::Ident(sym) if interner.resolve(*sym) == "env" => {
                eval_string_macro(args, interner, env_vars)
            }
            _ => None,
        },
        _ => None,
    }
}

fn eval_string_macro(
    args: &[TokenTree],
    interner: &Interner,
    env_vars: &[(String, String)],
) -> Option<String> {
    let name = match args.first() {
        Some(TokenTree::Token(tok)) => match &tok.kind {
            TokenKind::Ident(sym) => interner.resolve(*sym),
            _ => return None,
        },
        _ => return None,
    };
    if !matches!(args.get(1), Some(TokenTree::Token(tok)) if tok.kind == TokenKind::Not) {
        return None;
    }
    let inner = match args.get(2) {
        Some(TokenTree::Delimited(Delimiter::Paren, inner)) => inner,
        _ => return None,
    };
    match name {
        "concat" => {
            let mut out = String::new();
            for piece in split_token_args(inner) {
                out.push_str(&eval_string_piece(&piece, interner, env_vars)?);
            }
            Some(out)
        }
        "env" => {
            let key = match inner.first() {
                Some(TokenTree::Token(tok)) => match &tok.kind {
                    TokenKind::StringLit(key) => key,
                    _ => return None,
                },
                _ => return None,
            };
            lookup_env_var(key, env_vars)
        }
        _ => None,
    }
}

fn eval_string_piece(
    tokens: &[TokenTree],
    interner: &Interner,
    env_vars: &[(String, String)],
) -> Option<String> {
    if tokens.len() == 1 {
        if let TokenTree::Token(tok) = &tokens[0] {
            if let TokenKind::StringLit(value) = &tok.kind {
                return Some(value.clone());
            }
        }
    }
    eval_string_macro(tokens, interner, env_vars)
}

fn split_token_args(tokens: &[TokenTree]) -> Vec<Vec<TokenTree>> {
    let mut args = Vec::new();
    let mut current = Vec::new();
    for token in tokens {
        if matches!(token, TokenTree::Token(tok) if tok.kind == TokenKind::Comma) {
            args.push(current);
            current = Vec::new();
        } else {
            current.push(token.clone());
        }
    }
    if !current.is_empty() || !args.is_empty() {
        args.push(current);
    }
    args
}

fn lookup_env_var(key: &str, env_vars: &[(String, String)]) -> Option<String> {
    for (env_key, value) in env_vars.iter().rev() {
        if env_key == key {
            return Some(value.clone());
        }
    }
    let mut buf = [0u8; 1024];
    let len = anyos_std::env::get(key, &mut buf);
    if len == u32::MAX || len as usize > buf.len() {
        return None;
    }
    core::str::from_utf8(&buf[..len as usize])
        .ok()
        .map(String::from)
}

fn join_path(dir: &str, path: &str) -> String {
    if path.starts_with('/') || dir.is_empty() {
        path.to_string()
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), path)
    }
}

fn parent_dir(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn resolve_items(
    items: &mut Vec<Item>,
    dir: &str,
    interner: &mut Interner,
    loader: &dyn FileLoader,
    env_vars: &[(String, String)],
    loaded: &mut Vec<ModuleSource>,
) {
    for item in items.iter_mut() {
        if let Item::Mod(mod_def) = item {
            if mod_def.items.is_none() {
                // `mod foo;` — need to load from file
                let mod_name = interner.resolve(mod_def.name);
                let mut candidates = Vec::new();
                if let Some(path_attr) = mod_path_attr(mod_def, interner) {
                    candidates.push(format!("{}/{}", dir, path_attr));
                }
                candidates.push(format!("{}/{}.rs", dir, mod_name));
                candidates.push(format!("{}/{}/mod.rs", dir, mod_name));

                let mut chosen_path = None;
                let mut source = None;
                for candidate in candidates {
                    if let Some(src) = loader.read_file(&candidate) {
                        chosen_path = Some(candidate.clone());
                        source = Some(src);
                        break;
                    }
                }
                let (chosen_path, source) = match (chosen_path, source) {
                    (Some(path), Some(src)) => (path, src),
                    _ => continue,
                };
                let sub_dir = module_sub_dir(dir, mod_name, &chosen_path);
                let source_dir = parent_dir(&chosen_path);

                // Parse the loaded source
                let mut parser = Parser::new(&source, interner);
                let mut sub_crate = parser.parse_crate();
                resolve_includes_in_items(
                    &mut sub_crate.items,
                    &source_dir,
                    interner,
                    loader,
                    env_vars,
                    loaded,
                );

                // Recursively resolve sub-modules
                resolve_items(
                    &mut sub_crate.items,
                    &sub_dir,
                    interner,
                    loader,
                    env_vars,
                    loaded,
                );

                // Set the module's items
                mod_def.items = Some(sub_crate.items);
                loaded.push(ModuleSource {
                    path: chosen_path,
                    source,
                });
            } else {
                // Inline module `mod foo { ... }` — recurse into its items
                if let Some(ref mut sub_items) = mod_def.items {
                    let sub_dir = format!("{}/{}", dir, interner.resolve(mod_def.name));
                    resolve_includes_in_items(sub_items, dir, interner, loader, env_vars, loaded);
                    resolve_items(sub_items, &sub_dir, interner, loader, env_vars, loaded);
                }
            }
        }
    }
}

fn mod_path_attr(mod_def: &ModDef, interner: &Interner) -> Option<String> {
    for attr in &mod_def.attrs {
        if attr.path.segments.len() != 1 {
            continue;
        }
        let Some(seg) = attr.path.segments.first() else {
            continue;
        };
        if interner.resolve(seg.ident) != "path" {
            continue;
        }
        if let crate::ast::AttrArgs::Eq(expr) = &attr.args {
            if let crate::ast::Expr::Lit(crate::ast::Literal::String(path), _) = &**expr {
                return Some(path.clone());
            }
        }
    }
    None
}

fn module_sub_dir(base_dir: &str, mod_name: &str, chosen_path: &str) -> String {
    if chosen_path.ends_with("/mod.rs") {
        chosen_path.trim_end_matches("/mod.rs").to_string()
    } else if let Some(parent) = chosen_path.rsplit_once('/') {
        format!("{}/{}", parent.0, mod_name)
    } else {
        format!("{}/{}", base_dir, mod_name)
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
    /// Public interface source for downstream name resolution/type checking.
    pub interface_source: String,
    /// Structured public interface. This is the long-term compiler contract;
    /// `interface_source` remains as a compatibility bridge until resolver and
    /// type checking consume structured crate metadata directly.
    pub interface: CrateInterface,
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

#[derive(Clone, Default)]
pub struct CrateInterface {
    pub items: Vec<InterfaceItem>,
}

#[derive(Clone)]
pub struct InterfaceItem {
    pub name: String,
    pub kind: InterfaceItemKind,
    pub signature: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InterfaceItemKind {
    Function,
    Struct,
    Enum,
    Trait,
    TypeAlias,
    Const,
    Static,
    Impl,
    Use,
    Module,
    ExternBlock,
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
    let iface_bytes = meta.interface_source.as_bytes();
    buf.extend_from_slice(&(iface_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(iface_bytes);
    serialize_interface(&meta.interface, &mut buf);
    buf
}

/// Deserialize crate metadata from bytes.
pub fn deserialize_metadata(data: &[u8]) -> Option<CrateMetadata> {
    if data.len() < 4 || &data[0..4] != b"ARCM" {
        return None;
    }
    let mut pos = 4;

    let read_u16 = |data: &[u8], pos: &mut usize| -> Option<u16> {
        if *pos + 2 > data.len() {
            return None;
        }
        let v = u16::from_le_bytes(data[*pos..*pos + 2].try_into().ok()?);
        *pos += 2;
        Some(v)
    };

    let read_str = |data: &[u8], pos: &mut usize| -> Option<String> {
        let len = read_u16(data, pos)? as usize;
        if *pos + len > data.len() {
            return None;
        }
        let s = core::str::from_utf8(&data[*pos..*pos + len])
            .ok()?
            .to_string();
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
        if pos >= data.len() {
            return None;
        }
        let kind = match data[pos] {
            0 => ExportKind::Function,
            1 => ExportKind::Static,
            2 => ExportKind::Type,
            3 => ExportKind::Const,
            _ => return None,
        };
        pos += 1;
        exports.push(ExportedSymbol {
            name: exp_name,
            kind,
        });
    }

    let interface_source = if pos + 4 <= data.len() {
        let len = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
        pos += 4;
        if pos + len > data.len() {
            return None;
        }
        let s = core::str::from_utf8(&data[pos..pos + len])
            .ok()?
            .to_string();
        pos += len;
        s
    } else {
        String::new()
    };

    let interface = deserialize_interface(data, pos)?;

    Some(CrateMetadata {
        name,
        version,
        exports,
        deps,
        interface_source,
        interface,
    })
}

fn write_str16(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(bytes);
}

fn write_str32(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(bytes);
}

fn serialize_interface(interface: &CrateInterface, buf: &mut Vec<u8>) {
    buf.extend_from_slice(b"ARCI");
    buf.extend_from_slice(&(interface.items.len() as u16).to_le_bytes());
    for item in &interface.items {
        buf.push(interface_item_kind_to_u8(item.kind));
        write_str16(buf, &item.name);
        write_str32(buf, &item.signature);
    }
}

fn deserialize_interface(data: &[u8], mut pos: usize) -> Option<CrateInterface> {
    if pos >= data.len() {
        return Some(CrateInterface::default());
    }
    if pos + 4 > data.len() || &data[pos..pos + 4] != b"ARCI" {
        return Some(CrateInterface::default());
    }
    pos += 4;
    if pos + 2 > data.len() {
        return None;
    }
    let count = u16::from_le_bytes(data[pos..pos + 2].try_into().ok()?) as usize;
    pos += 2;

    let read_str16 = |data: &[u8], pos: &mut usize| -> Option<String> {
        if *pos + 2 > data.len() {
            return None;
        }
        let len = u16::from_le_bytes(data[*pos..*pos + 2].try_into().ok()?) as usize;
        *pos += 2;
        if *pos + len > data.len() {
            return None;
        }
        let s = core::str::from_utf8(&data[*pos..*pos + len])
            .ok()?
            .to_string();
        *pos += len;
        Some(s)
    };
    let read_str32 = |data: &[u8], pos: &mut usize| -> Option<String> {
        if *pos + 4 > data.len() {
            return None;
        }
        let len = u32::from_le_bytes(data[*pos..*pos + 4].try_into().ok()?) as usize;
        *pos += 4;
        if *pos + len > data.len() {
            return None;
        }
        let s = core::str::from_utf8(&data[*pos..*pos + len])
            .ok()?
            .to_string();
        *pos += len;
        Some(s)
    };

    let mut items = Vec::new();
    for _ in 0..count {
        if pos >= data.len() {
            return None;
        }
        let kind = interface_item_kind_from_u8(data[pos])?;
        pos += 1;
        let name = read_str16(data, &mut pos)?;
        let signature = read_str32(data, &mut pos)?;
        items.push(InterfaceItem {
            name,
            kind,
            signature,
        });
    }

    Some(CrateInterface { items })
}

fn interface_item_kind_to_u8(kind: InterfaceItemKind) -> u8 {
    match kind {
        InterfaceItemKind::Function => 0,
        InterfaceItemKind::Struct => 1,
        InterfaceItemKind::Enum => 2,
        InterfaceItemKind::Trait => 3,
        InterfaceItemKind::TypeAlias => 4,
        InterfaceItemKind::Const => 5,
        InterfaceItemKind::Static => 6,
        InterfaceItemKind::Impl => 7,
        InterfaceItemKind::Use => 8,
        InterfaceItemKind::Module => 9,
        InterfaceItemKind::ExternBlock => 10,
    }
}

fn interface_item_kind_from_u8(raw: u8) -> Option<InterfaceItemKind> {
    match raw {
        0 => Some(InterfaceItemKind::Function),
        1 => Some(InterfaceItemKind::Struct),
        2 => Some(InterfaceItemKind::Enum),
        3 => Some(InterfaceItemKind::Trait),
        4 => Some(InterfaceItemKind::TypeAlias),
        5 => Some(InterfaceItemKind::Const),
        6 => Some(InterfaceItemKind::Static),
        7 => Some(InterfaceItemKind::Impl),
        8 => Some(InterfaceItemKind::Use),
        9 => Some(InterfaceItemKind::Module),
        10 => Some(InterfaceItemKind::ExternBlock),
        _ => None,
    }
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
    if data.len() < 4 {
        return None;
    }
    let obj_size = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    if 4 + obj_size > data.len() {
        return None;
    }
    let obj = data[4..4 + obj_size].to_vec();
    let meta = deserialize_metadata(&data[4 + obj_size..])?;
    Some((obj, meta))
}

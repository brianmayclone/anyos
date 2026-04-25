use crate::prelude::*;
use crate::hir::*;
use crate::intern::{Interner, Symbol};
use crate::lang_items;
use crate::diagnostics::{Span, Diagnostic, Level};
use anyos_std::collections::HashMap;

/// Result of name resolution
pub struct ResolveResult {
    /// Map from HirId (of a path usage) to DefId (of the definition)
    pub resolutions: HashMap<HirId, DefId>,
    /// Errors encountered during resolution
    pub errors: Vec<Diagnostic>,
    /// Map from type name Symbol to list of (method_name, method_def_id)
    pub impl_methods: HashMap<Symbol, Vec<(Symbol, DefId)>>,
    /// Map from (enum_name, variant_name) to variant index
    pub variant_indices: HashMap<(Symbol, Symbol), usize>,
    /// Intrinsic DefIds: maps synthetic DefId to full path string (e.g. "core::ptr::null_mut")
    pub intrinsic_fns: HashMap<DefId, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Namespace {
    Type,
    Value,
}

struct Scope {
    parent: Option<usize>,
    bindings: HashMap<(Symbol, Namespace), DefId>,
}

/// Info about an enum for resolving Enum::Variant paths
struct EnumInfo {
    /// Map from variant name to a synthetic DefId
    variants: HashMap<Symbol, DefId>,
}

pub struct Resolver<'a> {
    scopes: Vec<Scope>,
    current_scope: usize,
    resolutions: HashMap<HirId, DefId>,
    errors: Vec<Diagnostic>,
    interner: &'a mut Interner,
    /// Enum DefId -> variant info
    enum_variants: HashMap<DefId, EnumInfo>,
    /// Next synthetic DefId for things without one (e.g. variants)
    next_synthetic_def_id: u32,
    /// Set of primitive type name strings
    primitives: Vec<&'static str>,
    /// Map from impl self_ty first segment symbol to list of method (name, def_id)
    impl_methods: HashMap<Symbol, Vec<(Symbol, DefId)>>,
    /// Map from impl self_ty first segment symbol to associated value items (consts/statics).
    impl_assoc_values: HashMap<Symbol, Vec<(Symbol, DefId)>>,
    /// Map from impl self_ty first segment symbol to associated type items.
    impl_assoc_types: HashMap<Symbol, Vec<(Symbol, DefId)>>,
    /// Module DefId -> scope index that contains the module's items
    module_scopes: HashMap<DefId, usize>,
    /// Root scope index (for `crate::` paths)
    root_scope: usize,
    /// Stack of module scope indices for `super::` resolution
    module_stack: Vec<usize>,
    /// Intrinsic DefIds: maps synthetic DefId to full path string
    intrinsic_fns: HashMap<DefId, String>,
    /// Extern module aliases imported with `use core::foo::bar;`.
    extern_path_aliases: HashMap<(usize, Symbol), String>,
    /// Current impl self type symbol for resolving `Self::assoc` within impls.
    current_impl_self_ty: Option<Symbol>,
    /// Current impl self type DefId for resolving `Self::Variant` on module-qualified types.
    current_impl_self_def_id: Option<DefId>,
    /// Module path and item used to make diagnostics useful for loaded modules.
    current_module_path: Vec<Symbol>,
    current_item_name: Option<Symbol>,
}

impl<'a> Resolver<'a> {
    pub fn new(interner: &'a mut Interner) -> Self {
        let mut this = Self {
            scopes: vec![Scope { parent: None, bindings: HashMap::new() }],
            current_scope: 0,
            resolutions: HashMap::new(),
            errors: Vec::new(),
            interner,
            enum_variants: HashMap::new(),
            next_synthetic_def_id: 10000,
            primitives: lang_items::PRIMITIVE_TYPES.to_vec(),
            impl_methods: HashMap::new(),
            impl_assoc_values: HashMap::new(),
            impl_assoc_types: HashMap::new(),
            module_scopes: HashMap::new(),
            root_scope: 0,
            module_stack: vec![0],
            intrinsic_fns: HashMap::new(),
            extern_path_aliases: HashMap::new(),
            current_impl_self_ty: None,
            current_impl_self_def_id: None,
            current_module_path: Vec::new(),
            current_item_name: None,
        };
        this.bootstrap_prelude_intrinsics();
        this
    }

    fn bootstrap_prelude_intrinsics(&mut self) {
        for item in lang_items::PRELUDE_TYPES {
            self.define_intrinsic_type(item.local_name, item.full_path);
        }

        for item in lang_items::PRELUDE_VALUES {
            self.define_intrinsic_value(item.local_name, item.full_path);
        }
    }

    fn define_intrinsic_type(&mut self, local_name: &str, full_path: &str) {
        let name = self.find_symbol(local_name).unwrap_or_else(|| self.interner.intern(local_name));
        let def_id = self.alloc_synthetic_def_id();
        self.intrinsic_fns.insert(def_id, full_path.to_string());
        self.define(name, Namespace::Type, def_id);
    }

    fn define_intrinsic_value(&mut self, local_name: &str, full_path: &str) {
        let name = self.find_symbol(local_name).unwrap_or_else(|| self.interner.intern(local_name));
        let def_id = self.alloc_synthetic_def_id();
        self.intrinsic_fns.insert(def_id, full_path.to_string());
        self.define(name, Namespace::Value, def_id);
    }

    fn alloc_synthetic_def_id(&mut self) -> DefId {
        let id = DefId(self.next_synthetic_def_id);
        self.next_synthetic_def_id += 1;
        id
    }

    fn error(&mut self, span: Span, msg: &str) {
        let mut path = self
            .current_module_path
            .iter()
            .map(|sym| self.interner.resolve(*sym).to_string())
            .collect::<Vec<_>>();
        if let Some(item) = self.current_item_name {
            path.push(self.interner.resolve(item).to_string());
        }
        let msg = if path.is_empty() {
            msg.to_string()
        } else {
            format!("in {}: {}", path.join("::"), msg)
        };
        self.errors.push(Diagnostic::new(Level::Error, &msg, span));
    }

    fn resolve_assoc_item_on_type(
        &mut self,
        type_name: Symbol,
        assoc_name: Symbol,
        hir_id: HirId,
    ) -> bool {
        if let Some(methods) = self.impl_methods.get(&type_name) {
            for &(method_name, method_def_id) in methods {
                if method_name == assoc_name {
                    if hir_id != HirId(u32::MAX) {
                        self.resolutions.insert(hir_id, method_def_id);
                    }
                    return true;
                }
            }
        }

        if let Some(values) = self.impl_assoc_values.get(&type_name) {
            for &(value_name, value_def_id) in values {
                if value_name == assoc_name {
                    if hir_id != HirId(u32::MAX) {
                        self.resolutions.insert(hir_id, value_def_id);
                    }
                    return true;
                }
            }
        }

        if let Some(types) = self.impl_assoc_types.get(&type_name) {
            for &(type_item_name, type_item_def_id) in types {
                if type_item_name == assoc_name {
                    if hir_id != HirId(u32::MAX) {
                        self.resolutions.insert(hir_id, type_item_def_id);
                    }
                    return true;
                }
            }
        }

        if let Some(type_def_id) = self.lookup(type_name, Namespace::Type) {
            if let Some(enum_info) = self.enum_variants.get(&type_def_id) {
                if let Some(&variant_def_id) = enum_info.variants.get(&assoc_name) {
                    if hir_id != HirId(u32::MAX) {
                        self.resolutions.insert(hir_id, variant_def_id);
                    }
                    return true;
                }
            }

            if self.intrinsic_fns.contains_key(&type_def_id) {
                let type_path = self.intrinsic_fns
                    .get(&type_def_id)
                    .cloned()
                    .unwrap_or_else(|| self.interner.resolve(type_name).to_string());
                let assoc_str = self.interner.resolve(assoc_name);
                let full_path = format!("{}::{}", type_path, assoc_str);
                let def_id = self.alloc_synthetic_def_id();
                self.intrinsic_fns.insert(def_id, full_path);
                if hir_id != HirId(u32::MAX) {
                    self.resolutions.insert(hir_id, def_id);
                }
                return true;
            }
        }

        let type_name_str = self.interner.resolve(type_name).to_string();
        if assoc_name == self.find_symbol("default").unwrap_or_else(|| self.interner.intern("default")) {
            let def_id = self.alloc_synthetic_def_id();
            self.intrinsic_fns.insert(def_id, "Default::default".to_string());
            if hir_id != HirId(u32::MAX) {
                self.resolutions.insert(hir_id, def_id);
            }
            return true;
        }
        if self.primitives.iter().any(|&p| p == type_name_str) {
            let assoc_str = self.interner.resolve(assoc_name);
            let is_assoc_const = matches!(
                (type_name_str.as_str(), assoc_str),
                ("u8", "MAX") | ("u16", "MAX") | ("u32", "MAX") | ("u64", "MAX") | ("u128", "MAX") | ("usize", "MAX")
                | ("i8", "MAX") | ("i16", "MAX") | ("i32", "MAX") | ("i64", "MAX") | ("i128", "MAX") | ("isize", "MAX")
                | ("i8", "MIN") | ("i16", "MIN") | ("i32", "MIN") | ("i64", "MIN") | ("i128", "MIN") | ("isize", "MIN")
                | ("u8", "MIN") | ("u16", "MIN") | ("u32", "MIN") | ("u64", "MIN") | ("u128", "MIN") | ("usize", "MIN")
                | ("f32", "MAX") | ("f32", "MIN") | ("f32", "MIN_POSITIVE")
                | ("f32", "INFINITY") | ("f32", "NEG_INFINITY") | ("f32", "NAN") | ("f32", "EPSILON")
                | ("f64", "MAX") | ("f64", "MIN") | ("f64", "MIN_POSITIVE")
                | ("f64", "INFINITY") | ("f64", "NEG_INFINITY") | ("f64", "NAN") | ("f64", "EPSILON")
            );
            let is_assoc_fn = matches!(
                assoc_str,
                "from" | "try_from" | "from_le" | "from_be" | "to_le" | "to_be"
                    | "from_le_bytes" | "from_be_bytes" | "from_ne_bytes" | "from_bits" | "from_str_radix"
                    | "min" | "max" | "to_string"
            )
                || (type_name_str == "char" && assoc_str == "from_u32");
            if is_assoc_const || is_assoc_fn {
                let full_path = format!("{}::{}", type_name_str, assoc_str);
                let def_id = self.alloc_synthetic_def_id();
                self.intrinsic_fns.insert(def_id, full_path);
                if hir_id != HirId(u32::MAX) {
                    self.resolutions.insert(hir_id, def_id);
                }
                return true;
            }
        }

        false
    }

    fn resolve_assoc_item_on_type_def(
        &mut self,
        type_def_id: DefId,
        assoc_name: Symbol,
        hir_id: HirId,
    ) -> bool {
        if let Some(enum_info) = self.enum_variants.get(&type_def_id) {
            if let Some(&variant_def_id) = enum_info.variants.get(&assoc_name) {
                if hir_id != HirId(u32::MAX) {
                    self.resolutions.insert(hir_id, variant_def_id);
                }
                return true;
            }
        }

        if self.intrinsic_fns.contains_key(&type_def_id) {
            let type_path = self.intrinsic_fns.get(&type_def_id).cloned().unwrap_or_default();
            let assoc_str = self.interner.resolve(assoc_name);
            let full_path = format!("{}::{}", type_path, assoc_str);
            let def_id = self.alloc_synthetic_def_id();
            self.intrinsic_fns.insert(def_id, full_path);
            if hir_id != HirId(u32::MAX) {
                self.resolutions.insert(hir_id, def_id);
            }
            return true;
        }

        false
    }

    fn register_assoc_items_in_current_scope(&mut self, items: &[HirItem]) {
        for item in items {
            match &item.kind {
                HirItemKind::TypeAlias(ta) => self.define(ta.name, Namespace::Type, ta.def_id),
                HirItemKind::Const(c) => self.define(c.name, Namespace::Value, c.def_id),
                HirItemKind::Static(s) => self.define(s.name, Namespace::Value, s.def_id),
                _ => {}
            }
        }
    }

    pub fn resolve_crate(&mut self, krate: &HirCrate) -> ResolveResult {
        // First pass: register all top-level items (recursing into modules)
        for item in &krate.items {
            self.register_item(item);
        }

        // Second pass: collect impl methods (recursing into modules)
        self.collect_impls_recursive(&krate.items);

        // Process use items to a fixed enough point. Interfaces from extern
        // crates often contain re-export chains (`pub use self::inner::Item`);
        // downstream uses may appear before those re-exports in the combined
        // synthetic crate, so a single source-order pass is not sufficient.
        for _ in 0..8 {
            self.process_use_items_recursive(&krate.items);
        }

        // Third pass: resolve all items
        for item in &krate.items {
            self.resolve_item(item);
        }

        // Build variant_indices: (enum_name, variant_name) -> variant index
        let mut variant_indices = HashMap::new();
        self.collect_variant_indices_recursive(&krate.items, &mut variant_indices);

        ResolveResult {
            resolutions: core::mem::take(&mut self.resolutions),
            errors: core::mem::take(&mut self.errors),
            impl_methods: self.impl_methods.clone(),
            variant_indices,
            intrinsic_fns: core::mem::take(&mut self.intrinsic_fns),
        }
    }

    fn collect_impls_recursive(&mut self, items: &[HirItem]) {
        for item in items {
            match &item.kind {
                HirItemKind::Impl(impl_block) => {
                    self.register_impl_methods(impl_block);
                }
                HirItemKind::Mod(m) => {
                    if let Some(sub_items) = &m.items {
                        self.collect_impls_recursive(sub_items);
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_variant_indices_recursive(
        &self,
        items: &[HirItem],
        variant_indices: &mut HashMap<(Symbol, Symbol), usize>,
    ) {
        for item in items {
            match &item.kind {
                HirItemKind::Enum(e) => {
                    for (idx, v) in e.variants.iter().enumerate() {
                        variant_indices.insert((e.name, v.name), idx);
                    }
                }
                HirItemKind::Mod(m) => {
                    if let Some(sub_items) = &m.items {
                        self.collect_variant_indices_recursive(sub_items, variant_indices);
                    }
                }
                _ => {}
            }
        }
    }

    fn process_use_items_recursive(&mut self, items: &[HirItem]) {
        for item in items {
            match &item.kind {
                HirItemKind::Use(u) => {
                    self.process_use_tree(u);
                }
                HirItemKind::Mod(m) => {
                    if let Some(sub_items) = &m.items {
                        // Enter the module scope to process its use items
                        if let Some(&scope_idx) = self.module_scopes.get(&m.def_id) {
                            let saved = self.current_scope;
                            self.current_scope = scope_idx;
                            self.module_stack.push(scope_idx);
                            self.process_use_items_recursive(sub_items);
                            self.module_stack.pop();
                            self.current_scope = saved;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn process_use_tree(&mut self, use_tree: &HirUseTree) {
        match &use_tree.kind {
            HirUseTreeKind::Simple(alias) => {
                let trailing_self = use_tree
                    .path
                    .last()
                    .map(|sym| self.interner.resolve(*sym) == "self")
                    .unwrap_or(false);
                let full_path: Vec<Symbol> = if trailing_self && use_tree.path.len() >= 2 {
                    use_tree.path[..use_tree.path.len() - 1].to_vec()
                } else {
                    use_tree.path.clone()
                };
                let local_name = alias.unwrap_or_else(|| {
                    if trailing_self && use_tree.path.len() >= 2 {
                        use_tree.path[use_tree.path.len() - 2]
                    } else {
                        *use_tree.path.last().unwrap()
                    }
                });
                // Handle `use core::...` imports as intrinsics
                if let Some(full_path) = self.extern_crate_path_string(&full_path) {
                    let def_id = self.alloc_synthetic_def_id();
                    self.intrinsic_fns.insert(def_id, full_path.clone());
                    self.define(local_name, Namespace::Value, def_id);
                    self.define(local_name, Namespace::Type, def_id);
                    self.extern_path_aliases.insert((self.current_scope, local_name), full_path);
                    return;
                }
                if let Some(full_path) = self.resolve_extern_alias_use_path(&full_path) {
                    let def_id = self.alloc_synthetic_def_id();
                    self.intrinsic_fns.insert(def_id, full_path);
                    self.define(local_name, Namespace::Value, def_id);
                    self.define(local_name, Namespace::Type, def_id);
                    return;
                }
                if let Some(full_path) = self.resolve_extern_backed_path_string(&full_path) {
                    let def_id = self.alloc_synthetic_def_id();
                    self.intrinsic_fns.insert(def_id, full_path.clone());
                    self.define(local_name, Namespace::Value, def_id);
                    self.define(local_name, Namespace::Type, def_id);
                    self.extern_path_aliases.insert((self.current_scope, local_name), full_path);
                    return;
                }
                // use a::b::c; or use a::b::c as d;
                if let Some((def_id, ns)) = self.resolve_use_path(&full_path, use_tree.span) {
                    self.define(local_name, ns, def_id);
                    // Also define in the other namespace for cross-ns usage
                    let other_ns = if ns == Namespace::Value { Namespace::Type } else { Namespace::Value };
                    // Try other ns too - don't error if not found
                    if let Some((def_id2, _)) = self.resolve_use_path_ns(&full_path, other_ns) {
                        self.define(local_name, other_ns, def_id2);
                    }
                }
            }
            HirUseTreeKind::Nested(trees) => {
                for sub in trees {
                    // The sub-tree's path is prefixed by use_tree.path
                    let mut full_path = use_tree.path.clone();
                    full_path.extend_from_slice(&sub.path);
                    let combined = HirUseTree {
                        id: sub.id,
                        vis: sub.vis,
                        path: full_path,
                        kind: sub.kind.clone(),
                        span: sub.span,
                    };
                    self.process_use_tree(&combined);
                }
            }
            HirUseTreeKind::Glob => {
                if self.is_extern_crate_path(&use_tree.path) {
                    self.import_extern_glob(&use_tree.path);
                    return;
                }
                // use foo::*; - import all public items from module
                // Resolve the path to find a module
                if let Some(mod_def_id) = self.resolve_use_path_to_module(&use_tree.path, use_tree.span) {
                    if let Some(&scope_idx) = self.module_scopes.get(&mod_def_id) {
                        // Copy all bindings from that scope into current scope
                        let bindings: Vec<_> = self.scopes[scope_idx].bindings.iter()
                            .map(|(&k, &v)| (k, v))
                            .collect();
                        for ((name, ns), def_id) in bindings {
                            self.define(name, ns, def_id);
                        }
                        let aliases: Vec<_> = self.extern_path_aliases
                            .iter()
                            .filter_map(|(&(scope, name), path)| {
                                if scope == scope_idx {
                                    Some((name, path.clone()))
                                } else {
                                    None
                                }
                            })
                            .collect();
                        for (name, path) in aliases {
                            self.extern_path_aliases.insert((self.current_scope, name), path);
                        }
                    }
                } else if let Some((type_def_id, _)) = self.resolve_use_path_ns(&use_tree.path, Namespace::Type) {
                    if let Some(enum_info) = self.enum_variants.get(&type_def_id) {
                        let variants: Vec<_> = enum_info
                            .variants
                            .iter()
                            .map(|(&name, &def_id)| (name, def_id))
                            .collect();
                        for (name, def_id) in variants {
                            self.define(name, Namespace::Value, def_id);
                        }
                    }
                }
            }
        }
    }

    fn import_extern_glob(&mut self, path: &[Symbol]) {
        let path_str = self.path_to_string(path);
        for name in Self::extern_glob_names(&path_str) {
            let sym = self.interner.intern(name);
            let def_id = self.alloc_synthetic_def_id();
            self.intrinsic_fns
                .insert(def_id, format!("{}::{}", path_str, name));
            self.define(sym, Namespace::Value, def_id);
            self.define(sym, Namespace::Type, def_id);
            self.extern_path_aliases
                .insert((self.current_scope, sym), format!("{}::{}", path_str, name));
        }
    }

    fn extern_glob_names(path: &str) -> &'static [&'static str] {
        match path {
            "core" | "std" => &[
                "borrow",
                "boxed",
                "cell",
                "cmp",
                "collections",
                "error",
                "f32",
                "f64",
                "ffi",
                "fmt",
                "hash",
                "io",
                "iter",
                "marker",
                "mem",
                "net",
                "num",
                "ops",
                "path",
                "rc",
                "result",
                "str",
                "string",
                "sync",
                "time",
                "vec",
            ],
            "alloc" => &[
                "borrow",
                "boxed",
                "collections",
                "ffi",
                "rc",
                "string",
                "sync",
                "vec",
            ],
            "alloc::collections" | "std::collections" => &[
                "BinaryHeap",
                "BTreeMap",
                "BTreeSet",
                "LinkedList",
                "VecDeque",
            ],
            "core::cmp::Ordering" => &[
                "Less",
                "Equal",
                "Greater",
            ],
            "core::sync::atomic::Ordering" => &[
                "Relaxed",
                "Release",
                "Acquire",
                "AcqRel",
                "SeqCst",
            ],
            "serde::de" => &[
                "Deserialize",
                "DeserializeOwned",
                "DeserializeSeed",
                "Deserializer",
                "EnumAccess",
                "Error",
                "Expected",
                "IgnoredAny",
                "IntoDeserializer",
                "MapAccess",
                "SeqAccess",
                "Unexpected",
                "VariantAccess",
                "Visitor",
            ],
            "serde::ser" => &[
                "Error",
                "Impossible",
                "Serialize",
                "SerializeMap",
                "SerializeSeq",
                "SerializeStruct",
                "SerializeStructVariant",
                "SerializeTuple",
                "SerializeTupleStruct",
                "SerializeTupleVariant",
                "Serializer",
            ],
            _ => &[],
        }
    }

    /// Resolve a use path to a (DefId, Namespace) pair
    fn resolve_use_path(&self, path: &[Symbol], span: Span) -> Option<(DefId, Namespace)> {
        if let Some(result) = self.resolve_use_path_ns(path, Namespace::Value) {
            return Some(result);
        }
        if let Some(result) = self.resolve_use_path_ns(path, Namespace::Type) {
            return Some(result);
        }
        None
    }

    /// Check if a path represents an external known module.
    fn is_extern_crate_path(&self, path: &[Symbol]) -> bool {
        self.extern_crate_path_string(path).is_some()
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

    fn extern_crate_path_string(&self, path: &[Symbol]) -> Option<String> {
        let (&first, rest) = path.split_first()?;
        let first_str = self.interner.resolve(first);
        if Self::is_compiler_known_external_crate(first_str) {
            if self
                .scopes[self.root_scope]
                .bindings
                .get(&(first, Namespace::Type))
                .is_some_and(|def_id| self.module_scopes.contains_key(&def_id))
            {
                return None;
            }
            return Some(self.path_to_string(path));
        }

        if first_str == "crate" {
            let (&extern_name, extern_rest) = rest.split_first()?;
            let extern_str = self.interner.resolve(extern_name);
            if Self::is_compiler_known_external_crate(extern_str)
                && self.scopes[self.root_scope]
                    .bindings
                    .get(&(extern_name, Namespace::Type))
                    .is_none()
            {
                let mut parts = Vec::new();
                parts.push(extern_str.to_string());
                for sym in extern_rest {
                    parts.push(self.interner.resolve(*sym).to_string());
                }
                return Some(parts.join("::"));
            }
        }

        None
    }

    /// Build a full path string like "core::ptr::null_mut" from symbols
    fn path_to_string(&self, path: &[Symbol]) -> String {
        path.iter()
            .map(|s| self.interner.resolve(*s).to_string())
            .collect::<Vec<_>>()
            .join("::")
    }

    fn resolve_extern_alias_use_path(&self, path: &[Symbol]) -> Option<String> {
        let (&first, rest) = path.split_first()?;
        let mut scope = Some(self.current_scope);
        while let Some(scope_idx) = scope {
            if let Some(prefix) = self.extern_path_aliases.get(&(scope_idx, first)) {
                if rest.is_empty() {
                    return Some(prefix.clone());
                }
                let suffix = rest.iter()
                    .map(|s| self.interner.resolve(*s).to_string())
                    .collect::<Vec<_>>()
                    .join("::");
                return Some(format!("{}::{}", prefix, suffix));
            }
            scope = self.scopes[scope_idx].parent;
        }
        None
    }

    fn resolve_extern_backed_path_string(&self, path: &[Symbol]) -> Option<String> {
        if path.is_empty() {
            return None;
        }

        let first_str = self.interner.resolve(path[0]);
        let (mut scope, start_idx) = if first_str == "crate" {
            (self.root_scope, 1)
        } else if first_str == "super" {
            let parent_scope = if self.module_stack.len() >= 2 {
                self.module_stack[self.module_stack.len() - 2]
            } else {
                self.root_scope
            };
            (parent_scope, 1)
        } else if first_str == "self" {
            (*self.module_stack.last().unwrap_or(&self.root_scope), 1)
        } else {
            (*self.module_stack.last().unwrap_or(&self.root_scope), 0)
        };

        let segments = &path[start_idx..];
        for (idx, &seg) in segments.iter().enumerate() {
            if let Some(prefix) = self.extern_path_aliases.get(&(scope, seg)) {
                let suffix = segments[idx + 1..]
                    .iter()
                    .map(|sym| self.interner.resolve(*sym).to_string())
                    .collect::<Vec<_>>();
                if suffix.is_empty() {
                    return Some(prefix.clone());
                }
                return Some(format!("{}::{}", prefix, suffix.join("::")));
            }

            if let Some(mod_def_id) = self.scopes[scope]
                .bindings
                .get(&(seg, Namespace::Type))
                .copied()
            {
                if let Some(&mod_scope) = self.module_scopes.get(&mod_def_id) {
                    scope = mod_scope;
                    continue;
                }
            }
            break;
        }

        None
    }

    fn resolve_use_path_ns(&self, path: &[Symbol], ns: Namespace) -> Option<(DefId, Namespace)> {
        if let Some(result) = self.resolve_use_path_ns_direct(path, ns) {
            return Some(result);
        }
        if path.len() >= 2 {
            return self.resolve_use_path_ns_direct(&path[1..], ns);
        }
        None
    }

    fn resolve_use_path_ns_direct(&self, path: &[Symbol], ns: Namespace) -> Option<(DefId, Namespace)> {
        if path.is_empty() { return None; }

        // Determine starting scope
        let first_str = self.interner.resolve(path[0]);
        let (start_scope, start_idx) = if first_str == "crate" {
            (self.root_scope, 1)
        } else if first_str == "super" {
            let parent_scope = if self.module_stack.len() >= 2 {
                self.module_stack[self.module_stack.len() - 2]
            } else {
                self.root_scope
            };
            (parent_scope, 1)
        } else if first_str == "self" {
            (*self.module_stack.last().unwrap_or(&self.root_scope), 1)
        } else {
            (*self.module_stack.last().unwrap_or(&self.root_scope), 0)
        };

        let mut scope = start_scope;
        for (i, &seg) in path[start_idx..].iter().enumerate() {
            let is_last = i == path.len() - start_idx - 1;
            if is_last {
                // Last segment: look up as value or type
                let direct = if i == 0 && start_idx == 0 {
                    self.lookup_from_scope(scope, seg, ns)
                } else {
                    self.scopes[scope].bindings.get(&(seg, ns)).copied()
                };
                if let Some(def_id) = direct {
                    return Some((def_id, ns));
                }
                // Try enum variant
                if ns == Namespace::Value {
                    // Try type ns for the segment - might be an enum/struct
                    let direct_ty = if i == 0 && start_idx == 0 {
                        self.lookup_from_scope(scope, seg, Namespace::Type)
                    } else {
                        self.scopes[scope].bindings.get(&(seg, Namespace::Type)).copied()
                    };
                    if let Some(def_id) = direct_ty {
                        return Some((def_id, Namespace::Type));
                    }
                }
                return None;
            } else {
                // Non-last segment: must be a module
                let mod_def_id = if i == 0 && start_idx == 0 {
                    self.lookup_from_scope(scope, seg, Namespace::Type)
                } else {
                    self.scopes[scope].bindings.get(&(seg, Namespace::Type)).copied()
                };
                if let Some(mod_def_id) = mod_def_id {
                    if let Some(&mod_scope) = self.module_scopes.get(&mod_def_id) {
                        scope = mod_scope;
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
        }
        None
    }

    fn resolve_use_path_to_module(&self, path: &[Symbol], span: Span) -> Option<DefId> {
        if path.is_empty() { return None; }

        let first_str = self.interner.resolve(path[0]);
        let (start_scope, start_idx) = if first_str == "crate" {
            (self.root_scope, 1)
        } else if first_str == "super" {
            let parent_scope = if self.module_stack.len() >= 2 {
                self.module_stack[self.module_stack.len() - 2]
            } else {
                self.root_scope
            };
            (parent_scope, 1)
        } else if first_str == "self" {
            (*self.module_stack.last().unwrap_or(&self.root_scope), 1)
        } else {
            (*self.module_stack.last().unwrap_or(&self.root_scope), 0)
        };

        let mut scope = start_scope;
        for (idx, &seg) in path[start_idx..].iter().enumerate() {
            let is_last = idx == path.len() - start_idx - 1;
            let mod_def_id = if idx == 0 && start_idx == 0 {
                self.lookup_from_scope(scope, seg, Namespace::Type)
            } else {
                self.scopes[scope].bindings.get(&(seg, Namespace::Type)).copied()
            };
            if let Some(mod_def_id) = mod_def_id {
                if let Some(&mod_scope) = self.module_scopes.get(&mod_def_id) {
                    scope = mod_scope;
                    if is_last {
                        return Some(mod_def_id);
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
        None
    }

    fn register_item(&mut self, item: &HirItem) {
        match &item.kind {
            HirItemKind::Fn(f) => {
                self.define(f.name, Namespace::Value, f.def_id);
            }
            HirItemKind::Struct(s) => {
                self.define(s.name, Namespace::Type, s.def_id);
                // Unit structs also go in value namespace
                if s.fields.is_empty() {
                    self.define(s.name, Namespace::Value, s.def_id);
                }
            }
            HirItemKind::Enum(e) => {
                self.define(e.name, Namespace::Type, e.def_id);
                let mut variants = HashMap::new();
                for v in &e.variants {
                    let vid = self.alloc_synthetic_def_id();
                    variants.insert(v.name, vid);
                }
                self.enum_variants.insert(e.def_id, EnumInfo { variants });
            }
            HirItemKind::Trait(t) => {
                self.define(t.name, Namespace::Type, t.def_id);
                self.register_trait_assoc_items(t);
            }
            HirItemKind::TypeAlias(ta) => {
                self.define(ta.name, Namespace::Type, ta.def_id);
            }
            HirItemKind::Const(c) => {
                self.define(c.name, Namespace::Value, c.def_id);
            }
            HirItemKind::Static(s) => {
                self.define(s.name, Namespace::Value, s.def_id);
            }
            HirItemKind::Mod(m) => {
                self.define(m.name, Namespace::Type, m.def_id);
                // Create a new scope for the module's items
                if let Some(sub_items) = &m.items {
                    let parent_scope = self.current_scope;
                    // Module scope's parent is the enclosing scope so items inside
                    // can see the enclosing scope's names.
                    self.push_scope();
                    let mod_scope_idx = self.current_scope;
                    self.module_scopes.insert(m.def_id, mod_scope_idx);
                    self.module_stack.push(mod_scope_idx);
                    // Register all items within the module scope
                    for sub in sub_items {
                        self.register_item(sub);
                    }
                    self.module_stack.pop();
                    self.current_scope = parent_scope;
                }
            }
            HirItemKind::Impl(_) | HirItemKind::Use(_) => {}
            HirItemKind::ExternBlock(eb) => {
                for sub in &eb.items {
                    self.register_item(sub);
                }
            }
        }
    }

    fn lookup_from_scope(&self, mut scope: usize, name: Symbol, ns: Namespace) -> Option<DefId> {
        loop {
            if let Some(&def_id) = self.scopes[scope].bindings.get(&(name, ns)) {
                return Some(def_id);
            }
            if let Some(parent) = self.scopes[scope].parent {
                scope = parent;
            } else {
                return None;
            }
        }
    }

    fn register_impl_methods(&mut self, impl_block: &HirImplBlock) {
        // Get the named self type from the path. Module-qualified impls such as
        // `impl crate::m::T` still register items against `T`.
        let self_ty_name = match &impl_block.self_ty {
            HirTy::Path(p) if !p.segments.is_empty() => p.segments.last().unwrap().ident,
            _ => return,
        };

        for item in &impl_block.items {
            match &item.kind {
                HirItemKind::Fn(f) => {
                    self.impl_methods
                        .entry(self_ty_name)
                        .or_default()
                        .push((f.name, f.def_id));
                }
                HirItemKind::Const(c) => {
                    self.impl_assoc_values
                        .entry(self_ty_name)
                        .or_default()
                        .push((c.name, c.def_id));
                }
                HirItemKind::Static(s) => {
                    self.impl_assoc_values
                        .entry(self_ty_name)
                        .or_default()
                        .push((s.name, s.def_id));
                }
                HirItemKind::TypeAlias(ta) => {
                    self.impl_assoc_types
                        .entry(self_ty_name)
                        .or_default()
                        .push((ta.name, ta.def_id));
                }
                _ => {}
            }
        }
    }

    fn register_trait_assoc_items(&mut self, trait_def: &HirTraitDef) {
        for item in &trait_def.items {
            match &item.kind {
                HirItemKind::Fn(f) => {
                    self.impl_methods
                        .entry(trait_def.name)
                        .or_default()
                        .push((f.name, f.def_id));
                }
                HirItemKind::Const(c) => {
                    self.impl_assoc_values
                        .entry(trait_def.name)
                        .or_default()
                        .push((c.name, c.def_id));
                }
                HirItemKind::Static(s) => {
                    self.impl_assoc_values
                        .entry(trait_def.name)
                        .or_default()
                        .push((s.name, s.def_id));
                }
                HirItemKind::TypeAlias(ta) => {
                    self.impl_assoc_types
                        .entry(trait_def.name)
                        .or_default()
                        .push((ta.name, ta.def_id));
                }
                _ => {}
            }
        }
    }

    fn resolve_item(&mut self, item: &HirItem) {
        let old_item_name = self.current_item_name;
        self.current_item_name = match &item.kind {
            HirItemKind::Fn(f) => Some(f.name),
            HirItemKind::Struct(s) => Some(s.name),
            HirItemKind::Enum(e) => Some(e.name),
            HirItemKind::Trait(t) => Some(t.name),
            HirItemKind::TypeAlias(t) => Some(t.name),
            HirItemKind::Const(c) => Some(c.name),
            HirItemKind::Static(s) => Some(s.name),
            HirItemKind::Mod(m) => Some(m.name),
            HirItemKind::Impl(_) | HirItemKind::Use(_) | HirItemKind::ExternBlock(_) => None,
        };
        match &item.kind {
            HirItemKind::Fn(f) => self.resolve_fn(f),
            HirItemKind::Struct(s) => {
                self.push_scope();
                if let Some(self_sym) = self.find_symbol("Self") {
                    self.define(self_sym, Namespace::Type, s.def_id);
                }
                self.resolve_generics(&s.generics);
                for field in &s.fields {
                    self.resolve_ty(&field.ty);
                }
                self.pop_scope();
            }
            HirItemKind::Enum(e) => {
                self.push_scope();
                if let Some(self_sym) = self.find_symbol("Self") {
                    self.define(self_sym, Namespace::Type, e.def_id);
                }
                self.resolve_generics(&e.generics);
                for v in &e.variants {
                    match &v.fields {
                        HirVariantFields::Tuple(tys) => {
                            for ty in tys { self.resolve_ty(ty); }
                        }
                        HirVariantFields::Struct(fields) => {
                            for f in fields { self.resolve_ty(&f.ty); }
                        }
                        HirVariantFields::Unit => {}
                    }
                    if let Some(disc) = &v.discriminant {
                        self.resolve_expr(disc);
                    }
                }
                self.pop_scope();
            }
            HirItemKind::Impl(ib) => self.resolve_impl(ib),
            HirItemKind::Trait(t) => self.resolve_trait(t),
            HirItemKind::TypeAlias(ta) => {
                self.resolve_generics(&ta.generics);
                if let Some(ty) = &ta.ty {
                    self.resolve_ty(ty);
                }
            }
            HirItemKind::Const(c) => {
                self.resolve_ty(&c.ty);
                if let Some(val) = &c.value {
                    self.resolve_expr(val);
                }
            }
            HirItemKind::Static(s) => {
                self.resolve_ty(&s.ty);
                if let Some(val) = &s.value {
                    self.resolve_expr(val);
                }
            }
            HirItemKind::Mod(m) => {
                if let Some(sub_items) = &m.items {
                    if let Some(&scope_idx) = self.module_scopes.get(&m.def_id) {
                        let saved = self.current_scope;
                        let saved_item_name = self.current_item_name;
                        self.current_scope = scope_idx;
                        self.module_stack.push(scope_idx);
                        self.current_module_path.push(m.name);
                        self.current_item_name = None;
                        for sub in sub_items {
                            self.resolve_item(sub);
                        }
                        self.current_item_name = saved_item_name;
                        self.current_module_path.pop();
                        self.module_stack.pop();
                        self.current_scope = saved;
                    }
                }
            }
            HirItemKind::Use(_) => {}
            HirItemKind::ExternBlock(eb) => {
                for sub in &eb.items {
                    self.resolve_item(sub);
                }
            }
        }
        self.current_item_name = old_item_name;
    }

    fn resolve_fn(&mut self, f: &HirFnDef) {
        self.push_scope();
        self.resolve_generics(&f.generics);
        for p in &f.params {
            self.resolve_ty(&p.ty);
            self.resolve_pattern_binding(&p.pat);
        }
        if let Some(ret) = &f.ret_ty {
            self.resolve_ty(ret);
        }
        if let Some(body) = &f.body {
            self.resolve_block(body);
        }
        self.pop_scope();
    }

    fn resolve_impl(&mut self, ib: &HirImplBlock) {
        self.push_scope();
        self.resolve_generics(&ib.generics);
        for item in &ib.items {
            match &item.kind {
                HirItemKind::Const(c) => self.define(c.name, Namespace::Value, c.def_id),
                HirItemKind::Static(s) => self.define(s.name, Namespace::Value, s.def_id),
                _ => {}
            }
        }

        let saved_impl_self_ty = self.current_impl_self_ty;
        let saved_impl_self_def_id = self.current_impl_self_def_id;

        // Define Self in type namespace when we can tie it back to a named type.
        if let HirTy::Path(p) = &ib.self_ty {
            if !p.segments.is_empty() {
                let self_name = p.segments.last().unwrap().ident;
                self.current_impl_self_ty = Some(self_name);
                let path = p.segments.iter().map(|seg| seg.ident).collect::<Vec<_>>();
                if let Some((def_id, _)) = self.resolve_use_path_ns_direct(&path, Namespace::Type) {
                    self.current_impl_self_def_id = Some(def_id);
                    // Register Self if we can find the symbol
                    if let Some(self_sym) = self.find_symbol("Self") {
                        self.define(self_sym, Namespace::Type, def_id);
                    }
                } else if let Some(def_id) = self.lookup(self_name, Namespace::Type) {
                    self.current_impl_self_def_id = Some(def_id);
                    if let Some(self_sym) = self.find_symbol("Self") {
                        self.define(self_sym, Namespace::Type, def_id);
                    }
                }
            }
        }
        if self.find_symbol("Self").is_some() && self.lookup(self.find_symbol("Self").unwrap(), Namespace::Type).is_none() {
            let synthetic_self = self.alloc_synthetic_def_id();
            self.define(self.find_symbol("Self").unwrap(), Namespace::Type, synthetic_self);
        }

        self.resolve_ty(&ib.self_ty);
        if let Some(tr) = &ib.trait_ref {
            self.resolve_path(tr, Namespace::Type, ib.id);
        }
        for item in &ib.items {
            match &item.kind {
                HirItemKind::TypeAlias(ta) => {
                    if let Some(ty) = &ta.ty {
                        self.resolve_ty(ty);
                    }
                }
                _ => self.resolve_item(item),
            }
        }
        self.current_impl_self_ty = saved_impl_self_ty;
        self.current_impl_self_def_id = saved_impl_self_def_id;
        self.pop_scope();
    }

    fn resolve_trait(&mut self, t: &HirTraitDef) {
        self.push_scope();

        // Define Self as the trait itself in type namespace
        if let Some(self_sym) = self.find_symbol("Self") {
            self.define(self_sym, Namespace::Type, t.def_id);
        }

        self.resolve_generics(&t.generics);
        self.register_assoc_items_in_current_scope(&t.items);
        for bound in &t.supertraits {
            self.resolve_path(&bound.path, Namespace::Type, HirId(u32::MAX));
        }
        for item in &t.items {
            self.resolve_item(item);
        }
        self.pop_scope();
    }

    fn resolve_generics(&mut self, generics: &HirGenerics) {
        for param in &generics.params {
            match param {
                HirGenericParam::Type(name, _, _, _) => {
                    let did = self.alloc_synthetic_def_id();
                    self.define(*name, Namespace::Type, did);
                }
                HirGenericParam::Lifetime(_, _, _) => {}
                HirGenericParam::Const(name, _, _) => {
                    let did = self.alloc_synthetic_def_id();
                    self.define(*name, Namespace::Value, did);
                }
            }
        }

        for param in &generics.params {
            match param {
                HirGenericParam::Type(_, bounds, default, _) => {
                    // Trait bounds are not enforced — resolve them if possible,
                    // but silently ignore unresolved ones.
                    for bound in bounds {
                        self.resolve_trait_bound_path(&bound.path);
                    }
                    if let Some(default) = default {
                        self.resolve_ty(default);
                    }
                }
                HirGenericParam::Lifetime(_, _, _) => {}
                HirGenericParam::Const(_, ty, _) => {
                    self.resolve_ty(ty);
                }
            }
        }
    }

    /// Try to resolve a trait bound path; silently ignore if not found
    /// (we don't have a trait solver, so bounds are informational only).
    fn resolve_trait_bound_path(&mut self, path: &HirPath) {
        if path.segments.is_empty() { return; }
        let name = path.segments[0].ident;
        // Resolve generic args in segments
        for seg in &path.segments {
            if let Some(args) = &seg.args {
                for arg in &args.args {
                    match arg {
                        HirGenericArg::Type(ty) => self.resolve_ty(ty),
                        HirGenericArg::AssocTypeBinding(_, ty) => self.resolve_ty(ty),
                        HirGenericArg::Const(e) => self.resolve_expr(e),
                        HirGenericArg::Lifetime(_) => {}
                    }
                }
            }
        }
        // Try to resolve but don't emit error if not found
        let _ = self.lookup(name, Namespace::Type);
    }

    fn resolve_expr(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::Lit(_) => {}
            HirExprKind::Path(path) => {
                self.resolve_path(path, Namespace::Value, expr.id);
            }
            HirExprKind::QualifiedPath(qpath) => {
                self.resolve_qualified_path(qpath, Namespace::Value, expr.id);
            }
            HirExprKind::Binary(_, l, r) => {
                self.resolve_expr(l);
                self.resolve_expr(r);
            }
            HirExprKind::Unary(_, e) => self.resolve_expr(e),
            HirExprKind::Call(callee, args) => {
                self.resolve_expr(callee);
                for a in args { self.resolve_expr(a); }
            }
            HirExprKind::MethodCall(recv, _, tys, args) => {
                self.resolve_expr(recv);
                for ty in tys { self.resolve_ty(ty); }
                for a in args { self.resolve_expr(a); }
            }
            HirExprKind::Field(e, _) => self.resolve_expr(e),
            HirExprKind::Index(a, b) => {
                self.resolve_expr(a);
                self.resolve_expr(b);
            }
            HirExprKind::Block(block) => self.resolve_block(block),
            HirExprKind::If(cond, then, else_) => {
                self.resolve_expr(cond);
                self.resolve_block(then);
                if let Some(e) = else_ { self.resolve_expr(e); }
            }
            HirExprKind::Match(scrutinee, arms) => {
                self.resolve_expr(scrutinee);
                for arm in arms {
                    self.push_scope();
                    self.resolve_pattern_binding(&arm.pat);
                    if let Some(guard) = &arm.guard { self.resolve_expr(guard); }
                    self.resolve_expr(&arm.body);
                    self.pop_scope();
                }
            }
            HirExprKind::Loop(block, _) => self.resolve_block(block),
            HirExprKind::Closure(params, ret_ty, body, _) => {
                self.push_scope();
                for p in params {
                    self.resolve_ty(&p.ty);
                    self.resolve_pattern_binding(&p.pat);
                }
                if let Some(ret) = ret_ty { self.resolve_ty(ret); }
                self.resolve_expr(body);
                self.pop_scope();
            }
            HirExprKind::Return(e) => {
                if let Some(e) = e { self.resolve_expr(e); }
            }
            HirExprKind::Break(_, e) => {
                if let Some(e) = e { self.resolve_expr(e); }
            }
            HirExprKind::Continue(_) => {}
            HirExprKind::Assign(l, r) | HirExprKind::AssignOp(_, l, r) => {
                self.resolve_expr(l);
                self.resolve_expr(r);
            }
            HirExprKind::Ref(e, _) | HirExprKind::Deref(e) | HirExprKind::Paren(e) | HirExprKind::Try(e) => {
                self.resolve_expr(e);
            }
            HirExprKind::Cast(e, ty) => {
                self.resolve_expr(e);
                self.resolve_ty(ty);
            }
            HirExprKind::Struct(path, fields, base) => {
                self.resolve_path(path, Namespace::Type, expr.id);
                for f in fields { self.resolve_expr(&f.value); }
                if let Some(b) = base { self.resolve_expr(b); }
            }
            HirExprKind::Tuple(es) | HirExprKind::Array(es) => {
                for e in es { self.resolve_expr(e); }
            }
            HirExprKind::ArrayRepeat(a, b) => {
                self.resolve_expr(a);
                self.resolve_expr(b);
            }
            HirExprKind::Range(a, b, _) => {
                if let Some(a) = a { self.resolve_expr(a); }
                if let Some(b) = b { self.resolve_expr(b); }
            }
            HirExprKind::Unsafe(block) => self.resolve_block(block),
            HirExprKind::For(pat, iter, body, _) => {
                self.resolve_expr(iter);
                self.push_scope();
                self.resolve_pattern_binding(pat);
                self.resolve_block(body);
                self.pop_scope();
            }
            HirExprKind::InlineAsm(asm) => {
                for op in &asm.operands {
                    match op {
                        crate::hir::HirAsmOperand::In { expr, .. } => self.resolve_expr(expr),
                        crate::hir::HirAsmOperand::Out { expr: Some(e), .. } => self.resolve_expr(e),
                        crate::hir::HirAsmOperand::InOut { expr, out_expr, .. } => {
                            self.resolve_expr(expr);
                            if let Some(out_expr) = out_expr {
                                self.resolve_expr(out_expr);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Define bindings introduced by a pattern
    fn resolve_pattern_binding(&mut self, pat: &HirPattern) {
        match pat {
            HirPattern::Ident(hir_id, name, _, sub, _) => {
                let did = self.alloc_synthetic_def_id();
                self.define(*name, Namespace::Value, did);
                self.resolutions.insert(*hir_id, did);
                if let Some(sub) = sub { self.resolve_pattern_binding(sub); }
            }
            HirPattern::Tuple(pats, _) | HirPattern::Slice(pats, _) => {
                for p in pats { self.resolve_pattern_binding(p); }
            }
            HirPattern::Struct(path, fields, _, _) => {
                self.resolve_path(path, Namespace::Type, HirId(u32::MAX));
                for fp in fields { self.resolve_pattern_binding(&fp.pat); }
            }
            HirPattern::TupleStruct(path, pats, _) => {
                self.resolve_path(path, Namespace::Value, HirId(u32::MAX));
                for p in pats { self.resolve_pattern_binding(p); }
            }
            HirPattern::Ref(p, _, _) | HirPattern::RefBinding(p, _, _) => self.resolve_pattern_binding(p),
            HirPattern::Or(pats, _) => {
                for p in pats { self.resolve_pattern_binding(p); }
            }
            HirPattern::Path(path) => {
                self.resolve_path(path, Namespace::Value, HirId(u32::MAX));
            }
            HirPattern::Wildcard(_)
            | HirPattern::Rest(_)
            | HirPattern::Literal(_, _)
            | HirPattern::Range(_, _, _, _) => {}
        }
    }

    fn resolve_block(&mut self, block: &HirBlock) {
        self.push_scope();
        // Rust item declarations are visible throughout the containing block,
        // independent of textual order. Register them before resolving
        // expressions so local structs/enums can refer to sibling items that
        // appear later in the block.
        for stmt in &block.stmts {
            if let HirStmt::Item(item) = stmt {
                self.register_item(item);
                if let HirItemKind::Use(u) = &item.kind {
                    self.process_use_tree(u);
                }
            }
        }
        for stmt in &block.stmts {
            match stmt {
                HirStmt::Let(_, pat, ty, init, _) => {
                    // Resolve init first (before introducing binding)
                    if let Some(init) = init { self.resolve_expr(init); }
                    if let Some(ty) = ty { self.resolve_ty(ty); }
                    self.resolve_pattern_binding(pat);
                }
                HirStmt::Expr(e) | HirStmt::Semi(e, _) => self.resolve_expr(e),
                HirStmt::Item(item) => {
                    self.resolve_item(item);
                }
            }
        }
        self.pop_scope();
    }

    fn resolve_ty(&mut self, ty: &HirTy) {
        match ty {
            HirTy::Path(path) => {
                self.resolve_path(path, Namespace::Type, HirId(u32::MAX));
            }
            HirTy::QualifiedPath(qpath) => {
                self.resolve_qualified_path(qpath, Namespace::Type, HirId(u32::MAX));
            }
            HirTy::Reference(_, t, _, _) | HirTy::RawPtr(t, _, _) | HirTy::Slice(t, _) => {
                self.resolve_ty(t);
            }
            HirTy::Tuple(tys, _) => {
                for t in tys { self.resolve_ty(t); }
            }
            HirTy::Array(t, len, _) => {
                self.resolve_ty(t);
                self.resolve_expr(len);
            }
            HirTy::FnPtr(params, ret, _) => {
                for p in params { self.resolve_ty(p); }
                if let Some(r) = ret { self.resolve_ty(r); }
            }
            HirTy::DynTrait(bounds, _) => {
                for bound in bounds {
                    self.resolve_trait_bound_path(&bound.path);
                }
            }
            HirTy::MacroCall(_, _) | HirTy::Infer(_) | HirTy::Never(_) => {}
        }
    }

    fn resolve_qualified_path(&mut self, qpath: &HirQualifiedPath, ns: Namespace, hir_id: HirId) {
        self.resolve_ty(&qpath.self_ty);
        if let Some(trait_path) = &qpath.trait_path {
            self.resolve_trait_bound_path(trait_path);
        }
        for seg in &qpath.path.segments {
            if let Some(args) = &seg.args {
                for arg in &args.args {
                    match arg {
                        HirGenericArg::Type(ty) => self.resolve_ty(ty),
                        HirGenericArg::AssocTypeBinding(_, ty) => self.resolve_ty(ty),
                        HirGenericArg::Const(e) => self.resolve_expr(e),
                        HirGenericArg::Lifetime(_) => {}
                    }
                }
            }
        }

        let Some(last) = qpath.path.segments.last() else {
            return;
        };
        if let Some(trait_path) = &qpath.trait_path {
            if let Some(trait_name) = trait_path.segments.last().map(|seg| seg.ident) {
                if self.resolve_assoc_item_on_type(trait_name, last.ident, hir_id) {
                    return;
                }
                if let Some(trait_def_id) = self.lookup(trait_name, Namespace::Type) {
                    if self.intrinsic_fns.contains_key(&trait_def_id) {
                        let trait_path_str = self.path_to_string(
                            &trait_path.segments.iter().map(|seg| seg.ident).collect::<Vec<_>>()
                        );
                        let assoc_str = self.interner.resolve(last.ident).to_string();
                        let def_id = self.alloc_synthetic_def_id();
                        self.intrinsic_fns.insert(def_id, format!("{}::{}", trait_path_str, assoc_str));
                        if hir_id != HirId(u32::MAX) {
                            self.resolutions.insert(hir_id, def_id);
                        }
                        return;
                    }
                    // A qualified associated item may be resolved by type checking once
                    // trait obligations are known.
                    return;
                }
            }
        }

        // `<T>::Assoc` is a projection on the self type. The resolver leaves it
        // for type checking instead of treating `Assoc` as a free name.
        if ns == Namespace::Type || hir_id != HirId(u32::MAX) {
            return;
        }
    }

    fn resolve_path(&mut self, path: &HirPath, ns: Namespace, hir_id: HirId) {
        if path.segments.is_empty() { return; }

        let first_seg = &path.segments[0];
        let name = first_seg.ident;
        let name_str = self.interner.resolve(name).to_string();

        // Check for primitive types in type namespace
        if ns == Namespace::Type && path.segments.len() == 1 {
            if self.primitives.iter().any(|&p| p == name_str) {
                // Primitive type - no resolution needed
                return;
            }
        }

        // Allow "self" parameter - try to resolve it, but don't error if not found
        if name_str == "self" && path.segments.len() == 1 {
            if let Some(def_id) = self.lookup(name, ns) {
                if hir_id != HirId(u32::MAX) {
                    self.resolutions.insert(hir_id, def_id);
                }
            }
            return;
        }

        // Resolve generic args in all segments
        for seg in &path.segments {
            if let Some(args) = &seg.args {
                for arg in &args.args {
                    match arg {
                        HirGenericArg::Type(ty) => self.resolve_ty(ty),
                        HirGenericArg::AssocTypeBinding(_, ty) => self.resolve_ty(ty),
                        HirGenericArg::Const(e) => self.resolve_expr(e),
                        HirGenericArg::Lifetime(_) => {}
                    }
                }
            }
        }

        if path.segments.len() >= 2 {
            let symbols = path.segments.iter().map(|seg| seg.ident).collect::<Vec<_>>();
            if let Some(full_path) = self.extern_crate_path_string(&symbols) {
                let def_id = self.alloc_synthetic_def_id();
                self.intrinsic_fns.insert(def_id, full_path);
                if hir_id != HirId(u32::MAX) {
                    self.resolutions.insert(hir_id, def_id);
                }
                return;
            }
            if let Some(full_path) = self.resolve_extern_backed_path_string(&symbols) {
                let def_id = self.alloc_synthetic_def_id();
                self.intrinsic_fns.insert(def_id, full_path);
                if hir_id != HirId(u32::MAX) {
                    self.resolutions.insert(hir_id, def_id);
                }
                return;
            }
        }

        // Handle crate::, super::, self:: prefixes for multi-segment paths
        if path.segments.len() >= 2 {
            if name_str == "crate" || name_str == "super" || name_str == "self" {
                self.resolve_module_path(path, ns, hir_id);
                return;
            }
        }

        // Handle sysroot and anyOS runtime crate paths as compiler-known intrinsics.
        if path.segments.len() >= 2
            && Self::is_compiler_known_external_crate(name_str.as_str())
            && !self
                .scopes[self.root_scope]
                .bindings
                .get(&(name, Namespace::Type))
                .is_some_and(|def_id| self.module_scopes.contains_key(&def_id))
        {
            let full_path = path.segments.iter()
                .map(|s| self.interner.resolve(s.ident).to_string())
                .collect::<Vec<_>>()
                .join("::");
            let def_id = self.alloc_synthetic_def_id();
            self.intrinsic_fns.insert(def_id, full_path);
            if hir_id != HirId(u32::MAX) {
                self.resolutions.insert(hir_id, def_id);
            }
            return;
        }

        if path.segments.len() == 1 {
            // Simple single-segment path
            if let Some(def_id) = self.lookup(name, ns) {
                if hir_id != HirId(u32::MAX) {
                    self.resolutions.insert(hir_id, def_id);
                }
            } else {
                // Try the other namespace as fallback
                let other_ns = if ns == Namespace::Value { Namespace::Type } else { Namespace::Value };
                if let Some(def_id) = self.lookup(name, other_ns) {
                    if hir_id != HirId(u32::MAX) {
                        self.resolutions.insert(hir_id, def_id);
                    }
                } else if self.resolve_arch_intrinsic_symbol(name, hir_id) {
                    return;
                } else {
                    self.error(path.span, &format!("`{}` not found in this scope", name_str));
                }
            }
        } else if path.segments.len() >= 2 {
            // Try module path resolution first
            if let Some(def_id) = self.try_resolve_through_modules(path, ns) {
                if hir_id != HirId(u32::MAX) {
                    self.resolutions.insert(hir_id, def_id);
                }
                return;
            }

            if path.segments.len() == 2 {
                // Two-segment path: Enum::Variant or Type::method
                let second_name = path.segments[1].ident;
                if name_str == "Self" && ns == Namespace::Type {
                    return;
                }
                let type_name = if name_str == "Self" {
                    if let Some(def_id) = self.current_impl_self_def_id {
                        if self.resolve_assoc_item_on_type_def(def_id, second_name, hir_id) {
                            return;
                        }
                    }
                    self.current_impl_self_ty.unwrap_or(name)
                } else {
                    name
                };
                if self.resolve_assoc_item_on_type(type_name, second_name, hir_id) {
                    return;
                }

                // Generic and trait-associated projections such as `T::Item`
                // are type-level names whose final meaning depends on bounds.
                // Leave them for type checking once the left side is a type.
                if self.lookup(type_name, Namespace::Type).is_some() {
                    return;
                }

                // Could not resolve
                let second_str = self.interner.resolve(second_name);
                self.error(path.span, &format!("`{}::{}` not found", name_str, second_str));
            }
            // else: 3+ segments without module match - skip (external paths)
        }
    }

    fn resolve_arch_intrinsic_symbol(&mut self, name: Symbol, hir_id: HirId) -> bool {
        let name_str = self.interner.resolve(name).to_string();
        if !Self::is_arch_intrinsic_symbol(&name_str) {
            return false;
        }
        let def_id = self.alloc_synthetic_def_id();
        self.intrinsic_fns
            .insert(def_id, format!("core::arch::x86_64::{}", name_str));
        self.define(name, Namespace::Value, def_id);
        self.define(name, Namespace::Type, def_id);
        if hir_id != HirId(u32::MAX) {
            self.resolutions.insert(hir_id, def_id);
        }
        true
    }

    fn is_arch_intrinsic_symbol(name: &str) -> bool {
        name.starts_with("_mm")
            || matches!(
                name,
                "__m64"
                    | "__m128"
                    | "__m128d"
                    | "__m128i"
                    | "__m256"
                    | "__m256d"
                    | "__m256i"
                    | "__m512"
                    | "__m512d"
                    | "__m512i"
            )
    }

    /// Try to resolve a path by walking through module scopes.
    /// Returns Some(DefId) if the first segment is a module and the path resolves.
    fn try_resolve_through_modules(&self, path: &HirPath, ns: Namespace) -> Option<DefId> {
        let first = path.segments[0].ident;

        // Check if first segment is a module
        let mod_def_id = self.lookup(first, Namespace::Type)?;
        let mut scope = *self.module_scopes.get(&mod_def_id)?;

        // Walk remaining segments
        for (i, seg) in path.segments[1..].iter().enumerate() {
            let is_last = i == path.segments.len() - 2;
            if is_last {
                // Last segment: look up as value or type
                if let Some(&def_id) = self.scopes[scope].bindings.get(&(seg.ident, ns)) {
                    return Some(def_id);
                }
                // Try other namespace
                let other_ns = if ns == Namespace::Value { Namespace::Type } else { Namespace::Value };
                if let Some(&def_id) = self.scopes[scope].bindings.get(&(seg.ident, other_ns)) {
                    return Some(def_id);
                }
                // Try enum variant: if the last two segments are Enum::Variant
                // Actually for module paths this doesn't apply at this level
                return None;
            } else {
                // Intermediate segment: must be a module or a type (for Type::method paths)
                if let Some(&sub_def_id) = self.scopes[scope].bindings.get(&(seg.ident, Namespace::Type)) {
                    if let Some(&sub_scope) = self.module_scopes.get(&sub_def_id) {
                        scope = sub_scope;
                    } else {
                        // Not a module - might be Type::method. Check if next segment
                        // is the last and resolves as an impl method.
                        let next_idx = i + 1;
                        if next_idx == path.segments.len() - 2 {
                            let method_name = path.segments[next_idx + 1].ident;
                            if let Some(methods) = self.impl_methods.get(&seg.ident) {
                                for &(mname, method_def_id) in methods {
                                    if mname == method_name {
                                        return Some(method_def_id);
                                    }
                                }
                            }
                            if let Some(enum_info) = self.enum_variants.get(&sub_def_id) {
                                if let Some(&variant_def_id) = enum_info.variants.get(&method_name) {
                                    return Some(variant_def_id);
                                }
                            }
                        }
                        return None;
                    }
                } else {
                    return None;
                }
            }
        }
        None
    }

    /// Resolve a path starting with crate::, super::, or self::
    fn resolve_module_path(&mut self, path: &HirPath, ns: Namespace, hir_id: HirId) {
        let first_str = self.interner.resolve(path.segments[0].ident);

        let start_scope = if first_str == "crate" {
            self.root_scope
        } else if first_str == "super" {
            if self.module_stack.len() >= 2 {
                self.module_stack[self.module_stack.len() - 2]
            } else {
                self.root_scope
            }
        } else {
            // "self"
            *self.module_stack.last().unwrap_or(&self.root_scope)
        };

        let mut scope = start_scope;
        let segments = &path.segments[1..]; // skip crate/super/self

        for (i, seg) in segments.iter().enumerate() {
            let is_last = i == segments.len() - 1;
            if is_last {
                // Last segment: look up as value or type
                if let Some(&def_id) = self.scopes[scope].bindings.get(&(seg.ident, ns)) {
                    if hir_id != HirId(u32::MAX) {
                        self.resolutions.insert(hir_id, def_id);
                    }
                    return;
                }
                // Try other namespace
                let other_ns = if ns == Namespace::Value { Namespace::Type } else { Namespace::Value };
                if let Some(&def_id) = self.scopes[scope].bindings.get(&(seg.ident, other_ns)) {
                    if hir_id != HirId(u32::MAX) {
                        self.resolutions.insert(hir_id, def_id);
                    }
                    return;
                }
                let seg_str = self.interner.resolve(seg.ident);
                self.error(path.span, &format!(
                    "`{}` not found in module{}",
                    seg_str,
                    self.debug_type_bindings_suffix(scope),
                ));
            } else {
                // Intermediate segment: must be a module
                if let Some(&mod_def_id) = self.scopes[scope].bindings.get(&(seg.ident, Namespace::Type)) {
                    if let Some(&mod_scope) = self.module_scopes.get(&mod_def_id) {
                        scope = mod_scope;
                    } else {
                        let next_idx = i + 1;
                        if next_idx == segments.len() - 1 {
                            let method_name = segments[next_idx].ident;
                            if self.resolve_assoc_item_on_type(seg.ident, method_name, hir_id) {
                                return;
                            }
                            // Trait/type associated paths reached through a module
                            // prefix (`crate::m::Trait::item`) are resolved by
                            // type checking if no concrete impl item is known yet.
                            return;
                        }
                        let seg_str = self.interner.resolve(seg.ident);
                        self.error(path.span, &format!("`{}` is not a module", seg_str));
                        return;
                    }
                } else {
                    let seg_str = self.interner.resolve(seg.ident);
                    self.error(path.span, &format!(
                        "`{}` not found in module{}",
                        seg_str,
                        self.debug_type_bindings_suffix(scope),
                    ));
                    return;
                }
            }
        }
    }

    fn debug_type_bindings_suffix(&self, scope: usize) -> String {
        let mut names = self.scopes[scope]
            .bindings
            .keys()
            .filter_map(|(name, ns)| {
                if *ns == Namespace::Type {
                    Some(self.interner.resolve(*name).to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        if names.is_empty() {
            String::new()
        } else {
            format!("; available types: {}", names.join(", "))
        }
    }

    fn push_scope(&mut self) {
        let new_scope = Scope {
            parent: Some(self.current_scope),
            bindings: HashMap::new(),
        };
        self.scopes.push(new_scope);
        self.current_scope = self.scopes.len() - 1;
    }

    fn pop_scope(&mut self) {
        if let Some(parent) = self.scopes[self.current_scope].parent {
            self.current_scope = parent;
        }
    }

    fn define(&mut self, name: Symbol, ns: Namespace, def_id: DefId) {
        self.scopes[self.current_scope].bindings.insert((name, ns), def_id);
    }

    fn lookup(&self, name: Symbol, ns: Namespace) -> Option<DefId> {
        let mut scope_idx = self.current_scope;
        loop {
            if let Some(&def_id) = self.scopes[scope_idx].bindings.get(&(name, ns)) {
                return Some(def_id);
            }
            if let Some(parent) = self.scopes[scope_idx].parent {
                scope_idx = parent;
            } else {
                return None;
            }
        }
    }

    /// Find a symbol by string in the interner without interning it.
    fn find_symbol(&self, s: &str) -> Option<Symbol> {
        self.interner.lookup(s)
    }
}

use crate::prelude::*;
use crate::hir::*;
use crate::intern::{Interner, Symbol};
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
    /// Module DefId -> scope index that contains the module's items
    module_scopes: HashMap<DefId, usize>,
    /// Root scope index (for `crate::` paths)
    root_scope: usize,
    /// Stack of module scope indices for `super::` resolution
    module_stack: Vec<usize>,
    /// Intrinsic DefIds: maps synthetic DefId to full path string
    intrinsic_fns: HashMap<DefId, String>,
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
            primitives: vec![
                "i8", "i16", "i32", "i64", "i128", "isize",
                "u8", "u16", "u32", "u64", "u128", "usize",
                "f32", "f64", "bool", "char", "str",
            ],
            impl_methods: HashMap::new(),
            module_scopes: HashMap::new(),
            root_scope: 0,
            module_stack: vec![0],
            intrinsic_fns: HashMap::new(),
        };
        this.bootstrap_prelude_intrinsics();
        this
    }

    fn bootstrap_prelude_intrinsics(&mut self) {
        self.define_intrinsic_type("Option", "Option");
        self.define_intrinsic_type("Result", "Result");
        self.define_intrinsic_type("Vec", "Vec");
        self.define_intrinsic_type("String", "String");
        self.define_intrinsic_type("Box", "Box");

        self.define_intrinsic_value("Some", "Option::Some");
        self.define_intrinsic_value("None", "Option::None");
        self.define_intrinsic_value("Ok", "Result::Ok");
        self.define_intrinsic_value("Err", "Result::Err");

        self.define_intrinsic_value("__anyrc_println", "__anyrc_println");
        self.define_intrinsic_value("__anyrc_format", "__anyrc_format");
        self.define_intrinsic_value("Vec::new", "Vec::new");
        self.define_intrinsic_value("exit", "exit");
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

    pub fn resolve_crate(&mut self, krate: &HirCrate) -> ResolveResult {
        // First pass: register all top-level items (recursing into modules)
        for item in &krate.items {
            self.register_item(item);
        }

        // Second pass: collect impl methods (recursing into modules)
        self.collect_impls_recursive(&krate.items);

        // Process use items
        self.process_use_items_recursive(&krate.items);

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
                // Handle `use core::...` imports as intrinsics
                if self.is_extern_crate_path(&use_tree.path) {
                    let local_name = alias.unwrap_or_else(|| {
                        *use_tree.path.last().unwrap()
                    });
                    let full_path = self.path_to_string(&use_tree.path);
                    let def_id = self.alloc_synthetic_def_id();
                    self.intrinsic_fns.insert(def_id, full_path);
                    self.define(local_name, Namespace::Value, def_id);
                    self.define(local_name, Namespace::Type, def_id);
                    return;
                }
                // use a::b::c; or use a::b::c as d;
                if let Some((def_id, ns)) = self.resolve_use_path(&use_tree.path, use_tree.span) {
                    let local_name = alias.unwrap_or_else(|| {
                        *use_tree.path.last().unwrap()
                    });
                    self.define(local_name, ns, def_id);
                    // Also define in the other namespace for cross-ns usage
                    let other_ns = if ns == Namespace::Value { Namespace::Type } else { Namespace::Value };
                    // Try other ns too - don't error if not found
                    if let Some((def_id2, _)) = self.resolve_use_path_ns(&use_tree.path, other_ns) {
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
                        path: full_path,
                        kind: sub.kind.clone(),
                        span: sub.span,
                    };
                    self.process_use_tree(&combined);
                }
            }
            HirUseTreeKind::Glob => {
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
                    }
                }
            }
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

    /// Check if a path represents an external known module (core, alloc)
    fn is_extern_crate_path(&self, path: &[Symbol]) -> bool {
        if path.is_empty() { return false; }
        let first = self.interner.resolve(path[0]);
        first == "core" || first == "alloc" || first == "anyos_std"
    }

    /// Build a full path string like "core::ptr::null_mut" from symbols
    fn path_to_string(&self, path: &[Symbol]) -> String {
        path.iter()
            .map(|s| self.interner.resolve(*s).to_string())
            .collect::<Vec<_>>()
            .join("::")
    }

    fn resolve_use_path_ns(&self, path: &[Symbol], ns: Namespace) -> Option<(DefId, Namespace)> {
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
                if let Some(&def_id) = self.scopes[scope].bindings.get(&(seg, ns)) {
                    return Some((def_id, ns));
                }
                // Try enum variant
                if ns == Namespace::Value {
                    // Try type ns for the segment - might be an enum/struct
                    if let Some(&def_id) = self.scopes[scope].bindings.get(&(seg, Namespace::Type)) {
                        return Some((def_id, Namespace::Type));
                    }
                }
                return None;
            } else {
                // Non-last segment: must be a module
                if let Some(&mod_def_id) = self.scopes[scope].bindings.get(&(seg, Namespace::Type)) {
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
        for &seg in &path[start_idx..] {
            if let Some(&mod_def_id) = self.scopes[scope].bindings.get(&(seg, Namespace::Type)) {
                if let Some(&mod_scope) = self.module_scopes.get(&mod_def_id) {
                    scope = mod_scope;
                    // Continue to next segment
                    // If this is the last segment, return this def_id
                    if core::ptr::eq(&seg, path.last().unwrap()) {
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

    fn register_impl_methods(&mut self, impl_block: &HirImplBlock) {
        // Get the self type name from the first segment of the type path
        let self_ty_name = match &impl_block.self_ty {
            HirTy::Path(p) if !p.segments.is_empty() => p.segments[0].ident,
            _ => return,
        };

        for item in &impl_block.items {
            if let HirItemKind::Fn(f) = &item.kind {
                self.impl_methods
                    .entry(self_ty_name)
                    .or_default()
                    .push((f.name, f.def_id));
            }
        }
    }

    fn resolve_item(&mut self, item: &HirItem) {
        match &item.kind {
            HirItemKind::Fn(f) => self.resolve_fn(f),
            HirItemKind::Struct(s) => {
                self.resolve_generics(&s.generics);
                for field in &s.fields {
                    self.resolve_ty(&field.ty);
                }
            }
            HirItemKind::Enum(e) => {
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
                        self.current_scope = scope_idx;
                        self.module_stack.push(scope_idx);
                        for sub in sub_items {
                            self.resolve_item(sub);
                        }
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

        // Define Self in type namespace - use the self_ty symbol directly
        if let HirTy::Path(p) = &ib.self_ty {
            if !p.segments.is_empty() {
                let first = p.segments[0].ident;
                if let Some(def_id) = self.lookup(first, Namespace::Type) {
                    // Register Self if we can find the symbol
                    if let Some(self_sym) = self.find_symbol("Self") {
                        self.define(self_sym, Namespace::Type, def_id);
                    }
                }
            }
        }

        self.resolve_ty(&ib.self_ty);
        if let Some(tr) = &ib.trait_ref {
            self.resolve_path(tr, Namespace::Type, ib.id);
        }
        for item in &ib.items {
            // Register methods in impl scope for potential use
            self.resolve_item(item);
        }
        self.pop_scope();
    }

    fn resolve_trait(&mut self, t: &HirTraitDef) {
        self.push_scope();

        // Define Self as the trait itself in type namespace
        if let Some(self_sym) = self.find_symbol("Self") {
            self.define(self_sym, Namespace::Type, t.def_id);
        }

        self.resolve_generics(&t.generics);
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
                HirGenericParam::Type(name, bounds, default, _) => {
                    let did = self.alloc_synthetic_def_id();
                    self.define(*name, Namespace::Type, did);
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
                HirGenericParam::Const(name, ty, _) => {
                    let did = self.alloc_synthetic_def_id();
                    self.define(*name, Namespace::Value, did);
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
                self.resolve_path(path, Namespace::Type, HirId(u32::MAX));
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
                        crate::hir::HirAsmOperand::InOut { expr, .. } => self.resolve_expr(expr),
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
            HirPattern::Tuple(pats, _) => {
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
            HirPattern::Ref(p, _, _) => self.resolve_pattern_binding(p),
            HirPattern::Or(pats, _) => {
                for p in pats { self.resolve_pattern_binding(p); }
            }
            HirPattern::Path(path) => {
                self.resolve_path(path, Namespace::Value, HirId(u32::MAX));
            }
            HirPattern::Wildcard(_) | HirPattern::Literal(_, _) | HirPattern::Range(_, _, _, _) => {}
        }
    }

    fn resolve_block(&mut self, block: &HirBlock) {
        self.push_scope();
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
                    self.register_item(item);
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
            HirTy::DynTrait(path, _) => {
                self.resolve_path(path, Namespace::Type, HirId(u32::MAX));
            }
            HirTy::Infer(_) | HirTy::Never(_) => {}
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
                        HirGenericArg::Const(e) => self.resolve_expr(e),
                        HirGenericArg::Lifetime(_) => {}
                    }
                }
            }
        }

        // Handle crate::, super::, self:: prefixes for multi-segment paths
        if path.segments.len() >= 2 {
            if name_str == "crate" || name_str == "super" || name_str == "self" {
                self.resolve_module_path(path, ns, hir_id);
                return;
            }
        }

        // Handle core::, alloc::, and anyos_std:: paths as intrinsics
        if path.segments.len() >= 2 && (name_str == "core" || name_str == "alloc" || name_str == "anyos_std") {
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
                } else {
                    self.errors.push(Diagnostic::new(
                        Level::Error,
                    &format!("`{}` not found in this scope", name_str),
                        path.span,
                    ));
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

                // Try enum variant first
                if let Some(enum_def_id) = self.lookup(name, Namespace::Type) {
                    if let Some(enum_info) = self.enum_variants.get(&enum_def_id) {
                        if let Some(&variant_def_id) = enum_info.variants.get(&second_name) {
                            if hir_id != HirId(u32::MAX) {
                                self.resolutions.insert(hir_id, variant_def_id);
                            }
                            return;
                        }
                    }
                    // Try impl methods
                    if let Some(methods) = self.impl_methods.get(&name) {
                        for &(method_name, method_def_id) in methods {
                            if method_name == second_name {
                                if hir_id != HirId(u32::MAX) {
                                    self.resolutions.insert(hir_id, method_def_id);
                                }
                                return;
                            }
                        }
                    }

                    // Check if first segment is an intrinsic type (e.g. AtomicBool::new, Ordering::Relaxed)
                    if self.intrinsic_fns.contains_key(&enum_def_id) {
                        let type_str = self.interner.resolve(name);
                        let method_str = self.interner.resolve(second_name);
                        let full_path = format!("{}::{}", type_str, method_str);
                        let def_id = self.alloc_synthetic_def_id();
                        self.intrinsic_fns.insert(def_id, full_path);
                        if hir_id != HirId(u32::MAX) {
                            self.resolutions.insert(hir_id, def_id);
                        }
                        return;
                    }
                }

                // Check if first segment is a primitive type with an associated constant
                if self.primitives.iter().any(|&p| p == name_str) {
                    let second_str = self.interner.resolve(second_name);
                    let is_assoc_const = matches!(
                        (name_str.as_str(), second_str),
                        ("u8", "MAX") | ("u16", "MAX") | ("u32", "MAX") | ("u64", "MAX") | ("u128", "MAX") | ("usize", "MAX")
                        | ("i8", "MAX") | ("i16", "MAX") | ("i32", "MAX") | ("i64", "MAX") | ("i128", "MAX") | ("isize", "MAX")
                        | ("i8", "MIN") | ("i16", "MIN") | ("i32", "MIN") | ("i64", "MIN") | ("i128", "MIN") | ("isize", "MIN")
                        | ("u8", "MIN") | ("u16", "MIN") | ("u32", "MIN") | ("u64", "MIN") | ("u128", "MIN") | ("usize", "MIN")
                    );
                    let is_assoc_fn = second_str == "from_le_bytes";
                    if is_assoc_const || is_assoc_fn {
                        let full_path = format!("{}::{}", name_str, second_str);
                        let def_id = self.alloc_synthetic_def_id();
                        self.intrinsic_fns.insert(def_id, full_path);
                        if hir_id != HirId(u32::MAX) {
                            self.resolutions.insert(hir_id, def_id);
                        }
                        return;
                    }
                }

                // Could not resolve
                let second_str = self.interner.resolve(second_name);
                self.errors.push(Diagnostic::new(
                    Level::Error,
                    &format!("`{}::{}` not found", name_str, second_str),
                    path.span,
                ));
            }
            // else: 3+ segments without module match - skip (external paths)
        }
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
                            // Look up impl methods for this type
                            if let Some(methods) = self.impl_methods.get(&seg.ident) {
                                for &(mname, method_def_id) in methods {
                                    if mname == method_name {
                                        return Some(method_def_id);
                                    }
                                }
                            }
                            // Try enum variants
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
                self.errors.push(Diagnostic::new(
                    Level::Error,
                    &format!("`{}` not found in module", seg_str),
                    path.span,
                ));
            } else {
                // Intermediate segment: must be a module
                if let Some(&mod_def_id) = self.scopes[scope].bindings.get(&(seg.ident, Namespace::Type)) {
                    if let Some(&mod_scope) = self.module_scopes.get(&mod_def_id) {
                        scope = mod_scope;
                    } else {
                        let seg_str = self.interner.resolve(seg.ident);
                        self.errors.push(Diagnostic::new(
                            Level::Error,
                            &format!("`{}` is not a module", seg_str),
                            path.span,
                        ));
                        return;
                    }
                } else {
                    let seg_str = self.interner.resolve(seg.ident);
                    self.errors.push(Diagnostic::new(
                        Level::Error,
                        &format!("`{}` not found in module", seg_str),
                        path.span,
                    ));
                    return;
                }
            }
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

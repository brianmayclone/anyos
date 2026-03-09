use crate::hir::*;
use crate::intern::{Interner, Symbol};
use crate::diagnostics::{Span, Diagnostic, Level};
use std::collections::HashMap;

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
    interner: &'a Interner,
    /// Enum DefId -> variant info
    enum_variants: HashMap<DefId, EnumInfo>,
    /// Next synthetic DefId for things without one (e.g. variants)
    next_synthetic_def_id: u32,
    /// Set of primitive type name strings
    primitives: Vec<&'static str>,
    /// Map from impl self_ty first segment symbol to list of method (name, def_id)
    impl_methods: HashMap<Symbol, Vec<(Symbol, DefId)>>,
}

impl<'a> Resolver<'a> {
    pub fn new(interner: &'a Interner) -> Self {
        Self {
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
        }
    }

    fn alloc_synthetic_def_id(&mut self) -> DefId {
        let id = DefId(self.next_synthetic_def_id);
        self.next_synthetic_def_id += 1;
        id
    }

    pub fn resolve_crate(&mut self, krate: &HirCrate) -> ResolveResult {
        // First pass: register all top-level items
        for item in &krate.items {
            self.register_item(item);
        }

        // Second pass: collect impl methods
        for item in &krate.items {
            if let HirItemKind::Impl(impl_block) = &item.kind {
                self.register_impl_methods(impl_block);
            }
        }

        // Third pass: resolve all items
        for item in &krate.items {
            self.resolve_item(item);
        }

        // Build variant_indices: (enum_name, variant_name) -> variant index
        let mut variant_indices = HashMap::new();
        for item in &krate.items {
            if let HirItemKind::Enum(e) = &item.kind {
                for (idx, v) in e.variants.iter().enumerate() {
                    variant_indices.insert((e.name, v.name), idx);
                }
            }
        }

        ResolveResult {
            resolutions: std::mem::take(&mut self.resolutions),
            errors: std::mem::take(&mut self.errors),
            impl_methods: self.impl_methods.clone(),
            variant_indices,
        }
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
            HirItemKind::Use(_) | HirItemKind::Mod(_) => {}
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
                    for bound in bounds {
                        self.resolve_path(&bound.path, Namespace::Type, HirId(u32::MAX));
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
            HirTy::Infer(_) | HirTy::Never(_) => {}
        }
    }

    fn resolve_path(&mut self, path: &HirPath, ns: Namespace, hir_id: HirId) {
        if path.segments.is_empty() { return; }

        let first_seg = &path.segments[0];
        let name = first_seg.ident;
        let name_str = self.interner.resolve(name);

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
        } else if path.segments.len() == 2 {
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
            }

            // Could not resolve
            let second_str = self.interner.resolve(second_name);
            self.errors.push(Diagnostic::new(
                Level::Error,
                &format!("`{}::{}` not found", name_str, second_str),
                path.span,
            ));
        } else {
            // Multi-segment paths (3+): just skip for now, don't error
            // These are likely external paths like std::mem::size_of
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

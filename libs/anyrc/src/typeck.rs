use crate::prelude::*;
use crate::hir::*;
use crate::ast::{BinOp, Literal, Mutability};
use crate::resolve::ResolveResult;
use crate::intern::{Interner, Symbol};
use crate::diagnostics::{Span, Diagnostic, Level};
use anyos_std::collections::HashMap;

/// Internal type representation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TyKind {
    Bool,
    Char,
    Int(IntTy),
    Uint(UintTy),
    Float(FloatTy),
    Str,
    Never,
    Unit,
    Tuple(Vec<TyKind>),
    Array(Box<TyKind>, usize),
    Slice(Box<TyKind>),
    Ref(Box<TyKind>, Mutability),
    RawPtr(Box<TyKind>, Mutability),
    FnDef(DefId, Vec<TyKind>),
    FnPtr(Vec<TyKind>, Box<TyKind>),
    Adt(DefId, Vec<TyKind>),
    DynTrait(DefId),
    Param(u32),
    Infer(InferVar),
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InferVar(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum InferKind {
    General,
    Integer,
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntTy { I8, I16, I32, I64, I128, Isize }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UintTy { U8, U16, U32, U64, U128, Usize }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FloatTy { F32, F64 }

const SYNTH_PARSED_ARGS_DEF_ID: DefId = DefId(0x7000_0001);

pub struct TypeckResult {
    pub expr_types: HashMap<HirId, TyKind>,
    pub struct_defs: HashMap<DefId, Vec<(Symbol, TyKind)>>,
    pub fn_sigs: HashMap<DefId, (Vec<TyKind>, TyKind)>,
    pub errors: Vec<Diagnostic>,
    /// For each call to a generic function: call-site HirId -> (callee DefId, concrete type args)
    pub generic_call_substs: HashMap<HirId, (DefId, Vec<TyKind>)>,
    /// DefIds of functions that have generic type params
    pub generic_fn_defs: HashMap<DefId, usize>,  // DefId -> number of type params
    /// Enum variant field types: enum DefId -> vec of (variant_name, field_types)
    pub enum_variants: HashMap<DefId, Vec<(Symbol, Vec<TyKind>)>>,
    /// Evaluated const values: DefId -> (value, type)
    pub const_values: HashMap<DefId, (ConstVal, TyKind)>,
    /// Static definitions: DefId -> (name, type, initial_value, is_mut)
    pub static_defs: HashMap<DefId, (Symbol, TyKind, ConstVal, bool)>,
    /// Closure expression HirId -> synthetic DefId
    pub closure_defs: HashMap<HirId, DefId>,
    /// Trait method ordering: trait DefId -> ordered list of (method_name, method_def_id)
    pub trait_methods: HashMap<DefId, Vec<(Symbol, DefId)>>,
    /// Trait impls: (concrete_type_name, trait_def_id) -> list of method DefIds in vtable order
    pub trait_impls: HashMap<(Symbol, DefId), Vec<(Symbol, DefId)>>,
    /// Trait DefId -> trait name Symbol
    pub trait_names: HashMap<DefId, Symbol>,
    /// Reverse map: DefId -> type name Symbol (for looking up impl_methods by DefId)
    pub type_def_to_name: HashMap<DefId, Symbol>,
    /// Associated types: (trait DefId, assoc_type_name) -> concrete TyKind
    /// Populated from impl blocks: `impl Trait for Foo { type Item = Bar; }`
    pub assoc_types: HashMap<(DefId, Symbol), TyKind>,
    /// Trait bounds on generic params: maps generic param DefId -> list of trait DefIds
    pub generic_param_bounds: HashMap<u32, Vec<DefId>>,
    /// Default trait method bodies: method DefId -> true if body exists in trait def
    pub trait_default_methods: HashMap<DefId, bool>,
}

/// Compile-time evaluated constant value
#[derive(Debug, Clone)]
pub enum ConstVal {
    Int(i128),
    Bool(bool),
    Char(char),
}

pub struct TypeChecker<'a> {
    interner: &'a Interner,
    resolve: &'a ResolveResult,

    expr_types: HashMap<HirId, TyKind>,
    local_types: HashMap<DefId, TyKind>,
    fn_sigs: HashMap<DefId, (Vec<TyKind>, TyKind)>,
    struct_defs: HashMap<DefId, Vec<(Symbol, TyKind)>>,
    /// Map type name Symbol -> DefId for structs and enums
    type_name_to_def: HashMap<Symbol, DefId>,
    /// Enum variant field types: enum DefId -> vec of (variant_name, field_types)
    enum_variant_fields: HashMap<DefId, Vec<(Symbol, Vec<TyKind>)>>,
    /// Map resolver variant DefId -> owning enum DefId
    resolver_variant_to_enum: HashMap<DefId, DefId>,
    /// Current function's generic param symbols -> param index
    current_generic_params: HashMap<Symbol, u32>,
    /// Per call-site: (callee DefId, concrete type args)
    generic_call_substs: HashMap<HirId, (DefId, Vec<TyKind>)>,
    /// Generic function defs: DefId -> number of type params
    generic_fn_defs: HashMap<DefId, usize>,

    /// Evaluated const values
    const_values: HashMap<DefId, (ConstVal, TyKind)>,
    /// Static definitions
    static_defs: HashMap<DefId, (Symbol, TyKind, ConstVal, bool)>,

    /// Trait method ordering: trait DefId -> ordered list of (method_name, method_def_id)
    trait_methods: HashMap<DefId, Vec<(Symbol, DefId)>>,
    /// Trait impls: (concrete_type_name, trait_def_id) -> list of impl method DefIds in vtable order
    trait_impls: HashMap<(Symbol, DefId), Vec<(Symbol, DefId)>>,
    /// Trait DefId -> trait name Symbol
    trait_names: HashMap<DefId, Symbol>,
    /// Associated types: (trait DefId, assoc_type_name) -> concrete TyKind
    assoc_types: HashMap<(DefId, Symbol), TyKind>,
    /// Trait bounds on current function's generic params: param_index -> list of trait DefIds
    current_generic_bounds: HashMap<u32, Vec<DefId>>,
    /// Default trait method bodies: method DefId -> true if body exists in trait def
    trait_default_methods: HashMap<DefId, bool>,

    next_infer: u32,
    infer_kinds: HashMap<InferVar, InferKind>,
    substitutions: HashMap<InferVar, TyKind>,

    current_fn_ret: Option<TyKind>,
    errors: Vec<Diagnostic>,

    /// Synthetic DefIds for closure expressions: HirId -> DefId
    closure_defs: HashMap<HirId, DefId>,
    next_closure_def_id: u32,
}

impl<'a> TypeChecker<'a> {
    pub fn new(interner: &'a Interner, resolve: &'a ResolveResult) -> Self {
        Self {
            interner,
            resolve,
            expr_types: HashMap::new(),
            local_types: HashMap::new(),
            fn_sigs: HashMap::new(),
            struct_defs: HashMap::new(),
            type_name_to_def: HashMap::new(),
            enum_variant_fields: HashMap::new(),
            resolver_variant_to_enum: HashMap::new(),
            current_generic_params: HashMap::new(),
            generic_call_substs: HashMap::new(),
            generic_fn_defs: HashMap::new(),
            const_values: HashMap::new(),
            static_defs: HashMap::new(),
            trait_methods: HashMap::new(),
            trait_impls: HashMap::new(),
            trait_names: HashMap::new(),
            assoc_types: HashMap::new(),
            current_generic_bounds: HashMap::new(),
            trait_default_methods: HashMap::new(),
            next_infer: 0,
            infer_kinds: HashMap::new(),
            substitutions: HashMap::new(),
            current_fn_ret: None,
            errors: Vec::new(),
            closure_defs: HashMap::new(),
            next_closure_def_id: 0x8000_0000,
        }
    }

    fn fresh_infer(&mut self, kind: InferKind) -> TyKind {
        let var = InferVar(self.next_infer);
        self.next_infer += 1;
        self.infer_kinds.insert(var, kind);
        TyKind::Infer(var)
    }

    fn error(&mut self, span: Span, msg: &str) {
        self.errors.push(Diagnostic::new(Level::Error, msg, span));
    }

    fn bootstrap_stdlib_shims(&mut self) {
        if let Some(sym) = self.interner.lookup("ParsedArgs") {
            self.type_name_to_def.insert(sym, SYNTH_PARSED_ARGS_DEF_ID);
        }

        let mut fields = Vec::new();
        if let Some(sym) = self.interner.lookup("positional") {
            fields.push((
                sym,
                TyKind::Array(
                    Box::new(TyKind::Ref(Box::new(TyKind::Str), Mutability::Immutable)),
                    32,
                ),
            ));
        }
        if let Some(sym) = self.interner.lookup("pos_count") {
            fields.push((sym, TyKind::Uint(UintTy::Usize)));
        }
        if !fields.is_empty() {
            self.struct_defs.insert(SYNTH_PARSED_ARGS_DEF_ID, fields);
        }
    }

    fn parsed_args_ty(&self) -> TyKind {
        TyKind::Adt(SYNTH_PARSED_ARGS_DEF_ID, vec![])
    }

    fn option_of(&self, inner: TyKind) -> Option<TyKind> {
        let sym = self.interner.lookup("Option")?;
        let def_id = *self.type_name_to_def.get(&sym)?;
        Some(TyKind::Adt(def_id, vec![inner]))
    }

    fn result_of(&self, ok_ty: TyKind) -> Option<TyKind> {
        let sym = self.interner.lookup("Result")?;
        let def_id = *self.type_name_to_def.get(&sym)?;
        Some(TyKind::Adt(def_id, vec![ok_ty, TyKind::Error]))
    }

    fn iterable_elem_ty(&mut self, iter: &HirExpr, iter_ty: TyKind) -> TyKind {
        let resolved = match self.shallow_resolve(iter_ty) {
            TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => self.shallow_resolve(*inner),
            other => other,
        };
        match resolved {
            TyKind::Array(elem, _) | TyKind::Slice(elem) => *elem,
            TyKind::Str => TyKind::Uint(UintTy::U8),
            TyKind::Adt(def_id, substs) if self.is_vec_def(def_id) && substs.len() == 1 => {
                substs[0].clone()
            }
            other => {
                if let HirExprKind::MethodCall(recv, method_name, _, _) = &iter.kind {
                    let recv_ty = self.get_expr_ty_cached(recv);
                    let recv_resolved = match self.shallow_resolve(recv_ty) {
                        TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => self.shallow_resolve(*inner),
                        other => other,
                    };
                    let method_str = self.interner.resolve(*method_name);
                    let recv_is_string_like = matches!(recv_resolved, TyKind::Str)
                        || matches!(recv_resolved, TyKind::Adt(def_id, _) if self.is_string_def(def_id));
                    if recv_is_string_like {
                        match method_str {
                            "bytes" => return TyKind::Uint(UintTy::U8),
                            "split" | "split_whitespace" | "split_ascii_whitespace" | "lines" => {
                                return TyKind::Ref(Box::new(TyKind::Str), Mutability::Immutable);
                            }
                            _ => {}
                        }
                    }
                }
                other
            }
        }
    }

    fn intrinsic_call_return_type(&mut self, path_str: &str) -> Option<TyKind> {
        match path_str {
            "anyos_std::process::args" | "args" => {
                Some(TyKind::Ref(Box::new(TyKind::Str), Mutability::Immutable))
            }
            "anyos_std::args::parse" => Some(self.parsed_args_ty()),
            "u8::from_le_bytes" => Some(TyKind::Uint(UintTy::U8)),
            "u16::from_le_bytes" => Some(TyKind::Uint(UintTy::U16)),
            "u32::from_le_bytes" => Some(TyKind::Uint(UintTy::U32)),
            "u64::from_le_bytes" => Some(TyKind::Uint(UintTy::U64)),
            "u128::from_le_bytes" => Some(TyKind::Uint(UintTy::U128)),
            "usize::from_le_bytes" => Some(TyKind::Uint(UintTy::Usize)),
            "i8::from_le_bytes" => Some(TyKind::Int(IntTy::I8)),
            "i16::from_le_bytes" => Some(TyKind::Int(IntTy::I16)),
            "i32::from_le_bytes" => Some(TyKind::Int(IntTy::I32)),
            "i64::from_le_bytes" => Some(TyKind::Int(IntTy::I64)),
            "i128::from_le_bytes" => Some(TyKind::Int(IntTy::I128)),
            "isize::from_le_bytes" => Some(TyKind::Int(IntTy::Isize)),
            "char::from_u32" => self.option_of(TyKind::Char),
            "u8::from_str_radix" => self.result_of(TyKind::Uint(UintTy::U8)),
            "u16::from_str_radix" => self.result_of(TyKind::Uint(UintTy::U16)),
            "u32::from_str_radix" => self.result_of(TyKind::Uint(UintTy::U32)),
            "u64::from_str_radix" => self.result_of(TyKind::Uint(UintTy::U64)),
            "u128::from_str_radix" => self.result_of(TyKind::Uint(UintTy::U128)),
            "usize::from_str_radix" => self.result_of(TyKind::Uint(UintTy::Usize)),
            "i8::from_str_radix" => self.result_of(TyKind::Int(IntTy::I8)),
            "i16::from_str_radix" => self.result_of(TyKind::Int(IntTy::I16)),
            "i32::from_str_radix" => self.result_of(TyKind::Int(IntTy::I32)),
            "i64::from_str_radix" => self.result_of(TyKind::Int(IntTy::I64)),
            "i128::from_str_radix" => self.result_of(TyKind::Int(IntTy::I128)),
            "isize::from_str_radix" => self.result_of(TyKind::Int(IntTy::Isize)),
            _ => self.intrinsic_constructor_type(path_str),
        }
    }

    pub fn check_crate(&mut self, krate: &HirCrate) -> TypeckResult {
        // Register intrinsic types (AtomicBool, etc.) so they can be used as type annotations.
        // We find symbols by scanning resolutions that point to intrinsic type DefIds.
        {
            let intrinsic_type_defs: Vec<(DefId, String)> = self.resolve.intrinsic_fns.iter()
                .filter(|(_, path)| {
                    let name = path.rsplit("::").next().unwrap_or(path);
                    name.starts_with("Atomic") || name == "Ordering"
                        || name == "UnsafeCell"
                        || name == "Vec" || name == "Box" || name == "String"
                        || name == "Option" || name == "Result"
                })
                .map(|(&d, p)| (d, p.clone()))
                .collect();
            // For each HirId->DefId resolution where DefId is an intrinsic type,
            // we need to find the symbol. We can get it from the path string.
            for (def_id, path) in &intrinsic_type_defs {
                let type_name = path.rsplit("::").next().unwrap_or(path);
                // Find a symbol with this name by scanning all interned symbols
                // that appear in resolutions mapping to this DefId
                // Actually, just search all existing symbols in type_name_to_def keys
                // or we can find the symbol from the interner by trying to look it up
                let sym = self.interner.lookup(type_name);
                if let Some(sym) = sym {
                    self.type_name_to_def.entry(sym).or_insert(*def_id);
                }
            }
        }

        self.bootstrap_stdlib_shims();

        // Pass 1: collect signatures
        for item in &krate.items {
            self.collect_item(item);
        }

        // Pass 2: check bodies
        for item in &krate.items {
            self.check_item(item);
        }

        // Resolve all infer vars
        let mut expr_types = core::mem::take(&mut self.expr_types);
        for ty in expr_types.values_mut() {
            *ty = self.resolve_ty_full(ty.clone());
        }

        // Also resolve types in generic_call_substs
        let mut generic_call_substs = core::mem::take(&mut self.generic_call_substs);
        for (_, (_, substs)) in generic_call_substs.iter_mut() {
            for ty in substs.iter_mut() {
                *ty = self.resolve_ty_full(ty.clone());
            }
        }

        TypeckResult {
            expr_types,
            struct_defs: core::mem::take(&mut self.struct_defs),
            fn_sigs: self.fn_sigs.clone(),
            errors: core::mem::take(&mut self.errors),
            generic_call_substs,
            generic_fn_defs: core::mem::take(&mut self.generic_fn_defs),
            enum_variants: core::mem::take(&mut self.enum_variant_fields),
            const_values: core::mem::take(&mut self.const_values),
            static_defs: core::mem::take(&mut self.static_defs),
            closure_defs: core::mem::take(&mut self.closure_defs),
            trait_methods: core::mem::take(&mut self.trait_methods),
            trait_impls: core::mem::take(&mut self.trait_impls),
            trait_names: core::mem::take(&mut self.trait_names),
            assoc_types: core::mem::take(&mut self.assoc_types),
            generic_param_bounds: HashMap::new(),
            trait_default_methods: core::mem::take(&mut self.trait_default_methods),
            type_def_to_name: self.type_name_to_def.iter()
                .filter(|(name, _)| self.interner.resolve(**name) != "Self")
                .map(|(name, &def_id)| (def_id, *name))
                .collect(),
        }
    }

    // ── Pass 1: Collect ──

    fn collect_item_recursive(&mut self, items: &[HirItem]) {
        for item in items {
            self.collect_item(item);
        }
    }

    fn collect_item(&mut self, item: &HirItem) {
        match &item.kind {
            HirItemKind::Mod(m) => {
                if let Some(sub_items) = &m.items {
                    self.collect_item_recursive(sub_items);
                }
                return;
            }
            HirItemKind::Fn(f) => {
                // Set up generic params for this function
                let old_generics = core::mem::take(&mut self.current_generic_params);
                let old_bounds = core::mem::take(&mut self.current_generic_bounds);
                let mut n_type_params = 0u32;
                for gp in &f.generics.params {
                    if let HirGenericParam::Type(name, bounds, _, _) = gp {
                        self.current_generic_params.insert(*name, n_type_params);
                        // Collect trait bounds for this param
                        let mut bound_def_ids = Vec::new();
                        for bound in bounds {
                            if !bound.path.segments.is_empty() {
                                let trait_name = bound.path.segments[0].ident;
                                if let Some(&trait_def_id) = self.type_name_to_def.get(&trait_name) {
                                    bound_def_ids.push(trait_def_id);
                                }
                            }
                        }
                        if !bound_def_ids.is_empty() {
                            self.current_generic_bounds.insert(n_type_params, bound_def_ids);
                        }
                        n_type_params += 1;
                    }
                }
                if n_type_params > 0 {
                    self.generic_fn_defs.insert(f.def_id, n_type_params as usize);
                }

                let params: Vec<TyKind> = f.params.iter()
                    .map(|p| self.hir_ty_to_ty(&p.ty))
                    .collect();
                let ret = f.ret_ty.as_ref()
                    .map(|t| self.hir_ty_to_ty(t))
                    .unwrap_or(TyKind::Unit);
                self.fn_sigs.insert(f.def_id, (params, ret));
                self.current_generic_params = old_generics;
                self.current_generic_bounds = old_bounds;
            }
            HirItemKind::Struct(s) => {
                let fields: Vec<(Symbol, TyKind)> = s.fields.iter()
                    .map(|f| (f.name, self.hir_ty_to_ty(&f.ty)))
                    .collect();
                self.struct_defs.insert(s.def_id, fields);
                self.type_name_to_def.insert(s.name, s.def_id);
            }
            HirItemKind::Enum(e) => {
                self.type_name_to_def.insert(e.name, e.def_id);
                // Collect variant field types
                let mut variants = Vec::new();
                for v in &e.variants {
                    let field_tys = match &v.fields {
                        HirVariantFields::Unit => vec![],
                        HirVariantFields::Tuple(tys) => {
                            tys.iter().map(|t| self.hir_ty_to_ty(t)).collect()
                        }
                        HirVariantFields::Struct(fields) => {
                            fields.iter().map(|f| self.hir_ty_to_ty(&f.ty)).collect()
                        }
                    };
                    variants.push((v.name, field_tys));
                }
                self.enum_variant_fields.insert(e.def_id, variants);
            }
            HirItemKind::Impl(ib) => {
                // Register Self as an alias for the impl'd type
                if let HirTy::Path(p) = &ib.self_ty {
                    if !p.segments.is_empty() {
                        let self_ty_name = p.segments[0].ident;
                        if let Some(&def_id) = self.type_name_to_def.get(&self_ty_name) {
                            if let Some(self_sym) = self.interner.lookup("Self") {
                                self.type_name_to_def.insert(self_sym, def_id);
                            }
                        }
                        // If this is a trait impl, record the mapping
                        if let Some(trait_ref) = &ib.trait_ref {
                            if !trait_ref.segments.is_empty() {
                                let trait_name = trait_ref.segments[0].ident;
                                if let Some(&trait_def_id) = self.type_name_to_def.get(&trait_name) {
                                    let mut impl_methods = Vec::new();
                                    for sub in &ib.items {
                                        match &sub.kind {
                                            HirItemKind::Fn(f) => {
                                                impl_methods.push((f.name, f.def_id));
                                            }
                                            HirItemKind::TypeAlias(ta) => {
                                                // Associated type: `type Item = Foo;`
                                                if let Some(ref ty) = ta.ty {
                                                    let resolved_ty = self.hir_ty_to_ty(ty);
                                                    self.assoc_types.insert((trait_def_id, ta.name), resolved_ty);
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    self.trait_impls.insert((self_ty_name, trait_def_id), impl_methods);
                                }
                            }
                        }
                    }
                }
                for sub in &ib.items {
                    self.collect_item(sub);
                }
            }
            HirItemKind::Const(c) => {
                let ty = self.hir_ty_to_ty(&c.ty);
                if let Some(val_expr) = &c.value {
                    if let Some(cv) = self.eval_const_expr(val_expr) {
                        self.const_values.insert(c.def_id, (cv, ty.clone()));
                    }
                }
                // Register type so paths can resolve
                self.local_types.insert(c.def_id, ty);
            }
            HirItemKind::Static(s) => {
                let ty = self.hir_ty_to_ty(&s.ty);
                if let Some(val_expr) = &s.value {
                    let cv = self.eval_const_expr(val_expr).unwrap_or(ConstVal::Int(0));
                    self.static_defs.insert(s.def_id, (s.name, ty.clone(), cv, s.is_mut));
                }
                self.local_types.insert(s.def_id, ty);
            }
            HirItemKind::ExternBlock(eb) => {
                for sub in &eb.items {
                    self.collect_item(sub);
                }
            }
            HirItemKind::Trait(t) => {
                // Register trait name so `dyn Trait` can resolve it
                self.type_name_to_def.insert(t.name, t.def_id);
                self.trait_names.insert(t.def_id, t.name);
                // Register trait method signatures and ordering
                let mut methods = Vec::new();
                for sub in &t.items {
                    if let HirItemKind::Fn(f) = &sub.kind {
                        methods.push((f.name, f.def_id));
                        // Track whether this method has a default body
                        self.trait_default_methods.insert(f.def_id, f.body.is_some());
                    }
                    self.collect_item(sub);
                }
                self.trait_methods.insert(t.def_id, methods);
            }
            _ => {}
        }
    }

    fn eval_const_expr(&self, expr: &HirExpr) -> Option<ConstVal> {
        match &expr.kind {
            HirExprKind::Lit(lit) => match lit {
                Literal::Int(v) => Some(ConstVal::Int(*v as i128)),
                Literal::Bool(v) => Some(ConstVal::Bool(*v)),
                Literal::Char(v) => Some(ConstVal::Char(*v)),
                _ => None,
            },
            // AtomicXxx::new(val) in const context → evaluate the inner value
            HirExprKind::Call(_, args) if args.len() == 1 => {
                self.eval_const_expr(&args[0])
            }
            HirExprKind::Binary(op, lhs, rhs) => {
                let l = match self.eval_const_expr(lhs)? {
                    ConstVal::Int(v) => v,
                    _ => return None,
                };
                let r = match self.eval_const_expr(rhs)? {
                    ConstVal::Int(v) => v,
                    _ => return None,
                };
                let result = match op {
                    BinOp::Add => l.wrapping_add(r),
                    BinOp::Sub => l.wrapping_sub(r),
                    BinOp::Mul => l.wrapping_mul(r),
                    BinOp::Div => if r != 0 { l / r } else { return None },
                    BinOp::Rem => if r != 0 { l % r } else { return None },
                    BinOp::BitAnd => l & r,
                    BinOp::BitOr => l | r,
                    BinOp::BitXor => l ^ r,
                    BinOp::Shl => l.wrapping_shl(r as u32),
                    BinOp::Shr => l.wrapping_shr(r as u32),
                    _ => return None,
                };
                Some(ConstVal::Int(result))
            }
            HirExprKind::Unary(op, inner) => {
                let v = match self.eval_const_expr(inner)? {
                    ConstVal::Int(v) => v,
                    ConstVal::Bool(b) => match op {
                        crate::ast::UnOp::Not => return Some(ConstVal::Bool(!b)),
                        _ => return None,
                    },
                    _ => return None,
                };
                match op {
                    crate::ast::UnOp::Neg => Some(ConstVal::Int(-v)),
                    crate::ast::UnOp::Not => Some(ConstVal::Int(!v)),
                    _ => None,
                }
            }
            // Resolve const references in const expressions (e.g., other const items)
            HirExprKind::Path(_) => {
                if let Some(&def_id) = self.resolve.resolutions.get(&expr.id) {
                    if let Some((cv, _)) = self.const_values.get(&def_id) {
                        return Some(cv.clone());
                    }
                }
                None
            }
            _ => None,
        }
    }

    // ── Pass 2: Check ──

    fn check_item(&mut self, item: &HirItem) {
        match &item.kind {
            HirItemKind::Fn(f) => self.check_fn(f),
            HirItemKind::Impl(ib) => {
                for sub in &ib.items {
                    self.check_item(sub);
                }
            }
            HirItemKind::Mod(m) => {
                if let Some(sub_items) = &m.items {
                    for sub in sub_items {
                        self.check_item(sub);
                    }
                }
            }
            _ => {}
        }
    }

    fn check_fn(&mut self, f: &HirFnDef) {
        // Set up generic params and bounds for this function's body
        let old_generics = core::mem::take(&mut self.current_generic_params);
        let old_bounds = core::mem::take(&mut self.current_generic_bounds);
        let mut n_type_params = 0u32;
        for gp in &f.generics.params {
            if let HirGenericParam::Type(name, bounds, _, _) = gp {
                self.current_generic_params.insert(*name, n_type_params);
                let mut bound_def_ids = Vec::new();
                for bound in bounds {
                    if !bound.path.segments.is_empty() {
                        let trait_name = bound.path.segments[0].ident;
                        if let Some(&trait_def_id) = self.type_name_to_def.get(&trait_name) {
                            bound_def_ids.push(trait_def_id);
                        }
                    }
                }
                if !bound_def_ids.is_empty() {
                    self.current_generic_bounds.insert(n_type_params, bound_def_ids);
                }
                n_type_params += 1;
            }
        }

        let sig = self.fn_sigs.get(&f.def_id).cloned();
        let (param_tys, ret_ty) = sig.unwrap_or_else(|| (vec![], TyKind::Unit));

        for (param, ty) in f.params.iter().zip(param_tys.iter()) {
            self.bind_pattern(&param.pat, ty.clone());
        }

        let old_ret = self.current_fn_ret.replace(ret_ty.clone());

        if let Some(body) = &f.body {
            let body_ty = self.check_block(body);
            if body_ty != TyKind::Never {
                self.unify(&ret_ty, &body_ty, body.span);
            }
        }

        self.current_fn_ret = old_ret;
        self.current_generic_params = old_generics;
        self.current_generic_bounds = old_bounds;
    }

    fn check_block(&mut self, block: &HirBlock) -> TyKind {
        let mut last_ty = TyKind::Unit;
        for (i, stmt) in block.stmts.iter().enumerate() {
            let is_last = i == block.stmts.len() - 1;
            match stmt {
                HirStmt::Let(_, pat, ty_ann, init, span) => {
                    let ty = if let Some(init) = init {
                        let init_ty = self.check_expr(init);
                        if let Some(ann) = ty_ann {
                            let ann_ty = self.hir_ty_to_ty(ann);
                            self.unify(&ann_ty, &init_ty, *span);
                            ann_ty
                        } else {
                            init_ty
                        }
                    } else if let Some(ann) = ty_ann {
                        self.hir_ty_to_ty(ann)
                    } else {
                        self.fresh_infer(InferKind::General)
                    };
                    self.bind_pattern(pat, ty);
                    last_ty = TyKind::Unit;
                }
                HirStmt::Expr(e) => {
                    let ty = self.check_expr(e);
                    last_ty = if is_last { ty } else { TyKind::Unit };
                }
                HirStmt::Semi(e, _) => {
                    let ty = self.check_expr(e);
                    // If this is the last statement and the expression diverges,
                    // the block type is Never (e.g. `return 42;`)
                    if is_last && ty == TyKind::Never {
                        last_ty = TyKind::Never;
                    } else {
                        last_ty = TyKind::Unit;
                    }
                }
                HirStmt::Item(item) => {
                    self.collect_item(item);
                    self.check_item(item);
                    last_ty = TyKind::Unit;
                }
            }
        }
        last_ty
    }

    fn check_expr(&mut self, expr: &HirExpr) -> TyKind {
        let ty = self.check_expr_inner(expr);
        self.expr_types.insert(expr.id, ty.clone());
        ty
    }

    fn get_expr_ty_cached(&self, expr: &HirExpr) -> TyKind {
        self.expr_types.get(&expr.id).cloned().unwrap_or(TyKind::Error)
    }

    fn check_expr_inner(&mut self, expr: &HirExpr) -> TyKind {
        match &expr.kind {
            HirExprKind::Lit(lit) => match lit {
                Literal::Int(_) => self.fresh_infer(InferKind::Integer),
                Literal::Float(_) => self.fresh_infer(InferKind::Float),
                Literal::Bool(_) => TyKind::Bool,
                Literal::Char(_) => TyKind::Char,
                Literal::String(_) => TyKind::Ref(Box::new(TyKind::Str), Mutability::Immutable),
                Literal::ByteString(bytes) => TyKind::Ref(
                    Box::new(TyKind::Array(Box::new(TyKind::Uint(UintTy::U8)), bytes.len())),
                    Mutability::Immutable,
                ),
            },

            HirExprKind::Path(path) => {
                self.infer_path_type(path, expr.id)
            }

            HirExprKind::Binary(op, lhs, rhs) => {
                let lty = self.check_expr(lhs);
                let rty = self.check_expr(rhs);
                match op {
                    BinOp::And | BinOp::Or => {
                        self.unify(&TyKind::Bool, &lty, lhs.span);
                        self.unify(&TyKind::Bool, &rty, rhs.span);
                        TyKind::Bool
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        self.unify(&lty, &rty, expr.span);
                        TyKind::Bool
                    }
                    _ => {
                        self.unify(&lty, &rty, expr.span);
                        lty
                    }
                }
            }

            HirExprKind::Unary(op, inner) => {
                let ty = self.check_expr(inner);
                match op {
                    crate::ast::UnOp::Neg | crate::ast::UnOp::Not => ty,
                    crate::ast::UnOp::Deref => {
                        match self.shallow_resolve(ty) {
                            TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => *inner,
                            _ => {
                                self.error(expr.span, "cannot dereference this type");
                                TyKind::Error
                            }
                        }
                    }
                }
            }

            HirExprKind::Call(callee, args) => {
                let callee_ty = self.check_expr(callee);
                match self.shallow_resolve(callee_ty) {
                    TyKind::FnDef(def_id, _) => {
                        // Intrinsic functions: accept any args
                        if let Some(intrinsic_path) = self.resolve.intrinsic_fns.get(&def_id).cloned() {
                            for a in args { self.check_expr(a); }
                            if let Some(ty) = self.intrinsic_call_return_type(&intrinsic_path) {
                                return ty;
                            }
                            // For Type::new() constructors, return the type as Adt
                            if intrinsic_path.ends_with("::new") {
                                let type_name = intrinsic_path.rsplit("::").nth(1).unwrap_or("");
                                if let Some(sym) = self.interner.lookup(type_name) {
                                    if let Some(&type_def_id) = self.type_name_to_def.get(&sym) {
                                        return TyKind::Adt(type_def_id, vec![]);
                                    }
                                }
                            }
                            return self.fresh_infer(InferKind::General);
                        }
                        if let Some((param_tys, ret_ty)) = self.fn_sigs.get(&def_id).cloned() {
                            // If this is a generic function, create fresh infer vars for each type param
                            let n_generics = self.generic_fn_defs.get(&def_id).copied().unwrap_or(0);
                            let (param_tys, ret_ty) = if n_generics > 0 {
                                let infer_vars: Vec<TyKind> = (0..n_generics)
                                    .map(|_| self.fresh_infer(InferKind::General))
                                    .collect();
                                let subst_params: Vec<TyKind> = param_tys.iter()
                                    .map(|t| self.substitute_params(t, &infer_vars))
                                    .collect();
                                let subst_ret = self.substitute_params(&ret_ty, &infer_vars);
                                // We'll record the concrete substs after unification
                                // Store the infer vars so we can resolve them later
                                self.generic_call_substs.insert(expr.id, (def_id, infer_vars.clone()));
                                (subst_params, subst_ret)
                            } else {
                                (param_tys, ret_ty)
                            };

                            if args.len() != param_tys.len() {
                                self.error(expr.span, &format!(
                                    "wrong number of arguments: expected {}, found {}",
                                    param_tys.len(), args.len()));
                                for a in args { self.check_expr(a); }
                            } else {
                                for (arg, pty) in args.iter().zip(param_tys.iter()) {
                                    let aty = self.check_expr(arg);
                                    self.unify(pty, &aty, arg.span);
                                }
                            }
                            ret_ty
                        } else {
                            for a in args { self.check_expr(a); }
                            TyKind::Error
                        }
                    }
                    TyKind::Adt(enum_def_id, _) => {
                        // Enum variant constructor call, e.g. Option::Some(42)
                        // Check args against variant field types
                        for a in args { self.check_expr(a); }
                        TyKind::Adt(enum_def_id, vec![])
                    }
                    _ => {
                        for a in args { self.check_expr(a); }
                        TyKind::Error
                    }
                }
            }

            HirExprKind::Field(base, field_name) => {
                let base_ty = self.check_expr(base);
                let resolved = self.shallow_resolve(base_ty);
                // Auto-deref references and raw pointers for field access.
                let resolved = match &resolved {
                    TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => {
                        self.shallow_resolve(inner.as_ref().clone())
                    }
                    other => other.clone(),
                };
                match resolved {
                    TyKind::Adt(def_id, _) => {
                        if let Some(fields) = self.struct_defs.get(&def_id) {
                            if let Some((_, fty)) = fields.iter().find(|(n, _)| *n == *field_name) {
                                fty.clone()
                            } else {
                                self.error(expr.span, "no such field");
                                TyKind::Error
                            }
                        } else {
                            TyKind::Error
                        }
                    }
                    TyKind::Tuple(tys) => {
                        let idx_str = self.interner.resolve(*field_name);
                        if let Ok(idx) = idx_str.parse::<usize>() {
                            if idx < tys.len() { tys[idx].clone() }
                            else {
                                self.error(expr.span, "tuple index out of bounds");
                                TyKind::Error
                            }
                        } else {
                            self.error(expr.span, "no such field");
                            TyKind::Error
                        }
                    }
                    _ => {
                        self.error(expr.span, "field access on non-struct type");
                        TyKind::Error
                    }
                }
            }

            HirExprKind::Struct(path, fields, _base) => {
                // Find struct DefId by name - use last segment for module-qualified paths
                let def_id = if !path.segments.is_empty() {
                    let name = path.segments.last().unwrap().ident;
                    self.type_name_to_def.get(&name).copied()
                } else {
                    None
                };

                if let Some(def_id) = def_id {
                    if let Some(expected_fields) = self.struct_defs.get(&def_id).cloned() {
                        for fe in fields {
                            let val_ty = self.check_expr(&fe.value);
                            if let Some((_, fty)) = expected_fields.iter().find(|(n, _)| *n == fe.name) {
                                self.unify(fty, &val_ty, fe.span);
                            }
                        }
                    } else {
                        for fe in fields { self.check_expr(&fe.value); }
                    }
                    TyKind::Adt(def_id, vec![])
                } else {
                    for fe in fields { self.check_expr(&fe.value); }
                    TyKind::Error
                }
            }

            HirExprKind::Block(block) => self.check_block(block),

            HirExprKind::If(cond, then_block, else_expr) => {
                let cond_ty = self.check_expr(cond);
                self.unify(&TyKind::Bool, &cond_ty, cond.span);
                let then_ty = self.check_block(then_block);
                if let Some(else_e) = else_expr {
                    let else_ty = self.check_expr(else_e);
                    self.unify(&then_ty, &else_ty, expr.span);
                    then_ty
                } else {
                    TyKind::Unit
                }
            }

            HirExprKind::Match(scrutinee, arms) => {
                let _scr_ty = self.check_expr(scrutinee);
                if arms.is_empty() {
                    return TyKind::Never;
                }
                let first_ty = self.check_expr(&arms[0].body);
                for arm in &arms[1..] {
                    let arm_ty = self.check_expr(&arm.body);
                    self.unify(&first_ty, &arm_ty, arm.span);
                }
                first_ty
            }

            HirExprKind::Assign(lhs, rhs) | HirExprKind::AssignOp(_, lhs, rhs) => {
                let lty = self.check_expr(lhs);
                let rty = self.check_expr(rhs);
                self.unify(&lty, &rty, expr.span);
                TyKind::Unit
            }

            HirExprKind::Return(val) => {
                let val_ty = val.as_ref()
                    .map(|e| self.check_expr(e))
                    .unwrap_or(TyKind::Unit);
                if let Some(ret_ty) = self.current_fn_ret.clone() {
                    let span = val.as_ref().map(|e| e.span).unwrap_or(expr.span);
                    self.unify(&ret_ty, &val_ty, span);
                }
                TyKind::Never
            }

            HirExprKind::Ref(inner, mutability) => {
                let inner_ty = self.check_expr(inner);
                TyKind::Ref(Box::new(inner_ty), *mutability)
            }

            HirExprKind::Deref(inner) => {
                let inner_ty = self.check_expr(inner);
                match self.shallow_resolve(inner_ty) {
                    TyKind::Ref(pointee, _) | TyKind::RawPtr(pointee, _) => *pointee,
                    _ => {
                        self.error(expr.span, "cannot dereference this type");
                        TyKind::Error
                    }
                }
            }

            HirExprKind::Tuple(es) => {
                let tys: Vec<TyKind> = es.iter().map(|e| self.check_expr(e)).collect();
                if tys.is_empty() { TyKind::Unit } else { TyKind::Tuple(tys) }
            }

            HirExprKind::Array(es) => {
                if es.is_empty() {
                    return TyKind::Array(Box::new(self.fresh_infer(InferKind::General)), 0);
                }
                let first = self.check_expr(&es[0]);
                for e in &es[1..] {
                    let t = self.check_expr(e);
                    self.unify(&first, &t, e.span);
                }
                TyKind::Array(Box::new(first), es.len())
            }

            HirExprKind::Index(base, idx) => {
                let base_ty = self.check_expr(base);
                let idx_ty = self.check_expr(idx);
                let resolved = self.shallow_resolve(base_ty);
                let resolved = match resolved {
                    TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => self.shallow_resolve(*inner),
                    other => other,
                };
                let is_range = matches!(idx.kind, HirExprKind::Range(_, _, _));
                if !is_range {
                    self.unify(&TyKind::Uint(UintTy::Usize), &idx_ty, idx.span);
                }
                match resolved {
                    TyKind::Array(elem, _) | TyKind::Slice(elem) => {
                        if is_range {
                            TyKind::Slice(elem)
                        } else {
                            *elem
                        }
                    }
                    TyKind::Str => {
                        if is_range {
                            TyKind::Str
                        } else {
                            self.error(expr.span, "cannot index this type");
                            TyKind::Error
                        }
                    }
                    TyKind::Adt(def_id, substs) => {
                        if self.is_vec_def(def_id) && substs.len() == 1 {
                            if is_range {
                                TyKind::Slice(Box::new(substs[0].clone()))
                            } else {
                                substs[0].clone()
                            }
                        } else if self.is_string_def(def_id) {
                            if is_range {
                                TyKind::Str
                            } else {
                                self.error(expr.span, "cannot index this type");
                                TyKind::Error
                            }
                        } else {
                            self.error(expr.span, "cannot index this type");
                            TyKind::Error
                        }
                    }
                    _ => {
                        self.error(expr.span, "cannot index this type");
                        TyKind::Error
                    }
                }
            }

            HirExprKind::Cast(inner, ty) => {
                self.check_expr(inner);
                self.hir_ty_to_ty(ty)
            }

            HirExprKind::Loop(block, _) => {
                self.check_block(block);
                TyKind::Never
            }

            HirExprKind::Break(_, val) => {
                if let Some(v) = val { self.check_expr(v); }
                TyKind::Never
            }

            HirExprKind::Continue(_) => TyKind::Never,
            HirExprKind::Paren(inner) => self.check_expr(inner),
            HirExprKind::Unsafe(block) => self.check_block(block),

            HirExprKind::MethodCall(recv, method_name, _, args) => {
                let recv_ty = self.check_expr(recv);
                for a in args { self.check_expr(a); }

                // Resolve the receiver type to find the method
                let base_ty = self.shallow_resolve(recv_ty.clone());
                // Unwrap references to get the underlying type name
                let inner_ty = match &base_ty {
                    TyKind::Ref(inner, _) => self.shallow_resolve(inner.as_ref().clone()),
                    other => other.clone(),
                };

                let method_str = self.interner.resolve(*method_name);
                let inner_is_string_like = matches!(&inner_ty, TyKind::Str)
                    || matches!(&inner_ty, TyKind::Adt(def_id, _) if self.is_string_def(*def_id));
                if matches!(&inner_ty, TyKind::RawPtr(_, _)) {
                    if method_str == "is_null" && args.is_empty() {
                        return TyKind::Bool;
                    }
                    if method_str == "add" && args.len() == 1 {
                        let aty = self.get_expr_ty_cached(&args[0]);
                        self.unify(&TyKind::Uint(UintTy::Usize), &aty, args[0].span);
                        return inner_ty.clone();
                    }
                }

                // Handle .len() on arrays and slices
                if method_str == "len" && args.is_empty() {
                    if matches!(&inner_ty, TyKind::Array(_, _) | TyKind::Slice(_) | TyKind::Str) {
                        return TyKind::Uint(UintTy::Usize);
                    }
                }

                if method_str == "is_empty" && args.is_empty() {
                    if matches!(&inner_ty, TyKind::Array(_, _) | TyKind::Slice(_) | TyKind::Str) {
                        return TyKind::Bool;
                    }
                }

                if inner_is_string_like {
                    match method_str {
                        "contains" | "starts_with" | "ends_with" if args.len() == 1 => {
                            return TyKind::Bool;
                        }
                        "find" if args.len() == 1 => {
                            if let Some(ty) = self.option_of(TyKind::Uint(UintTy::Usize)) {
                                return ty;
                            }
                            return self.fresh_infer(InferKind::General);
                        }
                        "as_bytes" if args.is_empty() => {
                            return TyKind::Ref(
                                Box::new(TyKind::Slice(Box::new(TyKind::Uint(UintTy::U8)))),
                                Mutability::Immutable,
                            );
                        }
                        "bytes" if args.is_empty() => {
                            return TyKind::Slice(Box::new(TyKind::Uint(UintTy::U8)));
                        }
                        "split" | "split_whitespace" | "split_ascii_whitespace" | "lines" if args.len() <= 1 => {
                            return TyKind::Slice(Box::new(TyKind::Ref(
                                Box::new(TyKind::Str),
                                Mutability::Immutable,
                            )));
                        }
                        _ => {}
                    }
                }

                if let TyKind::Slice(elem) | TyKind::Array(elem, _) = &inner_ty {
                    match method_str {
                        "copy_from_slice" if args.len() == 1 => return TyKind::Unit,
                        "to_vec" if args.is_empty() => {
                            if let Some(vec_sym) = self.interner.lookup("Vec") {
                                if let Some(&vec_def_id) = self.type_name_to_def.get(&vec_sym) {
                                    return TyKind::Adt(vec_def_id, vec![elem.as_ref().clone()]);
                                }
                            }
                            return self.fresh_infer(InferKind::General);
                        }
                        "first" if args.is_empty() => {
                            if let Some(ty) = self.option_of(TyKind::Ref(elem.clone(), Mutability::Immutable)) {
                                return ty;
                            }
                            return self.fresh_infer(InferKind::General);
                        }
                        _ => {}
                    }
                }

                if matches!(&inner_ty, TyKind::Adt(def_id, _) if *def_id == SYNTH_PARSED_ARGS_DEF_ID) {
                    match method_str {
                        "has" if args.len() == 1 => return TyKind::Bool,
                        "opt" if args.len() == 1 => {
                            let opt_sym = self.interner.lookup("Option");
                            if let Some(sym) = opt_sym {
                                if let Some(&def_id) = self.type_name_to_def.get(&sym) {
                                    return TyKind::Adt(
                                        def_id,
                                        vec![TyKind::Ref(Box::new(TyKind::Str), Mutability::Immutable)],
                                    );
                                }
                            }
                            return self.fresh_infer(InferKind::General);
                        }
                        "first_or" if args.len() == 1 => {
                            return TyKind::Ref(Box::new(TyKind::Str), Mutability::Immutable);
                        }
                        "pos" if args.len() == 1 => {
                            let opt_sym = self.interner.lookup("Option");
                            if let Some(sym) = opt_sym {
                                if let Some(&def_id) = self.type_name_to_def.get(&sym) {
                                    return TyKind::Adt(
                                        def_id,
                                        vec![TyKind::Ref(Box::new(TyKind::Str), Mutability::Immutable)],
                                    );
                                }
                            }
                            return self.fresh_infer(InferKind::General);
                        }
                        "opt_u32" if args.len() == 2 => return TyKind::Uint(UintTy::U32),
                        _ => {}
                    }
                }

                if let TyKind::Adt(def_id, substs) = &inner_ty {
                    if self.is_vec_def(*def_id) && substs.len() == 1 {
                        match method_str {
                            "len" if args.is_empty() => return TyKind::Uint(UintTy::Usize),
                            "is_empty" if args.is_empty() => return TyKind::Bool,
                            "as_slice" if args.is_empty() => {
                                return TyKind::Ref(
                                    Box::new(TyKind::Slice(Box::new(substs[0].clone()))),
                                    Mutability::Immutable,
                                );
                            }
                            _ => {}
                        }
                    }
                    if self.is_result_def(*def_id) && substs.len() == 2 {
                        if method_str == "ok" && args.is_empty() {
                            if let Some(ty) = self.option_of(substs[0].clone()) {
                                return ty;
                            }
                            return self.fresh_infer(InferKind::General);
                        }
                    }
                }

                // Handle dyn Trait method calls
                if let TyKind::DynTrait(trait_def_id) = &inner_ty {
                    if let Some(methods) = self.trait_methods.get(trait_def_id).cloned() {
                        if let Some((_, method_def_id)) = methods.iter().find(|(n, _)| *n == *method_name) {
                            if let Some((param_tys, ret_ty)) = self.fn_sigs.get(method_def_id).cloned() {
                                let user_params = if !param_tys.is_empty() { &param_tys[1..] } else { &param_tys[..] };
                                if args.len() != user_params.len() {
                                    self.error(expr.span, &format!(
                                        "wrong number of arguments: expected {}, found {}",
                                        user_params.len(), args.len()));
                                }
                                return ret_ty;
                            }
                        }
                    }
                }

                // Handle method calls on generic type params via trait bounds
                if let TyKind::Param(param_idx) = &inner_ty {
                    // Look through trait bounds for this param
                    if let Some(bounds) = self.current_generic_bounds.get(param_idx).cloned() {
                        for trait_def_id in &bounds {
                            if let Some(methods) = self.trait_methods.get(trait_def_id).cloned() {
                                if let Some((_, method_def_id)) = methods.iter().find(|(n, _)| *n == *method_name) {
                                    if let Some((param_tys, ret_ty)) = self.fn_sigs.get(method_def_id).cloned() {
                                        let user_params = if !param_tys.is_empty() { &param_tys[1..] } else { &param_tys[..] };
                                        if args.len() != user_params.len() {
                                            self.error(expr.span, &format!(
                                                "wrong number of arguments: expected {}, found {}",
                                                user_params.len(), args.len()));
                                        }
                                        return ret_ty;
                                    }
                                }
                            }
                        }
                    }
                }

                if let TyKind::Adt(def_id, _) = &inner_ty {
                    // Find the type name for this DefId, skipping "Self" alias
                    let type_name = self.type_name_to_def.iter()
                        .find(|(sym, &did)| did == *def_id && self.interner.resolve(**sym) != "Self")
                        .map(|(sym, _)| *sym);

                    if let Some(type_name) = type_name {
                        // Look up impl methods
                        if let Some(methods) = self.resolve.impl_methods.get(&type_name) {
                            if let Some((_, method_def_id)) = methods.iter().find(|(n, _)| *n == *method_name) {
                                if let Some((param_tys, ret_ty)) = self.fn_sigs.get(method_def_id).cloned() {
                                    // Check argument types (skip &self parameter)
                                    let user_params = if !param_tys.is_empty() { &param_tys[1..] } else { &param_tys[..] };
                                    if args.len() != user_params.len() {
                                        self.error(expr.span, &format!(
                                            "wrong number of arguments: expected {}, found {}",
                                            user_params.len(), args.len()));
                                    } else {
                                        for (arg, pty) in args.iter().zip(user_params.iter()) {
                                            let aty = self.get_expr_ty_cached(arg);
                                            self.unify(pty, &aty, arg.span);
                                        }
                                    }
                                    return ret_ty;
                                }
                            }
                        }
                    }
                }

                self.fresh_infer(InferKind::General)
            }

            HirExprKind::Closure(params, ret_ty, body, _) => {
                // Allocate a synthetic DefId for this closure
                let closure_def_id = DefId(self.next_closure_def_id);
                self.next_closure_def_id += 1;
                self.closure_defs.insert(expr.id, closure_def_id);

                // Resolve parameter types from annotations
                let param_tys: Vec<TyKind> = params.iter().map(|p| {
                    let ty = self.hir_ty_to_ty(&p.ty);
                    // Register param binding in local_types
                    if let HirPattern::Ident(hir_id, _, _, _, _) = &p.pat {
                        if let Some(&def_id) = self.resolve.resolutions.get(hir_id) {
                            self.local_types.insert(def_id, ty.clone());
                        }
                    }
                    ty
                }).collect();

                // Check body
                let body_ty = self.check_expr(body);

                // Return type: explicit annotation or inferred from body
                let ret = if let Some(rt) = ret_ty {
                    let rt_ty = self.hir_ty_to_ty(rt);
                    self.unify(&rt_ty, &body_ty, expr.span);
                    rt_ty
                } else {
                    body_ty
                };

                // Register fn signature for this closure
                self.fn_sigs.insert(closure_def_id, (param_tys, ret));

                TyKind::FnDef(closure_def_id, vec![])
            }

            HirExprKind::ArrayRepeat(val, count) => {
                let vty = self.check_expr(val);
                self.check_expr(count);
                let n = self.eval_const_usize(count);
                TyKind::Array(Box::new(vty), n)
            }

            HirExprKind::Range(a, b, _) => {
                if let Some(a) = a { self.check_expr(a); }
                if let Some(b) = b { self.check_expr(b); }
                self.fresh_infer(InferKind::General)
            }

            HirExprKind::Try(inner) => {
                self.check_expr(inner);
                self.fresh_infer(InferKind::General)
            }

            HirExprKind::For(pat, iter, body, _) => {
                let iter_ty = self.check_expr(iter);
                let elem_ty = match &iter.kind {
                    HirExprKind::Range(start, _end, _) => {
                        if let Some(s) = start {
                            self.get_expr_ty_cached(s)
                        } else {
                            TyKind::Int(IntTy::I64)
                        }
                    }
                    _ => self.iterable_elem_ty(iter, iter_ty),
                };
                self.bind_pattern(pat, elem_ty);
                self.check_block(body);
                TyKind::Unit
            }

            HirExprKind::InlineAsm(asm) => {
                for op in &asm.operands {
                    match op {
                        crate::hir::HirAsmOperand::In { expr, .. } => { self.check_expr(expr); }
                        crate::hir::HirAsmOperand::Out { expr, .. } => {
                            if let Some(e) = expr { self.check_expr(e); }
                        }
                        crate::hir::HirAsmOperand::InOut { expr, .. } => { self.check_expr(expr); }
                    }
                }
                TyKind::Unit
            }
        }
    }

    // ── Path type inference ──

    fn infer_path_type(&mut self, path: &HirPath, expr_id: HirId) -> TyKind {
        if let Some(&def_id) = self.resolve.resolutions.get(&expr_id) {
            // Intrinsic function from core/alloc, or primitive associated constant?
            if let Some(path_str) = self.resolve.intrinsic_fns.get(&def_id) {
                if let Some(ty) = Self::primitive_assoc_const_type(path_str) {
                    return ty;
                }
                if let Some(ty) = self.intrinsic_constructor_type(path_str) {
                    return ty;
                }
                return TyKind::FnDef(def_id, vec![]);
            }
            // Function?
            if self.fn_sigs.contains_key(&def_id) {
                return TyKind::FnDef(def_id, vec![]);
            }
            // Local variable?
            if let Some(ty) = self.local_types.get(&def_id) {
                return ty.clone();
            }
            // Struct (unit struct used as value)?
            if self.struct_defs.contains_key(&def_id) {
                return TyKind::Adt(def_id, vec![]);
            }
            // Enum variant? The resolver assigned a synthetic DefId.
            // For a path like Color::Red, the resolver resolved to a variant DefId.
            // We need to find which enum this variant belongs to.
            // Look at the path: if 2 segments, first is enum name.
            if path.segments.len() == 2 {
                let enum_name = path.segments[0].ident;
                if let Some(&enum_def_id) = self.type_name_to_def.get(&enum_name) {
                    return TyKind::Adt(enum_def_id, vec![]);
                }
            }
            // Const item?
            if let Some((_, ty)) = self.const_values.get(&def_id) {
                return ty.clone();
            }
            // Static item?
            if let Some((_, ty, _, _)) = self.static_defs.get(&def_id) {
                return ty.clone();
            }
            return TyKind::Error;
        }
        TyKind::Error
    }

    fn intrinsic_constructor_type(&mut self, path_str: &str) -> Option<TyKind> {
        match path_str {
            "Option::Some" | "Option::None" => {
                let sym = self.interner.lookup("Option")?;
                let def_id = *self.type_name_to_def.get(&sym)?;
                Some(TyKind::Adt(def_id, vec![self.fresh_infer(InferKind::General)]))
            }
            "Result::Ok" | "Result::Err" => {
                let sym = self.interner.lookup("Result")?;
                let def_id = *self.type_name_to_def.get(&sym)?;
                Some(TyKind::Adt(
                    def_id,
                    vec![
                        self.fresh_infer(InferKind::General),
                        self.fresh_infer(InferKind::General),
                    ],
                ))
            }
            _ => None,
        }
    }

    /// Check if a path string like "u32::MAX" is a primitive associated constant.
    /// Returns the type if so.
    fn primitive_assoc_const_type(path: &str) -> Option<TyKind> {
        match path {
            "u8::MAX" | "u8::MIN" => Some(TyKind::Uint(UintTy::U8)),
            "u16::MAX" | "u16::MIN" => Some(TyKind::Uint(UintTy::U16)),
            "u32::MAX" | "u32::MIN" => Some(TyKind::Uint(UintTy::U32)),
            "u64::MAX" | "u64::MIN" => Some(TyKind::Uint(UintTy::U64)),
            "u128::MAX" | "u128::MIN" => Some(TyKind::Uint(UintTy::U128)),
            "usize::MAX" | "usize::MIN" => Some(TyKind::Uint(UintTy::Usize)),
            "i8::MAX" | "i8::MIN" => Some(TyKind::Int(IntTy::I8)),
            "i16::MAX" | "i16::MIN" => Some(TyKind::Int(IntTy::I16)),
            "i32::MAX" | "i32::MIN" => Some(TyKind::Int(IntTy::I32)),
            "i64::MAX" | "i64::MIN" => Some(TyKind::Int(IntTy::I64)),
            "i128::MAX" | "i128::MIN" => Some(TyKind::Int(IntTy::I128)),
            "isize::MAX" | "isize::MIN" => Some(TyKind::Int(IntTy::Isize)),
            "f32::INFINITY" | "f32::NEG_INFINITY" | "f32::NAN" => Some(TyKind::Float(FloatTy::F32)),
            "f64::INFINITY" | "f64::NEG_INFINITY" | "f64::NAN" => Some(TyKind::Float(FloatTy::F64)),
            "anyos_std::fs::O_READ" | "anyos_std::fs::O_WRITE" | "anyos_std::fs::O_CREATE"
            | "anyos_std::fs::O_TRUNC" | "anyos_std::fs::O_APPEND" | "anyos_std::fs::O_SYNC" => {
                Some(TyKind::Uint(UintTy::U32))
            }
            _ => None,
        }
    }

    fn is_vec_def(&self, def_id: DefId) -> bool {
        self.interner
            .lookup("Vec")
            .and_then(|sym| self.type_name_to_def.get(&sym).copied())
            == Some(def_id)
    }

    fn is_string_def(&self, def_id: DefId) -> bool {
        self.interner
            .lookup("String")
            .and_then(|sym| self.type_name_to_def.get(&sym).copied())
            == Some(def_id)
    }

    fn is_result_def(&self, def_id: DefId) -> bool {
        self.interner
            .lookup("Result")
            .and_then(|sym| self.type_name_to_def.get(&sym).copied())
            == Some(def_id)
    }

    // ── Unification ──

    fn unify(&mut self, expected: &TyKind, actual: &TyKind, span: Span) {
        let expected = self.shallow_resolve(expected.clone());
        let actual = self.shallow_resolve(actual.clone());

        if expected == actual { return; }

        match (&expected, &actual) {
            (TyKind::Error, _) | (_, TyKind::Error) => return,
            (TyKind::Never, _) | (_, TyKind::Never) => return,

            (TyKind::Infer(v), _) => {
                self.substitutions.insert(*v, actual);
                return;
            }
            (_, TyKind::Infer(v)) => {
                self.substitutions.insert(*v, expected);
                return;
            }

            (TyKind::Ref(a, am), TyKind::Ref(b, bm)) if am == bm => {
                // Allow &T -> &dyn Trait coercion
                if matches!(a.as_ref(), TyKind::DynTrait(_)) && matches!(b.as_ref(), TyKind::Adt(_, _)) {
                    return;
                }
                if matches!(a.as_ref(), TyKind::Str)
                    && matches!(b.as_ref(), TyKind::Adt(def_id, _) if self.is_string_def(*def_id))
                {
                    return;
                }
                if matches!(b.as_ref(), TyKind::Str)
                    && matches!(a.as_ref(), TyKind::Adt(def_id, _) if self.is_string_def(*def_id))
                {
                    return;
                }
                if let TyKind::Slice(slice_elem) = a.as_ref() {
                    match b.as_ref() {
                        TyKind::Array(array_elem, _) => {
                            self.unify(slice_elem, array_elem, span);
                            return;
                        }
                        TyKind::Adt(def_id, substs) if self.is_vec_def(*def_id) && substs.len() == 1 => {
                            self.unify(slice_elem, &substs[0], span);
                            return;
                        }
                        _ => {}
                    }
                }
                if let TyKind::Slice(slice_elem) = b.as_ref() {
                    match a.as_ref() {
                        TyKind::Array(array_elem, _) => {
                            self.unify(slice_elem, array_elem, span);
                            return;
                        }
                        TyKind::Adt(def_id, substs) if self.is_vec_def(*def_id) && substs.len() == 1 => {
                            self.unify(slice_elem, &substs[0], span);
                            return;
                        }
                        _ => {}
                    }
                }
                self.unify(a, b, span);
                return;
            }

            (TyKind::RawPtr(a, am), TyKind::RawPtr(b, bm)) if am == bm => {
                self.unify(a, b, span);
                return;
            }

            (TyKind::Tuple(a), TyKind::Tuple(b)) if a.len() == b.len() => {
                for (x, y) in a.clone().iter().zip(b.clone().iter()) {
                    self.unify(x, y, span);
                }
                return;
            }

            (TyKind::Array(a, an), TyKind::Array(b, bn)) if an == bn => {
                self.unify(a, b, span);
                return;
            }

            (TyKind::Slice(a), TyKind::Array(b, _)) | (TyKind::Array(b, _), TyKind::Slice(a)) => {
                self.unify(a, b, span);
                return;
            }

            (TyKind::Slice(a), TyKind::Adt(def_id, substs))
            | (TyKind::Adt(def_id, substs), TyKind::Slice(a))
                if self.is_vec_def(*def_id) && substs.len() == 1 =>
            {
                self.unify(a, &substs[0], span);
                return;
            }

            (TyKind::Str, TyKind::Adt(def_id, _)) | (TyKind::Adt(def_id, _), TyKind::Str)
                if self.is_string_def(*def_id) =>
            {
                return;
            }

            (TyKind::Adt(a, _), TyKind::Adt(b, _)) if a == b => return,
            (TyKind::DynTrait(a), TyKind::DynTrait(b)) if a == b => return,

            // Allow coercion from FnDef to FnPtr if signatures match
            (TyKind::FnPtr(expected_params, expected_ret), TyKind::FnDef(def_id, _)) => {
                if let Some((actual_params, actual_ret)) = self.fn_sigs.get(def_id).cloned() {
                    if expected_params.len() == actual_params.len() {
                        for (ep, ap) in expected_params.clone().iter().zip(actual_params.iter()) {
                            self.unify(ep, ap, span);
                        }
                        self.unify(expected_ret, &actual_ret, span);
                        return;
                    }
                }
            }

            _ => {}
        }

        self.error(span, &format!("type mismatch: expected {:?}, found {:?}", expected, actual));
    }

    fn shallow_resolve(&self, ty: TyKind) -> TyKind {
        match &ty {
            TyKind::Infer(v) => {
                if let Some(resolved) = self.substitutions.get(v) {
                    self.shallow_resolve(resolved.clone())
                } else {
                    ty
                }
            }
            _ => ty,
        }
    }

    fn resolve_ty_full(&self, ty: TyKind) -> TyKind {
        match self.shallow_resolve(ty) {
            TyKind::Infer(v) => {
                match self.infer_kinds.get(&v) {
                    Some(InferKind::Integer) => TyKind::Int(IntTy::I32),
                    Some(InferKind::Float) => TyKind::Float(FloatTy::F64),
                    _ => TyKind::Error,
                }
            }
            TyKind::Ref(inner, m) => TyKind::Ref(Box::new(self.resolve_ty_full(*inner)), m),
            TyKind::RawPtr(inner, m) => TyKind::RawPtr(Box::new(self.resolve_ty_full(*inner)), m),
            TyKind::Tuple(tys) => TyKind::Tuple(tys.into_iter().map(|t| self.resolve_ty_full(t)).collect()),
            TyKind::Array(inner, n) => TyKind::Array(Box::new(self.resolve_ty_full(*inner)), n),
            TyKind::Slice(inner) => TyKind::Slice(Box::new(self.resolve_ty_full(*inner))),
            other => other,
        }
    }

    // ── Pattern binding ──

    fn bind_pattern(&mut self, pat: &HirPattern, ty: TyKind) {
        match pat {
            HirPattern::Ident(hir_id, _, _, _, _) => {
                if let Some(&def_id) = self.resolve.resolutions.get(hir_id) {
                    self.local_types.insert(def_id, ty);
                }
            }
            HirPattern::Tuple(pats, _) => {
                if let TyKind::Tuple(tys) = &ty {
                    for (p, t) in pats.iter().zip(tys.iter()) {
                        self.bind_pattern(p, t.clone());
                    }
                }
            }
            HirPattern::Wildcard(_) => {}
            HirPattern::Ref(inner, _, _) => {
                if let TyKind::Ref(inner_ty, _) = ty {
                    self.bind_pattern(inner, *inner_ty);
                }
            }
            _ => {}
        }
    }

    // ── Generic substitution ──

    /// Replace TyKind::Param(i) with the corresponding type from `substs`
    fn substitute_params(&self, ty: &TyKind, substs: &[TyKind]) -> TyKind {
        match ty {
            TyKind::Param(idx) => {
                if (*idx as usize) < substs.len() {
                    substs[*idx as usize].clone()
                } else {
                    ty.clone()
                }
            }
            TyKind::Ref(inner, m) => TyKind::Ref(Box::new(self.substitute_params(inner, substs)), *m),
            TyKind::RawPtr(inner, m) => TyKind::RawPtr(Box::new(self.substitute_params(inner, substs)), *m),
            TyKind::Tuple(tys) => TyKind::Tuple(tys.iter().map(|t| self.substitute_params(t, substs)).collect()),
            TyKind::Array(inner, n) => TyKind::Array(Box::new(self.substitute_params(inner, substs)), *n),
            TyKind::Slice(inner) => TyKind::Slice(Box::new(self.substitute_params(inner, substs))),
            TyKind::FnPtr(params, ret) => TyKind::FnPtr(
                params.iter().map(|t| self.substitute_params(t, substs)).collect(),
                Box::new(self.substitute_params(ret, substs)),
            ),
            _ => ty.clone(),
        }
    }

    // ── HIR type to TyKind ──

    fn hir_ty_to_ty(&self, ty: &HirTy) -> TyKind {
        match ty {
            HirTy::Path(path) => {
                if path.segments.is_empty() { return TyKind::Error; }

                // Handle associated type projections: `Self::Item`, `T::Output`, etc.
                if path.segments.len() >= 2 {
                    let first_sym = path.segments[0].ident;
                    let assoc_name = path.segments[1].ident;
                    let first_str = self.interner.resolve(first_sym);

                    // Self::AssocType — look up which trait's impl we're in
                    if first_str == "Self" {
                        // Try to find the associated type in any trait impl
                        for (&(trait_def_id, type_name), concrete_ty) in &self.assoc_types {
                            if type_name == assoc_name {
                                return concrete_ty.clone();
                            }
                        }
                    }

                    // T::AssocType where T is a generic param with trait bounds
                    if let Some(&param_idx) = self.current_generic_params.get(&first_sym) {
                        if let Some(bounds) = self.current_generic_bounds.get(&param_idx) {
                            for trait_def_id in bounds {
                                if let Some(ty) = self.assoc_types.get(&(*trait_def_id, assoc_name)) {
                                    return ty.clone();
                                }
                            }
                        }
                        // Even without a concrete resolution, return the param
                        return TyKind::Param(param_idx);
                    }

                    // Enum::Variant or Module::Type — fall through to single-segment handling
                }

                let name = self.interner.resolve(path.segments[0].ident);
                match name {
                    "i8" => TyKind::Int(IntTy::I8),
                    "i16" => TyKind::Int(IntTy::I16),
                    "i32" => TyKind::Int(IntTy::I32),
                    "i64" => TyKind::Int(IntTy::I64),
                    "i128" => TyKind::Int(IntTy::I128),
                    "isize" => TyKind::Int(IntTy::Isize),
                    "u8" => TyKind::Uint(UintTy::U8),
                    "u16" => TyKind::Uint(UintTy::U16),
                    "u32" => TyKind::Uint(UintTy::U32),
                    "u64" => TyKind::Uint(UintTy::U64),
                    "u128" => TyKind::Uint(UintTy::U128),
                    "usize" => TyKind::Uint(UintTy::Usize),
                    "f32" => TyKind::Float(FloatTy::F32),
                    "f64" => TyKind::Float(FloatTy::F64),
                    "bool" => TyKind::Bool,
                    "char" => TyKind::Char,
                    "str" => TyKind::Str,
                    _ => {
                        let sym = path.segments[0].ident;
                        // Check generic type params first
                        if let Some(&idx) = self.current_generic_params.get(&sym) {
                            return TyKind::Param(idx);
                        }
                        // Look up as ADT by name
                        if let Some(&def_id) = self.type_name_to_def.get(&sym) {
                            // Collect generic args if present
                            let type_args = if let Some(ref args) = path.segments[0].args {
                                args.args.iter().filter_map(|a| {
                                    if let HirGenericArg::Type(ty) = a {
                                        Some(self.hir_ty_to_ty(ty))
                                    } else {
                                        None
                                    }
                                }).collect()
                            } else {
                                vec![]
                            };
                            TyKind::Adt(def_id, type_args)
                        } else {
                            TyKind::Error
                        }
                    }
                }
            }
            HirTy::Reference(_, inner, mutability, _) => {
                TyKind::Ref(Box::new(self.hir_ty_to_ty(inner)), *mutability)
            }
            HirTy::RawPtr(inner, mutability, _) => {
                TyKind::RawPtr(Box::new(self.hir_ty_to_ty(inner)), *mutability)
            }
            HirTy::Tuple(tys, _) => {
                let inner: Vec<TyKind> = tys.iter().map(|t| self.hir_ty_to_ty(t)).collect();
                if inner.is_empty() { TyKind::Unit } else { TyKind::Tuple(inner) }
            }
            HirTy::Array(inner, len_expr, _) => {
                let n = self.eval_const_usize(len_expr);
                TyKind::Array(Box::new(self.hir_ty_to_ty(inner)), n)
            }
            HirTy::Slice(inner, _) => {
                TyKind::Slice(Box::new(self.hir_ty_to_ty(inner)))
            }
            HirTy::FnPtr(params, ret, _) => {
                let ptys: Vec<TyKind> = params.iter().map(|t| self.hir_ty_to_ty(t)).collect();
                let rty = ret.as_ref()
                    .map(|t| self.hir_ty_to_ty(t))
                    .unwrap_or(TyKind::Unit);
                TyKind::FnPtr(ptys, Box::new(rty))
            }
            HirTy::DynTrait(path, _) => {
                if path.segments.is_empty() { return TyKind::Error; }
                let sym = path.segments[0].ident;
                if let Some(&def_id) = self.type_name_to_def.get(&sym) {
                    TyKind::DynTrait(def_id)
                } else {
                    TyKind::Error
                }
            }
            HirTy::Never(_) => TyKind::Never,
            HirTy::Infer(_) => TyKind::Error,
        }
    }

    /// Evaluate a const expression to a usize (for array lengths).
    fn eval_const_usize(&self, expr: &HirExpr) -> usize {
        match &expr.kind {
            HirExprKind::Lit(Literal::Int(n)) => *n as usize,
            _ => 0,
        }
    }
}

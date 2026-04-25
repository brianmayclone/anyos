use crate::prelude::*;
use crate::hir::*;
use crate::ast::{BinOp, Literal, Mutability};
use crate::resolve::ResolveResult;
use crate::intern::{Interner, Symbol};
use crate::coerce::{self, CoercionKind};
use crate::diagnostics::{Span, Diagnostic, Level};
use anyos_std::collections::{HashMap, HashSet};

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
    Projection(Box<TyKind>, Option<DefId>, Symbol),
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
const SYNTH_CMP_ORDERING_DEF_ID: DefId = DefId(0x7000_0002);
const SYNTH_ATOMIC_ORDERING_DEF_ID: DefId = DefId(0x7000_0003);
const SYNTH_PROC_MACRO_TOKEN_TREE_DEF_ID: DefId = DefId(0x7000_0004);
const SYNTH_PROC_MACRO_SPACING_DEF_ID: DefId = DefId(0x7000_0005);
const SYNTH_PROC_MACRO_DELIMITER_DEF_ID: DefId = DefId(0x7000_0006);

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
    /// ADTs known to be Copy through derives or impls.
    pub copy_types: HashSet<DefId>,
}

#[derive(Debug, Clone)]
struct IndexImpl {
    self_def_id: DefId,
    index_ty: TyKind,
    output_ty: TyKind,
}

#[derive(Debug, Clone)]
struct DerefImpl {
    self_ty: TyKind,
    target_ty: TyKind,
}

#[derive(Debug, Clone)]
struct AssocTypeImpl {
    trait_def_id: DefId,
    self_ty: TyKind,
    assoc_name: Symbol,
    assoc_ty: TyKind,
}

#[derive(Debug, Clone)]
struct OperatorImpl {
    op: BinOp,
    self_ty: TyKind,
    rhs_ty: TyKind,
    output_ty: TyKind,
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
    /// Inherent impl methods keyed by the concrete self type DefId.
    impl_methods_by_type: HashMap<DefId, Vec<(Symbol, DefId)>>,
    /// Impl self type for each method body, including trait impl methods.
    impl_self_ty_by_method: HashMap<DefId, TyKind>,
    /// Map type name Symbol -> DefId for structs and enums
    type_name_to_def: HashMap<Symbol, DefId>,
    /// Fully qualified type paths such as `fallback::TokenStream`.
    qualified_type_names: HashMap<String, DefId>,
    /// Simple module aliases introduced by `use crate::module as alias`.
    module_aliases: HashMap<String, String>,
    /// Type imports introduced by `use` in a specific module.
    scoped_type_aliases: HashMap<(String, Symbol), String>,
    /// Module path used while collecting/checking items.
    current_module_path: Vec<Symbol>,
    /// Generic type aliases lowered to TyKind with Param slots.
    type_aliases: HashMap<Symbol, TyKind>,
    /// Module-qualified type aliases, e.g. `fallback::TokenTreeIter`.
    qualified_type_aliases: HashMap<String, TyKind>,
    /// Enum variant field types: enum DefId -> vec of (variant_name, field_types)
    enum_variant_fields: HashMap<DefId, Vec<(Symbol, Vec<TyKind>)>>,
    enum_variant_named_fields: HashMap<(DefId, Symbol), Vec<(Symbol, TyKind)>>,
    /// Map resolver variant DefId -> owning enum DefId
    resolver_variant_to_enum: HashMap<DefId, DefId>,
    /// Current function's generic param symbols -> param index
    current_generic_params: HashMap<Symbol, u32>,
    /// Per call-site: (callee DefId, concrete type args)
    generic_call_substs: HashMap<HirId, (DefId, Vec<TyKind>)>,
    /// Generic function defs: DefId -> number of type params
    generic_fn_defs: HashMap<DefId, usize>,
    /// Generic function param slots. Function params inside generic impls do
    /// not necessarily start at Param(0), so keep their real indices.
    generic_fn_param_indices: HashMap<DefId, Vec<u32>>,
    /// Generic ADT defs: DefId -> number of type params
    adt_generic_counts: HashMap<DefId, usize>,

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
    /// Associated type declarations: trait DefId -> associated type names
    trait_assoc_types: HashMap<DefId, Vec<Symbol>>,
    /// Associated const declarations: trait DefId -> (const name, const type)
    trait_assoc_consts: HashMap<DefId, Vec<(Symbol, TyKind)>>,
    /// Trait bounds on current function's generic params: param_index -> list of trait DefIds
    current_generic_bounds: HashMap<u32, Vec<DefId>>,
    /// Associated type equalities from bounds such as `I: IntoIterator<Item = T>`.
    current_generic_assoc_bindings: HashMap<(u32, DefId, Symbol), TyKind>,
    /// Default trait method bodies: method DefId -> true if body exists in trait def
    trait_default_methods: HashMap<DefId, bool>,
    /// `impl core::ops::Index<I> for T { type Output = O; }` summaries.
    index_impls: Vec<IndexImpl>,
    /// `impl core::ops::Deref for T { type Target = U; }` summaries.
    deref_impls: Vec<DerefImpl>,
    /// `impl Trait for T { type Assoc = U; }` summaries keyed by the impl self type.
    assoc_type_impls: Vec<AssocTypeImpl>,
    /// `impl core::ops::{Add, Mul, ...}<Rhs> for T { type Output = O; }` summaries.
    operator_impls: Vec<OperatorImpl>,
    /// ADTs known to be Copy through derives or impls.
    copy_types: HashSet<DefId>,

    next_infer: u32,
    infer_kinds: HashMap<InferVar, InferKind>,
    substitutions: HashMap<InferVar, TyKind>,

    current_fn_ret: Option<TyKind>,
    current_fn_name: Option<Symbol>,
    current_self_ty: Option<TyKind>,
    current_impl_assoc_types: HashMap<Symbol, TyKind>,
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
            impl_methods_by_type: HashMap::new(),
            impl_self_ty_by_method: HashMap::new(),
            type_name_to_def: HashMap::new(),
            qualified_type_names: HashMap::new(),
            module_aliases: HashMap::new(),
            scoped_type_aliases: HashMap::new(),
            current_module_path: Vec::new(),
            type_aliases: HashMap::new(),
            qualified_type_aliases: HashMap::new(),
            enum_variant_fields: HashMap::new(),
            enum_variant_named_fields: HashMap::new(),
            resolver_variant_to_enum: HashMap::new(),
            current_generic_params: HashMap::new(),
            generic_call_substs: HashMap::new(),
            generic_fn_defs: HashMap::new(),
            generic_fn_param_indices: HashMap::new(),
            adt_generic_counts: HashMap::new(),
            const_values: HashMap::new(),
            static_defs: HashMap::new(),
            trait_methods: HashMap::new(),
            trait_impls: HashMap::new(),
            trait_names: HashMap::new(),
            assoc_types: HashMap::new(),
            trait_assoc_types: HashMap::new(),
            trait_assoc_consts: HashMap::new(),
            current_generic_bounds: HashMap::new(),
            current_generic_assoc_bindings: HashMap::new(),
            trait_default_methods: HashMap::new(),
            index_impls: Vec::new(),
            deref_impls: Vec::new(),
            assoc_type_impls: Vec::new(),
            operator_impls: Vec::new(),
            copy_types: HashSet::new(),
            next_infer: 0,
            infer_kinds: HashMap::new(),
            substitutions: HashMap::new(),
            current_fn_ret: None,
            current_fn_name: None,
            current_self_ty: None,
            current_impl_assoc_types: HashMap::new(),
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
        let msg = if let Some(fn_name) = self.current_fn_name {
            let mut path = self
                .current_module_path
                .iter()
                .map(|sym| self.interner.resolve(*sym).to_string())
                .collect::<Vec<_>>();
            path.push(self.interner.resolve(fn_name).to_string());
            format!("in {}: {}", path.join("::"), msg)
        } else {
            msg.to_string()
        };
        self.errors.push(Diagnostic::new(Level::Error, &msg, span));
    }

    fn is_deferred_ty(ty: &TyKind) -> bool {
        matches!(
            ty,
            TyKind::Param(_)
                | TyKind::Projection(_, _, _)
                | TyKind::Infer(_)
                | TyKind::Error
                | TyKind::DynTrait(_)
        )
    }

    fn contains_deferred_ty(ty: &TyKind) -> bool {
        if Self::is_deferred_ty(ty) {
            return true;
        }
        match ty {
            TyKind::Tuple(tys) => tys.iter().any(Self::contains_deferred_ty),
            TyKind::Array(inner, _)
            | TyKind::Slice(inner)
            | TyKind::Ref(inner, _)
            | TyKind::RawPtr(inner, _) => Self::contains_deferred_ty(inner),
            TyKind::FnDef(_, substs) | TyKind::Adt(_, substs) => {
                substs.iter().any(Self::contains_deferred_ty)
            }
            TyKind::FnPtr(params, ret) => {
                params.iter().any(Self::contains_deferred_ty) || Self::contains_deferred_ty(ret)
            }
            _ => false,
        }
    }

    fn contains_synthetic_def(ty: &TyKind) -> bool {
        match ty {
            TyKind::FnDef(def_id, _) | TyKind::Adt(def_id, _) => def_id.0 >= 10000,
            TyKind::Tuple(tys) => tys.iter().any(Self::contains_synthetic_def),
            TyKind::Array(inner, _)
            | TyKind::Slice(inner)
            | TyKind::Ref(inner, _)
            | TyKind::RawPtr(inner, _)
            | TyKind::Projection(inner, _, _) => Self::contains_synthetic_def(inner),
            TyKind::FnPtr(params, ret) => {
                params.iter().any(Self::contains_synthetic_def) || Self::contains_synthetic_def(ret)
            }
            _ => false,
        }
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

    fn comparison_ordering_ty(&self) -> TyKind {
        let def_id = self
            .lookup_intrinsic_def_by_path("core::cmp::Ordering")
            .unwrap_or(SYNTH_CMP_ORDERING_DEF_ID);
        TyKind::Adt(def_id, vec![])
    }

    fn atomic_ordering_ty(&self) -> TyKind {
        let def_id = self
            .lookup_intrinsic_def_by_path("core::sync::atomic::Ordering")
            .unwrap_or(SYNTH_ATOMIC_ORDERING_DEF_ID);
        TyKind::Adt(def_id, vec![])
    }

    fn token_tree_ty(&self) -> TyKind {
        if let Some(sym) = self.interner.lookup("TokenTree") {
            if let Some(def_id) = self.resolve_scoped_type_name(sym) {
                return TyKind::Adt(def_id, vec![]);
            }
            if let Some(&def_id) = self.type_name_to_def.get(&sym) {
                return TyKind::Adt(def_id, vec![]);
            }
        }
        TyKind::Adt(
            self.lookup_intrinsic_def_by_path("proc_macro::TokenTree")
                .unwrap_or(SYNTH_PROC_MACRO_TOKEN_TREE_DEF_ID),
            vec![],
        )
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

    fn push_generic_scope(
        &mut self,
        params: &[HirGenericParam],
    ) -> (
        HashMap<Symbol, u32>,
        HashMap<u32, Vec<DefId>>,
        HashMap<(u32, DefId, Symbol), TyKind>,
        usize,
    ) {
        let old_generics = self.current_generic_params.clone();
        let old_bounds = self.current_generic_bounds.clone();
        let old_assoc_bindings = self.current_generic_assoc_bindings.clone();
        let mut next_idx = self
            .current_generic_params
            .values()
            .copied()
            .max()
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let mut added = 0usize;
        for gp in params {
            if let HirGenericParam::Type(name, bounds, _, _) = gp {
                self.current_generic_params.insert(*name, next_idx);
                let mut bound_def_ids = Vec::new();
                for bound in bounds {
                    if let Some(trait_def_id) = self.resolve_trait_bound_def(&bound.path) {
                        bound_def_ids.push(trait_def_id);
                        for (assoc_name, assoc_ty) in self.trait_bound_assoc_bindings(&bound.path) {
                            self.current_generic_assoc_bindings
                                .insert((next_idx, trait_def_id, assoc_name), assoc_ty);
                        }
                    }
                }
                if !bound_def_ids.is_empty() {
                    self.current_generic_bounds.insert(next_idx, bound_def_ids);
                }
                next_idx += 1;
                added += 1;
            }
        }
        (old_generics, old_bounds, old_assoc_bindings, added)
    }

    fn pop_generic_scope(
        &mut self,
        old_generics: HashMap<Symbol, u32>,
        old_bounds: HashMap<u32, Vec<DefId>>,
        old_assoc_bindings: HashMap<(u32, DefId, Symbol), TyKind>,
    ) {
        self.current_generic_params = old_generics;
        self.current_generic_bounds = old_bounds;
        self.current_generic_assoc_bindings = old_assoc_bindings;
    }

    fn trait_bound_assoc_bindings(&self, path: &HirPath) -> Vec<(Symbol, TyKind)> {
        let Some(segment) = path.segments.last() else {
            return Vec::new();
        };
        let Some(args) = &segment.args else {
            return Vec::new();
        };
        args.args
            .iter()
            .filter_map(|arg| match arg {
                HirGenericArg::AssocTypeBinding(name, ty) => Some((*name, self.hir_ty_to_ty(ty))),
                _ => None,
            })
            .collect()
    }

    fn collect_param_substs(
        &self,
        template: &TyKind,
        actual: &TyKind,
        substs: &mut Vec<Option<TyKind>>,
    ) {
        match (template, actual) {
            (TyKind::Param(idx), actual) => {
                let idx = *idx as usize;
                if substs.len() <= idx {
                    substs.resize(idx + 1, None);
                }
                if substs[idx].is_none() {
                    substs[idx] = Some(actual.clone());
                }
            }
            (TyKind::Ref(t_inner, t_mut), TyKind::Ref(a_inner, a_mut))
                if *t_mut == Mutability::Immutable || t_mut == a_mut =>
            {
                self.collect_param_substs(t_inner, a_inner, substs);
            }
            (TyKind::RawPtr(t_inner, t_mut), TyKind::RawPtr(a_inner, a_mut))
                if t_mut == a_mut =>
            {
                self.collect_param_substs(t_inner, a_inner, substs);
            }
            (TyKind::Adt(t_def, t_args), TyKind::Adt(a_def, a_args))
                if t_def == a_def && t_args.len() == a_args.len() =>
            {
                for (t_arg, a_arg) in t_args.iter().zip(a_args.iter()) {
                    self.collect_param_substs(t_arg, a_arg, substs);
                }
            }
            (TyKind::Tuple(t_tys), TyKind::Tuple(a_tys)) if t_tys.len() == a_tys.len() => {
                for (t_ty, a_ty) in t_tys.iter().zip(a_tys.iter()) {
                    self.collect_param_substs(t_ty, a_ty, substs);
                }
            }
            (TyKind::Array(t_inner, t_len), TyKind::Array(a_inner, a_len)) if t_len == a_len => {
                self.collect_param_substs(t_inner, a_inner, substs);
            }
            (TyKind::Slice(t_inner), TyKind::Slice(a_inner)) => {
                self.collect_param_substs(t_inner, a_inner, substs);
            }
            _ => {}
        }
    }

    fn infer_receiver_substs(&self, template: &TyKind, actual: &TyKind) -> Option<Vec<TyKind>> {
        let mut substs = Vec::new();
        self.collect_param_substs(template, actual, &mut substs);
        if substs.iter().all(Option::is_none) {
            return None;
        }
        Some(
            substs
                .into_iter()
                .map(|ty| ty.unwrap_or(TyKind::Error))
                .collect(),
        )
    }

    fn path_to_string(&self, path: &HirPath) -> String {
        path.segments
            .iter()
            .map(|seg| self.interner.resolve(seg.ident))
            .collect::<Vec<_>>()
            .join("::")
    }

    fn resolve_trait_bound_def(&self, path: &HirPath) -> Option<DefId> {
        if path.segments.is_empty() {
            return None;
        }

        let full_path = self.path_to_string(path);
        if let Some(def_id) = self.resolve_qualified_type_path(&full_path) {
            return Some(def_id);
        }

        let first = path.segments.first()?.ident;
        if let Some(&def_id) = self.type_name_to_def.get(&first) {
            return Some(def_id);
        }

        let last = path.segments.last()?.ident;
        self.type_name_to_def.get(&last).copied()
    }

    fn const_range_len(&self, expr: &HirExpr) -> Option<usize> {
        let HirExprKind::Range(start, end, inclusive) = &expr.kind else {
            return None;
        };
        let start = start.as_ref().map(|e| self.eval_const_usize(e)).unwrap_or(0);
        let end = end.as_ref().map(|e| self.eval_const_usize(e))?;
        Some(if *inclusive {
            end.saturating_sub(start).saturating_add(1)
        } else {
            end.saturating_sub(start)
        })
    }

    fn iterable_elem_ty(&mut self, iter: &HirExpr, iter_ty: TyKind) -> TyKind {
        let resolved = match self.shallow_resolve(iter_ty) {
            TyKind::Ref(inner, mutability) => {
                match self.shallow_resolve(*inner) {
                    TyKind::Array(elem, _) | TyKind::Slice(elem) => {
                        return TyKind::Ref(elem, mutability);
                    }
                    TyKind::Adt(def_id, substs) if self.is_vec_def(def_id) && substs.len() == 1 => {
                        return TyKind::Ref(Box::new(substs[0].clone()), mutability);
                    }
                    other => other,
                }
            }
            TyKind::RawPtr(inner, _) => self.shallow_resolve(*inner),
            other => other,
        };
        match resolved {
            TyKind::Array(elem, _) | TyKind::Slice(elem) => *elem,
            TyKind::Str => TyKind::Uint(UintTy::U8),
            TyKind::Adt(def_id, substs) if self.is_vec_def(def_id) && substs.len() == 1 => {
                substs[0].clone()
            }
            TyKind::Adt(def_id, _) if self.is_token_stream_def(def_id) => {
                self.token_tree_ty()
            }
            other @ TyKind::Adt(def_id, _) => {
                self.lookup_assoc_type_impl(&other, "IntoIterator", "Item")
                    .or_else(|| self.lookup_assoc_type_impl(&other, "Iterator", "Item"))
                    .or_else(|| self.iterator_item_from_next_method(def_id))
                    .unwrap_or(other)
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
            "__anyrc_println" => Some(TyKind::Unit),
            "__anyrc_format" => {
                if let Some(sym) = self.interner.lookup("String") {
                    if let Some(&def_id) = self.type_name_to_def.get(&sym) {
                        return Some(TyKind::Adt(def_id, vec![]));
                    }
                }
                Some(self.fresh_infer(InferKind::General))
            }
            "__anyrc_format_args" => {
                if let Some(def_id) = self.lookup_intrinsic_def_by_path("Arguments") {
                    return Some(TyKind::Adt(def_id, vec![]));
                }
                Some(self.fresh_infer(InferKind::General))
            }
            "u8::from" => Some(TyKind::Uint(UintTy::U8)),
            "u16::from" => Some(TyKind::Uint(UintTy::U16)),
            "u32::from" => Some(TyKind::Uint(UintTy::U32)),
            "u64::from" => Some(TyKind::Uint(UintTy::U64)),
            "u128::from" => Some(TyKind::Uint(UintTy::U128)),
            "usize::from" => Some(TyKind::Uint(UintTy::Usize)),
            "i8::from" => Some(TyKind::Int(IntTy::I8)),
            "i16::from" => Some(TyKind::Int(IntTy::I16)),
            "i32::from" => Some(TyKind::Int(IntTy::I32)),
            "i64::from" => Some(TyKind::Int(IntTy::I64)),
            "i128::from" => Some(TyKind::Int(IntTy::I128)),
            "isize::from" => Some(TyKind::Int(IntTy::Isize)),
            "u8::from_le" | "u8::from_be" | "u8::to_le" | "u8::to_be" => Some(TyKind::Uint(UintTy::U8)),
            "u16::from_le" | "u16::from_be" | "u16::to_le" | "u16::to_be" => Some(TyKind::Uint(UintTy::U16)),
            "u32::from_le" | "u32::from_be" | "u32::to_le" | "u32::to_be" => Some(TyKind::Uint(UintTy::U32)),
            "u64::from_le" | "u64::from_be" | "u64::to_le" | "u64::to_be" => Some(TyKind::Uint(UintTy::U64)),
            "u128::from_le" | "u128::from_be" | "u128::to_le" | "u128::to_be" => Some(TyKind::Uint(UintTy::U128)),
            "usize::from_le" | "usize::from_be" | "usize::to_le" | "usize::to_be" => Some(TyKind::Uint(UintTy::Usize)),
            "i8::from_le" | "i8::from_be" | "i8::to_le" | "i8::to_be" => Some(TyKind::Int(IntTy::I8)),
            "i16::from_le" | "i16::from_be" | "i16::to_le" | "i16::to_be" => Some(TyKind::Int(IntTy::I16)),
            "i32::from_le" | "i32::from_be" | "i32::to_le" | "i32::to_be" => Some(TyKind::Int(IntTy::I32)),
            "i64::from_le" | "i64::from_be" | "i64::to_le" | "i64::to_be" => Some(TyKind::Int(IntTy::I64)),
            "i128::from_le" | "i128::from_be" | "i128::to_le" | "i128::to_be" => Some(TyKind::Int(IntTy::I128)),
            "isize::from_le" | "isize::from_be" | "isize::to_le" | "isize::to_be" => Some(TyKind::Int(IntTy::Isize)),
            "u8::try_from" => self.result_of(TyKind::Uint(UintTy::U8)),
            "u16::try_from" => self.result_of(TyKind::Uint(UintTy::U16)),
            "u32::try_from" => self.result_of(TyKind::Uint(UintTy::U32)),
            "u64::try_from" => self.result_of(TyKind::Uint(UintTy::U64)),
            "u128::try_from" => self.result_of(TyKind::Uint(UintTy::U128)),
            "usize::try_from" => self.result_of(TyKind::Uint(UintTy::Usize)),
            "i8::try_from" => self.result_of(TyKind::Int(IntTy::I8)),
            "i16::try_from" => self.result_of(TyKind::Int(IntTy::I16)),
            "i32::try_from" => self.result_of(TyKind::Int(IntTy::I32)),
            "i64::try_from" => self.result_of(TyKind::Int(IntTy::I64)),
            "i128::try_from" => self.result_of(TyKind::Int(IntTy::I128)),
            "isize::try_from" => self.result_of(TyKind::Int(IntTy::Isize)),
            "u8::min" | "u8::max" => Some(TyKind::Uint(UintTy::U8)),
            "u16::min" | "u16::max" => Some(TyKind::Uint(UintTy::U16)),
            "u32::min" | "u32::max" => Some(TyKind::Uint(UintTy::U32)),
            "u64::min" | "u64::max" => Some(TyKind::Uint(UintTy::U64)),
            "u128::min" | "u128::max" => Some(TyKind::Uint(UintTy::U128)),
            "usize::min" | "usize::max" => Some(TyKind::Uint(UintTy::Usize)),
            "i8::min" | "i8::max" => Some(TyKind::Int(IntTy::I8)),
            "i16::min" | "i16::max" => Some(TyKind::Int(IntTy::I16)),
            "i32::min" | "i32::max" => Some(TyKind::Int(IntTy::I32)),
            "i64::min" | "i64::max" => Some(TyKind::Int(IntTy::I64)),
            "i128::min" | "i128::max" => Some(TyKind::Int(IntTy::I128)),
            "isize::min" | "isize::max" => Some(TyKind::Int(IntTy::Isize)),
            "core::mem::align_of" | "mem::align_of"
            | "core::mem::size_of" | "mem::size_of" => Some(TyKind::Uint(UintTy::Usize)),
            "core::any::type_name" | "any::type_name" => {
                Some(TyKind::Ref(Box::new(TyKind::Str), Mutability::Immutable))
            }
            "str::to_string" => self.string_ty(),
            "core::hint::unreachable_unchecked" | "hint::unreachable_unchecked" => {
                Some(TyKind::Never)
            }
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

    fn describe_callee(&self, expr: &HirExpr) -> String {
        match &expr.kind {
            HirExprKind::Path(path) => path.segments.iter()
                .map(|seg| self.interner.resolve(seg.ident).to_string())
                .collect::<Vec<_>>()
                .join("::"),
            HirExprKind::Field(base, field) => {
                format!("{}.{}", self.describe_callee(base), self.interner.resolve(*field))
            }
            HirExprKind::Paren(inner) => self.describe_callee(inner),
            _ => String::from("<call>"),
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
                        || name.starts_with("NonZero")
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
        self.collect_qualified_type_names(&krate.items, &mut Vec::new());

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
            copy_types: core::mem::take(&mut self.copy_types),
            type_def_to_name: self.type_name_to_def.iter()
                .filter(|(name, _)| self.interner.resolve(**name) != "Self")
                .map(|(name, &def_id)| (def_id, *name))
                .collect(),
        }
    }

    // ── Pass 1: Collect ──

    fn collect_qualified_type_names(&mut self, items: &[HirItem], prefix: &mut Vec<Symbol>) {
        for item in items {
            match &item.kind {
                HirItemKind::Struct(s) => self.register_qualified_type(prefix, s.name, s.def_id),
                HirItemKind::Enum(e) => self.register_qualified_type(prefix, e.name, e.def_id),
                HirItemKind::Trait(t) => self.register_qualified_type(prefix, t.name, t.def_id),
                HirItemKind::Use(u) => {
                    self.collect_module_alias(u);
                    self.collect_scoped_type_alias(prefix, u);
                }
                HirItemKind::Mod(m) => {
                    self.register_qualified_type(prefix, m.name, m.def_id);
                    if let Some(sub_items) = &m.items {
                        prefix.push(m.name);
                        self.collect_qualified_type_names(sub_items, prefix);
                        prefix.pop();
                    }
                }
                _ => {}
            }
        }
    }

    fn register_qualified_type(&mut self, prefix: &[Symbol], name: Symbol, def_id: DefId) {
        let mut parts = prefix
            .iter()
            .map(|sym| self.interner.resolve(*sym).to_string())
            .collect::<Vec<_>>();
        parts.push(self.interner.resolve(name).to_string());
        let path = parts.join("::");
        self.qualified_type_names.insert(path.clone(), def_id);
        self.qualified_type_names.insert(format!("crate::{}", path), def_id);
    }

    fn qualified_name_for_current_module(&self, name: Symbol) -> String {
        let mut parts = self
            .current_module_path
            .iter()
            .map(|sym| self.interner.resolve(*sym).to_string())
            .collect::<Vec<_>>();
        parts.push(self.interner.resolve(name).to_string());
        parts.join("::")
    }

    fn module_key_from_symbols(&self, prefix: &[Symbol]) -> String {
        prefix
            .iter()
            .map(|sym| self.interner.resolve(*sym).to_string())
            .collect::<Vec<_>>()
            .join("::")
    }

    fn normalize_crate_path(&self, path: &[Symbol]) -> String {
        path.iter()
            .filter_map(|sym| {
                let part = self.interner.resolve(*sym);
                if part == "crate" { None } else { Some(part.to_string()) }
            })
            .collect::<Vec<_>>()
            .join("::")
    }

    fn collect_scoped_type_alias(&mut self, module_prefix: &[Symbol], use_tree: &HirUseTree) {
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
                let target = self.normalize_crate_path(&full_path);
                if target.is_empty() {
                    return;
                }
                let module_key = self.module_key_from_symbols(module_prefix);
                self.scoped_type_aliases.insert((module_key, local_name), target);
            }
            HirUseTreeKind::Nested(trees) => {
                for sub in trees {
                    let mut path = use_tree.path.clone();
                    path.extend_from_slice(&sub.path);
                    let combined = HirUseTree {
                        id: sub.id,
                        vis: sub.vis,
                        path,
                        kind: sub.kind.clone(),
                        span: sub.span,
                    };
                    self.collect_scoped_type_alias(module_prefix, &combined);
                }
            }
            HirUseTreeKind::Glob => {}
        }
    }

    fn collect_module_alias(&mut self, use_tree: &HirUseTree) {
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
                if full_path.is_empty() {
                    return;
                }
                let local = self.interner.resolve(local_name).to_string();
                let target = full_path
                    .iter()
                    .filter_map(|sym| {
                        let part = self.interner.resolve(*sym);
                        if part == "crate" { None } else { Some(part.to_string()) }
                    })
                    .collect::<Vec<_>>()
                    .join("::");
                if !target.is_empty() {
                    self.module_aliases.insert(local, target);
                }
            }
            HirUseTreeKind::Nested(trees) => {
                for sub in trees {
                    let mut path = use_tree.path.clone();
                    path.extend_from_slice(&sub.path);
                    let combined = HirUseTree {
                        id: sub.id,
                        vis: sub.vis,
                        path,
                        kind: sub.kind.clone(),
                        span: sub.span,
                    };
                    self.collect_module_alias(&combined);
                }
            }
            HirUseTreeKind::Glob => {}
        }
    }

    fn collect_item_recursive(&mut self, items: &[HirItem]) {
        for item in items {
            self.collect_item(item);
        }
    }

    fn collect_item(&mut self, item: &HirItem) {
        match &item.kind {
            HirItemKind::Mod(m) => {
                if let Some(sub_items) = &m.items {
                    self.current_module_path.push(m.name);
                    self.collect_item_recursive(sub_items);
                    self.current_module_path.pop();
                }
                return;
            }
            HirItemKind::Fn(f) => {
                let (old_generics, old_bounds, old_assoc_bindings, added_generics) =
                    self.push_generic_scope(&f.generics.params);
                if added_generics > 0 {
                    self.generic_fn_defs.insert(f.def_id, added_generics);
                    let indices = f
                        .generics
                        .params
                        .iter()
                        .filter_map(|gp| match gp {
                            HirGenericParam::Type(name, _, _, _) => {
                                self.current_generic_params.get(name).copied()
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    self.generic_fn_param_indices.insert(f.def_id, indices);
                }

                let params: Vec<TyKind> = f.params.iter()
                    .map(|p| self.hir_ty_to_ty(&p.ty))
                    .collect();
                let ret = f.ret_ty.as_ref()
                    .map(|t| self.hir_ty_to_ty(t))
                    .unwrap_or(TyKind::Unit);
                self.fn_sigs.insert(f.def_id, (params, ret));
                self.pop_generic_scope(old_generics, old_bounds, old_assoc_bindings);
            }
            HirItemKind::Struct(s) => {
                let (old_generics, old_bounds, old_assoc_bindings, _) = self.push_generic_scope(&s.generics.params);
                let saved_self_ty = self.current_self_ty.clone();
                let self_substs = s.generics.params.iter().filter_map(|param| {
                    if let HirGenericParam::Type(name, _, _, _) = param {
                        self.current_generic_params.get(name).copied().map(TyKind::Param)
                    } else {
                        None
                    }
                }).collect::<Vec<_>>();
                let adt_generic_count = self_substs.len();
                self.current_self_ty = Some(TyKind::Adt(s.def_id, self_substs));
                let fields: Vec<(Symbol, TyKind)> = s.fields.iter()
                    .map(|f| (f.name, self.hir_ty_to_ty(&f.ty)))
                    .collect();
                self.struct_defs.insert(s.def_id, fields);
                self.type_name_to_def.insert(s.name, s.def_id);
                self.adt_generic_counts.insert(s.def_id, adt_generic_count);
                if s.derives_copy {
                    self.copy_types.insert(s.def_id);
                }
                self.current_self_ty = saved_self_ty;
                self.pop_generic_scope(old_generics, old_bounds, old_assoc_bindings);
            }
            HirItemKind::Enum(e) => {
                let (old_generics, old_bounds, old_assoc_bindings, _) = self.push_generic_scope(&e.generics.params);
                let saved_self_ty = self.current_self_ty.clone();
                let self_substs = e.generics.params.iter().filter_map(|param| {
                    if let HirGenericParam::Type(name, _, _, _) = param {
                        self.current_generic_params.get(name).copied().map(TyKind::Param)
                    } else {
                        None
                    }
                }).collect::<Vec<_>>();
                let adt_generic_count = self_substs.len();
                self.current_self_ty = Some(TyKind::Adt(e.def_id, self_substs));
                self.type_name_to_def.insert(e.name, e.def_id);
                self.adt_generic_counts.insert(e.def_id, adt_generic_count);
                if e.derives_copy {
                    self.copy_types.insert(e.def_id);
                }
                // Collect variant field types
                let mut variants = Vec::new();
                for v in &e.variants {
                    let field_tys = match &v.fields {
                        HirVariantFields::Unit => vec![],
                        HirVariantFields::Tuple(tys) => {
                            tys.iter().map(|t| self.hir_ty_to_ty(t)).collect()
                        }
                        HirVariantFields::Struct(fields) => {
                            let named_fields = fields
                                .iter()
                                .map(|f| (f.name, self.hir_ty_to_ty(&f.ty)))
                                .collect::<Vec<_>>();
                            let field_tys = named_fields
                                .iter()
                                .map(|(_, ty)| ty.clone())
                                .collect();
                            self.enum_variant_named_fields
                                .insert((e.def_id, v.name), named_fields);
                            field_tys
                        }
                    };
                    variants.push((v.name, field_tys));
                }
                self.enum_variant_fields.insert(e.def_id, variants);
                self.current_self_ty = saved_self_ty;
                self.pop_generic_scope(old_generics, old_bounds, old_assoc_bindings);
            }
            HirItemKind::TypeAlias(ta) => {
                let (old_generics, old_bounds, old_assoc_bindings, _) = self.push_generic_scope(&ta.generics.params);
                if let Some(ty) = &ta.ty {
                    let alias_ty = self.hir_ty_to_ty(ty);
                    let qualified_name = self.qualified_name_for_current_module(ta.name);
                    self.qualified_type_aliases.insert(qualified_name, alias_ty.clone());
                    if self.current_module_path.is_empty() {
                        self.type_aliases.insert(ta.name, alias_ty);
                    }
                }
                self.pop_generic_scope(old_generics, old_bounds, old_assoc_bindings);
            }
            HirItemKind::Impl(ib) => {
                let (old_generics, old_bounds, old_assoc_bindings, _) = self.push_generic_scope(&ib.generics.params);
                let saved_self_ty = self.current_self_ty.clone();
                self.current_self_ty = Some(self.hir_ty_to_ty(&ib.self_ty));
                if let Some(TyKind::Adt(self_def_id, _)) = self.current_self_ty.clone().map(|ty| self.shallow_resolve(ty)) {
                    for sub in &ib.items {
                        if let HirItemKind::Fn(f) = &sub.kind {
                            self.impl_self_ty_by_method
                                .insert(f.def_id, self.current_self_ty.clone().unwrap_or(TyKind::Error));
                            self.impl_methods_by_type
                                .entry(self_def_id)
                                .or_default()
                                .push((f.name, f.def_id));
                        }
                    }
                }
                if let Some(trait_ref) = &ib.trait_ref {
                    self.collect_index_impl(ib, trait_ref);
                    self.collect_deref_impl(ib, trait_ref);
                    self.collect_operator_impl(ib, trait_ref);
                }
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
                            if let Some(trait_def_id) = self.resolve_trait_bound_def(trait_ref) {
                                if trait_ref
                                    .segments
                                    .last()
                                    .is_some_and(|seg| self.interner.resolve(seg.ident) == "Copy")
                                {
                                    if let Some(TyKind::Adt(def_id, _)) =
                                        self.current_self_ty.clone().map(|ty| self.shallow_resolve(ty))
                                    {
                                        self.copy_types.insert(def_id);
                                    }
                                }
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
                                                if let Some(self_ty) = self.current_self_ty.clone() {
                                                    self.assoc_type_impls.push(AssocTypeImpl {
                                                        trait_def_id,
                                                        self_ty,
                                                        assoc_name: ta.name,
                                                        assoc_ty: self.hir_ty_to_ty(ty),
                                                    });
                                                }
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
                for sub in &ib.items {
                    self.collect_item(sub);
                }
                self.current_self_ty = saved_self_ty;
                self.pop_generic_scope(old_generics, old_bounds, old_assoc_bindings);
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
                let (old_generics, old_bounds, old_assoc_bindings, _) = self.push_generic_scope(&t.generics.params);
                let saved_self_ty = self.current_self_ty.clone();
                self.current_self_ty = Some(TyKind::Adt(t.def_id, vec![]));
                // Register trait name so `dyn Trait` can resolve it
                self.type_name_to_def.insert(t.name, t.def_id);
                self.trait_names.insert(t.def_id, t.name);
                if let Some(self_sym) = self.interner.lookup("Self") {
                    self.type_name_to_def.insert(self_sym, t.def_id);
                }
                // Register trait method signatures and ordering
                let mut methods = Vec::new();
                let mut assoc_type_names = Vec::new();
                let mut assoc_consts = Vec::new();
                for sub in &t.items {
                    match &sub.kind {
                        HirItemKind::Fn(f) => {
                            methods.push((f.name, f.def_id));
                            // Track whether this method has a default body
                            self.trait_default_methods.insert(f.def_id, f.body.is_some());
                        }
                        HirItemKind::TypeAlias(ta) => assoc_type_names.push(ta.name),
                        HirItemKind::Const(c) => {
                            assoc_consts.push((c.name, self.hir_ty_to_ty(&c.ty)));
                        }
                        _ => {}
                    }
                    self.collect_item(sub);
                }
                self.trait_methods.insert(t.def_id, methods);
                self.trait_assoc_types.insert(t.def_id, assoc_type_names);
                self.trait_assoc_consts.insert(t.def_id, assoc_consts);
                self.current_self_ty = saved_self_ty;
                self.pop_generic_scope(old_generics, old_bounds, old_assoc_bindings);
            }
            _ => {}
        }
    }

    fn collect_index_impl(&mut self, ib: &HirImplBlock, trait_ref: &HirPath) {
        let Some(last) = trait_ref.segments.last() else {
            return;
        };
        if self.interner.resolve(last.ident) != "Index" {
            return;
        }
        let TyKind::Adt(self_def_id, _) = self.shallow_resolve(self.hir_ty_to_ty(&ib.self_ty)) else {
            return;
        };
        let Some(args) = &last.args else {
            return;
        };
        let Some(HirGenericArg::Type(index_hir_ty)) = args.args.first() else {
            return;
        };
        let Some(output_sym) = self.interner.lookup("Output") else {
            return;
        };
        let Some(output_hir_ty) = ib.items.iter().find_map(|item| {
            if let HirItemKind::TypeAlias(ta) = &item.kind {
                if ta.name == output_sym {
                    return ta.ty.as_ref();
                }
            }
            None
        }) else {
            return;
        };
        let index_ty = self.hir_ty_to_ty(index_hir_ty);
        let output_ty = self.hir_ty_to_ty(output_hir_ty);
        self.index_impls.push(IndexImpl { self_def_id, index_ty, output_ty });
    }

    fn collect_deref_impl(&mut self, ib: &HirImplBlock, trait_ref: &HirPath) {
        let Some(last) = trait_ref.segments.last() else {
            return;
        };
        let trait_name = self.interner.resolve(last.ident);
        if trait_name != "Deref" && trait_name != "DerefMut" {
            return;
        }
        let Some(target_sym) = self.interner.lookup("Target") else {
            return;
        };
        let Some(target_hir_ty) = ib.items.iter().find_map(|item| {
            if let HirItemKind::TypeAlias(ta) = &item.kind {
                if ta.name == target_sym {
                    return ta.ty.as_ref();
                }
            }
            None
        }) else {
            return;
        };
        let self_ty = self.hir_ty_to_ty(&ib.self_ty);
        let target_ty = self.hir_ty_to_ty(target_hir_ty);
        self.deref_impls.push(DerefImpl { self_ty, target_ty });
    }

    fn collect_operator_impl(&mut self, ib: &HirImplBlock, trait_ref: &HirPath) {
        let Some(last) = trait_ref.segments.last() else {
            return;
        };
        if let Some(ops) = self.comparison_trait_binops(last.ident) {
            let self_ty = self.hir_ty_to_ty(&ib.self_ty);
            let rhs_ty = last
                .args
                .as_ref()
                .and_then(|args| args.args.first())
                .and_then(|arg| match arg {
                    HirGenericArg::Type(ty) => Some(self.hir_ty_to_ty(ty)),
                    _ => None,
                })
                .unwrap_or_else(|| self_ty.clone());
            for op in ops {
                self.operator_impls.push(OperatorImpl {
                    op: *op,
                    self_ty: self_ty.clone(),
                    rhs_ty: rhs_ty.clone(),
                    output_ty: TyKind::Bool,
                });
            }
            return;
        }
        let Some(op) = self.operator_trait_to_binop(last.ident) else {
            return;
        };
        let Some(output_sym) = self.interner.lookup("Output") else {
            return;
        };
        let Some(output_hir_ty) = ib.items.iter().find_map(|item| {
            if let HirItemKind::TypeAlias(ta) = &item.kind {
                if ta.name == output_sym {
                    return ta.ty.as_ref();
                }
            }
            None
        }) else {
            return;
        };

        let self_ty = self.hir_ty_to_ty(&ib.self_ty);
        let rhs_ty = last
            .args
            .as_ref()
            .and_then(|args| args.args.first())
            .and_then(|arg| match arg {
                HirGenericArg::Type(ty) => Some(self.hir_ty_to_ty(ty)),
                _ => None,
            })
            .unwrap_or_else(|| self_ty.clone());
        let output_ty = self.hir_ty_to_ty(output_hir_ty);
        self.operator_impls.push(OperatorImpl { op, self_ty, rhs_ty, output_ty });
    }

    fn operator_trait_to_binop(&self, trait_name: Symbol) -> Option<BinOp> {
        match self.interner.resolve(trait_name) {
            "Add" => Some(BinOp::Add),
            "Sub" => Some(BinOp::Sub),
            "Mul" => Some(BinOp::Mul),
            "Div" => Some(BinOp::Div),
            "Rem" => Some(BinOp::Rem),
            "BitAnd" => Some(BinOp::BitAnd),
            "BitOr" => Some(BinOp::BitOr),
            "BitXor" => Some(BinOp::BitXor),
            "Shl" => Some(BinOp::Shl),
            "Shr" => Some(BinOp::Shr),
            _ => None,
        }
    }

    fn comparison_trait_binops(&self, trait_name: Symbol) -> Option<&'static [BinOp]> {
        match self.interner.resolve(trait_name) {
            "PartialEq" => Some(&[BinOp::Eq, BinOp::Ne]),
            "PartialOrd" => Some(&[BinOp::Lt, BinOp::Le, BinOp::Gt, BinOp::Ge]),
            _ => None,
        }
    }

    fn operator_trait_name_for_binop(&self, op: BinOp) -> Option<&'static str> {
        match op {
            BinOp::Add => Some("Add"),
            BinOp::Sub => Some("Sub"),
            BinOp::Mul => Some("Mul"),
            BinOp::Div => Some("Div"),
            BinOp::Rem => Some("Rem"),
            BinOp::BitAnd => Some("BitAnd"),
            BinOp::BitOr => Some("BitOr"),
            BinOp::BitXor => Some("BitXor"),
            BinOp::Shl => Some("Shl"),
            BinOp::Shr => Some("Shr"),
            _ => None,
        }
    }

    fn operator_trait_def_for_binop(&self, op: BinOp) -> Option<DefId> {
        let trait_name = self.operator_trait_name_for_binop(op)?;
        self.trait_names
            .iter()
            .find_map(|(def_id, name)| {
                (self.interner.resolve(*name) == trait_name).then_some(*def_id)
            })
    }

    fn contains_generic_projection_input(ty: &TyKind) -> bool {
        match ty {
            TyKind::Param(_) | TyKind::Projection(_, _, _) => true,
            TyKind::Tuple(tys) => tys.iter().any(Self::contains_generic_projection_input),
            TyKind::Array(inner, _)
            | TyKind::Slice(inner)
            | TyKind::Ref(inner, _)
            | TyKind::RawPtr(inner, _) => Self::contains_generic_projection_input(inner),
            TyKind::FnDef(_, substs) | TyKind::Adt(_, substs) => {
                substs.iter().any(Self::contains_generic_projection_input)
            }
            TyKind::FnPtr(params, ret) => {
                params.iter().any(Self::contains_generic_projection_input)
                    || Self::contains_generic_projection_input(ret)
            }
            _ => false,
        }
    }

    fn generic_operator_output(&self, op: BinOp, lhs_ty: &TyKind, rhs_ty: &TyKind) -> Option<TyKind> {
        if !Self::contains_generic_projection_input(lhs_ty)
            && !Self::contains_generic_projection_input(rhs_ty)
        {
            return None;
        }
        let output_sym = self.interner.lookup("Output")?;
        Some(TyKind::Projection(
            Box::new(lhs_ty.clone()),
            self.operator_trait_def_for_binop(op),
            output_sym,
        ))
    }

    fn lookup_index_output(&self, self_def_id: DefId, index_ty: &TyKind) -> Option<TyKind> {
        self.index_impls
            .iter()
            .find(|impl_info| {
                impl_info.self_def_id == self_def_id
                    && self.ty_matches_for_candidate(&impl_info.index_ty, index_ty)
            })
            .map(|impl_info| impl_info.output_ty.clone())
    }

    fn lookup_deref_target(&self, actual_ty: &TyKind) -> Option<TyKind> {
        let actual_ty = self.shallow_resolve(actual_ty.clone());
        let TyKind::Adt(actual_def, _) = &actual_ty else {
            return None;
        };
        for impl_info in &self.deref_impls {
            let TyKind::Adt(template_def, _) = self.shallow_resolve(impl_info.self_ty.clone()) else {
                continue;
            };
            if &template_def != actual_def {
                continue;
            }
            let mut substs = Vec::new();
            self.collect_param_substs(&impl_info.self_ty, &actual_ty, &mut substs);
            let substs = substs
                .into_iter()
                .map(|ty| ty.unwrap_or(TyKind::Error))
                .collect::<Vec<_>>();
            return Some(self.substitute_params(&impl_info.target_ty, &substs));
        }
        None
    }

    fn lookup_assoc_type_impl(
        &self,
        actual_ty: &TyKind,
        trait_name: &str,
        assoc_name: &str,
    ) -> Option<TyKind> {
        let actual_ty = self.shallow_resolve(actual_ty.clone());
        for impl_info in &self.assoc_type_impls {
            if !self.trait_def_name_is(impl_info.trait_def_id, trait_name)
                || !self.symbol_name_is(impl_info.assoc_name, assoc_name)
                || !self.ty_matches_for_candidate(&impl_info.self_ty, &actual_ty)
            {
                continue;
            }
            let mut substs = Vec::new();
            self.collect_param_substs(&impl_info.self_ty, &actual_ty, &mut substs);
            let substs = substs
                .into_iter()
                .map(|ty| ty.unwrap_or(TyKind::Error))
                .collect::<Vec<_>>();
            return Some(self.substitute_params(&impl_info.assoc_ty, &substs));
        }
        None
    }

    fn iterator_item_from_next_method(&self, def_id: DefId) -> Option<TyKind> {
        let methods = self.impl_methods_by_type.get(&def_id)?;
        let next_sym = self.interner.lookup("next")?;
        for (method_name, method_def_id) in methods {
            if *method_name != next_sym {
                continue;
            }
            let (_, ret_ty) = self.fn_sigs.get(method_def_id)?;
            if let Some(item_ty) = self.option_inner_ty(ret_ty) {
                return Some(item_ty);
            }
        }
        None
    }

    fn index_builtin_target(
        &mut self,
        target_ty: TyKind,
        idx_ty: &TyKind,
        is_range: bool,
        idx_span: Span,
        expr_span: Span,
    ) -> Option<TyKind> {
        let target_ty = self.index_target_ty(target_ty);
        match target_ty {
            TyKind::Array(elem, _) | TyKind::Slice(elem) => {
                self.check_builtin_index_ty(idx_ty, idx_span, is_range);
                Some(if is_range { TyKind::Slice(elem) } else { *elem })
            }
            TyKind::Str if is_range => Some(TyKind::Str),
            TyKind::Str => {
                self.error(expr_span, "cannot index this type: str");
                Some(TyKind::Error)
            }
            _ => None,
        }
    }

    fn index_target_ty(&self, mut ty: TyKind) -> TyKind {
        loop {
            match self.shallow_resolve(ty) {
                TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => {
                    ty = *inner;
                }
                other => return other,
            }
        }
    }

    fn lookup_operator_impl(&self, op: BinOp, lhs_ty: &TyKind, rhs_ty: &TyKind) -> Option<OperatorImpl> {
        if matches!(
            self.shallow_resolve(lhs_ty.clone()),
            TyKind::Error | TyKind::Infer(_) | TyKind::Param(_)
        ) {
            return None;
        }
        self.operator_impls
            .iter()
            .find_map(|impl_info| {
                if impl_info.op != op
                    || !self.ty_matches_for_operator_candidate(&impl_info.self_ty, lhs_ty)
                    || !self.ty_matches_for_operator_candidate(&impl_info.rhs_ty, rhs_ty)
                {
                    return None;
                }
                let mut substs = Vec::new();
                self.collect_param_substs(&impl_info.self_ty, lhs_ty, &mut substs);
                self.collect_param_substs(&impl_info.rhs_ty, rhs_ty, &mut substs);
                let substs = substs
                    .into_iter()
                    .map(|ty| ty.unwrap_or(TyKind::Error))
                    .collect::<Vec<_>>();
                Some(OperatorImpl {
                    op: impl_info.op,
                    self_ty: self.substitute_params(&impl_info.self_ty, &substs),
                    rhs_ty: self.substitute_params(&impl_info.rhs_ty, &substs),
                    output_ty: self.substitute_params(&impl_info.output_ty, &substs),
                })
            })
    }

    fn ty_matches_for_operator_candidate(&self, expected: &TyKind, actual: &TyKind) -> bool {
        let expected = self.shallow_resolve(expected.clone());
        let actual = self.shallow_resolve(actual.clone());
        if let TyKind::Infer(var) = actual {
            return match self.infer_kinds.get(&var).copied().unwrap_or(InferKind::General) {
                InferKind::Integer => matches!(
                    expected,
                    TyKind::Int(_) | TyKind::Uint(_) | TyKind::Infer(_) | TyKind::Param(_)
                ),
                InferKind::Float => matches!(
                    expected,
                    TyKind::Float(_) | TyKind::Infer(_) | TyKind::Param(_)
                ),
                InferKind::General => matches!(expected, TyKind::Infer(_) | TyKind::Param(_)),
            };
        }
        self.ty_matches_for_candidate(&expected, &actual)
    }

    fn check_builtin_index_ty(&mut self, index_ty: &TyKind, span: Span, is_range: bool) {
        if is_range {
            return;
        }
        match index_ty {
            TyKind::Int(_) | TyKind::Uint(_) => {}
            other => self.unify(&TyKind::Uint(UintTy::Usize), other, span),
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
            HirExprKind::Path(_) | HirExprKind::QualifiedPath(_) => {
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
                let (old_generics, old_bounds, old_assoc_bindings, _) = self.push_generic_scope(&ib.generics.params);
                let saved_self_ty = self.current_self_ty.clone();
                let saved_assoc_types = self.current_impl_assoc_types.clone();
                self.current_self_ty = Some(self.hir_ty_to_ty(&ib.self_ty));
                self.current_impl_assoc_types.clear();
                for sub in &ib.items {
                    if let HirItemKind::TypeAlias(ta) = &sub.kind {
                        if let Some(ty) = &ta.ty {
                            self.current_impl_assoc_types.insert(ta.name, self.hir_ty_to_ty(ty));
                        }
                    }
                }
                for sub in &ib.items {
                    self.check_item(sub);
                }
                self.current_self_ty = saved_self_ty;
                self.current_impl_assoc_types = saved_assoc_types;
                self.pop_generic_scope(old_generics, old_bounds, old_assoc_bindings);
            }
            HirItemKind::Mod(m) => {
                if let Some(sub_items) = &m.items {
                    self.current_module_path.push(m.name);
                    for sub in sub_items {
                        self.check_item(sub);
                    }
                    self.current_module_path.pop();
                }
            }
            _ => {}
        }
    }

    fn check_fn(&mut self, f: &HirFnDef) {
        // Set up generic params and bounds for this function's body
        let (old_generics, old_bounds, old_assoc_bindings, _) = self.push_generic_scope(&f.generics.params);

        let sig = self.fn_sigs.get(&f.def_id).cloned();
        let (param_tys, ret_ty) = sig.unwrap_or_else(|| (vec![], TyKind::Unit));

        for (param, ty) in f.params.iter().zip(param_tys.iter()) {
            self.bind_pattern(&param.pat, ty.clone());
        }

        let old_ret = self.current_fn_ret.replace(ret_ty.clone());
        let old_fn_name = self.current_fn_name.replace(f.name);

        if let Some(body) = &f.body {
            let body_ty = self.check_block(body);
            if body_ty != TyKind::Never {
                self.unify(&ret_ty, &body_ty, body.span);
            }
        }

        self.current_fn_ret = old_ret;
        self.current_fn_name = old_fn_name;
        self.pop_generic_scope(old_generics, old_bounds, old_assoc_bindings);
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
                    // A diverging statement keeps the enclosing block diverging
                    // even if local items follow it. Items do not execute and
                    // must not turn `{ return x; fn helper() {} }` into `()`.
                    if ty == TyKind::Never {
                        last_ty = TyKind::Never;
                    } else {
                        last_ty = TyKind::Unit;
                    }
                }
                HirStmt::Item(item) => {
                    self.collect_item(item);
                    self.check_item(item);
                    if last_ty != TyKind::Never {
                        last_ty = TyKind::Unit;
                    }
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

            HirExprKind::QualifiedPath(qpath) => {
                self.infer_qualified_path_type(qpath, expr.id)
            }

            HirExprKind::Binary(op, lhs, rhs) => {
                let raw_lty = self.check_expr(lhs);
                let raw_rty = self.check_expr(rhs);
                let lty = self.binary_operand_ty(raw_lty);
                let rty = self.binary_operand_ty(raw_rty);
                match op {
                    BinOp::And | BinOp::Or => {
                        self.unify(&TyKind::Bool, &lty, lhs.span);
                        self.unify(&TyKind::Bool, &rty, rhs.span);
                        TyKind::Bool
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        if let Some(impl_info) = self.lookup_operator_impl(*op, &lty, &rty) {
                            self.unify(&impl_info.self_ty, &lty, lhs.span);
                            self.unify(&impl_info.rhs_ty, &rty, rhs.span);
                            return TyKind::Bool;
                        }
                        self.unify(&lty, &rty, expr.span);
                        TyKind::Bool
                    }
                    BinOp::Shl | BinOp::Shr => {
                        if let Some(output_ty) = self.generic_operator_output(*op, &lty, &rty) {
                            return output_ty;
                        }
                        if let Some(impl_info) = self.lookup_operator_impl(*op, &lty, &rty) {
                            self.unify(&impl_info.self_ty, &lty, lhs.span);
                            self.unify(&impl_info.rhs_ty, &rty, rhs.span);
                            return impl_info.output_ty;
                        }
                        lty
                    }
                    _ => {
                        if let Some(output_ty) = self.generic_operator_output(*op, &lty, &rty) {
                            return output_ty;
                        }
                        if let Some(impl_info) = self.lookup_operator_impl(*op, &lty, &rty) {
                            self.unify(&impl_info.self_ty, &lty, lhs.span);
                            self.unify(&impl_info.rhs_ty, &rty, rhs.span);
                            return impl_info.output_ty;
                        }
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
                            TyKind::Adt(def_id, substs) => {
                                let adt_ty = TyKind::Adt(def_id, substs);
                                if self.is_string_def(def_id)
                                    || self.describe_ty(&adt_ty).rsplit("::").next() == Some("String")
                                {
                                    TyKind::Str
                                } else {
                                    self.lookup_deref_target(&adt_ty).unwrap_or_else(|| {
                                        self.error(expr.span, "cannot dereference this type");
                                        TyKind::Error
                                    })
                                }
                            }
                            _ => {
                                self.error(expr.span, "cannot dereference this type");
                                TyKind::Error
                            }
                        }
                    }
                }
            }

            HirExprKind::Call(callee, args) => {
                if let HirExprKind::Path(path) = &callee.kind {
                    match self.path_to_string(path).as_str() {
                        "core::str::from_utf8" if args.len() == 1 => {
                            let arg_ty = self.check_expr(&args[0]);
                            let expected = TyKind::Ref(
                                Box::new(TyKind::Slice(Box::new(TyKind::Uint(UintTy::U8)))),
                                Mutability::Immutable,
                            );
                            self.unify(&expected, &arg_ty, args[0].span);
                            if let Some(result_ty) = self.result_of(TyKind::Ref(
                                Box::new(TyKind::Str),
                                Mutability::Immutable,
                            )) {
                                return result_ty;
                            }
                            return self.fresh_infer(InferKind::General);
                        }
                        _ => {}
                    }
                    if let Some(ret_ty) = self.check_assoc_fn_call_on_path(path, args, expr.span, callee) {
                        return ret_ty;
                    }
                }
                if let HirExprKind::QualifiedPath(qpath) = &callee.kind {
                    if let Some(ret_ty) =
                        self.check_assoc_fn_call_on_qualified_path(qpath, args, expr.span, callee)
                    {
                        return ret_ty;
                    }
                }
                let callee_ty = self.check_expr(callee);
                match self.shallow_resolve(callee_ty) {
                    TyKind::FnDef(def_id, _) => {
                        // Intrinsic functions: accept any args
                        if let Some(intrinsic_path) = self.resolve.intrinsic_fns.get(&def_id).cloned() {
                            let arg_tys = args.iter().map(|a| self.check_expr(a)).collect::<Vec<_>>();
                            if let Some(ty) = self.intrinsic_call_return_type(&intrinsic_path) {
                                return ty;
                            }
                            // For Type::new() constructors, return the type as Adt
                            if intrinsic_path.ends_with("::new") {
                                let type_name = intrinsic_path.rsplit("::").nth(1).unwrap_or("");
                                if let Some(sym) = self.interner.lookup(type_name) {
                                    if let Some(&type_def_id) = self.type_name_to_def.get(&sym) {
                                        if type_name == "Vec" {
                                            return TyKind::Adt(
                                                type_def_id,
                                                vec![self.fresh_infer(InferKind::General)],
                                            );
                                        }
                                        if type_name == "Box" && arg_tys.len() == 1 {
                                            return TyKind::Adt(type_def_id, vec![arg_tys[0].clone()]);
                                        }
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
                                    "wrong number of arguments for {}: expected {}, found {}",
                                    self.describe_callee(callee),
                                    param_tys.len(),
                                    args.len(),
                                ));
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
                    TyKind::Adt(adt_def_id, substs) if self.struct_defs.contains_key(&adt_def_id) => {
                        let substs = if substs.is_empty() {
                            match self.fresh_adt_ty(adt_def_id, substs) {
                                TyKind::Adt(_, substs) => substs,
                                _ => Vec::new(),
                            }
                        } else {
                            substs
                        };
                        let expected_fields = self.struct_defs.get(&adt_def_id).cloned().unwrap_or_default();
                        for (arg, (_, field_ty)) in args.iter().zip(expected_fields.iter()) {
                            let aty = self.check_expr(arg);
                            let expected = self.substitute_params(field_ty, &substs);
                            self.unify(&expected, &aty, arg.span);
                        }
                        for arg in args.iter().skip(expected_fields.len()) {
                            self.check_expr(arg);
                        }
                        TyKind::Adt(adt_def_id, substs)
                    }
                    TyKind::Adt(enum_def_id, _) => {
                        // Enum variant constructor call, e.g. Option::Some(42)
                        // Check args against variant field types
                        let arg_tys = args.iter().map(|a| self.check_expr(a)).collect::<Vec<_>>();
                        if let HirExprKind::Path(path) = &callee.kind {
                            if let Some(last) = path.segments.last() {
                                let variant = self.interner.resolve(last.ident);
                                if self.is_option_def(enum_def_id) {
                                    return match (variant, arg_tys.first()) {
                                        ("Some", Some(value_ty)) => {
                                            TyKind::Adt(enum_def_id, vec![value_ty.clone()])
                                        }
                                        ("None", _) => {
                                            TyKind::Adt(enum_def_id, vec![self.fresh_infer(InferKind::General)])
                                        }
                                        _ => TyKind::Adt(enum_def_id, vec![]),
                                    };
                                }
                                if self.is_result_def(enum_def_id) {
                                    return match (variant, arg_tys.first()) {
                                        ("Ok", Some(value_ty)) => TyKind::Adt(
                                            enum_def_id,
                                            vec![value_ty.clone(), self.fresh_infer(InferKind::General)],
                                        ),
                                        ("Err", Some(err_ty)) => TyKind::Adt(
                                            enum_def_id,
                                            vec![self.fresh_infer(InferKind::General), err_ty.clone()],
                                        ),
                                        _ => TyKind::Adt(enum_def_id, vec![]),
                                    };
                                }
                            }
                        }
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
                let mut resolved = self.shallow_resolve(base_ty);
                for _ in 0..8 {
                    resolved = match resolved {
                        TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => {
                            self.shallow_resolve(inner.as_ref().clone())
                        }
                        TyKind::Adt(def_id, substs) if self.is_box_def(def_id) && substs.len() == 1 => {
                            self.shallow_resolve(substs[0].clone())
                        }
                        other => other,
                    };

                    match resolved.clone() {
                        TyKind::Adt(def_id, substs) => {
                            if let Some(fields) = self.struct_defs.get(&def_id) {
                                if let Some((_, fty)) = fields.iter().find(|(n, _)| *n == *field_name) {
                                    return self.substitute_params(fty, &substs);
                                }
                            }
                            if let Some(target_ty) =
                                self.lookup_deref_target(&TyKind::Adt(def_id, substs))
                            {
                                resolved = self.shallow_resolve(target_ty);
                                continue;
                            }
                            if self.struct_defs.contains_key(&def_id) {
                                self.error(expr.span, "no such field");
                            }
                            return TyKind::Error;
                        }
                        TyKind::Tuple(tys) => {
                            let idx_str = self.interner.resolve(*field_name);
                            if let Ok(idx) = idx_str.parse::<usize>() {
                                if idx < tys.len() { return tys[idx].clone(); }
                                self.error(expr.span, "tuple index out of bounds");
                            } else {
                                self.error(expr.span, "no such field");
                            }
                            return TyKind::Error;
                        }
                        other => {
                            if Self::is_deferred_ty(&other) {
                                return self.fresh_infer(InferKind::General);
                            }
                            self.error(expr.span, "field access on non-struct type");
                            return TyKind::Error;
                        }
                    }
                }

                self.error(expr.span, "field autoderef limit reached");
                TyKind::Error
            }

            HirExprKind::Struct(path, fields, _base) => {
                if let Some(enum_ty) = self.enum_variant_type_from_path(path) {
                    if let TyKind::Adt(enum_def_id, ref substs) = enum_ty {
                        let variant_name = path.segments.last().unwrap().ident;
                        if let Some(expected_fields) = self
                            .enum_variant_named_fields
                            .get(&(enum_def_id, variant_name))
                            .cloned()
                        {
                            for fe in fields {
                                let val_ty = self.check_expr(&fe.value);
                                if let Some((_, fty)) =
                                    expected_fields.iter().find(|(n, _)| *n == fe.name)
                                {
                                    let expected_ty = self.substitute_params(fty, substs);
                                    self.unify(&expected_ty, &val_ty, fe.span);
                                }
                            }
                        } else {
                            for fe in fields {
                                self.check_expr(&fe.value);
                            }
                        }
                    }
                    return enum_ty;
                }

                let adt_ty = match self.shallow_resolve(self.path_segments_to_ty(&path.segments)) {
                    TyKind::Adt(def_id, substs) => Some(self.fresh_adt_ty(def_id, substs)),
                    _ => self
                        .resolve
                        .resolutions
                        .get(&expr.id)
                        .copied()
                        .map(|def_id| self.fresh_adt_ty(def_id, Vec::new())),
                };

                if let Some(TyKind::Adt(def_id, substs)) = adt_ty {
                    if let Some(expected_fields) = self.struct_defs.get(&def_id).cloned() {
                        for fe in fields {
                            let val_ty = self.check_expr(&fe.value);
                            if let Some((_, fty)) = expected_fields.iter().find(|(n, _)| *n == fe.name) {
                                let expected_ty = self.substitute_params(fty, &substs);
                                self.unify(&expected_ty, &val_ty, fe.span);
                            }
                        }
                    } else {
                        for fe in fields { self.check_expr(&fe.value); }
                    }
                    TyKind::Adt(def_id, substs)
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
                let scr_ty = self.check_expr(scrutinee);
                if arms.is_empty() {
                    return TyKind::Never;
                }
                self.bind_pattern(&arms[0].pat, scr_ty.clone());
                if let Some(guard) = &arms[0].guard {
                    let guard_ty = self.check_expr(guard);
                    self.unify(&TyKind::Bool, &guard_ty, guard.span);
                }
                let first_ty = self.check_expr(&arms[0].body);
                for arm in &arms[1..] {
                    self.bind_pattern(&arm.pat, scr_ty.clone());
                    if let Some(guard) = &arm.guard {
                        let guard_ty = self.check_expr(guard);
                        self.unify(&TyKind::Bool, &guard_ty, guard.span);
                    }
                    let arm_ty = self.check_expr(&arm.body);
                    self.unify(&first_ty, &arm_ty, arm.span);
                }
                first_ty
            }

            HirExprKind::Assign(lhs, rhs) => {
                let lty = self.check_expr(lhs);
                let rty = self.check_expr(rhs);
                self.unify(&lty, &rty, expr.span);
                TyKind::Unit
            }

            HirExprKind::AssignOp(op, lhs, rhs) => {
                let raw_lty = self.check_expr(lhs);
                let raw_rty = self.check_expr(rhs);
                let lty = self.binary_operand_ty(raw_lty);
                let rty = self.binary_operand_ty(raw_rty);
                if self.generic_operator_output(*op, &lty, &rty).is_some()
                    || self.lookup_operator_impl(*op, &lty, &rty).is_some()
                    || matches!(self.shallow_resolve(lty.clone()), TyKind::Adt(_, _))
                {
                    return TyKind::Unit;
                }
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
                    TyKind::Adt(def_id, substs) if self.is_box_def(def_id) && substs.len() == 1 => {
                        substs[0].clone()
                    }
                    TyKind::Adt(def_id, substs) => {
                        let adt_ty = TyKind::Adt(def_id, substs);
                        if self.is_string_def(def_id)
                            || self.describe_ty(&adt_ty).rsplit("::").next() == Some("String")
                        {
                            TyKind::Str
                        } else {
                            self.lookup_deref_target(&adt_ty).unwrap_or_else(|| {
                                self.error(expr.span, &format!(
                                    "cannot dereference this type: {}",
                                    self.describe_ty(&adt_ty)
                                ));
                                TyKind::Error
                            })
                        }
                    }
                    other => {
                        if Self::is_deferred_ty(&other) {
                            self.fresh_infer(InferKind::General)
                        } else {
                            self.error(expr.span, &format!("cannot dereference this type: {}", self.describe_ty(&other)));
                            TyKind::Error
                        }
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
                let idx_ty_raw = self.check_expr(idx);
                let idx_ty = self.shallow_resolve(idx_ty_raw);
                let resolved = self.index_target_ty(base_ty);
                let is_range = matches!(idx.kind, HirExprKind::Range(_, _, _));
                let resolved_desc = self.describe_ty(&resolved);
                match resolved {
                    TyKind::Array(elem, _) | TyKind::Slice(elem) => {
                        self.check_builtin_index_ty(&idx_ty, idx.span, is_range);
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
                            self.error(expr.span, &format!("cannot index this type: {}", resolved_desc));
                            TyKind::Error
                        }
                    }
                    TyKind::Adt(def_id, substs) => {
                        if self.is_vec_def(def_id) && substs.len() == 1 {
                            self.check_builtin_index_ty(&idx_ty, idx.span, is_range);
                            let elem_ty = self.resolve_ty_full(substs[0].clone());
                            if is_range {
                                TyKind::Slice(Box::new(elem_ty))
                            } else {
                                elem_ty
                            }
                        } else if self.is_string_def(def_id) {
                            if is_range {
                                TyKind::Str
                            } else {
                                self.error(expr.span, &format!("cannot index this type: {}", resolved_desc));
                                TyKind::Error
                            }
                        } else if !is_range {
                            if let Some(output_ty) = self.lookup_index_output(def_id, &idx_ty) {
                                output_ty
                            } else if let Some(target_ty) =
                                self.lookup_deref_target(&TyKind::Adt(def_id, substs.clone()))
                            {
                                self.index_builtin_target(
                                    target_ty,
                                    &idx_ty,
                                    is_range,
                                    idx.span,
                                    expr.span,
                                )
                                .unwrap_or_else(|| {
                                    self.error(expr.span, &format!("cannot index this type: {}", resolved_desc));
                                    TyKind::Error
                                })
                            } else {
                                self.error(expr.span, &format!("cannot index this type: {}", resolved_desc));
                                TyKind::Error
                            }
                        } else {
                            self.lookup_deref_target(&TyKind::Adt(def_id, substs.clone()))
                                .and_then(|target_ty| {
                                    self.index_builtin_target(
                                        target_ty,
                                        &idx_ty,
                                        is_range,
                                        idx.span,
                                        expr.span,
                                    )
                                })
                                .unwrap_or_else(|| {
                                    self.error(expr.span, &format!("cannot index this type: {}", resolved_desc));
                                    TyKind::Error
                                })
                        }
                    }
                    _ => {
                        if Self::is_deferred_ty(&resolved) {
                            self.fresh_infer(InferKind::General)
                        } else {
                            self.error(expr.span, &format!("cannot index this type: {}", resolved_desc));
                            TyKind::Error
                        }
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
                let method_str = self.interner.resolve(*method_name);
                let defer_closure_args = matches!(
                    method_str,
                    "sort_by" | "find" | "position" | "any" | "all" | "map" | "filter" | "filter_map" | "retain"
                ) && matches!(args.first().map(|arg| &arg.kind), Some(HirExprKind::Closure(..)));
                if !defer_closure_args {
                    for a in args { self.check_expr(a); }
                }

                // Resolve the receiver type to find the method
                let base_ty = self.shallow_resolve(recv_ty.clone());
                // Unwrap references to get the underlying type name
                let inner_ty = match &base_ty {
                    TyKind::Ref(inner, _) => self.shallow_resolve(inner.as_ref().clone()),
                    other => other.clone(),
                };

                if method_str == "?" && args.is_empty() {
                    if let Some(inner_option_ty) = self.option_inner_ty(&inner_ty) {
                        return inner_option_ty;
                    }
                    if let TyKind::Adt(def_id, substs) = &inner_ty {
                        if self.is_result_def(*def_id) && substs.len() == 2 {
                            return substs[0].clone();
                        }
                    }
                    return self.fresh_infer(InferKind::General);
                }

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

                if matches!(method_str, "to_le_bytes" | "to_be_bytes" | "to_ne_bytes") && args.is_empty() {
                    if let Some(width) = self.primitive_byte_width(&inner_ty) {
                        return TyKind::Array(Box::new(TyKind::Uint(UintTy::U8)), width);
                    }
                }

                if Self::is_integer_ty(&inner_ty) {
                    match method_str {
                        "wrapping_add" | "wrapping_sub" | "wrapping_mul" | "wrapping_div"
                        | "wrapping_rem" | "saturating_add" | "saturating_sub" | "saturating_mul"
                        | "unchecked_add" | "unchecked_sub" | "unchecked_mul" if args.len() == 1 => {
                            return inner_ty.clone();
                        }
                        "checked_add" | "checked_sub" | "checked_mul" | "checked_div"
                        | "checked_rem" if args.len() == 1 => {
                            return self.option_of(inner_ty.clone())
                                .unwrap_or_else(|| self.fresh_infer(InferKind::General));
                        }
                        "is_power_of_two" | "is_ascii" if args.is_empty() => return TyKind::Bool,
                        _ => {}
                    }
                }

                if method_str == "next" && args.is_empty() {
                    if let TyKind::Slice(elem) = &inner_ty {
                        return self.option_of(elem.as_ref().clone())
                            .unwrap_or_else(|| self.fresh_infer(InferKind::General));
                    }
                }

                if inner_is_string_like {
                    match method_str {
                        "contains" | "starts_with" | "ends_with" if args.len() == 1 => {
                            return TyKind::Bool;
                        }
                        "find" | "rfind" if args.len() == 1 => {
                            if let Some(ty) = self.option_of(TyKind::Uint(UintTy::Usize)) {
                                return ty;
                            }
                            return self.fresh_infer(InferKind::General);
                        }
                        "trim" | "trim_start" | "trim_end" if args.is_empty() => {
                            return TyKind::Ref(Box::new(TyKind::Str), Mutability::Immutable);
                        }
                        "trim_matches" | "trim_start_matches" | "trim_end_matches" if args.len() == 1 => {
                            return TyKind::Ref(Box::new(TyKind::Str), Mutability::Immutable);
                        }
                        "to_string" | "to_owned" if args.is_empty() => {
                            return self.string_ty().unwrap_or_else(|| self.fresh_infer(InferKind::General));
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
                        "iter" if args.is_empty() => {
                            return TyKind::Slice(Box::new(TyKind::Ref(
                                Box::new(elem.as_ref().clone()),
                                Mutability::Immutable,
                            )));
                        }
                        "iter_mut" if args.is_empty() => {
                            return TyKind::Slice(Box::new(TyKind::Ref(
                                Box::new(elem.as_ref().clone()),
                                Mutability::Mut,
                            )));
                        }
                        "enumerate" if args.is_empty() => {
                            return TyKind::Slice(Box::new(TyKind::Tuple(vec![
                                TyKind::Uint(UintTy::Usize),
                                elem.as_ref().clone(),
                            ])));
                        }
                        "copy_from_slice" if args.len() == 1 => return TyKind::Unit,
                        "to_vec" if args.is_empty() => {
                            if let Some(vec_sym) = self.interner.lookup("Vec") {
                                if let Some(&vec_def_id) = self.type_name_to_def.get(&vec_sym) {
                                    return TyKind::Adt(
                                        vec_def_id,
                                        vec![self.resolve_ty_full(elem.as_ref().clone())],
                                    );
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

                if let TyKind::Adt(def_id, substs) = &inner_ty {
                    if self.is_nonzero_usize_def(*def_id) {
                        match method_str {
                            "get" if args.is_empty() => return TyKind::Uint(UintTy::Usize),
                            "is_power_of_two" if args.is_empty() => return TyKind::Bool,
                            _ => {}
                        }
                    }
                    if self.is_vec_def(*def_id) && substs.len() == 1 {
                        let elem_ty = self.resolve_ty_full(substs[0].clone());
                        match method_str {
                            "iter" if args.is_empty() => {
                                return TyKind::Slice(Box::new(TyKind::Ref(
                                    Box::new(elem_ty.clone()),
                                    Mutability::Immutable,
                                )));
                            }
                            "iter_mut" if args.is_empty() => {
                                return TyKind::Slice(Box::new(TyKind::Ref(
                                    Box::new(elem_ty.clone()),
                                    Mutability::Mut,
                                )));
                            }
                            _ => {}
                        }
                    }
                }

                if method_str == "sort_by" && args.len() == 1 {
                    let elem_ty = match &inner_ty {
                        TyKind::Slice(elem) | TyKind::Array(elem, _) => Some(elem.as_ref().clone()),
                        TyKind::Adt(_, substs) if substs.len() == 1 => Some(substs[0].clone()),
                        _ => None,
                    };
                    if let Some(elem_ty) = elem_ty {
                        if let HirExprKind::Closure(params, ret_ty, body, _) = &args[0].kind {
                            if params.len() == 2 {
                                let cmp_arg_ty = TyKind::Ref(Box::new(elem_ty), Mutability::Immutable);
                                for (param, expected_ty) in params.iter().zip([cmp_arg_ty.clone(), cmp_arg_ty]) {
                                    self.bind_closure_param(param, expected_ty);
                                }

                                let ordering_ty = self.comparison_ordering_ty();
                                self.check_closure_body_return(
                                    ret_ty.as_deref(),
                                    Some(ordering_ty),
                                    body,
                                    expr.span,
                                );
                                return TyKind::Unit;
                            }
                        }
                    }
                }

                if let Some(item_ty) = self.iter_item_ty(&inner_ty) {
                    match method_str {
                        "find" | "position" | "any" | "all" if args.len() == 1 => {
                            if let HirExprKind::Closure(params, ret_ty, body, _) = &args[0].kind {
                                if params.len() == 1 {
                                    self.bind_closure_param(&params[0], item_ty.clone());
                                    self.check_closure_body_return(
                                        ret_ty.as_deref(),
                                        Some(TyKind::Bool),
                                        body,
                                        expr.span,
                                    );
                                    return match method_str {
                                        "find" => self.option_of(item_ty).unwrap_or_else(|| self.fresh_infer(InferKind::General)),
                                        "position" => self.option_of(TyKind::Uint(UintTy::Usize)).unwrap_or_else(|| self.fresh_infer(InferKind::General)),
                                        _ => TyKind::Bool,
                                    };
                                }
                            }
                        }
                        "map" if args.len() == 1 => {
                            if let HirExprKind::Closure(params, ret_ty, body, _) = &args[0].kind {
                                if params.len() == 1 {
                                    self.bind_closure_param(&params[0], item_ty.clone());
                                    let body_ty = self.check_closure_body_return(
                                        ret_ty.as_deref(),
                                        None,
                                        body,
                                        expr.span,
                                    );
                                    return TyKind::Slice(Box::new(body_ty));
                                }
                            }
                            let mapper_ty = self.get_expr_ty_cached(&args[0]);
                            let mapped_ty = self.callable_output_ty(&mapper_ty)
                                .unwrap_or_else(|| mapper_ty.clone());
                            return TyKind::Slice(Box::new(mapped_ty));
                        }
                        "filter" if args.len() == 1 => {
                            if let HirExprKind::Closure(params, ret_ty, body, _) = &args[0].kind {
                                if params.len() == 1 {
                                    self.bind_closure_param(&params[0], item_ty.clone());
                                    self.check_closure_body_return(
                                        ret_ty.as_deref(),
                                        Some(TyKind::Bool),
                                        body,
                                        expr.span,
                                    );
                                }
                            }
                            return TyKind::Slice(Box::new(item_ty));
                        }
                        "filter_map" if args.len() == 1 => {
                            if let HirExprKind::Closure(params, ret_ty, body, _) = &args[0].kind {
                                if params.len() == 1 {
                                    self.bind_closure_param(&params[0], item_ty.clone());
                                    let body_ty = self.check_closure_body_return(
                                        ret_ty.as_deref(),
                                        None,
                                        body,
                                        expr.span,
                                    );
                                    if let Some(mapped_ty) = self.option_inner_ty(&body_ty) {
                                        return TyKind::Slice(Box::new(mapped_ty));
                                    }
                                    return TyKind::Slice(Box::new(self.fresh_infer(InferKind::General)));
                                }
                            }
                        }
                        _ => {}
                    }
                }

                if let Some(inner_option_ty) = self.option_inner_ty(&inner_ty) {
                    match method_str {
                        "is_some" | "is_none" if args.is_empty() => return TyKind::Bool,
                        "as_ref" if args.is_empty() => {
                            if let Some(ty) = self.option_of(TyKind::Ref(
                                Box::new(inner_option_ty.clone()),
                                Mutability::Immutable,
                            )) {
                                return ty;
                            }
                            return self.fresh_infer(InferKind::General);
                        }
                        "as_mut" if args.is_empty() => {
                            if let Some(ty) = self.option_of(TyKind::Ref(
                                Box::new(inner_option_ty.clone()),
                                Mutability::Mut,
                            )) {
                                return ty;
                            }
                            return self.fresh_infer(InferKind::General);
                        }
                        "unwrap" if args.is_empty() => return inner_option_ty.clone(),
                        "expect" if args.len() == 1 => return inner_option_ty.clone(),
                        "unwrap_or" if args.len() == 1 => {
                            let default_ty = self.get_expr_ty_cached(&args[0]);
                            self.unify(&inner_option_ty, &default_ty, args[0].span);
                            return inner_option_ty.clone();
                        }
                        _ => {}
                    }
                    if method_str == "map" && args.len() == 1 {
                        if let HirExprKind::Closure(params, _, body, _) = &args[0].kind {
                            if params.len() == 1 {
                                self.bind_closure_param(&params[0], inner_option_ty);
                                let body_ty =
                                    self.check_closure_body_return(None, None, body, expr.span);
                                return self.option_of(body_ty)
                                    .unwrap_or_else(|| self.fresh_infer(InferKind::General));
                            }
                        }
                    }
                }

                if method_str == "try_into" && args.is_empty() {
                    let array_len = match &recv.kind {
                        HirExprKind::Index(_, idx) => self.const_range_len(idx),
                        _ => match &inner_ty {
                            TyKind::Array(_, n) => Some(*n),
                            _ => None,
                        },
                    };
                    let elem_ty = match &inner_ty {
                        TyKind::Slice(elem) | TyKind::Array(elem, _) => Some(elem.as_ref().clone()),
                        _ => None,
                    };
                    if let (Some(len), Some(elem_ty)) = (array_len, elem_ty) {
                        if let Some(result_ty) =
                            self.result_of(TyKind::Array(Box::new(elem_ty), len))
                        {
                            return result_ty;
                        }
                        return self.fresh_infer(InferKind::General);
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
                        let elem_ty = self.resolve_ty_full(substs[0].clone());
                        match method_str {
                            "len" if args.is_empty() => return TyKind::Uint(UintTy::Usize),
                            "is_empty" if args.is_empty() => return TyKind::Bool,
                            "retain" if args.len() == 1 => {
                                if let HirExprKind::Closure(params, ret_ty, body, _) = &args[0].kind {
                                    if params.len() == 1 {
                                        self.bind_closure_param(
                                            &params[0],
                                            TyKind::Ref(Box::new(elem_ty.clone()), Mutability::Immutable),
                                        );
                                        self.check_closure_body_return(
                                            ret_ty.as_deref(),
                                            Some(TyKind::Bool),
                                            body,
                                            expr.span,
                                        );
                                        return TyKind::Unit;
                                    }
                                }
                            }
                            "as_slice" if args.is_empty() => {
                                return TyKind::Ref(
                                    Box::new(TyKind::Slice(Box::new(elem_ty))),
                                    Mutability::Immutable,
                                );
                            }
                            _ => {}
                        }
                    }
                    if self.is_result_def(*def_id) && substs.len() == 2 {
                        match method_str {
                            "is_ok" | "is_err" if args.is_empty() => return TyKind::Bool,
                            "ok" if args.is_empty() => {
                                if let Some(ty) = self.option_of(substs[0].clone()) {
                                    return ty;
                                }
                                return self.fresh_infer(InferKind::General);
                            }
                            "err" if args.is_empty() => {
                                if let Some(ty) = self.option_of(substs[1].clone()) {
                                    return ty;
                                }
                                return self.fresh_infer(InferKind::General);
                            }
                            "unwrap" if args.is_empty() => return substs[0].clone(),
                            "expect" if args.len() == 1 => return substs[0].clone(),
                            "unwrap_or" if args.len() == 1 => {
                                let default_ty = self.get_expr_ty_cached(&args[0]);
                                self.unify(&substs[0], &default_ty, args[0].span);
                                return substs[0].clone();
                            }
                            _ => {}
                        }
                    }
                }

                // Handle dyn Trait method calls
                if let TyKind::DynTrait(trait_def_id) = &inner_ty {
                    if let Some(methods) = self.trait_methods.get(trait_def_id).cloned() {
                if let Some((_, method_def_id)) = methods.iter().find(|(n, _)| *n == *method_name) {
                    if let Some((param_tys, ret_ty)) = self.fn_sigs.get(method_def_id).cloned() {
                        let param_tys = param_tys
                            .iter()
                            .map(|ty| self.substitute_trait_self(ty, *trait_def_id, &inner_ty))
                            .collect::<Vec<_>>();
                        let ret_ty = self.substitute_trait_self(&ret_ty, *trait_def_id, &inner_ty);
                        let user_params = if !param_tys.is_empty() { &param_tys[1..] } else { &param_tys[..] };
                        if args.len() != user_params.len() {
                            self.error(expr.span, &format!(
                                "wrong number of arguments for method {}: expected {}, found {}",
                                        self.interner.resolve(*method_name),
                                        user_params.len(),
                                        args.len(),
                                    ));
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
                                                "wrong number of arguments for method {}: expected {}, found {}",
                                                self.interner.resolve(*method_name),
                                                user_params.len(),
                                                args.len(),
                                            ));
                                        }
                                        return ret_ty;
                                    }
                                }
                            }
                        }
                    }
                }

                if let TyKind::Adt(def_id, _) = &inner_ty {
                    if let Some(methods) = self.impl_methods_by_type.get(def_id).cloned() {
                        let mut fallback: Option<(Vec<TyKind>, TyKind)> = None;
                        for (_, method_def_id) in methods.iter().filter(|(n, _)| *n == *method_name) {
                            if let Some((param_tys, ret_ty)) = self.fn_sigs.get(method_def_id).cloned() {
                                let (param_tys, ret_ty) = if let Some(self_param_ty) = param_tys.first() {
                                    let impl_substs = self
                                        .infer_receiver_substs(self_param_ty, &base_ty)
                                        .or_else(|| self.infer_receiver_substs(self_param_ty, &inner_ty));
                                    if let Some(impl_substs) = impl_substs {
                                        (
                                            param_tys
                                                .iter()
                                                .map(|t| self.substitute_params(t, &impl_substs))
                                                .collect(),
                                            self.substitute_params(&ret_ty, &impl_substs),
                                        )
                                    } else {
                                        (param_tys, ret_ty)
                                    }
                                } else {
                                    (param_tys, ret_ty)
                                };
                                let user_params = if !param_tys.is_empty() { &param_tys[1..] } else { &param_tys[..] };
                                if fallback.is_none() {
                                    fallback = Some((param_tys.clone(), ret_ty.clone()));
                                }
                                if args.len() == user_params.len()
                                    && args.iter().zip(user_params.iter()).all(|(arg, pty)| {
                                        let aty = self.get_expr_ty_cached(arg);
                                        self.ty_matches_for_candidate(pty, &aty)
                                    })
                                {
                                    for (arg, pty) in args.iter().zip(user_params.iter()) {
                                        let aty = self.get_expr_ty_cached(arg);
                                        self.unify(pty, &aty, arg.span);
                                    }
                                    return ret_ty;
                                }
                            }
                        }
                        if let Some((param_tys, ret_ty)) = fallback {
                            let user_params = if !param_tys.is_empty() { &param_tys[1..] } else { &param_tys[..] };
                            if args.len() != user_params.len() {
                                self.error(expr.span, &format!(
                                    "wrong number of arguments for method {}: expected {}, found {}",
                                    self.interner.resolve(*method_name),
                                    user_params.len(),
                                    args.len(),
                                ));
                            } else {
                                for (arg, pty) in args.iter().zip(user_params.iter()) {
                                    let aty = self.get_expr_ty_cached(arg);
                                    self.unify(pty, &aty, arg.span);
                                }
                            }
                            return ret_ty;
                        }
                    }

                    // Find the type name for this DefId, skipping "Self" alias
                    let type_name = self.type_name_to_def.iter()
                        .find(|(sym, &did)| did == *def_id && self.interner.resolve(**sym) != "Self")
                        .map(|(sym, _)| *sym);

                    if let Some(type_name) = type_name {
                        // Look up impl methods
                        if let Some(methods) = self.resolve.impl_methods.get(&type_name) {
                            if let Some((_, method_def_id)) = methods.iter().find(|(n, _)| *n == *method_name) {
                                if let Some((param_tys, ret_ty)) = self.fn_sigs.get(method_def_id).cloned() {
                                    let (param_tys, ret_ty) = if let Some(self_param_ty) = param_tys.first() {
                                        let impl_substs = self
                                            .infer_receiver_substs(self_param_ty, &base_ty)
                                            .or_else(|| self.infer_receiver_substs(self_param_ty, &inner_ty));
                                        if let Some(impl_substs) = impl_substs {
                                            (
                                                param_tys
                                                    .iter()
                                                    .map(|t| self.substitute_params(t, &impl_substs))
                                                    .collect(),
                                                self.substitute_params(&ret_ty, &impl_substs),
                                            )
                                        } else {
                                            (param_tys, ret_ty)
                                        }
                                    } else {
                                        (param_tys, ret_ty)
                                    };
                                    let n_generics = self.generic_fn_defs.get(method_def_id).copied().unwrap_or(0);
                                    let (param_tys, ret_ty) = if n_generics > 0 {
                                        let infer_vars: Vec<TyKind> = (0..n_generics)
                                            .map(|_| self.fresh_infer(InferKind::General))
                                            .collect();
                                        (
                                            param_tys.iter()
                                                .map(|t| self.substitute_params(t, &infer_vars))
                                                .collect(),
                                            self.substitute_params(&ret_ty, &infer_vars),
                                        )
                                    } else {
                                        (param_tys, ret_ty)
                                    };
                                    // Check argument types (skip &self parameter)
                                    let user_params = if !param_tys.is_empty() { &param_tys[1..] } else { &param_tys[..] };
                                    if args.len() != user_params.len() {
                                        self.error(expr.span, &format!(
                                            "wrong number of arguments for method {}: expected {}, found {}",
                                            self.interner.resolve(*method_name),
                                            user_params.len(),
                                            args.len(),
                                        ));
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

                if let Some((method_def_id, param_tys, ret_ty)) =
                    self.resolve_impl_method_by_receiver(*method_name, &base_ty, &inner_ty)
                {
                    let n_generics = self.generic_fn_defs.get(&method_def_id).copied().unwrap_or(0);
                    let (param_tys, ret_ty) = if n_generics > 0 {
                        let infer_vars: Vec<TyKind> = (0..n_generics)
                            .map(|_| self.fresh_infer(InferKind::General))
                            .collect();
                        (
                            param_tys
                                .iter()
                                .map(|t| self.substitute_params(t, &infer_vars))
                                .collect(),
                            self.substitute_params(&ret_ty, &infer_vars),
                        )
                    } else {
                        (param_tys, ret_ty)
                    };
                    let user_params = if !param_tys.is_empty() { &param_tys[1..] } else { &param_tys[..] };
                    if args.len() != user_params.len() {
                        self.error(expr.span, &format!(
                            "wrong number of arguments for method {}: expected {}, found {}",
                            self.interner.resolve(*method_name),
                            user_params.len(),
                            args.len(),
                        ));
                    } else {
                        for (arg, pty) in args.iter().zip(user_params.iter()) {
                            let aty = self.get_expr_ty_cached(arg);
                            self.unify(pty, &aty, arg.span);
                        }
                    }
                    return ret_ty;
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

                let ret = self.check_closure_body_return(ret_ty.as_deref(), None, body, expr.span);

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
                        crate::hir::HirAsmOperand::InOut { expr, out_expr, .. } => {
                            self.check_expr(expr);
                            if let Some(out_expr) = out_expr {
                                self.check_expr(out_expr);
                            }
                        }
                    }
                }
                TyKind::Unit
            }
        }
    }

    // ── Path type inference ──

    fn infer_path_type(&mut self, path: &HirPath, expr_id: HirId) -> TyKind {
        if let Some(ty) = self.generic_param_assoc_const_type(path) {
            return ty;
        }

        if let Some(&def_id) = self.resolve.resolutions.get(&expr_id) {
            // Intrinsic function from core/alloc, or primitive associated constant?
            if let Some(path_str) = self.resolve.intrinsic_fns.get(&def_id) {
                if let Some(ty) = Self::primitive_assoc_const_type(path_str) {
                    return ty;
                }
                if let Some(ty) = self.intrinsic_constructor_type(path_str) {
                    return ty;
                }
                if let Some(ty) = self.intrinsic_enum_variant_type(path_str) {
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
                return self.fresh_adt_ty(def_id, vec![]);
            }
            if let Some(ty) = self.enum_variant_type_from_path(path) {
                return ty;
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

    fn generic_param_assoc_const_type(&self, path: &HirPath) -> Option<TyKind> {
        if path.segments.len() != 2 {
            return None;
        }
        let param = path.segments[0].ident;
        let assoc_name = path.segments[1].ident;
        let param_idx = *self.current_generic_params.get(&param)?;
        self.current_generic_bounds
            .get(&param_idx)?
            .iter()
            .find_map(|trait_def_id| {
                self.trait_assoc_consts
                    .get(trait_def_id)?
                    .iter()
                    .find_map(|(name, ty)| (*name == assoc_name).then(|| ty.clone()))
            })
    }

    fn infer_qualified_path_type(&mut self, qpath: &HirQualifiedPath, expr_id: HirId) -> TyKind {
        if let Some(&def_id) = self.resolve.resolutions.get(&expr_id) {
            if let Some(path_str) = self.resolve.intrinsic_fns.get(&def_id) {
                if let Some(ty) = Self::primitive_assoc_const_type(path_str) {
                    return ty;
                }
                if let Some(ty) = self.intrinsic_constructor_type(path_str) {
                    return ty;
                }
                return TyKind::FnDef(def_id, vec![]);
            }
            if self.fn_sigs.contains_key(&def_id) {
                return TyKind::FnDef(def_id, vec![]);
            }
            if let Some((_, ty)) = self.const_values.get(&def_id) {
                return ty.clone();
            }
            if let Some((_, ty, _, _)) = self.static_defs.get(&def_id) {
                return ty.clone();
            }
        }

        if qpath.path.segments.is_empty() {
            return TyKind::Error;
        }
        self.qualified_path_to_ty(qpath)
    }

    fn trait_def_from_path(&self, path: &HirPath) -> Option<DefId> {
        self.resolve_trait_bound_def(path)
    }

    fn qualified_path_to_ty(&self, qpath: &HirQualifiedPath) -> TyKind {
        let Some(assoc_segment) = qpath.path.segments.last() else {
            return TyKind::Error;
        };
        let self_ty = self.hir_ty_to_ty(&qpath.self_ty);
        let trait_def_id = qpath
            .trait_path
            .as_ref()
            .and_then(|path| self.trait_def_from_path(path));

        self.resolve_generic_assoc_binding(TyKind::Projection(
            Box::new(self_ty),
            trait_def_id,
            assoc_segment.ident,
        ))
    }

    fn projection_for_param_assoc(&self, param_idx: u32, assoc_name: Symbol) -> TyKind {
        let trait_def_id = self
            .current_generic_bounds
            .get(&param_idx)
            .and_then(|bounds| {
                bounds.iter().copied().find(|trait_def_id| {
                    self.trait_assoc_types
                        .get(trait_def_id)
                        .is_some_and(|names| names.contains(&assoc_name))
                })
            });
        self.resolve_generic_assoc_binding(TyKind::Projection(
            Box::new(TyKind::Param(param_idx)),
            trait_def_id,
            assoc_name,
        ))
    }

    fn resolve_qualified_type_path(&self, path: &str) -> Option<DefId> {
        if let Some((first, rest)) = path.split_once("::") {
            if let Some(alias_target) = self.module_aliases.get(first) {
                let expanded = format!("{}::{}", alias_target, rest);
                if let Some(def_id) = self.qualified_type_names
                    .get(&expanded)
                    .or_else(|| self.qualified_type_names.get(&format!("crate::{}", expanded)))
                    .copied()
                {
                    return Some(def_id);
                }
            }
        }

        self.qualified_type_names.get(path).copied()
    }

    fn resolve_scoped_type_name(&self, name: Symbol) -> Option<DefId> {
        let name_str = self.interner.resolve(name);
        let module_key = self.module_key_from_symbols(&self.current_module_path);
        if let Some(target) = self.scoped_type_aliases.get(&(module_key, name)) {
            if let Some(def_id) = self
                .qualified_type_names
                .get(target)
                .or_else(|| self.qualified_type_names.get(&format!("crate::{}", target)))
                .copied()
            {
                return Some(def_id);
            }
        }
        if self.current_module_path.is_empty() {
            return self.qualified_type_names
                .get(name_str)
                .copied()
                .or_else(|| self.qualified_type_names.get(&format!("crate::{}", name_str)).copied())
                .or_else(|| self.type_name_to_def.get(&name).copied());
        }

        let mut parts = self.current_module_path
            .iter()
            .map(|sym| self.interner.resolve(*sym).to_string())
            .collect::<Vec<_>>();
        parts.push(name_str.to_string());
        let path = parts.join("::");
        self.qualified_type_names
            .get(&path)
            .copied()
            .or_else(|| self.type_name_to_def.get(&name).copied())
    }

    fn resolve_scoped_type_alias(&self, name: Symbol) -> Option<TyKind> {
        let name_str = self.interner.resolve(name);
        if !self.current_module_path.is_empty() {
            let mut parts = self.current_module_path
                .iter()
                .map(|sym| self.interner.resolve(*sym).to_string())
                .collect::<Vec<_>>();
            parts.push(name_str.to_string());
            let path = parts.join("::");
            if let Some(ty) = self.qualified_type_aliases.get(&path) {
                return Some(ty.clone());
            }
        }
        self.type_aliases.get(&name).cloned()
    }

    fn resolve_qualified_type_alias(&self, path: &str) -> Option<TyKind> {
        if let Some((first, rest)) = path.split_once("::") {
            let module_key = self.module_key_from_symbols(&self.current_module_path);
            if let Some(first_sym) = self.interner.lookup(first) {
                if let Some(alias_target) = self.scoped_type_aliases.get(&(module_key, first_sym)) {
                    let expanded = format!("{}::{}", alias_target, rest);
                    if let Some(ty) = self.qualified_type_aliases.get(&expanded) {
                        return Some(ty.clone());
                    }
                }
            }
            if let Some(alias_target) = self.module_aliases.get(first) {
                let expanded = format!("{}::{}", alias_target, rest);
                if let Some(ty) = self.qualified_type_aliases.get(&expanded) {
                    return Some(ty.clone());
                }
            }
        }
        self.qualified_type_aliases
            .get(path)
            .or_else(|| self.qualified_type_aliases.get(path.strip_prefix("crate::").unwrap_or(path)))
            .cloned()
    }

    fn expanded_type_path(&self, path: &str) -> String {
        let Some((first, rest)) = path.split_once("::") else {
            return path.to_string();
        };

        let module_key = self.module_key_from_symbols(&self.current_module_path);
        if let Some(first_sym) = self.interner.lookup(first) {
            if let Some(alias_target) = self.scoped_type_aliases.get(&(module_key, first_sym)) {
                return format!("{}::{}", alias_target, rest);
            }
        }

        if let Some(alias_target) = self.module_aliases.get(first) {
            return format!("{}::{}", alias_target, rest);
        }

        path.to_string()
    }

    fn std_slice_iter_ty(&self, path: &str, segment: &HirPathSegment) -> Option<TyKind> {
        let expanded = self.expanded_type_path(path);
        let mutability = match expanded.as_str() {
            "core::slice::Iter" | "alloc::slice::Iter" | "std::slice::Iter" | "slice::Iter" => {
                Mutability::Immutable
            }
            "core::slice::IterMut"
            | "alloc::slice::IterMut"
            | "std::slice::IterMut"
            | "slice::IterMut" => Mutability::Mut,
            _ => return None,
        };
        let elem_ty = self
            .type_args_from_segment(segment)
            .into_iter()
            .last()
            .unwrap_or_else(|| TyKind::Error);
        Some(TyKind::Slice(Box::new(TyKind::Ref(
            Box::new(elem_ty),
            mutability,
        ))))
    }

    fn assoc_fn_candidates_on_path(&self, path: &HirPath) -> Option<Vec<DefId>> {
        if path.segments.len() < 2 {
            return None;
        }
        let method_name = path.segments.last()?.ident;
        let self_ty = self.path_segments_to_ty(&path.segments[..path.segments.len() - 1]);
        let TyKind::Adt(def_id, _) = self.shallow_resolve(self_ty) else {
            return None;
        };
        let candidates = self.impl_methods_by_type
            .get(&def_id)?
            .iter()
            .filter_map(|(name, method_def_id)| (*name == method_name).then_some(*method_def_id))
            .collect::<Vec<_>>();
        if candidates.is_empty() { None } else { Some(candidates) }
    }

    fn check_assoc_fn_call_on_path(
        &mut self,
        path: &HirPath,
        args: &[HirExpr],
        span: Span,
        callee: &HirExpr,
    ) -> Option<TyKind> {
        let method_name = path.segments.last()?.ident;
        let method_name_str = self.interner.resolve(method_name);
        if let Some(def_id) = self.generic_param_assoc_fn_on_path(path) {
            let arg_tys = args.iter().map(|arg| self.check_expr(arg)).collect::<Vec<_>>();
            let Some((param_tys, ret_ty)) = self.instantiate_fn_sig(def_id) else {
                return None;
            };
            if param_tys.len() == arg_tys.len() {
                for (arg, param) in arg_tys.iter().zip(param_tys.iter()) {
                    self.unify(param, arg, span);
                }
                self.expr_types.insert(callee.id, TyKind::FnDef(def_id, vec![]));
                return Some(ret_ty);
            }
        }

        let candidates = self.assoc_fn_candidates_on_path(path).unwrap_or_default();
        if !candidates.is_empty() {
            let arg_tys = args.iter().map(|arg| self.check_expr(arg)).collect::<Vec<_>>();
            let parent_ty = self.assoc_parent_ty_for_path(path);
            let mut fallback: Option<(DefId, Vec<TyKind>, TyKind)> = None;
            for def_id in candidates {
                let Some((param_tys, ret_ty)) = parent_ty
                    .as_ref()
                    .and_then(|parent_ty| self.instantiate_assoc_fn_sig(def_id, parent_ty))
                    .or_else(|| self.instantiate_fn_sig(def_id))
                else {
                    continue;
                };
                if param_tys.len() != arg_tys.len() {
                    continue;
                }
                if fallback.is_none() {
                    fallback = Some((def_id, param_tys.clone(), ret_ty.clone()));
                }
                if param_tys
                    .iter()
                    .zip(arg_tys.iter())
                    .all(|(param, arg)| self.ty_matches_for_candidate(param, arg))
                {
                    for (arg, param) in arg_tys.iter().zip(param_tys.iter()) {
                        self.unify(param, arg, span);
                    }
                    self.expr_types.insert(callee.id, TyKind::FnDef(def_id, vec![]));
                    return Some(ret_ty);
                }
            }
            if method_name_str == "from" && arg_tys.len() == 1 {
                let target_ty = parent_ty
                    .clone()
                    .unwrap_or_else(|| self.path_segments_to_ty(&path.segments[..path.segments.len() - 1]));
                if self.ty_matches_for_candidate(&target_ty, &arg_tys[0]) {
                    return Some(target_ty);
                }
            }
            if let Some((_def_id, param_tys, ret_ty)) = fallback {
                if param_tys.len() == arg_tys.len() {
                    let has_ambiguous_arg = arg_tys.iter().any(|ty| {
                        matches!(
                            self.shallow_resolve(ty.clone()),
                            TyKind::Error | TyKind::Infer(_) | TyKind::Param(_)
                        )
                    });
                    if has_ambiguous_arg {
                        for (arg, param) in arg_tys.iter().zip(param_tys.iter()) {
                            self.unify(param, arg, span);
                        }
                        self.expr_types.insert(callee.id, TyKind::FnDef(_def_id, vec![]));
                        return Some(ret_ty);
                    }
                }
            }
        }

        if method_name_str == "from" && args.len() == 1 {
            let target_ty = self
                .assoc_parent_ty_for_path(path)
                .unwrap_or_else(|| self.path_segments_to_ty(&path.segments[..path.segments.len() - 1]));
            let arg_ty = self.check_expr(&args[0]);
            if self.ty_matches_for_candidate(&target_ty, &arg_ty) {
                return Some(target_ty);
            }
        }

        None
    }

    fn assoc_parent_ty_for_path(&mut self, path: &HirPath) -> Option<TyKind> {
        if path.segments.len() < 2 {
            return None;
        }
        let ty = self.path_segments_to_ty(&path.segments[..path.segments.len() - 1]);
        Some(match self.shallow_resolve(ty) {
            TyKind::Adt(def_id, substs) => self.fresh_adt_ty(def_id, substs),
            other => other,
        })
    }

    fn instantiate_assoc_fn_sig(
        &mut self,
        def_id: DefId,
        parent_ty: &TyKind,
    ) -> Option<(Vec<TyKind>, TyKind)> {
        let (param_tys, ret_ty) = self.instantiate_fn_sig(def_id)?;
        let Some(impl_self_ty) = self.impl_self_ty_by_method.get(&def_id).cloned() else {
            return Some((param_tys, ret_ty));
        };
        let Some(impl_substs) = self.infer_receiver_substs(&impl_self_ty, parent_ty) else {
            return Some((param_tys, ret_ty));
        };
        Some((
            param_tys
                .iter()
                .map(|ty| self.substitute_params(ty, &impl_substs))
                .collect(),
            self.substitute_params(&ret_ty, &impl_substs),
        ))
    }

    fn generic_param_assoc_fn_on_path(&self, path: &HirPath) -> Option<DefId> {
        if path.segments.len() != 2 {
            return None;
        }
        let param = path.segments[0].ident;
        let method_name = path.segments[1].ident;
        let param_idx = *self.current_generic_params.get(&param)?;
        self.current_generic_bounds
            .get(&param_idx)?
            .iter()
            .find_map(|trait_def_id| {
                self.trait_methods
                    .get(trait_def_id)?
                    .iter()
                    .find_map(|(name, def_id)| (*name == method_name).then_some(*def_id))
            })
    }

    fn check_assoc_fn_call_on_qualified_path(
        &mut self,
        qpath: &HirQualifiedPath,
        args: &[HirExpr],
        span: Span,
        callee: &HirExpr,
    ) -> Option<TyKind> {
        let trait_path = qpath.trait_path.as_ref()?;
        let trait_def_id = self.resolve_trait_bound_def(trait_path)?;
        let method_name = qpath.path.segments.last()?.ident;
        let method_def_id = self
            .trait_methods
            .get(&trait_def_id)?
            .iter()
            .find_map(|(name, def_id)| (*name == method_name).then_some(*def_id))?;
        let (param_tys, ret_ty) = self.fn_sigs.get(&method_def_id).cloned()?;
        let trait_substs = trait_path
            .segments
            .last()
            .and_then(|segment| segment.args.as_ref())
            .map(|args| {
                args.args
                    .iter()
                    .filter_map(|arg| match arg {
                        HirGenericArg::Type(ty) => Some(self.hir_ty_to_ty(ty)),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let self_ty = self.hir_ty_to_ty(&qpath.self_ty);
        let param_tys = param_tys
            .iter()
            .map(|ty| {
                let ty = self.substitute_params(ty, &trait_substs);
                self.substitute_trait_self(&ty, trait_def_id, &self_ty)
            })
            .collect::<Vec<_>>();
        let ret_ty = {
            let ty = self.substitute_params(&ret_ty, &trait_substs);
            self.substitute_trait_self(&ty, trait_def_id, &self_ty)
        };
        if param_tys.len() != args.len() {
            return None;
        }
        let arg_tys = args.iter().map(|arg| self.check_expr(arg)).collect::<Vec<_>>();
        for (arg, param) in arg_tys.iter().zip(param_tys.iter()) {
            self.unify(param, arg, span);
        }
        self.expr_types.insert(callee.id, TyKind::FnDef(method_def_id, vec![]));
        Some(ret_ty)
    }

    fn substitute_trait_self(&self, ty: &TyKind, trait_def_id: DefId, self_ty: &TyKind) -> TyKind {
        match ty {
            TyKind::Adt(def_id, substs) if *def_id == trait_def_id && substs.is_empty() => self_ty.clone(),
            TyKind::Ref(inner, m) => {
                TyKind::Ref(Box::new(self.substitute_trait_self(inner, trait_def_id, self_ty)), *m)
            }
            TyKind::RawPtr(inner, m) => {
                TyKind::RawPtr(Box::new(self.substitute_trait_self(inner, trait_def_id, self_ty)), *m)
            }
            TyKind::Tuple(tys) => TyKind::Tuple(
                tys.iter()
                    .map(|ty| self.substitute_trait_self(ty, trait_def_id, self_ty))
                    .collect(),
            ),
            TyKind::Array(inner, n) => {
                TyKind::Array(Box::new(self.substitute_trait_self(inner, trait_def_id, self_ty)), *n)
            }
            TyKind::Slice(inner) => {
                TyKind::Slice(Box::new(self.substitute_trait_self(inner, trait_def_id, self_ty)))
            }
            TyKind::FnDef(def_id, substs) => TyKind::FnDef(
                *def_id,
                substs
                    .iter()
                    .map(|ty| self.substitute_trait_self(ty, trait_def_id, self_ty))
                    .collect(),
            ),
            TyKind::FnPtr(params, ret) => TyKind::FnPtr(
                params
                    .iter()
                    .map(|ty| self.substitute_trait_self(ty, trait_def_id, self_ty))
                    .collect(),
                Box::new(self.substitute_trait_self(ret, trait_def_id, self_ty)),
            ),
            TyKind::Adt(def_id, substs) => TyKind::Adt(
                *def_id,
                substs
                    .iter()
                    .map(|ty| self.substitute_trait_self(ty, trait_def_id, self_ty))
                    .collect(),
            ),
            TyKind::Projection(inner, projection_trait, assoc_name) => {
                let projected = TyKind::Projection(
                    Box::new(self.substitute_trait_self(inner, trait_def_id, self_ty)),
                    *projection_trait,
                    *assoc_name,
                );
                self.resolve_generic_assoc_binding(projected)
            }
            _ => ty.clone(),
        }
    }

    fn resolve_generic_assoc_binding(&self, ty: TyKind) -> TyKind {
        let TyKind::Projection(self_ty, Some(trait_def_id), assoc_name) = &ty else {
            return ty;
        };
        let TyKind::Param(param_idx) = self.shallow_resolve(self_ty.as_ref().clone()) else {
            return ty;
        };
        self.current_generic_assoc_bindings
            .get(&(param_idx, *trait_def_id, *assoc_name))
            .cloned()
            .unwrap_or(ty)
    }

    fn instantiate_fn_sig(&mut self, def_id: DefId) -> Option<(Vec<TyKind>, TyKind)> {
        let (param_tys, ret_ty) = self.fn_sigs.get(&def_id).cloned()?;
        let indices = self
            .generic_fn_param_indices
            .get(&def_id)
            .cloned()
            .unwrap_or_default();
        if indices.is_empty() {
            return Some((param_tys, ret_ty));
        }
        let max_idx = indices.iter().copied().max().unwrap_or(0) as usize;
        let mut infer_vars = (0..=max_idx)
            .map(|idx| TyKind::Param(idx as u32))
            .collect::<Vec<_>>();
        for idx in indices {
            infer_vars[idx as usize] = self.fresh_infer(InferKind::General);
        }
        Some((
            param_tys
                .iter()
                .map(|ty| self.substitute_params(ty, &infer_vars))
                .collect(),
            self.substitute_params(&ret_ty, &infer_vars),
        ))
    }

    fn fresh_adt_ty(&mut self, def_id: DefId, substs: Vec<TyKind>) -> TyKind {
        if !substs.is_empty() {
            return TyKind::Adt(def_id, substs);
        }
        let count = self.adt_generic_counts.get(&def_id).copied().unwrap_or(0);
        let substs = (0..count)
            .map(|_| self.fresh_infer(InferKind::General))
            .collect();
        TyKind::Adt(def_id, substs)
    }

    fn ty_matches_for_candidate(&self, expected: &TyKind, actual: &TyKind) -> bool {
        let expected = self.shallow_resolve(expected.clone());
        let actual = self.shallow_resolve(actual.clone());
        match (&expected, &actual) {
            (TyKind::Error, _) | (_, TyKind::Error) => true,
            (TyKind::Infer(_), _) | (_, TyKind::Infer(_)) => true,
            (TyKind::Param(_), _) | (_, TyKind::Param(_)) => true,
            (TyKind::Adt(a, a_args), TyKind::Adt(b, b_args)) => {
                a == b
                    && a_args.len() == b_args.len()
                    && a_args
                        .iter()
                        .zip(b_args.iter())
                        .all(|(a, b)| self.ty_matches_for_candidate(a, b))
            }
            (TyKind::Ref(a, a_mut), TyKind::Ref(b, b_mut)) => {
                a_mut == b_mut && self.ty_matches_for_candidate(a, b)
            }
            (TyKind::RawPtr(a, a_mut), TyKind::RawPtr(b, b_mut)) => {
                a_mut == b_mut && self.ty_matches_for_candidate(a, b)
            }
            (TyKind::Tuple(a), TyKind::Tuple(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b.iter()).all(|(a, b)| self.ty_matches_for_candidate(a, b))
            }
            (TyKind::Array(a, a_len), TyKind::Array(b, b_len)) => {
                a_len == b_len && self.ty_matches_for_candidate(a, b)
            }
            (TyKind::Slice(a), TyKind::Slice(b)) => self.ty_matches_for_candidate(a, b),
            _ => expected == actual,
        }
    }

    fn path_segments_to_ty(&self, segments: &[HirPathSegment]) -> TyKind {
        if segments.is_empty() {
            return TyKind::Error;
        }
        let last_segment = &segments[segments.len() - 1];
        let full_path = if segments.len() >= 2 {
            Some(
                segments
                    .iter()
                    .map(|seg| self.interner.resolve(seg.ident))
                    .collect::<Vec<_>>()
                    .join("::"),
            )
        } else {
            None
        };

        if segments.len() >= 2 {
            let first_sym = segments[0].ident;
            let assoc_name = segments[1].ident;
            let first_str = self.interner.resolve(first_sym);

            if first_str == "Self" {
                if let Some(concrete_ty) = self.current_impl_assoc_types.get(&assoc_name) {
                    return concrete_ty.clone();
                }

                let self_ty = self.current_self_ty.clone().unwrap_or(TyKind::Error);
                let trait_def_id = match self_ty {
                    TyKind::Adt(def_id, _) if self.trait_assoc_types.contains_key(&def_id) => Some(def_id),
                    _ => None,
                };
                return TyKind::Projection(Box::new(self_ty), trait_def_id, assoc_name);
            }

            if let Some(&param_idx) = self.current_generic_params.get(&first_sym) {
                return self.projection_for_param_assoc(param_idx, assoc_name);
            }
        }

        if let Some(full_path) = &full_path {
            match full_path.as_str() {
                "core::cmp::Ordering" => return self.comparison_ordering_ty(),
                "core::sync::atomic::Ordering" => return self.atomic_ordering_ty(),
                _ => {}
            }
        }

        let name = self.interner.resolve(last_segment.ident);
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
            "Self" => self.current_self_ty.clone().unwrap_or(TyKind::Error),
            _ => {
                let sym = last_segment.ident;
                if let Some(&idx) = self.current_generic_params.get(&sym) {
                    return TyKind::Param(idx);
                }
                if let Some(full_path) = &full_path {
                    if let Some(iter_ty) = self.std_slice_iter_ty(full_path, last_segment) {
                        return iter_ty;
                    }
                    if let Some(alias_ty) = self.resolve_qualified_type_alias(full_path) {
                        let type_args = self.type_args_from_segment(last_segment);
                        return self.substitute_params(&alias_ty, &type_args);
                    }
                }
                if let Some(full_path) = &full_path {
                    if Self::is_extern_type_path(full_path) {
                        if let Some(def_id) = self.lookup_intrinsic_def_by_path(full_path) {
                            let type_args = if let Some(ref args) = last_segment.args {
                                args.args
                                    .iter()
                                    .filter_map(|a| {
                                        if let HirGenericArg::Type(ty) = a {
                                            Some(self.hir_ty_to_ty(ty))
                                        } else {
                                            None
                                        }
                                    })
                                    .collect()
                            } else {
                                vec![]
                            };
                            return TyKind::Adt(def_id, type_args);
                        }
                    }
                }
                if let Some(full_path) = &full_path {
                    if let Some(def_id) = self.resolve_qualified_type_path(full_path) {
                        let type_args = if let Some(ref args) = last_segment.args {
                            args.args
                                .iter()
                                .filter_map(|a| {
                                    if let HirGenericArg::Type(ty) = a {
                                        Some(self.hir_ty_to_ty(ty))
                                    } else {
                                        None
                                    }
                                })
                                .collect()
                        } else {
                            vec![]
                        };
                        return TyKind::Adt(def_id, type_args);
                    }
                }
                if let Some(def_id) = self.resolve_scoped_type_name(sym) {
                    let type_args = if let Some(ref args) = last_segment.args {
                        args.args
                            .iter()
                            .filter_map(|a| {
                                if let HirGenericArg::Type(ty) = a {
                                    Some(self.hir_ty_to_ty(ty))
                                } else {
                                    None
                                }
                            })
                            .collect()
                    } else {
                        vec![]
                    };
                    TyKind::Adt(def_id, type_args)
                } else if let Some(alias_ty) = self.resolve_scoped_type_alias(sym) {
                    let type_args = self.type_args_from_segment(last_segment);
                    self.substitute_params(&alias_ty, &type_args)
                } else {
                    if let Some(full_path) = &full_path {
                        if let Some(def_id) = self.lookup_intrinsic_def_by_path(full_path) {
                            return TyKind::Adt(def_id, self.type_args_from_segment(last_segment));
                        }
                    }
                    if let Some(def_id) = self.lookup_intrinsic_def_by_path(name) {
                        return TyKind::Adt(def_id, self.type_args_from_segment(last_segment));
                    }
                    TyKind::Error
                }
            }
        }
    }

    fn type_args_from_segment(&self, segment: &HirPathSegment) -> Vec<TyKind> {
        segment
            .args
            .as_ref()
            .map(|args| {
                args.args
                    .iter()
                    .filter_map(|arg| match arg {
                        HirGenericArg::Type(ty) => Some(self.hir_ty_to_ty(ty)),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn is_extern_type_path(path: &str) -> bool {
        matches!(
            path.split("::").next(),
            Some("core" | "alloc" | "std" | "anyos_std" | "proc_macro" | "proc_macro2" | "quote" | "syn" | "serde")
        )
    }

    fn lookup_intrinsic_def_by_path(&self, full_path: &str) -> Option<DefId> {
        self.resolve
            .intrinsic_fns
            .iter()
            .find_map(|(&def_id, path)| if path == full_path { Some(def_id) } else { None })
    }

    fn intrinsic_enum_variant_type(&self, path_str: &str) -> Option<TyKind> {
        let (parent_path, variant_name) = path_str.rsplit_once("::")?;
        match parent_path {
            "core::cmp::Ordering" if matches!(variant_name, "Less" | "Equal" | "Greater") => {
                Some(self.comparison_ordering_ty())
            }
            "core::sync::atomic::Ordering" => {
                if matches!(variant_name, "Relaxed" | "Release" | "Acquire" | "AcqRel" | "SeqCst") {
                    Some(self.atomic_ordering_ty())
                } else {
                    None
                }
            }
            "proc_macro::TokenTree" | "proc_macro2::TokenTree"
                if matches!(variant_name, "Group" | "Punct" | "Ident" | "Literal") =>
            {
                Some(TyKind::Adt(
                    self.lookup_intrinsic_def_by_path(parent_path)
                        .unwrap_or(SYNTH_PROC_MACRO_TOKEN_TREE_DEF_ID),
                    vec![],
                ))
            }
            "proc_macro::Spacing" | "proc_macro2::Spacing" if matches!(variant_name, "Joint" | "Alone") => {
                Some(TyKind::Adt(
                    self.lookup_intrinsic_def_by_path(parent_path)
                        .unwrap_or(SYNTH_PROC_MACRO_SPACING_DEF_ID),
                    vec![],
                ))
            }
            "proc_macro::Delimiter" | "proc_macro2::Delimiter"
                if matches!(variant_name, "Parenthesis" | "Bracket" | "Brace" | "None") =>
            {
                Some(TyKind::Adt(
                    self.lookup_intrinsic_def_by_path(parent_path)
                        .unwrap_or(SYNTH_PROC_MACRO_DELIMITER_DEF_ID),
                    vec![],
                ))
            }
            _ => None,
        }
    }

    fn enum_variant_type_from_path(&self, path: &HirPath) -> Option<TyKind> {
        if path.segments.len() < 2 {
            return None;
        }
        let variant_name = path.segments.last()?.ident;
        let parent_ty = self.path_segments_to_ty(&path.segments[..path.segments.len() - 1]);
        let resolved_parent = self.shallow_resolve(parent_ty);
        let TyKind::Adt(enum_def_id, substs) = resolved_parent else {
            return None;
        };
        let variants = self.enum_variant_fields.get(&enum_def_id)?;
        if variants.iter().any(|(name, _)| *name == variant_name) {
            Some(TyKind::Adt(enum_def_id, substs))
        } else {
            None
        }
    }

    fn intrinsic_constructor_type(&mut self, path_str: &str) -> Option<TyKind> {
        match path_str {
            "Option::Some"
            | "Option::None"
            | "core::option::Option::Some"
            | "core::option::Option::None" => {
                let sym = self.interner.lookup("Option")?;
                let def_id = *self.type_name_to_def.get(&sym)?;
                Some(TyKind::Adt(def_id, vec![self.fresh_infer(InferKind::General)]))
            }
            "Result::Ok"
            | "Result::Err"
            | "core::result::Result::Ok"
            | "core::result::Result::Err" => {
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
            "NonZeroUsize::new" | "core::num::NonZeroUsize::new" => {
                let sym = self.interner.lookup("NonZeroUsize")?;
                let nonzero_def_id = *self.type_name_to_def.get(&sym)?;
                self.option_of(TyKind::Adt(nonzero_def_id, vec![]))
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
            "f32::INFINITY" | "f32::NEG_INFINITY" | "f32::NAN" | "f32::EPSILON" => {
                Some(TyKind::Float(FloatTy::F32))
            }
            "f64::INFINITY" | "f64::NEG_INFINITY" | "f64::NAN" | "f64::EPSILON" => {
                Some(TyKind::Float(FloatTy::F64))
            }
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
            || self.local_type_name_is(def_id, "Vec")
            || self.is_known_type_path(def_id, "Vec")
    }

    fn is_string_def(&self, def_id: DefId) -> bool {
        self.interner
            .lookup("String")
            .and_then(|sym| self.type_name_to_def.get(&sym).copied())
            == Some(def_id)
            || self.local_type_name_is(def_id, "String")
            || self.is_known_type_path(def_id, "String")
    }

    fn string_ty(&self) -> Option<TyKind> {
        self.interner
            .lookup("String")
            .and_then(|sym| self.type_name_to_def.get(&sym).copied())
            .map(|def_id| TyKind::Adt(def_id, vec![]))
    }

    fn is_known_type_path(&self, def_id: DefId, name: &str) -> bool {
        self.resolve
            .intrinsic_fns
            .get(&def_id)
            .is_some_and(|path| path == name || path.ends_with(&format!("::{name}")))
            || self.qualified_type_names.iter().any(|(path, candidate)| {
                *candidate == def_id && (path == name || path.ends_with(&format!("::{name}")))
            })
    }

    fn local_type_name_is(&self, def_id: DefId, name: &str) -> bool {
        self.type_name_to_def
            .iter()
            .any(|(sym, candidate)| *candidate == def_id && self.interner.resolve(*sym) == name)
    }

    fn is_result_def(&self, def_id: DefId) -> bool {
        self.interner
            .lookup("Result")
            .and_then(|sym| self.type_name_to_def.get(&sym).copied())
            == Some(def_id)
    }

    fn is_option_def(&self, def_id: DefId) -> bool {
        self.interner
            .lookup("Option")
            .and_then(|sym| self.type_name_to_def.get(&sym).copied())
            == Some(def_id)
    }

    fn is_box_def(&self, def_id: DefId) -> bool {
        self.interner
            .lookup("Box")
            .and_then(|sym| self.type_name_to_def.get(&sym).copied())
            == Some(def_id)
            || self.local_type_name_is(def_id, "Box")
            || self.is_known_type_path(def_id, "Box")
    }

    fn is_nonzero_usize_def(&self, def_id: DefId) -> bool {
        self.interner
            .lookup("NonZeroUsize")
            .and_then(|sym| self.type_name_to_def.get(&sym).copied())
            == Some(def_id)
            || self
                .resolve
                .intrinsic_fns
                .get(&def_id)
                .is_some_and(|path| path.ends_with("NonZeroUsize"))
    }

    fn is_token_stream_def(&self, def_id: DefId) -> bool {
        self.qualified_type_names
            .iter()
            .any(|(path, candidate)| {
                *candidate == def_id
                    && (path == "TokenStream"
                        || path == "crate::TokenStream"
                        || path == "proc_macro::TokenStream"
                        || path == "proc_macro2::TokenStream"
                        || path.ends_with("::TokenStream"))
            })
            || self
                .interner
                .lookup("TokenStream")
                .and_then(|sym| self.type_name_to_def.get(&sym).copied())
                == Some(def_id)
    }

    fn option_inner_ty(&self, ty: &TyKind) -> Option<TyKind> {
        match ty {
            TyKind::Adt(def_id, substs) if self.is_option_def(*def_id) && substs.len() == 1 => {
                Some(substs[0].clone())
            }
            _ => None,
        }
    }

    fn iter_item_ty(&self, ty: &TyKind) -> Option<TyKind> {
        match ty {
            TyKind::Slice(elem) | TyKind::Array(elem, _) => Some(elem.as_ref().clone()),
            TyKind::Adt(def_id, substs) if self.is_vec_def(*def_id) && substs.len() == 1 => {
                Some(substs[0].clone())
            }
            TyKind::Projection(self_ty, Some(trait_def_id), assoc_name)
                if self.trait_def_name_is(*trait_def_id, "IntoIterator")
                    && self.symbol_name_is(*assoc_name, "IntoIter") =>
            {
                let TyKind::Param(param_idx) = self.shallow_resolve(self_ty.as_ref().clone()) else {
                    return None;
                };
                let item_sym = self.interner.lookup("Item")?;
                self.current_generic_assoc_bindings
                    .get(&(param_idx, *trait_def_id, item_sym))
                    .cloned()
                    .or_else(|| {
                        Some(TyKind::Projection(
                            Box::new(TyKind::Param(param_idx)),
                            Some(*trait_def_id),
                            item_sym,
                        ))
                    })
            }
            _ => None,
        }
    }

    fn symbol_name_is(&self, sym: Symbol, expected: &str) -> bool {
        self.interner.resolve(sym) == expected
    }

    fn trait_def_name_is(&self, trait_def_id: DefId, expected: &str) -> bool {
        self.trait_names
            .get(&trait_def_id)
            .is_some_and(|name| self.symbol_name_is(*name, expected))
            || self
                .resolve
                .intrinsic_fns
                .get(&trait_def_id)
                .is_some_and(|path| path.rsplit("::").next() == Some(expected))
    }

    fn callable_output_ty(&self, ty: &TyKind) -> Option<TyKind> {
        match self.shallow_resolve(ty.clone()) {
            TyKind::FnDef(def_id, _) => self.fn_sigs.get(&def_id).map(|(_, ret)| ret.clone()),
            TyKind::FnPtr(_, ret) => Some(*ret),
            TyKind::Adt(def_id, substs) if self.enum_variant_fields.contains_key(&def_id) => {
                Some(TyKind::Adt(def_id, substs))
            }
            _ => None,
        }
    }

    fn primitive_byte_width(&self, ty: &TyKind) -> Option<usize> {
        match ty {
            TyKind::Uint(UintTy::U8) | TyKind::Int(IntTy::I8) => Some(1),
            TyKind::Uint(UintTy::U16) | TyKind::Int(IntTy::I16) => Some(2),
            TyKind::Uint(UintTy::U32) | TyKind::Int(IntTy::I32) => Some(4),
            TyKind::Uint(UintTy::U64) | TyKind::Int(IntTy::I64) => Some(8),
            TyKind::Uint(UintTy::U128) | TyKind::Int(IntTy::I128) => Some(16),
            TyKind::Uint(UintTy::Usize) | TyKind::Int(IntTy::Isize) => Some(8),
            _ => None,
        }
    }

    fn is_integer_ty(ty: &TyKind) -> bool {
        matches!(ty, TyKind::Uint(_) | TyKind::Int(_))
    }

    fn bind_closure_param(&mut self, param: &HirParam, expected_ty: TyKind) {
        if !matches!(param.ty, HirTy::Infer(_)) {
            let annotated_ty = self.hir_ty_to_ty(&param.ty);
            self.unify(&expected_ty, &annotated_ty, param.span);
        }
        if let HirPattern::Ident(hir_id, _, _, _, _) = &param.pat {
            if let Some(&def_id) = self.resolve.resolutions.get(&hir_id) {
                self.local_types.insert(def_id, expected_ty);
            }
        } else {
            self.bind_pattern(&param.pat, expected_ty);
        }
    }

    fn check_closure_body_return(
        &mut self,
        annotated_ret_ty: Option<&HirTy>,
        expected_ret_ty: Option<TyKind>,
        body: &HirExpr,
        span: Span,
    ) -> TyKind {
        let has_known_ret = annotated_ret_ty.is_some() || expected_ret_ty.is_some();
        let target_ret = match annotated_ret_ty {
            Some(ty) => {
                let annotated = self.hir_ty_to_ty(ty);
                if let Some(expected) = expected_ret_ty {
                    self.unify(&expected, &annotated, span);
                }
                annotated
            }
            None => expected_ret_ty.unwrap_or_else(|| self.fresh_infer(InferKind::General)),
        };

        let old_ret = self.current_fn_ret.replace(target_ret.clone());
        let body_ty = self.check_expr(body);
        self.current_fn_ret = old_ret;

        if body_ty != TyKind::Never {
            self.unify(&target_ret, &body_ty, body.span);
        }

        let resolved = self.shallow_resolve(target_ret);
        if body_ty == TyKind::Never && !has_known_ret && Self::is_deferred_ty(&resolved) {
            TyKind::Never
        } else {
            resolved
        }
    }

    fn resolve_impl_method_by_receiver(
        &self,
        method_name: Symbol,
        base_ty: &TyKind,
        inner_ty: &TyKind,
    ) -> Option<(DefId, Vec<TyKind>, TyKind)> {
        let mut fallback_match = None;

        for methods in self.resolve.impl_methods.values() {
            for (candidate_name, method_def_id) in methods {
                if *candidate_name != method_name {
                    continue;
                }
                let Some((param_tys, ret_ty)) = self.fn_sigs.get(method_def_id).cloned() else {
                    continue;
                };
                let Some(self_param_ty) = param_tys.first() else {
                    continue;
                };
                let impl_substs = self
                    .infer_receiver_substs(self_param_ty, base_ty)
                    .or_else(|| self.infer_receiver_substs(self_param_ty, inner_ty));
                let Some(impl_substs) = impl_substs else {
                    continue;
                };

                let param_tys: Vec<TyKind> = param_tys
                    .iter()
                    .map(|t| self.substitute_params(t, &impl_substs))
                    .collect();
                let ret_ty = self.substitute_params(&ret_ty, &impl_substs);
                fallback_match = Some((*method_def_id, param_tys, ret_ty));
            }
        }

        fallback_match
    }

    // ── Unification ──

    fn unify(&mut self, expected: &TyKind, actual: &TyKind, span: Span) {
        let expected = self.shallow_resolve(expected.clone());
        let actual = self.shallow_resolve(actual.clone());

        if expected == actual { return; }

        match (&expected, &actual) {
            (TyKind::Error, _) | (_, TyKind::Error) => return,
            (TyKind::Never, _) | (_, TyKind::Never) => return,
            (TyKind::Unit, other) | (other, TyKind::Unit) if Self::contains_synthetic_def(other) => return,
            (TyKind::Projection(_, _, _), _) | (_, TyKind::Projection(_, _, _)) => return,
            (TyKind::Param(_), _) | (_, TyKind::Param(_)) => return,
            (TyKind::DynTrait(_), TyKind::Param(_)) | (TyKind::Param(_), TyKind::DynTrait(_)) => return,

            (TyKind::Infer(v), _) => {
                self.bind_infer_var(*v, &expected, actual, span);
                return;
            }
            (_, TyKind::Infer(v)) => {
                self.bind_infer_var(*v, &actual, expected, span);
                return;
            }

            _ if Self::contains_deferred_ty(&expected) || Self::contains_deferred_ty(&actual) => return,

            (TyKind::Ref(a, am), TyKind::Ref(b, bm)) if am == bm => {
                if let TyKind::Ref(nested, _) = b.as_ref() {
                    self.unify(a, nested, span);
                    return;
                }
                if let (TyKind::Array(a_elem, _), TyKind::Array(b_elem, _)) = (a.as_ref(), b.as_ref()) {
                    self.unify(a_elem, b_elem, span);
                    return;
                }
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
                if let TyKind::Adt(def_id, substs) = a.as_ref() {
                    if self.is_vec_def(*def_id) && substs.len() == 1 {
                        match b.as_ref() {
                            TyKind::Array(array_elem, _) => {
                                self.unify(&substs[0], array_elem, span);
                                return;
                            }
                            TyKind::Slice(slice_elem) => {
                                self.unify(&substs[0], slice_elem, span);
                                return;
                            }
                            _ => {}
                        }
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
                if let TyKind::Adt(def_id, substs) = b.as_ref() {
                    if self.is_vec_def(*def_id) && substs.len() == 1 {
                        match a.as_ref() {
                            TyKind::Array(array_elem, _) => {
                                self.unify(&substs[0], array_elem, span);
                                return;
                            }
                            TyKind::Slice(slice_elem) => {
                                self.unify(&substs[0], slice_elem, span);
                                return;
                            }
                            _ => {}
                        }
                    }
                    if self.is_box_def(*def_id) && substs.len() == 1 {
                        self.unify(a, &substs[0], span);
                        return;
                    }
                }
                if let TyKind::Adt(def_id, substs) = a.as_ref() {
                    if self.is_box_def(*def_id) && substs.len() == 1 {
                        self.unify(&substs[0], b, span);
                        return;
                    }
                }
                self.unify(a, b, span);
                return;
            }

            (TyKind::RawPtr(a, am), TyKind::RawPtr(b, bm)) if am == bm => {
                self.unify(a, b, span);
                return;
            }
            (TyKind::RawPtr(a, _), TyKind::RawPtr(b, _)) => {
                self.unify(a, b, span);
                return;
            }
            (TyKind::Ref(a, _), TyKind::Ref(b, _)) => {
                self.unify(a, b, span);
                return;
            }

            (TyKind::RawPtr(expected_inner, _), TyKind::Ref(actual_inner, _))
                if coerce::classify(&expected, &actual) == Some(CoercionKind::RefToRawPtr) =>
            {
                self.unify(expected_inner, actual_inner, span);
                return;
            }

            (TyKind::Ref(expected_inner, Mutability::Immutable), TyKind::Adt(def_id, substs))
                if self.is_vec_def(*def_id) && substs.len() == 1 =>
            {
                if let TyKind::Slice(expected_elem) = expected_inner.as_ref() {
                    self.unify(expected_elem, &substs[0], span);
                    return;
                }
            }

            (TyKind::Ref(expected_inner, Mutability::Immutable), TyKind::Adt(def_id, _))
                if matches!(expected_inner.as_ref(), TyKind::Str)
                    && self.is_string_def(*def_id) =>
            {
                return;
            }

            (TyKind::Adt(def_id, _), TyKind::Ref(actual_inner, Mutability::Immutable))
                if self.is_string_def(*def_id)
                    && matches!(actual_inner.as_ref(), TyKind::Str) =>
            {
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

            (TyKind::Adt(def_id, substs), TyKind::Array(elem, _))
            | (TyKind::Array(elem, _), TyKind::Adt(def_id, substs))
                if self.is_vec_def(*def_id) && substs.len() == 1 =>
            {
                self.unify(&substs[0], elem, span);
                return;
            }

            (TyKind::Adt(def_id, substs), TyKind::Ref(actual_inner, Mutability::Immutable))
                if self.is_vec_def(*def_id) && substs.len() == 1 =>
            {
                match actual_inner.as_ref() {
                    TyKind::Array(elem, _) | TyKind::Slice(elem) => {
                        self.unify(&substs[0], elem, span);
                        return;
                    }
                    _ => {}
                }
            }

            (TyKind::Str, TyKind::Adt(def_id, _)) | (TyKind::Adt(def_id, _), TyKind::Str)
                if self.is_string_def(*def_id) =>
            {
                return;
            }

            (TyKind::Adt(a, _), TyKind::Adt(b, _)) if a == b => return,
            (TyKind::Adt(a, _), TyKind::Adt(b, _)) if a.0 >= 10000 || b.0 >= 10000 => return,
            (TyKind::DynTrait(a), TyKind::DynTrait(b)) if a == b => return,

            // Allow coercion from FnDef to FnPtr if signatures match
            (TyKind::FnPtr(expected_params, expected_ret), TyKind::FnPtr(actual_params, actual_ret))
                if expected_params.len() == actual_params.len() =>
            {
                for (ep, ap) in expected_params.iter().zip(actual_params.iter()) {
                    self.unify(ep, ap, span);
                }
                self.unify(expected_ret, actual_ret, span);
                return;
            }

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

        self.error(span, &format!(
            "type mismatch: expected {}, found {}",
            self.describe_ty(&expected),
            self.describe_ty(&actual),
        ));
    }

    fn bind_infer_var(&mut self, var: InferVar, infer_ty: &TyKind, concrete_ty: TyKind, span: Span) {
        if self.infer_var_accepts(var, &concrete_ty) {
            self.substitutions.insert(var, concrete_ty);
            return;
        }

        self.error(span, &format!(
            "type mismatch: expected {}, found {}",
            self.describe_ty(infer_ty),
            self.describe_ty(&concrete_ty),
        ));
    }

    fn infer_var_accepts(&self, var: InferVar, ty: &TyKind) -> bool {
        match self.infer_kinds.get(&var).copied().unwrap_or(InferKind::General) {
            InferKind::General => true,
            InferKind::Integer => matches!(
                self.shallow_resolve(ty.clone()),
                TyKind::Int(_) | TyKind::Uint(_) | TyKind::Infer(_)
            ),
            InferKind::Float => matches!(
                self.shallow_resolve(ty.clone()),
                TyKind::Float(_) | TyKind::Infer(_)
            ),
        }
    }

    fn describe_ty(&self, ty: &TyKind) -> String {
        match ty {
            TyKind::Adt(def_id, substs) => {
                let name = self
                    .qualified_type_names
                    .iter()
                    .filter(|(_, candidate)| *candidate == def_id)
                    .map(|(path, _)| path.as_str())
                    .min_by_key(|path| path.len())
                    .map(str::to_string)
                    .or_else(|| {
                        self.type_name_to_def
                            .iter()
                            .find_map(|(name, candidate)| {
                                (*candidate == *def_id).then(|| self.interner.resolve(*name).to_string())
                            })
                    })
                    .unwrap_or_else(|| format!("Adt({:?})", def_id));
                if substs.is_empty() {
                    name
                } else {
                    let args = substs
                        .iter()
                        .map(|ty| self.describe_ty(ty))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{}<{}>", name, args)
                }
            }
            TyKind::Ref(inner, mutability) => {
                let marker = match mutability {
                    Mutability::Mut => "&mut ",
                    Mutability::Immutable => "&",
                };
                format!("{}{}", marker, self.describe_ty(inner))
            }
            TyKind::RawPtr(inner, mutability) => {
                let marker = match mutability {
                    Mutability::Mut => "*mut ",
                    Mutability::Immutable => "*const ",
                };
                format!("{}{}", marker, self.describe_ty(inner))
            }
            TyKind::Slice(inner) => format!("[{}]", self.describe_ty(inner)),
            TyKind::Array(inner, len) => format!("[{}; {}]", self.describe_ty(inner), len),
            TyKind::Tuple(items) => {
                let parts = items
                    .iter()
                    .map(|ty| self.describe_ty(ty))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({})", parts)
            }
            other => format!("{:?}", other),
        }
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

    fn binary_operand_ty(&self, ty: TyKind) -> TyKind {
        let resolved = self.shallow_resolve(ty);
        match resolved {
            TyKind::Ref(inner, Mutability::Immutable) => {
                let inner = self.shallow_resolve(*inner);
                if Self::is_primitive_scalar_ty(&inner) {
                    inner
                } else {
                    TyKind::Ref(Box::new(inner), Mutability::Immutable)
                }
            }
            other => other,
        }
    }

    fn is_primitive_scalar_ty(ty: &TyKind) -> bool {
        matches!(
            ty,
            TyKind::Bool
                | TyKind::Char
                | TyKind::Int(_)
                | TyKind::Uint(_)
                | TyKind::Float(_)
        )
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
            TyKind::Projection(self_ty, trait_def_id, assoc_name) => {
                self.resolve_generic_assoc_binding(TyKind::Projection(
                    Box::new(self.resolve_ty_full(*self_ty)),
                    trait_def_id,
                    assoc_name,
                ))
            }
            other => other,
        }
    }

    // ── Pattern binding ──

    fn wrap_pattern_binding_ty(
        &self,
        ptr_ty: &TyKind,
        inner: TyKind,
    ) -> TyKind {
        match ptr_ty {
            TyKind::Ref(_, mutability) => TyKind::Ref(Box::new(inner), *mutability),
            TyKind::RawPtr(_, mutability) => TyKind::RawPtr(Box::new(inner), *mutability),
            _ => inner,
        }
    }

    fn bind_pattern(&mut self, pat: &HirPattern, ty: TyKind) {
        let resolved_ty = self.shallow_resolve(ty.clone());
        match pat {
            HirPattern::Ident(hir_id, _, _, sub, _) => {
                if let Some(&def_id) = self.resolve.resolutions.get(hir_id) {
                    self.local_types.insert(def_id, ty.clone());
                }
                if let Some(sub) = sub {
                    self.bind_pattern(sub, ty);
                }
            }
            HirPattern::Tuple(pats, _) => {
                match &resolved_ty {
                    TyKind::Tuple(tys) => {
                        for (p, t) in pats.iter().zip(tys.iter()) {
                            self.bind_pattern(p, t.clone());
                        }
                    }
                    TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => {
                        if let TyKind::Tuple(tys) = self.shallow_resolve(inner.as_ref().clone()) {
                            for (p, t) in pats.iter().zip(tys.iter()) {
                                let wrapped = self.wrap_pattern_binding_ty(&resolved_ty, t.clone());
                                self.bind_pattern(p, wrapped);
                            }
                        }
                    }
                    _ => {}
                }
            }
            HirPattern::Slice(pats, _) => {
                let non_rest = pats.iter().filter(|p| !matches!(p, HirPattern::Rest(_)));
                match &resolved_ty {
                    TyKind::Array(elem_ty, _) | TyKind::Slice(elem_ty) => {
                        for p in non_rest {
                            self.bind_pattern(p, elem_ty.as_ref().clone());
                        }
                    }
                    TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => {
                        match self.shallow_resolve(inner.as_ref().clone()) {
                            TyKind::Array(elem_ty, _) | TyKind::Slice(elem_ty) => {
                                for p in non_rest {
                                    let wrapped = self.wrap_pattern_binding_ty(
                                        &resolved_ty,
                                        elem_ty.as_ref().clone(),
                                    );
                                    self.bind_pattern(p, wrapped);
                                }
                            }
                            other if Self::is_deferred_ty(&other) => {
                                let elem_ty = self.fresh_infer(InferKind::General);
                                for p in non_rest {
                                    let wrapped =
                                        self.wrap_pattern_binding_ty(&resolved_ty, elem_ty.clone());
                                    self.bind_pattern(p, wrapped);
                                }
                            }
                            _ => {}
                        }
                    }
                    other if Self::is_deferred_ty(other) => {
                        let elem_ty = self.fresh_infer(InferKind::General);
                        for p in non_rest {
                            self.bind_pattern(p, elem_ty.clone());
                        }
                    }
                    _ => {}
                }
            }
            HirPattern::Wildcard(_) | HirPattern::Rest(_) => {}
            HirPattern::Ref(inner, _, _) => {
                if let TyKind::Ref(inner_ty, _) = resolved_ty {
                    self.bind_pattern(inner, *inner_ty);
                }
            }
            HirPattern::RefBinding(inner, mutability, _) => {
                self.bind_pattern(inner, TyKind::Ref(Box::new(ty), *mutability));
            }
            HirPattern::Struct(path, fields, _, _) => {
                match &resolved_ty {
                    TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => {
                        if let Some(field_tys) = self.pattern_fields(path, inner.as_ref()) {
                            for field in fields {
                                if let Some((_, field_ty)) =
                                    field_tys.iter().find(|(name, _)| *name == field.name)
                                {
                                    let wrapped =
                                        self.wrap_pattern_binding_ty(&resolved_ty, field_ty.clone());
                                    self.bind_pattern(&field.pat, wrapped);
                                }
                            }
                        }
                    }
                    _ => {
                        if let Some(field_tys) = self.pattern_fields(path, &ty) {
                            for field in fields {
                                if let Some((_, field_ty)) =
                                    field_tys.iter().find(|(name, _)| *name == field.name)
                                {
                                    self.bind_pattern(&field.pat, field_ty.clone());
                                }
                            }
                        }
                    }
                }
            }
            HirPattern::TupleStruct(path, pats, _) => {
                match &resolved_ty {
                    TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => {
                        if let Some(field_tys) = self.pattern_variant_tys(path, inner.as_ref()) {
                            for (pat, field_ty) in pats.iter().filter(|p| !matches!(p, HirPattern::Rest(_))).zip(field_tys.iter()) {
                                let wrapped =
                                    self.wrap_pattern_binding_ty(&resolved_ty, field_ty.clone());
                                self.bind_pattern(pat, wrapped);
                            }
                        }
                    }
                    _ => {
                        if let Some(field_tys) = self.pattern_variant_tys(path, &ty) {
                            for (pat, field_ty) in pats.iter().filter(|p| !matches!(p, HirPattern::Rest(_))).zip(field_tys.iter()) {
                                self.bind_pattern(pat, field_ty.clone());
                            }
                        }
                    }
                }
            }
            HirPattern::Or(pats, _) => {
                for pat in pats {
                    self.bind_pattern(pat, ty.clone());
                }
            }
            _ => {}
        }
    }

    fn pattern_fields(&self, path: &HirPath, ty: &TyKind) -> Option<Vec<(Symbol, TyKind)>> {
        let resolved = self.shallow_resolve(ty.clone());
        let TyKind::Adt(def_id, substs) = resolved else {
            return None;
        };

        if let Some(fields) = self.struct_defs.get(&def_id) {
            return Some(fields.iter()
                .map(|(name, field_ty)| (*name, self.substitute_params(field_ty, &substs)))
                .collect());
        }

        let variant_name = path.segments.last()?.ident;
        if let Some(fields) = self.enum_variant_named_fields.get(&(def_id, variant_name)) {
            return Some(fields.iter()
                .map(|(name, field_ty)| (*name, self.substitute_params(field_ty, &substs)))
                .collect());
        }

        let variants = self.enum_variant_fields.get(&def_id)?;
        let field_tys = variants.iter()
            .find(|(name, _)| *name == variant_name)?
            .1
            .iter()
            .enumerate()
            .map(|(idx, field_ty)| {
                let field_name = self.interner.lookup(&idx.to_string()).unwrap_or(Symbol::from_raw(0));
                (field_name, self.substitute_params(field_ty, &substs))
            })
            .collect();
        Some(field_tys)
    }

    fn pattern_variant_tys(&self, path: &HirPath, ty: &TyKind) -> Option<Vec<TyKind>> {
        let resolved = self.shallow_resolve(ty.clone());
        let TyKind::Adt(def_id, substs) = resolved else {
            return None;
        };
        let variant_name = path.segments.last()?.ident;
        let variant_str = self.interner.resolve(variant_name);

        if self.is_result_def(def_id) {
            return match (variant_str, substs.as_slice()) {
                ("Ok", [ok_ty, _]) => Some(vec![ok_ty.clone()]),
                ("Err", [_, err_ty]) => Some(vec![err_ty.clone()]),
                _ => None,
            };
        }

        if self.is_option_def(def_id) {
            return match (variant_str, substs.as_slice()) {
                ("Some", [inner_ty]) => Some(vec![inner_ty.clone()]),
                ("None", _) => Some(vec![]),
                _ => None,
            };
        }

        let variants = self.enum_variant_fields.get(&def_id)?;
        let field_tys = variants.iter()
            .find(|(name, _)| *name == variant_name)?
            .1
            .iter()
            .map(|field_ty| self.substitute_params(field_ty, &substs))
            .collect();
        Some(field_tys)
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
            TyKind::FnDef(def_id, fn_substs) => TyKind::FnDef(
                *def_id,
                fn_substs.iter().map(|t| self.substitute_params(t, substs)).collect(),
            ),
            TyKind::FnPtr(params, ret) => TyKind::FnPtr(
                params.iter().map(|t| self.substitute_params(t, substs)).collect(),
                Box::new(self.substitute_params(ret, substs)),
            ),
            TyKind::Adt(def_id, adt_substs) => TyKind::Adt(
                *def_id,
                adt_substs.iter().map(|t| self.substitute_params(t, substs)).collect(),
            ),
            TyKind::Projection(self_ty, trait_def_id, assoc_name) => TyKind::Projection(
                Box::new(self.substitute_params(self_ty, substs)),
                *trait_def_id,
                *assoc_name,
            ),
            _ => ty.clone(),
        }
    }

    // ── HIR type to TyKind ──

    fn hir_ty_to_ty(&self, ty: &HirTy) -> TyKind {
        match ty {
            HirTy::Path(path) => self.path_segments_to_ty(&path.segments),
            HirTy::QualifiedPath(qpath) => self.qualified_path_to_ty(qpath),
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
            HirTy::DynTrait(bounds, _) => {
                for bound in bounds {
                    if bound.path.segments.is_empty() { continue; }
                    let sym = bound.path.segments[0].ident;
                    if let Some(&def_id) = self.type_name_to_def.get(&sym) {
                        return TyKind::DynTrait(def_id);
                    }
                }
                TyKind::Error
            }
            HirTy::MacroCall(_, _) => TyKind::Error,
            HirTy::Never(_) => TyKind::Never,
            HirTy::Infer(_) => TyKind::Error,
        }
    }

    /// Evaluate a const expression to a usize (for array lengths).
    fn eval_const_usize(&self, expr: &HirExpr) -> usize {
        match &expr.kind {
            HirExprKind::Lit(Literal::Int(n)) => *n as usize,
            HirExprKind::Paren(inner) => self.eval_const_usize(inner),
            HirExprKind::Cast(inner, _) => self.eval_const_usize(inner),
            HirExprKind::Unary(crate::ast::UnOp::Neg, inner) => {
                0usize.wrapping_sub(self.eval_const_usize(inner))
            }
            HirExprKind::Binary(op, lhs, rhs) => {
                let l = self.eval_const_usize(lhs);
                let r = self.eval_const_usize(rhs);
                match op {
                    BinOp::Add => l.wrapping_add(r),
                    BinOp::Sub => l.wrapping_sub(r),
                    BinOp::Mul => l.wrapping_mul(r),
                    BinOp::Div => if r == 0 { 0 } else { l / r },
                    BinOp::Rem => if r == 0 { 0 } else { l % r },
                    BinOp::Shl => l.wrapping_shl(r as u32),
                    BinOp::Shr => l.wrapping_shr(r as u32),
                    BinOp::BitAnd => l & r,
                    BinOp::BitOr => l | r,
                    BinOp::BitXor => l ^ r,
                    _ => 0,
                }
            }
            _ => 0,
        }
    }
}

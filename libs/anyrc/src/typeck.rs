use crate::hir::*;
use crate::ast::{BinOp, Literal, Mutability};
use crate::resolve::ResolveResult;
use crate::intern::{Interner, Symbol};
use crate::diagnostics::{Span, Diagnostic, Level};
use std::collections::HashMap;

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

    next_infer: u32,
    infer_kinds: HashMap<InferVar, InferKind>,
    substitutions: HashMap<InferVar, TyKind>,

    current_fn_ret: Option<TyKind>,
    errors: Vec<Diagnostic>,
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
            next_infer: 0,
            infer_kinds: HashMap::new(),
            substitutions: HashMap::new(),
            current_fn_ret: None,
            errors: Vec::new(),
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

    pub fn check_crate(&mut self, krate: &HirCrate) -> TypeckResult {
        // Pass 1: collect signatures
        for item in &krate.items {
            self.collect_item(item);
        }

        // Pass 2: check bodies
        for item in &krate.items {
            self.check_item(item);
        }

        // Resolve all infer vars
        let mut expr_types = std::mem::take(&mut self.expr_types);
        for ty in expr_types.values_mut() {
            *ty = self.resolve_ty_full(ty.clone());
        }

        // Also resolve types in generic_call_substs
        let mut generic_call_substs = std::mem::take(&mut self.generic_call_substs);
        for (_, (_, substs)) in generic_call_substs.iter_mut() {
            for ty in substs.iter_mut() {
                *ty = self.resolve_ty_full(ty.clone());
            }
        }

        TypeckResult {
            expr_types,
            struct_defs: std::mem::take(&mut self.struct_defs),
            fn_sigs: self.fn_sigs.clone(),
            errors: std::mem::take(&mut self.errors),
            generic_call_substs,
            generic_fn_defs: std::mem::take(&mut self.generic_fn_defs),
            enum_variants: std::mem::take(&mut self.enum_variant_fields),
        }
    }

    // ── Pass 1: Collect ──

    fn collect_item(&mut self, item: &HirItem) {
        match &item.kind {
            HirItemKind::Fn(f) => {
                // Set up generic params for this function
                let old_generics = std::mem::take(&mut self.current_generic_params);
                let mut n_type_params = 0u32;
                for gp in &f.generics.params {
                    if let HirGenericParam::Type(name, _, _, _) = gp {
                        self.current_generic_params.insert(*name, n_type_params);
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
                    }
                }
                for sub in &ib.items {
                    self.collect_item(sub);
                }
            }
            HirItemKind::ExternBlock(eb) => {
                for sub in &eb.items {
                    self.collect_item(sub);
                }
            }
            _ => {}
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
            _ => {}
        }
    }

    fn check_fn(&mut self, f: &HirFnDef) {
        // Set up generic params for this function's body
        let old_generics = std::mem::take(&mut self.current_generic_params);
        let mut n_type_params = 0u32;
        for gp in &f.generics.params {
            if let HirGenericParam::Type(name, _, _, _) = gp {
                self.current_generic_params.insert(*name, n_type_params);
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
                Literal::ByteString(_) => TyKind::Ref(
                    Box::new(TyKind::Slice(Box::new(TyKind::Uint(UintTy::U8)))),
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
                // Auto-deref references for field access
                let resolved = match &resolved {
                    TyKind::Ref(inner, _) => self.shallow_resolve(inner.as_ref().clone()),
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
                // Find struct DefId by name
                let def_id = if !path.segments.is_empty() {
                    let name = path.segments[0].ident;
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
                self.check_expr(idx);
                match self.shallow_resolve(base_ty) {
                    TyKind::Array(elem, _) | TyKind::Slice(elem) => *elem,
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

                if let TyKind::Adt(def_id, _) = &inner_ty {
                    // Find the type name for this DefId
                    let type_name = self.type_name_to_def.iter()
                        .find(|(_, &did)| did == *def_id)
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

            HirExprKind::Closure(_, _, body, _) => {
                self.check_expr(body);
                self.fresh_infer(InferKind::General)
            }

            HirExprKind::ArrayRepeat(val, count) => {
                let vty = self.check_expr(val);
                self.check_expr(count);
                TyKind::Array(Box::new(vty), 0)
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

            HirExprKind::For(_, iter, body, _) => {
                self.check_expr(iter);
                self.check_block(body);
                TyKind::Unit
            }
        }
    }

    // ── Path type inference ──

    fn infer_path_type(&self, path: &HirPath, expr_id: HirId) -> TyKind {
        if let Some(&def_id) = self.resolve.resolutions.get(&expr_id) {
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
            // Could be a const or static - return Error for now
            return TyKind::Error;
        }
        TyKind::Error
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

            (TyKind::Adt(a, _), TyKind::Adt(b, _)) if a == b => return,

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
                            TyKind::Adt(def_id, vec![])
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
            HirTy::Array(inner, _, _) => {
                TyKind::Array(Box::new(self.hir_ty_to_ty(inner)), 0)
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
            HirTy::Never(_) => TyKind::Never,
            HirTy::Infer(_) => TyKind::Error,
        }
    }
}

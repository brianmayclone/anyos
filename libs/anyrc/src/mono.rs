use crate::hir::*;
use crate::intern::{Interner, Symbol};
use crate::mir::*;
use crate::mir_build::MirBuilder;
use crate::prelude::*;
use crate::resolve::ResolveResult;
use crate::typeck::{FloatTy, IntTy, TyKind, TypeckResult, UintTy};
use anyos_std::collections::{HashMap, HashSet};

/// Mangle a function name with its type arguments
fn mangle_name(base: &str, substs: &[TyKind]) -> String {
    let mut name = base.to_string();
    for ty in substs {
        name.push_str("__");
        name.push_str(&ty_to_mangled(ty));
    }
    name
}

fn ty_to_mangled(ty: &TyKind) -> String {
    match ty {
        TyKind::Bool => "bool".to_string(),
        TyKind::Char => "char".to_string(),
        TyKind::Int(IntTy::I8) => "i8".to_string(),
        TyKind::Int(IntTy::I16) => "i16".to_string(),
        TyKind::Int(IntTy::I32) => "i32".to_string(),
        TyKind::Int(IntTy::I64) => "i64".to_string(),
        TyKind::Int(IntTy::I128) => "i128".to_string(),
        TyKind::Int(IntTy::Isize) => "isize".to_string(),
        TyKind::Uint(UintTy::U8) => "u8".to_string(),
        TyKind::Uint(UintTy::U16) => "u16".to_string(),
        TyKind::Uint(UintTy::U32) => "u32".to_string(),
        TyKind::Uint(UintTy::U64) => "u64".to_string(),
        TyKind::Uint(UintTy::U128) => "u128".to_string(),
        TyKind::Uint(UintTy::Usize) => "usize".to_string(),
        TyKind::Float(FloatTy::F32) => "f32".to_string(),
        TyKind::Float(FloatTy::F64) => "f64".to_string(),
        TyKind::Unit => "unit".to_string(),
        TyKind::Adt(def_id, _) => format!("adt{}", def_id.0),
        TyKind::Ref(inner, _) => format!("ref_{}", ty_to_mangled(inner)),
        _ => "unknown".to_string(),
    }
}

/// Substitute TyKind::Param(i) with concrete types everywhere in a TyKind
fn subst_ty(ty: &TyKind, substs: &[TyKind]) -> TyKind {
    match ty {
        TyKind::Param(idx) => {
            if (*idx as usize) < substs.len() {
                substs[*idx as usize].clone()
            } else {
                ty.clone()
            }
        }
        TyKind::Ref(inner, m) => TyKind::Ref(Box::new(subst_ty(inner, substs)), *m),
        TyKind::RawPtr(inner, m) => TyKind::RawPtr(Box::new(subst_ty(inner, substs)), *m),
        TyKind::Tuple(tys) => TyKind::Tuple(tys.iter().map(|t| subst_ty(t, substs)).collect()),
        TyKind::Array(inner, n) => TyKind::Array(Box::new(subst_ty(inner, substs)), *n),
        TyKind::Slice(inner) => TyKind::Slice(Box::new(subst_ty(inner, substs))),
        _ => ty.clone(),
    }
}

/// Monomorphize all generic function calls.
///
/// Strategy: We rebuild MIR for each unique (generic_fn, type_args) pair using a
/// modified TypeckResult where the fn signature has concrete types. We also
/// rewrite call targets in all MIR bodies to point to the mangled names.
pub fn monomorphize(
    mut mir_bodies: Vec<MirBody>,
    typeck: &TypeckResult,
    interner: &mut Interner,
    hir: &HirCrate,
    resolve: &ResolveResult,
) -> Vec<MirBody> {
    if typeck.generic_fn_defs.is_empty() {
        return mir_bodies;
    }

    // Collect unique instantiations: (def_id, substs) -> mangled_name
    let mut instances: HashMap<(DefId, Vec<TyKind>), String> = HashMap::new();
    // Also build a map: original fn name symbol -> [(substs, mangled_name)]
    // We need this to rewrite call targets.

    let fn_symbols = MirBuilder::collect_qualified_fn_symbols(interner, hir);

    for (_call_id, (def_id, substs)) in &typeck.generic_call_substs {
        let key = (*def_id, substs.clone());
        if !instances.contains_key(&key) {
            // Find the original function name
            if let Some(fn_def) = find_fn_def(hir, *def_id) {
                let base_sym = fn_symbols.get(def_id).copied().unwrap_or(fn_def.name);
                let base_name = interner.resolve(base_sym).to_string();
                let mangled = mangle_name(&base_name, substs);
                instances.insert(key, mangled);
            }
        }
    }

    // Keep the original generic bodies as baseline symbols as well.
    //
    // Cross-crate users currently import an rlib object plus an interface, but
    // not enough HIR/MIR to instantiate a dependency's generic body in the
    // downstream crate. Until that full cross-crate monomorphization exists, a
    // generic function must remain linkable under its base symbol name. Local
    // concrete instances are still emitted below with mangled names.

    // For each unique instantiation, build a specialized MIR body
    for ((def_id, substs), mangled_name) in &instances {
        if let Some(fn_def) = find_fn_def(hir, *def_id) {
            // Create a modified typeck result with concrete types for this instantiation
            let mut specialized_typeck = typeck.clone_for_mono();

            // Substitute the fn signature
            if let Some((param_tys, ret_ty)) = typeck.fn_sigs.get(def_id) {
                let concrete_params: Vec<TyKind> =
                    param_tys.iter().map(|t| subst_ty(t, substs)).collect();
                let concrete_ret = subst_ty(ret_ty, substs);
                specialized_typeck
                    .fn_sigs
                    .insert(*def_id, (concrete_params, concrete_ret));
            }

            // Substitute types in expr_types that contain Param
            for ty in specialized_typeck.expr_types.values_mut() {
                *ty = subst_ty(ty, substs);
            }

            // Build MIR for this specialization
            let mut built =
                MirBuilder::build_fn(interner, resolve, &specialized_typeck, fn_def, &fn_symbols);
            // The main function body is last; any closures come before it
            let mut body = built.pop().expect("build_fn returned empty");
            // Add any closure bodies
            mir_bodies.extend(built);

            // Substitute types in all locals
            for local in &mut body.locals {
                local.ty = subst_ty(&local.ty, substs);
            }

            // Set the mangled name
            let mangled_sym = interner.intern(&mangled_name);
            body.name = mangled_sym;

            mir_bodies.push(body);
        }
    }

    // Rewrite call targets in all bodies: if a call goes to a generic fn,
    // replace the FnItem symbol with the mangled name
    // Build map: (original_name_str, substs) -> mangled_name_symbol
    let mut name_to_def: HashMap<Symbol, DefId> = HashMap::new();
    for (def_id, _) in &typeck.generic_fn_defs {
        if let Some(fn_def) = find_fn_def(hir, *def_id) {
            name_to_def.insert(fn_def.name, *def_id);
            if let Some(&qualified) = fn_symbols.get(def_id) {
                name_to_def.insert(qualified, *def_id);
            }
        }
    }

    // Build per-call-site map: call expr HirId -> mangled symbol
    let mut call_mangled: HashMap<DefId, Vec<(Vec<TyKind>, Symbol)>> = HashMap::new();
    for ((def_id, substs), mangled_name) in &instances {
        let mangled_sym = interner.intern(&mangled_name);
        call_mangled
            .entry(*def_id)
            .or_default()
            .push((substs.clone(), mangled_sym));
    }

    // For each MIR body, rewrite Call terminators that reference generic fns
    // The problem: we need to know which substs each call site uses.
    // The MIR Call terminator stores a FnItem(symbol) but not the call HirId.
    // We need a different approach: for each call to a generic fn, the MIR stores
    // the original fn name. We need to look up the call-site's substs.
    //
    // Simpler approach: walk through typeck.generic_call_substs which maps
    // call-site HirId -> (DefId, substs). But MIR doesn't track HirIds.
    //
    // Alternative: since each generic fn call in MIR has the original fn name,
    // and we know the call's argument types from the MIR locals, we can match.
    // But this is complex.
    //
    // Simplest approach: for functions that have only one instantiation,
    // just replace the name. For multiple instantiations, we need to match
    // by examining the argument types in the MIR Call terminator.
    //
    // Even simpler: we can match by the types of the call arguments in the MIR.
    // The dest local's type tells us what the return type is, which combined with
    // the callee name, uniquely identifies the instantiation.

    for body in &mut mir_bodies {
        for bb in &mut body.basic_blocks {
            if let Terminator::Call {
                func, args, dest, ..
            } = &mut bb.terminator
            {
                if let Operand::Constant(c) = func {
                    if let ConstValue::FnItem(sym) = &c.value {
                        if let Some(&def_id) = name_to_def.get(sym) {
                            // This call targets a generic fn. Find the right instantiation.
                            if let Some(entries) = call_mangled.get(&def_id) {
                                if entries.len() == 1 {
                                    c.value = ConstValue::FnItem(entries[0].1);
                                } else {
                                    // Match by return type
                                    let ret_ty = &body.locals[dest.local.0].ty;
                                    for (substs, mangled_sym) in entries {
                                        if let Some((_, orig_ret)) = typeck.fn_sigs.get(&def_id) {
                                            let concrete_ret = subst_ty(orig_ret, substs);
                                            if &concrete_ret == ret_ty {
                                                c.value = ConstValue::FnItem(*mangled_sym);
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    mir_bodies
}

fn collect_fn_symbols(hir: &HirCrate) -> HashMap<DefId, Symbol> {
    let mut out = HashMap::new();
    for item in &hir.items {
        collect_fn_symbols_from_item(item, &mut out);
    }
    out
}

fn collect_fn_symbols_from_item(item: &HirItem, out: &mut HashMap<DefId, Symbol>) {
    match &item.kind {
        HirItemKind::Fn(f) => {
            out.insert(f.def_id, f.name);
            if let Some(body) = &f.body {
                collect_fn_symbols_from_block(body, out);
            }
        }
        HirItemKind::Impl(ib) => {
            for sub in &ib.items {
                collect_fn_symbols_from_item(sub, out);
            }
        }
        HirItemKind::ExternBlock(eb) => {
            for sub in &eb.items {
                collect_fn_symbols_from_item(sub, out);
            }
        }
        HirItemKind::Mod(m) => {
            if let Some(sub_items) = &m.items {
                for sub in sub_items {
                    collect_fn_symbols_from_item(sub, out);
                }
            }
        }
        _ => {}
    }
}

fn collect_fn_symbols_from_block(block: &HirBlock, out: &mut HashMap<DefId, Symbol>) {
    for stmt in &block.stmts {
        match stmt {
            HirStmt::Let(_, _, _, Some(expr), _) => collect_fn_symbols_from_expr(expr, out),
            HirStmt::Expr(expr) | HirStmt::Semi(expr, _) => collect_fn_symbols_from_expr(expr, out),
            HirStmt::Item(item) => collect_fn_symbols_from_item(item, out),
            _ => {}
        }
    }
}

fn collect_fn_symbols_from_expr(expr: &HirExpr, out: &mut HashMap<DefId, Symbol>) {
    match &expr.kind {
        HirExprKind::Call(callee, args) => {
            collect_fn_symbols_from_expr(callee, out);
            for arg in args {
                collect_fn_symbols_from_expr(arg, out);
            }
        }
        HirExprKind::MethodCall(receiver, _, _, args) => {
            collect_fn_symbols_from_expr(receiver, out);
            for arg in args {
                collect_fn_symbols_from_expr(arg, out);
            }
        }
        HirExprKind::Block(block) | HirExprKind::Unsafe(block) => {
            collect_fn_symbols_from_block(block, out);
        }
        HirExprKind::Closure(_, _, body, _) => {
            collect_fn_symbols_from_expr(body, out);
        }
        _ => {}
    }
}

fn find_fn_def<'a>(hir: &'a HirCrate, target: DefId) -> Option<&'a HirFnDef> {
    for item in &hir.items {
        if let Some(f) = find_fn_in_item(item, target) {
            return Some(f);
        }
    }
    None
}

fn find_fn_in_item<'a>(item: &'a HirItem, target: DefId) -> Option<&'a HirFnDef> {
    match &item.kind {
        HirItemKind::Fn(f) if f.def_id == target => Some(f),
        HirItemKind::Fn(f) => f
            .body
            .as_ref()
            .and_then(|body| find_fn_in_block(body, target)),
        HirItemKind::Impl(ib) => {
            for sub in &ib.items {
                if let Some(f) = find_fn_in_item(sub, target) {
                    return Some(f);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_fn_in_block<'a>(block: &'a HirBlock, target: DefId) -> Option<&'a HirFnDef> {
    for stmt in &block.stmts {
        match stmt {
            HirStmt::Item(item) => {
                if let Some(f) = find_fn_in_item(item, target) {
                    return Some(f);
                }
            }
            HirStmt::Let(_, _, _, Some(expr), _) => {
                if let Some(f) = find_fn_in_expr(expr, target) {
                    return Some(f);
                }
            }
            HirStmt::Expr(expr) | HirStmt::Semi(expr, _) => {
                if let Some(f) = find_fn_in_expr(expr, target) {
                    return Some(f);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_fn_in_expr<'a>(expr: &'a HirExpr, target: DefId) -> Option<&'a HirFnDef> {
    match &expr.kind {
        HirExprKind::Block(block) | HirExprKind::Unsafe(block) => find_fn_in_block(block, target),
        HirExprKind::Call(callee, args) => find_fn_in_expr(callee, target)
            .or_else(|| args.iter().find_map(|arg| find_fn_in_expr(arg, target))),
        HirExprKind::MethodCall(receiver, _, _, args) => find_fn_in_expr(receiver, target)
            .or_else(|| args.iter().find_map(|arg| find_fn_in_expr(arg, target))),
        HirExprKind::If(cond, then_block, else_expr) => find_fn_in_expr(cond, target)
            .or_else(|| find_fn_in_block(then_block, target))
            .or_else(|| {
                else_expr
                    .as_ref()
                    .and_then(|else_expr| find_fn_in_expr(else_expr, target))
            }),
        HirExprKind::Match(scrutinee, arms) => find_fn_in_expr(scrutinee, target).or_else(|| {
            arms.iter().find_map(|arm| {
                arm.guard
                    .as_ref()
                    .and_then(|guard| find_fn_in_expr(guard, target))
                    .or_else(|| find_fn_in_expr(&arm.body, target))
            })
        }),
        HirExprKind::Struct(_, fields, base) => fields
            .iter()
            .find_map(|field| find_fn_in_expr(&field.value, target))
            .or_else(|| base.as_ref().and_then(|base| find_fn_in_expr(base, target))),
        HirExprKind::Array(elems) | HirExprKind::Tuple(elems) => {
            elems.iter().find_map(|elem| find_fn_in_expr(elem, target))
        }
        HirExprKind::Closure(_, _, body, _) => find_fn_in_expr(body, target),
        _ => None,
    }
}

fn is_generic_fn_body(
    body: &MirBody,
    generic_def_ids: &HashSet<DefId>,
    typeck: &TypeckResult,
    interner: &Interner,
) -> bool {
    let name = interner.resolve(body.name);
    for (&def_id, _) in &typeck.fn_sigs {
        if generic_def_ids.contains(&def_id) {
            // We need to check if this body corresponds to this generic fn
            // The MirBody name is the fn name symbol
            if typeck.fn_sigs.get(&def_id).is_some() {
                // Check by looking up the fn name from the generic_fn_defs
                for item_def_id in generic_def_ids {
                    if typeck.fn_sigs.contains_key(item_def_id) {
                        // We can't easily get the name from DefId alone.
                        // Use a different approach: check if body.name matches any generic fn name
                    }
                }
            }
        }
    }
    // Simpler: check if any generic fn def has this name as its symbol in fn_sigs
    // Actually, we need the HirFnDef to get the name. Let's just store it differently.
    // For now, use a simpler check: does the MIR body have any TyKind::Param in its locals?
    body.locals.iter().any(|l| contains_param(&l.ty))
}

fn contains_param(ty: &TyKind) -> bool {
    match ty {
        TyKind::Param(_) => true,
        TyKind::Ref(inner, _)
        | TyKind::RawPtr(inner, _)
        | TyKind::Slice(inner)
        | TyKind::Array(inner, _) => contains_param(inner),
        TyKind::Tuple(tys) => tys.iter().any(contains_param),
        _ => false,
    }
}

impl TypeckResult {
    /// Clone the essential data needed for monomorphization
    pub fn clone_for_mono(&self) -> TypeckResult {
        TypeckResult {
            expr_types: self.expr_types.clone(),
            local_types: self.local_types.clone(),
            struct_defs: self.struct_defs.clone(),
            fn_sigs: self.fn_sigs.clone(),
            errors: Vec::new(),
            generic_call_substs: HashMap::new(),
            generic_fn_defs: HashMap::new(),
            enum_variants: self.enum_variants.clone(),
            const_values: self.const_values.clone(),
            static_defs: self.static_defs.clone(),
            closure_defs: self.closure_defs.clone(),
            trait_methods: self.trait_methods.clone(),
            trait_impls: self.trait_impls.clone(),
            trait_names: self.trait_names.clone(),
            type_def_to_name: self.type_def_to_name.clone(),
            assoc_types: self.assoc_types.clone(),
            generic_param_bounds: self.generic_param_bounds.clone(),
            trait_default_methods: self.trait_default_methods.clone(),
            copy_types: self.copy_types.clone(),
        }
    }
}

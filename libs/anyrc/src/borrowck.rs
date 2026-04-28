use crate::prelude::*;
use anyos_std::collections::{HashMap, HashSet};

use crate::diagnostics::{Diagnostic, Level, Span};
use crate::hir::DefId;
use crate::intern::{Interner, Symbol};
use crate::mir::*;
use crate::typeck::TyKind;

pub struct BorrowckResult {
    pub errors: Vec<Diagnostic>,
}

#[derive(Clone)]
struct ActiveBorrow {
    holder: Local,
    kind: BorrowKind,
    place: Place,
    span: Span,
    temporary: bool,
}

fn places_conflict(a: &Place, b: &Place) -> bool {
    if a.local != b.local {
        return false;
    }
    for (a_proj, b_proj) in a.projections.iter().zip(b.projections.iter()) {
        match (a_proj, b_proj) {
            (Projection::Field(a_idx), Projection::Field(b_idx)) if a_idx != b_idx => {
                return false;
            }
            _ => {}
        }
    }
    true
}

pub fn check_borrows(
    body: &MirBody,
    interner: &Interner,
    struct_defs: &HashMap<DefId, Vec<(Symbol, TyKind)>>,
    enum_variants: &HashMap<DefId, Vec<(Symbol, Vec<TyKind>)>>,
    copy_types: &HashSet<DefId>,
) -> BorrowckResult {
    let mut errors = Vec::new();
    let mut moved: HashSet<usize> = HashSet::new();
    let mut moved_at_block_entry: HashMap<usize, HashSet<usize>> = HashMap::new();
    let mut borrows: Vec<ActiveBorrow> = Vec::new();

    for (block_idx, block) in body.basic_blocks.iter().enumerate() {
        if let Some(entry_moved) = moved_at_block_entry.remove(&block_idx) {
            moved = entry_moved;
        }
        for stmt in &block.statements {
            match &stmt.kind {
                StatementKind::Assign(place, rvalue) => {
                    let assign_local = place.local.0;
                    let self_reborrow_assignment = matches!(
                        rvalue,
                        Rvalue::Ref(_, borrowed_place) if borrowed_place.local.0 == assign_local
                    );
                    let local_ref_reassignment = matches!(
                        body.locals.get(assign_local).map(|local| &local.ty),
                        Some(TyKind::Ref(_, _) | TyKind::RawPtr(_, _))
                    );
                    if local_ref_reassignment {
                        borrows.retain(|b| b.holder.0 != assign_local);
                    }

                    // Check operands in rvalue aren't moved
                    check_rvalue_operands(
                        rvalue,
                        &moved,
                        &body.locals,
                        interner,
                        &mut errors,
                        stmt.span,
                    );

                    // Check borrow-related rules
                    match rvalue {
                        Rvalue::Ref(kind, borrowed_place) => {
                            let assigned_to_temp = body
                                .locals
                                .get(assign_local)
                                .and_then(|local| local.name)
                                .is_none();
                            let temporary_projected_borrow =
                                assigned_to_temp && !borrowed_place.projections.is_empty();
                            if !self_reborrow_assignment && !temporary_projected_borrow {
                                match kind {
                                    BorrowKind::Mutable => {
                                        // No existing borrow (shared or mutable) on this local
                                        for b in &borrows {
                                            if places_conflict(&b.place, borrowed_place) {
                                                errors.push(Diagnostic::new(
                                                    Level::Error,
                                                    &format!(
                                                        "cannot borrow as mutable: already borrowed at {:?}",
                                                        b.span
                                                    ),
                                                    stmt.span,
                                                ));
                                                break;
                                            }
                                        }
                                        borrows.push(ActiveBorrow {
                                            holder: Local(assign_local),
                                            kind: BorrowKind::Mutable,
                                            place: borrowed_place.clone(),
                                            span: stmt.span,
                                            temporary: assigned_to_temp,
                                        });
                                    }
                                    BorrowKind::Shared => {
                                        // No active mutable borrow on this local
                                        for b in &borrows {
                                            if places_conflict(&b.place, borrowed_place)
                                                && b.kind == BorrowKind::Mutable
                                                && !b.temporary
                                            {
                                                errors.push(Diagnostic::new(
                                                    Level::Error,
                                                    &format!(
                                                        "cannot borrow as shared: already mutably borrowed at {:?}",
                                                        b.span
                                                    ),
                                                    stmt.span,
                                                ));
                                                break;
                                            }
                                        }
                                        borrows.push(ActiveBorrow {
                                            holder: Local(assign_local),
                                            kind: BorrowKind::Shared,
                                            place: borrowed_place.clone(),
                                            span: stmt.span,
                                            temporary: assigned_to_temp,
                                        });
                                    }
                                }
                            } else {
                                borrows.retain(|b| b.holder.0 != assign_local);
                            }
                        }
                        _ => {}
                    }

                    if local_ref_reassignment {
                        if let Rvalue::Use(Operand::Copy(src) | Operand::Move(src)) = rvalue {
                            if let Some(existing) =
                                borrows.iter().find(|b| b.holder.0 == src.local.0).cloned()
                            {
                                let assigned_to_temp = body
                                    .locals
                                    .get(assign_local)
                                    .and_then(|local| local.name)
                                    .is_none();
                                borrows.push(ActiveBorrow {
                                    holder: Local(assign_local),
                                    temporary: assigned_to_temp,
                                    ..existing
                                });
                            }
                        }
                    }

                    // If assigning to a place that has active borrows, error
                    for b in &borrows {
                        if places_conflict(&b.place, place)
                            && !self_reborrow_assignment
                            && !local_ref_reassignment
                            && !b.temporary
                        {
                            errors.push(Diagnostic::new(
                                Level::Error,
                                &format!(
                                    "cannot assign to borrowed value: borrow active at {:?}",
                                    b.span
                                ),
                                stmt.span,
                            ));
                            break;
                        }
                    }
                    if !matches!(rvalue, Rvalue::Ref(_, _)) {
                        end_temporary_borrows(&mut borrows, &body.locals, body.arg_count);
                    }

                    // Track moves in rvalue. Moves into compiler temporaries are often
                    // the MIR builder's way to preserve a scrutinee for pattern tests;
                    // user-visible moves are still recorded through named locals and calls.
                    let assigns_to_named_local = body
                        .locals
                        .get(assign_local)
                        .map(|local| local.name.is_some())
                        .unwrap_or(false);
                    if assigns_to_named_local {
                        record_moves(
                            rvalue,
                            &mut moved,
                            &body.locals,
                            struct_defs,
                            enum_variants,
                            copy_types,
                        );
                    }
                }
                StatementKind::StorageDead(local) => {
                    // End borrows and moves for this local
                    borrows.retain(|b| b.holder.0 != local.0 && b.place.local.0 != local.0);
                    moved.remove(&local.0);
                }
                _ => {}
            }
        }

        // Check operands in terminator
        match &block.terminator {
            Terminator::Call { args, func, .. } => {
                check_operand_not_moved(
                    func,
                    &moved,
                    &body.locals,
                    interner,
                    &mut errors,
                    body.span,
                );
                for arg in args {
                    check_operand_not_moved(
                        arg,
                        &moved,
                        &body.locals,
                        interner,
                        &mut errors,
                        body.span,
                    );
                }
                record_operand_move(
                    func,
                    &mut moved,
                    &body.locals,
                    struct_defs,
                    enum_variants,
                    copy_types,
                );
                for arg in args {
                    record_operand_move(
                        arg,
                        &mut moved,
                        &body.locals,
                        struct_defs,
                        enum_variants,
                        copy_types,
                    );
                }
                // Temporary borrows for call args end when the call returns
                borrows.clear();
                if let Terminator::Call { target, .. } = &block.terminator {
                    merge_moved_entry(&mut moved_at_block_entry, target.0, &moved);
                }
            }
            Terminator::Goto(target) => {
                end_temporary_borrows(&mut borrows, &body.locals, body.arg_count);
                merge_moved_entry(&mut moved_at_block_entry, target.0, &moved);
            }
            Terminator::SwitchInt {
                targets, default, ..
            } => {
                end_temporary_borrows(&mut borrows, &body.locals, body.arg_count);
                for (_, target) in targets {
                    merge_moved_entry(&mut moved_at_block_entry, target.0, &moved);
                }
                merge_moved_entry(&mut moved_at_block_entry, default.0, &moved);
            }
            Terminator::Return | Terminator::Unreachable => {
                end_temporary_borrows(&mut borrows, &body.locals, body.arg_count);
            }
            _ => {}
        }
    }

    BorrowckResult { errors }
}

fn merge_moved_entry(
    moved_at_block_entry: &mut HashMap<usize, HashSet<usize>>,
    block: usize,
    moved: &HashSet<usize>,
) {
    if let Some(existing) = moved_at_block_entry.get_mut(&block) {
        for local in moved {
            existing.insert(*local);
        }
    } else {
        moved_at_block_entry.insert(block, moved.clone());
    }
}

fn end_temporary_borrows(
    borrows: &mut Vec<ActiveBorrow>,
    _locals: &[LocalDecl],
    _arg_count: usize,
) {
    borrows.retain(|b| !b.temporary);
}

fn check_rvalue_operands(
    rvalue: &Rvalue,
    moved: &HashSet<usize>,
    locals: &[LocalDecl],
    interner: &Interner,
    errors: &mut Vec<Diagnostic>,
    span: Span,
) {
    match rvalue {
        Rvalue::Use(op) => check_operand_not_moved(op, moved, locals, interner, errors, span),
        Rvalue::BinaryOp(_, l, r) => {
            check_operand_not_moved(l, moved, locals, interner, errors, span);
            check_operand_not_moved(r, moved, locals, interner, errors, span);
        }
        Rvalue::UnaryOp(_, op) => {
            check_operand_not_moved(op, moved, locals, interner, errors, span)
        }
        Rvalue::Cast(op, _) => check_operand_not_moved(op, moved, locals, interner, errors, span),
        Rvalue::Aggregate(_, ops) => {
            for op in ops {
                check_operand_not_moved(op, moved, locals, interner, errors, span);
            }
        }
        Rvalue::Ref(_, _) | Rvalue::Discriminant(_) | Rvalue::Len(_) | Rvalue::MakeVtable(_) => {}
    }
}

fn check_operand_not_moved(
    op: &Operand,
    moved: &HashSet<usize>,
    locals: &[LocalDecl],
    interner: &Interner,
    errors: &mut Vec<Diagnostic>,
    span: Span,
) {
    match op {
        Operand::Move(place) | Operand::Copy(place) | Operand::Ref(place, _) => {
            if moved.contains(&place.local.0) {
                let name = locals[place.local.0]
                    .name
                    .map(|sym| interner.resolve(sym))
                    .unwrap_or("value");
                errors.push(Diagnostic::new(
                    Level::Error,
                    &format!("use of moved {}", name),
                    span,
                ));
            }
        }
        Operand::Constant(_) => {}
    }
}

fn record_moves(
    rvalue: &Rvalue,
    moved: &mut HashSet<usize>,
    locals: &[LocalDecl],
    struct_defs: &HashMap<DefId, Vec<(Symbol, TyKind)>>,
    enum_variants: &HashMap<DefId, Vec<(Symbol, Vec<TyKind>)>>,
    copy_types: &HashSet<DefId>,
) {
    let mut check_op = |op: &Operand| {
        record_operand_move(op, moved, locals, struct_defs, enum_variants, copy_types);
    };
    match rvalue {
        Rvalue::Use(op) => check_op(op),
        Rvalue::BinaryOp(_, l, r) => {
            check_op(l);
            check_op(r);
        }
        Rvalue::UnaryOp(_, op) => check_op(op),
        Rvalue::Cast(op, _) => check_op(op),
        Rvalue::Aggregate(_, ops) => {
            for op in ops {
                check_op(op);
            }
        }
        _ => {}
    }
}

fn record_operand_move(
    op: &Operand,
    moved: &mut HashSet<usize>,
    locals: &[LocalDecl],
    struct_defs: &HashMap<DefId, Vec<(Symbol, TyKind)>>,
    enum_variants: &HashMap<DefId, Vec<(Symbol, Vec<TyKind>)>>,
    copy_types: &HashSet<DefId>,
) {
    if let Operand::Move(place) = op {
        if place
            .projections
            .iter()
            .any(|proj| matches!(proj, Projection::Deref))
        {
            return;
        }
        let ty = &locals[place.local.0].ty;
        if matches!(ty, TyKind::Ref(_, _) | TyKind::RawPtr(_, _)) {
            return;
        }
        if !is_copy_type(ty, struct_defs, enum_variants, copy_types) {
            moved.insert(place.local.0);
        }
    }
}

fn is_copy_type(
    ty: &TyKind,
    struct_defs: &HashMap<DefId, Vec<(Symbol, TyKind)>>,
    enum_variants: &HashMap<DefId, Vec<(Symbol, Vec<TyKind>)>>,
    copy_types: &HashSet<DefId>,
) -> bool {
    match ty {
        TyKind::Bool | TyKind::Char | TyKind::Str => true,
        TyKind::Int(_) | TyKind::Uint(_) | TyKind::Float(_) => true,
        TyKind::Ref(_, crate::ast::Mutability::Immutable) => true,
        TyKind::Ref(_, crate::ast::Mutability::Mut) => false,
        TyKind::RawPtr(_, _) => true,
        TyKind::Unit | TyKind::Never | TyKind::Error | TyKind::Infer(_) | TyKind::Param(_) => true,
        TyKind::Projection(_, _, _) | TyKind::DynTrait(_) => true,
        TyKind::FnDef(_, _) | TyKind::FnPtr(_, _) => true,
        TyKind::Adt(def_id, _) => {
            if copy_types.contains(def_id) {
                return true;
            }
            // A struct is Copy if all its fields are Copy
            if let Some(fields) = struct_defs.get(def_id) {
                fields
                    .iter()
                    .all(|(_, fty)| is_copy_type(fty, struct_defs, enum_variants, copy_types))
            } else if let Some(variants) = enum_variants.get(def_id) {
                variants.iter().all(|(_, fields)| {
                    fields
                        .iter()
                        .all(|fty| is_copy_type(fty, struct_defs, enum_variants, copy_types))
                })
            } else {
                // External/intrinsic ADTs do not carry trait metadata yet, so
                // move checking has to defer instead of rejecting generic code.
                def_id.0 >= 10000
            }
        }
        TyKind::Tuple(elems) => elems
            .iter()
            .all(|e| is_copy_type(e, struct_defs, enum_variants, copy_types)),
        TyKind::Array(inner, _) => is_copy_type(inner, struct_defs, enum_variants, copy_types),
        _ => false,
    }
}

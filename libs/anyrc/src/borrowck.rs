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

struct ActiveBorrow {
    holder: Local,
    kind: BorrowKind,
    place: Place,
    span: Span,
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

pub fn check_borrows(body: &MirBody, interner: &Interner, struct_defs: &HashMap<DefId, Vec<(Symbol, TyKind)>>) -> BorrowckResult {
    let mut errors = Vec::new();
    let mut moved: HashSet<usize> = HashSet::new();
    let mut borrows: Vec<ActiveBorrow> = Vec::new();

    for block in &body.basic_blocks {
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
                    check_rvalue_operands(rvalue, &moved, &body.locals, &mut errors, stmt.span);

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
                                        });
                                    }
                                    BorrowKind::Shared => {
                                        // No active mutable borrow on this local
                                        for b in &borrows {
                                            if places_conflict(&b.place, borrowed_place)
                                                && b.kind == BorrowKind::Mutable
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
                                        });
                                    }
                                }
                            } else {
                                borrows.retain(|b| b.holder.0 != assign_local);
                            }
                        }
                        _ => {}
                    }

                    // If assigning to a place that has active borrows, error
                    for b in &borrows {
                        if places_conflict(&b.place, place)
                            && !self_reborrow_assignment
                            && !local_ref_reassignment
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

                    // Track moves in rvalue
                    record_moves(rvalue, &mut moved, &body.locals, struct_defs);
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
                check_operand_not_moved(func, &moved, &body.locals, &mut errors, body.span);
                for arg in args {
                    check_operand_not_moved(arg, &moved, &body.locals, &mut errors, body.span);
                }
                record_operand_move(func, &mut moved, &body.locals, struct_defs);
                for arg in args {
                    record_operand_move(arg, &mut moved, &body.locals, struct_defs);
                }
                // Temporary borrows for call args end when the call returns
                borrows.clear();
            }
            Terminator::Goto(_)
            | Terminator::SwitchInt { .. }
            | Terminator::Return
            | Terminator::Unreachable => {
                end_temporary_borrows(&mut borrows, &body.locals, body.arg_count);
            }
            _ => {}
        }
    }

    BorrowckResult { errors }
}

fn end_temporary_borrows(
    borrows: &mut Vec<ActiveBorrow>,
    locals: &[LocalDecl],
    arg_count: usize,
) {
    borrows.retain(|b| {
        locals
            .get(b.holder.0)
            .map(|local| local.name.is_some() || b.holder.0 < arg_count)
            .unwrap_or(false)
    });
}

fn check_rvalue_operands(
    rvalue: &Rvalue,
    moved: &HashSet<usize>,
    locals: &[LocalDecl],
    errors: &mut Vec<Diagnostic>,
    span: Span,
) {
    match rvalue {
        Rvalue::Use(op) => check_operand_not_moved(op, moved, locals, errors, span),
        Rvalue::BinaryOp(_, l, r) => {
            check_operand_not_moved(l, moved, locals, errors, span);
            check_operand_not_moved(r, moved, locals, errors, span);
        }
        Rvalue::UnaryOp(_, op) => check_operand_not_moved(op, moved, locals, errors, span),
        Rvalue::Cast(op, _) => check_operand_not_moved(op, moved, locals, errors, span),
        Rvalue::Aggregate(_, ops) => {
            for op in ops {
                check_operand_not_moved(op, moved, locals, errors, span);
            }
        }
        Rvalue::Ref(_, _) | Rvalue::Discriminant(_) | Rvalue::Len(_) | Rvalue::MakeVtable(_) => {}
    }
}

fn check_operand_not_moved(
    op: &Operand,
    moved: &HashSet<usize>,
    locals: &[LocalDecl],
    errors: &mut Vec<Diagnostic>,
    span: Span,
) {
    match op {
        Operand::Move(place) | Operand::Copy(place) | Operand::Ref(place, _) => {
            if moved.contains(&place.local.0) {
                let name = locals[place.local.0]
                    .name
                    .map(|_| "value")
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

fn record_moves(rvalue: &Rvalue, moved: &mut HashSet<usize>, locals: &[LocalDecl], struct_defs: &HashMap<DefId, Vec<(Symbol, TyKind)>>) {
    let mut check_op = |op: &Operand| {
        record_operand_move(op, moved, locals, struct_defs);
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
) {
    if let Operand::Move(place) = op {
        if place.projections.iter().any(|proj| matches!(proj, Projection::Deref)) {
            return;
        }
        let ty = &locals[place.local.0].ty;
        if !is_copy_type(ty, struct_defs) {
            moved.insert(place.local.0);
        }
    }
}

fn is_copy_type(ty: &TyKind, struct_defs: &HashMap<DefId, Vec<(Symbol, TyKind)>>) -> bool {
    match ty {
        TyKind::Bool | TyKind::Char => true,
        TyKind::Int(_) | TyKind::Uint(_) | TyKind::Float(_) => true,
        TyKind::Ref(_, crate::ast::Mutability::Immutable) => true,
        TyKind::Ref(_, crate::ast::Mutability::Mut) => false,
        TyKind::RawPtr(_, _) => true,
        TyKind::Unit | TyKind::Never | TyKind::Error | TyKind::Infer(_) | TyKind::Param(_) => true,
        TyKind::Projection(_, _, _) | TyKind::DynTrait(_) => true,
        TyKind::FnDef(_, _) | TyKind::FnPtr(_, _) => true,
        TyKind::Adt(def_id, _) => {
            // A struct is Copy if all its fields are Copy
            if let Some(fields) = struct_defs.get(def_id) {
                fields.iter().all(|(_, fty)| is_copy_type(fty, struct_defs))
            } else {
                // External/intrinsic ADTs do not carry trait metadata yet, so
                // move checking has to defer instead of rejecting generic code.
                def_id.0 >= 10000
            }
        }
        TyKind::Tuple(elems) => elems.iter().all(|e| is_copy_type(e, struct_defs)),
        TyKind::Array(inner, _) => is_copy_type(inner, struct_defs),
        _ => false,
    }
}

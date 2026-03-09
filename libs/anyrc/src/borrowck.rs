use std::collections::{HashMap, HashSet};

use crate::diagnostics::{Diagnostic, Level, Span};
use crate::hir::DefId;
use crate::intern::{Interner, Symbol};
use crate::mir::*;
use crate::typeck::TyKind;

pub struct BorrowckResult {
    pub errors: Vec<Diagnostic>,
}

struct ActiveBorrow {
    kind: BorrowKind,
    place_local: Local,
    span: Span,
}

pub fn check_borrows(body: &MirBody, interner: &Interner, struct_defs: &HashMap<DefId, Vec<(Symbol, TyKind)>>) -> BorrowckResult {
    let mut errors = Vec::new();
    let mut moved: HashSet<usize> = HashSet::new();
    let mut borrows: Vec<ActiveBorrow> = Vec::new();

    for block in &body.basic_blocks {
        for stmt in &block.statements {
            match &stmt.kind {
                StatementKind::Assign(place, rvalue) => {
                    // Check operands in rvalue aren't moved
                    check_rvalue_operands(rvalue, &moved, &body.locals, &mut errors, stmt.span);

                    // Check borrow-related rules
                    match rvalue {
                        Rvalue::Ref(kind, borrowed_place) => {
                            let local_idx = borrowed_place.local.0;
                            match kind {
                                BorrowKind::Mutable => {
                                    // No existing borrow (shared or mutable) on this local
                                    for b in &borrows {
                                        if b.place_local.0 == local_idx {
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
                                        kind: BorrowKind::Mutable,
                                        place_local: Local(local_idx),
                                        span: stmt.span,
                                    });
                                }
                                BorrowKind::Shared => {
                                    // No active mutable borrow on this local
                                    for b in &borrows {
                                        if b.place_local.0 == local_idx
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
                                        kind: BorrowKind::Shared,
                                        place_local: Local(local_idx),
                                        span: stmt.span,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }

                    // If assigning to a place that has active borrows, error
                    let assign_local = place.local.0;
                    for b in &borrows {
                        if b.place_local.0 == assign_local {
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
                    borrows.retain(|b| b.place_local.0 != local.0);
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
                    // Record moves from call args
                    if let Operand::Move(place) = arg {
                        let ty = &body.locals[place.local.0].ty;
                        if !is_copy_type(ty, struct_defs) {
                            moved.insert(place.local.0);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    BorrowckResult { errors }
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
        Operand::Move(place) | Operand::Copy(place) => {
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
        if let Operand::Move(place) = op {
            let ty = &locals[place.local.0].ty;
            if !is_copy_type(ty, struct_defs) {
                moved.insert(place.local.0);
            }
        }
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

fn is_copy_type(ty: &TyKind, struct_defs: &HashMap<DefId, Vec<(Symbol, TyKind)>>) -> bool {
    match ty {
        TyKind::Bool | TyKind::Char => true,
        TyKind::Int(_) | TyKind::Uint(_) | TyKind::Float(_) => true,
        TyKind::Ref(_, crate::ast::Mutability::Immutable) => true,
        TyKind::Ref(_, crate::ast::Mutability::Mut) => false,
        TyKind::RawPtr(_, _) => true,
        TyKind::Unit | TyKind::Never | TyKind::Error => true,
        TyKind::FnDef(_, _) | TyKind::FnPtr(_, _) => true,
        TyKind::Adt(def_id, _) => {
            // A struct is Copy if all its fields are Copy
            if let Some(fields) = struct_defs.get(def_id) {
                fields.iter().all(|(_, fty)| is_copy_type(fty, struct_defs))
            } else {
                // Unknown struct (e.g., enum or external) — conservatively non-Copy
                false
            }
        }
        TyKind::Tuple(elems) => elems.iter().all(|e| is_copy_type(e, struct_defs)),
        TyKind::Array(inner, _) => is_copy_type(inner, struct_defs),
        _ => false,
    }
}

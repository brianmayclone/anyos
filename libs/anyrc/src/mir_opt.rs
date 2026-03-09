use std::collections::HashSet;

use crate::mir::*;

pub fn optimize(body: &mut MirBody) {
    const_prop(body);
    dead_code_elim(body);
    simplify_cfg(body);
}

/// Constant propagation: if a local is assigned exactly one constant and never
/// reassigned, replace all uses with that constant.
fn const_prop(body: &mut MirBody) {
    // 1. Find locals assigned exactly once, to a constant, with no other assignments.
    let mut assignments: Vec<Option<Constant>> = vec![None; body.locals.len()];
    let mut multi_assigned: HashSet<usize> = HashSet::new();

    for bb in &body.basic_blocks {
        for stmt in &bb.statements {
            if let StatementKind::Assign(place, rvalue) = &stmt.kind {
                if !place.projections.is_empty() {
                    continue;
                }
                let local = place.local.0;
                if multi_assigned.contains(&local) {
                    continue;
                }
                if let Rvalue::Use(Operand::Constant(c)) = rvalue {
                    if assignments[local].is_some() {
                        // assigned more than once
                        multi_assigned.insert(local);
                        assignments[local] = None;
                    } else {
                        assignments[local] = Some(c.clone());
                    }
                } else {
                    // assigned something non-constant
                    multi_assigned.insert(local);
                    assignments[local] = None;
                }
            }
        }
        // Check terminators for assignments (Call dest)
        if let Terminator::Call { dest, .. } = &bb.terminator {
            if dest.projections.is_empty() {
                multi_assigned.insert(dest.local.0);
                assignments[dest.local.0] = None;
            }
        }
    }

    // Also mark function arguments as non-propagatable
    // locals 0 = return place, 1..=arg_count = arguments
    for i in 0..=body.arg_count {
        multi_assigned.insert(i);
        assignments[i] = None;
    }

    // 2. Replace uses
    for bb in &mut body.basic_blocks {
        for stmt in &mut bb.statements {
            if let StatementKind::Assign(_, rvalue) = &mut stmt.kind {
                replace_operands_in_rvalue(rvalue, &assignments);
            }
        }
        replace_operands_in_terminator(&mut bb.terminator, &assignments);
    }
}

fn replace_operands_in_rvalue(rvalue: &mut Rvalue, constants: &[Option<Constant>]) {
    match rvalue {
        Rvalue::Use(op) => replace_operand(op, constants),
        Rvalue::BinaryOp(_, a, b) => {
            replace_operand(a, constants);
            replace_operand(b, constants);
        }
        Rvalue::UnaryOp(_, op) => replace_operand(op, constants),
        Rvalue::Cast(op, _) => replace_operand(op, constants),
        Rvalue::Aggregate(_, ops) => {
            for op in ops {
                replace_operand(op, constants);
            }
        }
        Rvalue::Ref(_, _) | Rvalue::Discriminant(_) | Rvalue::Len(_) | Rvalue::MakeVtable(_) => {}
    }
}

fn replace_operand(op: &mut Operand, constants: &[Option<Constant>]) {
    let local = match op {
        Operand::Copy(p) | Operand::Move(p) if p.projections.is_empty() => p.local.0,
        _ => return,
    };
    if let Some(Some(c)) = constants.get(local) {
        *op = Operand::Constant(c.clone());
    }
}

fn replace_operands_in_terminator(term: &mut Terminator, constants: &[Option<Constant>]) {
    match term {
        Terminator::SwitchInt { operand, .. } => replace_operand(operand, constants),
        Terminator::Call { func, args, .. } => {
            replace_operand(func, constants);
            for arg in args {
                replace_operand(arg, constants);
            }
        }
        Terminator::Goto(_) | Terminator::Return | Terminator::Unreachable => {}
    }
}

/// Dead code elimination: remove assignments to locals that are never read.
fn dead_code_elim(body: &mut MirBody) {
    let used = collect_used_locals(body);

    for bb in &mut body.basic_blocks {
        bb.statements.retain(|stmt| {
            match &stmt.kind {
                StatementKind::Assign(place, _) => {
                    if place.projections.is_empty() {
                        let local = place.local.0;
                        // Keep return place (_0) and arguments
                        if local == 0 || local <= body.arg_count {
                            return true;
                        }
                        used.contains(&local)
                    } else {
                        true
                    }
                }
                StatementKind::Nop => false,
                _ => true,
            }
        });
    }
}

fn collect_used_locals(body: &MirBody) -> HashSet<usize> {
    let mut used = HashSet::new();

    for bb in &body.basic_blocks {
        for stmt in &bb.statements {
            if let StatementKind::Assign(place, rvalue) = &stmt.kind {
                // The place's projections may use locals
                collect_place_reads(place, &mut used);
                collect_rvalue_reads(rvalue, &mut used);
            }
        }
        collect_terminator_reads(&bb.terminator, &mut used);
    }
    used
}

fn collect_place_reads(place: &Place, used: &mut HashSet<usize>) {
    for proj in &place.projections {
        if let Projection::Index(local) = proj {
            used.insert(local.0);
        }
        // Field/Deref projections mean the local itself is read
        used.insert(place.local.0);
    }
}

fn collect_operand_reads(op: &Operand, used: &mut HashSet<usize>) {
    match op {
        Operand::Copy(p) | Operand::Move(p) => {
            used.insert(p.local.0);
            for proj in &p.projections {
                if let Projection::Index(l) = proj {
                    used.insert(l.0);
                }
            }
        }
        Operand::Constant(_) => {}
    }
}

fn collect_rvalue_reads(rvalue: &Rvalue, used: &mut HashSet<usize>) {
    match rvalue {
        Rvalue::Use(op) => collect_operand_reads(op, used),
        Rvalue::Ref(_, place) => {
            used.insert(place.local.0);
        }
        Rvalue::BinaryOp(_, a, b) => {
            collect_operand_reads(a, used);
            collect_operand_reads(b, used);
        }
        Rvalue::UnaryOp(_, op) => collect_operand_reads(op, used),
        Rvalue::Cast(op, _) => collect_operand_reads(op, used),
        Rvalue::Aggregate(_, ops) => {
            for op in ops {
                collect_operand_reads(op, used);
            }
        }
        Rvalue::Discriminant(p) | Rvalue::Len(p) => {
            used.insert(p.local.0);
        }
        Rvalue::MakeVtable(_) => {}
    }
}

fn collect_terminator_reads(term: &Terminator, used: &mut HashSet<usize>) {
    match term {
        Terminator::SwitchInt { operand, .. } => collect_operand_reads(operand, used),
        Terminator::Call { func, args, .. } => {
            collect_operand_reads(func, used);
            for arg in args {
                collect_operand_reads(arg, used);
            }
        }
        Terminator::Return => {
            used.insert(0); // return place
        }
        Terminator::Goto(_) | Terminator::Unreachable => {}
    }
}

/// SimplifyCFG: remove empty goto-only blocks by redirecting predecessors.
fn simplify_cfg(body: &mut MirBody) {
    if body.basic_blocks.is_empty() {
        return;
    }

    // Build a forwarding map: if block i is just a Goto(j) with no statements,
    // forward i → ultimate target.
    let len = body.basic_blocks.len();
    let mut forward: Vec<usize> = (0..len).collect();

    // Chase chains
    for i in 0..len {
        let bb = &body.basic_blocks[i];
        if bb.statements.is_empty() {
            if let Terminator::Goto(target) = &bb.terminator {
                forward[i] = target.0;
            }
        }
    }

    // Resolve transitive forwarding
    for i in 0..len {
        let mut target = forward[i];
        let mut visited = HashSet::new();
        visited.insert(i);
        while target != forward[target] && !visited.contains(&target) {
            visited.insert(target);
            target = forward[target];
        }
        forward[i] = target;
    }

    // Rewrite all block references
    for bb in &mut body.basic_blocks {
        rewrite_terminator_targets(&mut bb.terminator, &forward);
    }
}

fn rewrite_terminator_targets(term: &mut Terminator, forward: &[usize]) {
    match term {
        Terminator::Goto(target) => {
            target.0 = forward[target.0];
        }
        Terminator::SwitchInt { targets, default, .. } => {
            for (_, target) in targets {
                target.0 = forward[target.0];
            }
            default.0 = forward[default.0];
        }
        Terminator::Call { target, .. } => {
            target.0 = forward[target.0];
        }
        Terminator::Return | Terminator::Unreachable => {}
    }
}

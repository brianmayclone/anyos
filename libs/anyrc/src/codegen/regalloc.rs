use crate::hir::DefId;
use crate::mir::{
    Local, MirAsmOperandKind, MirBody, Operand, Place, Projection, Rvalue, StatementKind,
    Terminator,
};
use crate::prelude::*;
use crate::typeck::TyKind;
use anyos_std::collections::HashMap;

pub struct RegAlloc {
    /// Stack offset for each local (negative from RBP)
    pub stack_slots: Vec<i32>,
    /// Size in bytes for each local
    pub local_sizes: Vec<i32>,
    /// Total stack frame size (positive number)
    pub frame_size: i32,
}

/// Map from struct DefId to stack storage size in bytes.
pub type StructSizes = HashMap<DefId, usize>;

/// Map from struct DefId to byte offset of each field.
pub type StructFieldOffsets = HashMap<DefId, Vec<i32>>;

/// Map from struct DefId to the type of each field.
pub type StructFieldTypes = HashMap<DefId, Vec<TyKind>>;

fn align_to(value: i32, align: i32) -> i32 {
    ((value + align - 1) / align) * align
}

/// Return the natural in-memory size for a type.
pub fn ty_layout_size(ty: &TyKind, struct_sizes: &StructSizes) -> i32 {
    match ty {
        TyKind::Bool => 1,
        TyKind::Char => 4,
        TyKind::Int(crate::typeck::IntTy::I8) | TyKind::Uint(crate::typeck::UintTy::U8) => 1,
        TyKind::Int(crate::typeck::IntTy::I16) | TyKind::Uint(crate::typeck::UintTy::U16) => 2,
        TyKind::Int(crate::typeck::IntTy::I32)
        | TyKind::Uint(crate::typeck::UintTy::U32)
        | TyKind::Float(crate::typeck::FloatTy::F32) => 4,
        TyKind::Int(crate::typeck::IntTy::I128) | TyKind::Uint(crate::typeck::UintTy::U128) => 16,
        TyKind::Tuple(elems) => {
            if elems.is_empty() {
                0
            } else {
                elems.iter().map(|ty| ty_size(ty, struct_sizes)).sum()
            }
        }
        TyKind::Adt(def_id, _) => struct_sizes.get(def_id).copied().unwrap_or(8) as i32,
        TyKind::Array(elem, len) if *len > (1usize << 30) => {
            ty_layout_size(elem, struct_sizes).max(1)
        }
        TyKind::Array(elem, len) => ty_layout_size(elem, struct_sizes).max(1) * (*len as i32),
        TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _)
            if matches!(
                inner.as_ref(),
                TyKind::Slice(_) | TyKind::Str | TyKind::DynTrait(_)
            ) =>
        {
            16
        }
        TyKind::Slice(_) | TyKind::Str | TyKind::DynTrait(_) => 16,
        TyKind::Unit | TyKind::Never => 0,
        _ => 8,
    }
}

/// Return the size in bytes reserved for a type on the stack.
pub fn ty_size(ty: &TyKind, struct_sizes: &StructSizes) -> i32 {
    align_to(ty_layout_size(ty, struct_sizes).max(1), 8)
}

pub fn allocate(body: &MirBody, struct_sizes: &StructSizes) -> RegAlloc {
    // First pass: determine sizes, looking at Aggregate assignments to find struct sizes
    let mut local_sizes: Vec<i32> = body
        .locals
        .iter()
        .map(|l| ty_size(&l.ty, struct_sizes))
        .collect();

    // Scan statements for Aggregate assignments to learn actual field counts
    for bb in &body.basic_blocks {
        for stmt in &bb.statements {
            if let crate::mir::StatementKind::Assign(place, rvalue) = &stmt.kind {
                if place.projections.is_empty() {
                    match rvalue {
                        crate::mir::Rvalue::Aggregate(_, operands)
                            if operands.len() > 1
                                && !matches!(
                                    body.locals[place.local.0].ty,
                                    TyKind::Array(_, _)
                                ) =>
                        {
                            let needed = operands.len() as i32 * 8;
                            if needed > local_sizes[place.local.0] {
                                local_sizes[place.local.0] = needed;
                            }
                        }
                        crate::mir::Rvalue::MakeVtable(fn_names) => {
                            let needed = fn_names.len() as i32 * 8;
                            if needed > local_sizes[place.local.0] {
                                local_sizes[place.local.0] = needed;
                            }
                        }
                        // Propagate sizes through Use(Copy/Move) assignments
                        crate::mir::Rvalue::Use(
                            crate::mir::Operand::Copy(src) | crate::mir::Operand::Move(src),
                        ) if src.projections.is_empty() => {
                            let src_size = local_sizes[src.local.0];
                            if src_size > local_sizes[place.local.0] {
                                local_sizes[place.local.0] = src_size;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Propagate sizes iteratively (in case of chains: a = b, c = a)
    let mut changed = true;
    while changed {
        changed = false;
        for bb in &body.basic_blocks {
            for stmt in &bb.statements {
                if let crate::mir::StatementKind::Assign(
                    place,
                    crate::mir::Rvalue::Use(
                        crate::mir::Operand::Copy(src) | crate::mir::Operand::Move(src),
                    ),
                ) = &stmt.kind
                {
                    if place.projections.is_empty() && src.projections.is_empty() {
                        let src_size = local_sizes[src.local.0];
                        if src_size > local_sizes[place.local.0] {
                            local_sizes[place.local.0] = src_size;
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    let (stack_slots, used_size) = allocate_stack_slots(body, &local_sizes);
    // Align to 16 so every emitted call keeps the SysV stack contract.
    let frame_size = ((used_size + 15) / 16) * 16;
    RegAlloc {
        stack_slots,
        local_sizes,
        frame_size,
    }
}

#[derive(Clone, Copy)]
struct LiveInterval {
    local: usize,
    first: usize,
    last: usize,
    size: i32,
}

#[derive(Clone, Copy)]
struct ActiveSlot {
    last: usize,
    offset: i32,
    size: i32,
}

fn allocate_stack_slots(body: &MirBody, local_sizes: &[i32]) -> (Vec<i32>, i32) {
    let intervals = compute_live_intervals(body, local_sizes);
    let mut stack_slots = vec![0; local_sizes.len()];
    let mut active: Vec<ActiveSlot> = Vec::new();
    let mut free: Vec<(i32, i32)> = Vec::new(); // (slot_size, offset)
    let mut next_offset = 0i32;
    let mut max_used = 0i32;

    for interval in intervals {
        let mut i = 0;
        while i < active.len() {
            if active[i].last < interval.first {
                let expired = active.remove(i);
                free.push((expired.size, expired.offset));
            } else {
                i += 1;
            }
        }

        let free_idx = free
            .iter()
            .enumerate()
            .filter(|(_, (size, _))| *size >= interval.size)
            .min_by_key(|(_, (size, _))| *size)
            .map(|(idx, _)| idx);

        let (slot_size, offset) = if let Some(idx) = free_idx {
            free.swap_remove(idx)
        } else {
            next_offset -= interval.size;
            max_used = max_used.max(-next_offset);
            (interval.size, next_offset)
        };

        stack_slots[interval.local] = offset;
        active.push(ActiveSlot {
            last: interval.last,
            offset,
            size: slot_size,
        });
    }

    (stack_slots, max_used)
}

fn compute_live_intervals(body: &MirBody, local_sizes: &[i32]) -> Vec<LiveInterval> {
    let mut first = vec![usize::MAX; body.locals.len()];
    let mut last = vec![0usize; body.locals.len()];
    let mut address_taken = vec![false; body.locals.len()];
    let mut pos = 0usize;

    // Return place and incoming arguments exist at function entry. The return
    // place is read by every explicit return terminator.
    mark_local(Local(0), 0, &mut first, &mut last);
    for local in 1..=body.arg_count {
        mark_local(Local(local), 0, &mut first, &mut last);
    }

    for bb in &body.basic_blocks {
        for stmt in &bb.statements {
            pos += 1;
            match &stmt.kind {
                StatementKind::Assign(place, rvalue) => {
                    mark_place(place, pos, &mut first, &mut last);
                    mark_rvalue(rvalue, pos, &mut first, &mut last, &mut address_taken);
                }
                StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                    mark_local(*local, pos, &mut first, &mut last);
                }
                StatementKind::InlineAsm { operands, .. } => {
                    for operand in operands {
                        match &operand.kind {
                            MirAsmOperandKind::In(op) => {
                                mark_operand(op, pos, &mut first, &mut last, &mut address_taken);
                            }
                            MirAsmOperandKind::Out(place) => {
                                if let Some(place) = place {
                                    mark_place(place, pos, &mut first, &mut last);
                                }
                            }
                            MirAsmOperandKind::InOut(op, place) => {
                                mark_operand(op, pos, &mut first, &mut last, &mut address_taken);
                                if let Some(place) = place {
                                    mark_place(place, pos, &mut first, &mut last);
                                }
                            }
                        }
                    }
                }
                StatementKind::Nop => {}
            }
        }
        pos += 1;
        mark_terminator(
            &bb.terminator,
            pos,
            &mut first,
            &mut last,
            &mut address_taken,
        );
    }

    let end = pos.saturating_add(1);
    let liveness = compute_block_liveness(body);
    last[0] = end;
    for idx in 0..body.locals.len() {
        if first[idx] == usize::MAX {
            continue;
        }
        for block_idx in 0..body.basic_blocks.len() {
            if liveness.live_in[block_idx][idx] {
                first[idx] = first[idx].min(liveness.block_start[block_idx]);
            }
            if liveness.live_out[block_idx][idx] {
                last[idx] = last[idx].max(liveness.block_end[block_idx]);
            }
        }
        if address_taken[idx] {
            last[idx] = end;
        }
    }

    let mut intervals = Vec::new();
    for local in 0..body.locals.len() {
        if first[local] == usize::MAX {
            continue;
        }
        intervals.push(LiveInterval {
            local,
            first: first[local],
            last: last[local].max(first[local]),
            size: local_sizes[local].max(1),
        });
    }
    intervals.sort_by_key(|interval| (interval.first, core::cmp::Reverse(interval.size)));
    intervals
}

struct BlockLiveness {
    live_in: Vec<Vec<bool>>,
    live_out: Vec<Vec<bool>>,
    block_start: Vec<usize>,
    block_end: Vec<usize>,
}

fn compute_block_liveness(body: &MirBody) -> BlockLiveness {
    let block_count = body.basic_blocks.len();
    let local_count = body.locals.len();
    let mut block_use = vec![vec![false; local_count]; block_count];
    let mut block_def = vec![vec![false; local_count]; block_count];
    let mut block_start = vec![0usize; block_count];
    let mut block_end = vec![0usize; block_count];
    let mut pos = 0usize;

    for (block_idx, bb) in body.basic_blocks.iter().enumerate() {
        block_start[block_idx] = pos.saturating_add(1);
        for stmt in &bb.statements {
            collect_statement_use_def(
                &stmt.kind,
                &mut block_use[block_idx],
                &mut block_def[block_idx],
            );
            pos += 1;
        }
        collect_terminator_use_def(
            &bb.terminator,
            &mut block_use[block_idx],
            &mut block_def[block_idx],
        );
        pos += 1;
        block_end[block_idx] = pos;
    }

    let successors = body
        .basic_blocks
        .iter()
        .map(|bb| terminator_successors(&bb.terminator))
        .collect::<Vec<_>>();

    let mut live_in = vec![vec![false; local_count]; block_count];
    let mut live_out = vec![vec![false; local_count]; block_count];
    let mut changed = true;
    while changed {
        changed = false;
        for block_idx in (0..block_count).rev() {
            let mut new_out = vec![false; local_count];
            for succ in &successors[block_idx] {
                if succ.0 >= block_count {
                    continue;
                }
                for local in 0..local_count {
                    new_out[local] |= live_in[succ.0][local];
                }
            }

            let mut new_in = block_use[block_idx].clone();
            for local in 0..local_count {
                if new_out[local] && !block_def[block_idx][local] {
                    new_in[local] = true;
                }
            }

            if new_out != live_out[block_idx] || new_in != live_in[block_idx] {
                live_out[block_idx] = new_out;
                live_in[block_idx] = new_in;
                changed = true;
            }
        }
    }

    BlockLiveness {
        live_in,
        live_out,
        block_start,
        block_end,
    }
}

fn terminator_successors(term: &Terminator) -> Vec<crate::mir::BlockId> {
    match term {
        Terminator::Goto(target) => vec![*target],
        Terminator::SwitchInt {
            targets, default, ..
        } => {
            let mut out = targets
                .iter()
                .map(|(_, target)| *target)
                .collect::<Vec<_>>();
            out.push(*default);
            out
        }
        Terminator::Call { target, .. } => vec![*target],
        Terminator::Return | Terminator::Unreachable => Vec::new(),
    }
}

fn collect_statement_use_def(kind: &StatementKind, uses: &mut [bool], defs: &mut [bool]) {
    match kind {
        StatementKind::Assign(place, rvalue) => {
            collect_rvalue_uses(rvalue, uses, defs);
            collect_place_def(place, uses, defs);
        }
        StatementKind::StorageLive(local) => collect_local_def(*local, defs),
        StatementKind::StorageDead(_) | StatementKind::Nop => {}
        StatementKind::InlineAsm { operands, .. } => {
            for operand in operands {
                match &operand.kind {
                    MirAsmOperandKind::In(op) => collect_operand_use(op, uses, defs),
                    MirAsmOperandKind::Out(place) => {
                        if let Some(place) = place {
                            collect_place_def(place, uses, defs);
                        }
                    }
                    MirAsmOperandKind::InOut(op, place) => {
                        collect_operand_use(op, uses, defs);
                        if let Some(place) = place {
                            collect_place_def(place, uses, defs);
                        }
                    }
                }
            }
        }
    }
}

fn collect_terminator_use_def(term: &Terminator, uses: &mut [bool], defs: &mut [bool]) {
    match term {
        Terminator::Goto(_) | Terminator::Unreachable => {}
        Terminator::SwitchInt { operand, .. } => collect_operand_use(operand, uses, defs),
        Terminator::Call {
            func, args, dest, ..
        } => {
            collect_operand_use(func, uses, defs);
            for arg in args {
                collect_operand_use(arg, uses, defs);
            }
            collect_place_def(dest, uses, defs);
        }
        Terminator::Return => collect_local_use(Local(0), uses, defs),
    }
}

fn collect_rvalue_uses(rvalue: &Rvalue, uses: &mut [bool], defs: &mut [bool]) {
    match rvalue {
        Rvalue::Use(op) | Rvalue::Cast(op, _) | Rvalue::UnaryOp(_, op) => {
            collect_operand_use(op, uses, defs);
        }
        Rvalue::Ref(_, place) | Rvalue::Discriminant(place) | Rvalue::Len(place) => {
            collect_place_use(place, uses, defs);
        }
        Rvalue::BinaryOp(_, lhs, rhs) => {
            collect_operand_use(lhs, uses, defs);
            collect_operand_use(rhs, uses, defs);
        }
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                collect_operand_use(operand, uses, defs);
            }
        }
        Rvalue::MakeVtable(_) => {}
    }
}

fn collect_operand_use(op: &Operand, uses: &mut [bool], defs: &mut [bool]) {
    match op {
        Operand::Copy(place) | Operand::Move(place) | Operand::Ref(place, _) => {
            collect_place_use(place, uses, defs);
        }
        Operand::Constant(_) => {}
    }
}

fn collect_place_use(place: &Place, uses: &mut [bool], defs: &mut [bool]) {
    collect_local_use(place.local, uses, defs);
    for projection in &place.projections {
        if let Projection::Index(local) = projection {
            collect_local_use(*local, uses, defs);
        }
    }
}

fn collect_place_def(place: &Place, uses: &mut [bool], defs: &mut [bool]) {
    for projection in &place.projections {
        if let Projection::Index(local) = projection {
            collect_local_use(*local, uses, defs);
        }
    }
    if place.projections.is_empty() {
        collect_local_def(place.local, defs);
    } else {
        collect_local_use(place.local, uses, defs);
    }
}

fn collect_local_use(local: Local, uses: &mut [bool], defs: &[bool]) {
    let idx = local.0;
    if idx < uses.len() && !defs[idx] {
        uses[idx] = true;
    }
}

fn collect_local_def(local: Local, defs: &mut [bool]) {
    if local.0 < defs.len() {
        defs[local.0] = true;
    }
}

fn mark_terminator(
    terminator: &Terminator,
    pos: usize,
    first: &mut [usize],
    last: &mut [usize],
    address_taken: &mut [bool],
) {
    match terminator {
        Terminator::Goto(_) | Terminator::Unreachable => {}
        Terminator::SwitchInt { operand, .. } => {
            mark_operand(operand, pos, first, last, address_taken);
        }
        Terminator::Call {
            func, args, dest, ..
        } => {
            mark_operand(func, pos, first, last, address_taken);
            for arg in args {
                mark_operand(arg, pos, first, last, address_taken);
            }
            mark_place(dest, pos, first, last);
        }
        Terminator::Return => {
            mark_local(Local(0), pos, first, last);
        }
    }
}

fn mark_rvalue(
    rvalue: &Rvalue,
    pos: usize,
    first: &mut [usize],
    last: &mut [usize],
    address_taken: &mut [bool],
) {
    match rvalue {
        Rvalue::Use(op) | Rvalue::Cast(op, _) | Rvalue::UnaryOp(_, op) => {
            mark_operand(op, pos, first, last, address_taken);
        }
        Rvalue::Ref(_, place) => {
            mark_address_taken(place, pos, first, last, address_taken);
        }
        Rvalue::BinaryOp(_, lhs, rhs) => {
            mark_operand(lhs, pos, first, last, address_taken);
            mark_operand(rhs, pos, first, last, address_taken);
        }
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                mark_operand(operand, pos, first, last, address_taken);
            }
        }
        Rvalue::Discriminant(place) | Rvalue::Len(place) => {
            mark_place(place, pos, first, last);
        }
        Rvalue::MakeVtable(_) => {}
    }
}

fn mark_operand(
    operand: &Operand,
    pos: usize,
    first: &mut [usize],
    last: &mut [usize],
    address_taken: &mut [bool],
) {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => {
            mark_place(place, pos, first, last);
        }
        Operand::Ref(place, _) => {
            mark_address_taken(place, pos, first, last, address_taken);
        }
        Operand::Constant(_) => {}
    }
}

fn mark_address_taken(
    place: &Place,
    pos: usize,
    first: &mut [usize],
    last: &mut [usize],
    address_taken: &mut [bool],
) {
    mark_place(place, pos, first, last);
    if place.local.0 < address_taken.len() {
        address_taken[place.local.0] = true;
    }
}

fn mark_place(place: &Place, pos: usize, first: &mut [usize], last: &mut [usize]) {
    mark_local(place.local, pos, first, last);
    for projection in &place.projections {
        if let Projection::Index(local) = projection {
            mark_local(*local, pos, first, last);
        }
    }
}

fn mark_local(local: Local, pos: usize, first: &mut [usize], last: &mut [usize]) {
    let idx = local.0;
    if idx >= first.len() {
        return;
    }
    first[idx] = first[idx].min(pos);
    last[idx] = last[idx].max(pos);
}

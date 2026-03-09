use crate::mir::MirBody;
use crate::typeck::TyKind;

pub struct RegAlloc {
    /// Stack offset for each local (negative from RBP)
    pub stack_slots: Vec<i32>,
    /// Size in bytes for each local
    pub local_sizes: Vec<i32>,
    /// Total stack frame size (positive number)
    pub frame_size: i32,
}

/// Return the size in bytes for a type on the stack (each field/element = 8 bytes).
pub fn ty_size(ty: &TyKind) -> i32 {
    match ty {
        TyKind::Tuple(elems) => {
            if elems.is_empty() { 8 } else { elems.len() as i32 * 8 }
        }
        TyKind::Adt(_, _) => {
            // We don't know field count from TyKind alone; handled via MIR Aggregate.
            // The MIR builder stores the aggregate into a local whose type is Adt.
            // We'll use a conservative default of 8 and let aggregate emit handle multi-slot.
            // Actually, we need to know the field count. We'll over-allocate later via patch.
            8
        }
        TyKind::Array(_, len) => (*len as i32).max(1) * 8,
        _ => 8,
    }
}

pub fn allocate(body: &MirBody) -> RegAlloc {
    // First pass: determine sizes, looking at Aggregate assignments to find struct sizes
    let mut local_sizes: Vec<i32> = body.locals.iter().map(|l| ty_size(&l.ty)).collect();

    // Scan statements for Aggregate assignments to learn actual field counts
    for bb in &body.basic_blocks {
        for stmt in &bb.statements {
            if let crate::mir::StatementKind::Assign(place, rvalue) = &stmt.kind {
                if place.projections.is_empty() {
                    match rvalue {
                        crate::mir::Rvalue::Aggregate(_, operands) if operands.len() > 1 => {
                            let needed = operands.len() as i32 * 8;
                            if needed > local_sizes[place.local.0] {
                                local_sizes[place.local.0] = needed;
                            }
                        }
                        // Propagate sizes through Use(Copy/Move) assignments
                        crate::mir::Rvalue::Use(crate::mir::Operand::Copy(src) | crate::mir::Operand::Move(src)) if src.projections.is_empty() => {
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
                if let crate::mir::StatementKind::Assign(place, crate::mir::Rvalue::Use(
                    crate::mir::Operand::Copy(src) | crate::mir::Operand::Move(src)
                )) = &stmt.kind {
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

    let mut offset = 0i32;
    let mut stack_slots = Vec::new();
    for size in &local_sizes {
        offset -= size;
        stack_slots.push(offset);
    }
    // Align to 16
    let frame_size = ((-offset + 15) / 16) * 16;
    RegAlloc { stack_slots, local_sizes, frame_size }
}

use crate::prelude::*;
use anyos_std::collections::HashMap;
use crate::hir::DefId;
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
        TyKind::Array(elem, len) if *len > (1usize << 30) => ty_layout_size(elem, struct_sizes).max(1),
        TyKind::Array(elem, len) => ty_layout_size(elem, struct_sizes).max(1) * (*len as i32),
        TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _)
            if matches!(inner.as_ref(), TyKind::Slice(_) | TyKind::Str | TyKind::DynTrait(_)) =>
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
    let mut local_sizes: Vec<i32> = body.locals.iter().map(|l| ty_size(&l.ty, struct_sizes)).collect();

    // Scan statements for Aggregate assignments to learn actual field counts
    for bb in &body.basic_blocks {
        for stmt in &bb.statements {
            if let crate::mir::StatementKind::Assign(place, rvalue) = &stmt.kind {
                if place.projections.is_empty() {
                    match rvalue {
                        crate::mir::Rvalue::Aggregate(_, operands)
                            if operands.len() > 1
                                && !matches!(body.locals[place.local.0].ty, TyKind::Array(_, _)) =>
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

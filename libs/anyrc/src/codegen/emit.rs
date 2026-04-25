use crate::prelude::*;
use crate::ast::BinOp;
use crate::codegen::regalloc::{self, RegAlloc, StructFieldOffsets, StructFieldTypes, StructSizes};
use crate::codegen::x86asm::{CondCode, Label, Reg, Relocation, X86Assembler};
use crate::hir::DefId;
use crate::intern::Interner;
use crate::mir::*;
use crate::typeck::TyKind;

/// System V AMD64 argument registers
const ARG_REGS: [Reg; 6] = [Reg::RDI, Reg::RSI, Reg::RDX, Reg::RCX, Reg::R8, Reg::R9];

pub struct CodeEmitter<'a> {
    asm: X86Assembler,
    alloc: &'a RegAlloc,
    body: &'a MirBody,
    interner: &'a Interner,
    struct_sizes: &'a StructSizes,
    field_offsets: &'a StructFieldOffsets,
    field_types: &'a StructFieldTypes,
    block_labels: Vec<Label>,
}

impl<'a> CodeEmitter<'a> {
    pub fn emit_fn(
        body: &MirBody,
        alloc: &RegAlloc,
        interner: &Interner,
        struct_sizes: &StructSizes,
        field_offsets: &StructFieldOffsets,
        field_types: &StructFieldTypes,
    ) -> (Vec<u8>, Vec<Relocation>) {
        let mut asm = X86Assembler::new();

        // Create labels for each basic block
        let block_labels: Vec<Label> = (0..body.basic_blocks.len())
            .map(|_| asm.new_label())
            .collect();

        let mut emitter = CodeEmitter {
            asm,
            alloc,
            body,
            interner,
            struct_sizes,
            field_offsets,
            field_types,
            block_labels,
        };

        emitter.emit_prologue();
        emitter.store_args_to_stack();

        for (i, bb) in body.basic_blocks.iter().enumerate() {
            emitter.asm.bind_label(emitter.block_labels[i]);
            for stmt in &bb.statements {
                emitter.emit_statement(stmt);
            }
            emitter.emit_terminator(&bb.terminator, interner);
        }

        emitter.asm.resolve_fixups();

        let relocations = core::mem::take(&mut emitter.asm.relocations);
        (emitter.asm.code().to_vec(), relocations)
    }

    /// Get the byte offset of a field within its parent struct.
    /// Walks the projection chain up to the Field to determine the struct type.
    fn substitute_params(ty: &TyKind, substs: &[TyKind]) -> TyKind {
        match ty {
            TyKind::Param(idx) => substs.get(*idx as usize).cloned().unwrap_or_else(|| ty.clone()),
            TyKind::Ref(inner, m) => TyKind::Ref(Box::new(Self::substitute_params(inner, substs)), *m),
            TyKind::RawPtr(inner, m) => TyKind::RawPtr(Box::new(Self::substitute_params(inner, substs)), *m),
            TyKind::Tuple(items) => {
                TyKind::Tuple(items.iter().map(|item| Self::substitute_params(item, substs)).collect())
            }
            TyKind::Array(inner, len) => TyKind::Array(Box::new(Self::substitute_params(inner, substs)), *len),
            TyKind::Slice(inner) => TyKind::Slice(Box::new(Self::substitute_params(inner, substs))),
            TyKind::Adt(def_id, args) => {
                TyKind::Adt(*def_id, args.iter().map(|arg| Self::substitute_params(arg, substs)).collect())
            }
            TyKind::FnDef(def_id, args) => {
                TyKind::FnDef(*def_id, args.iter().map(|arg| Self::substitute_params(arg, substs)).collect())
            }
            TyKind::FnPtr(params, ret) => TyKind::FnPtr(
                params.iter().map(|param| Self::substitute_params(param, substs)).collect(),
                Box::new(Self::substitute_params(ret, substs)),
            ),
            TyKind::Projection(self_ty, trait_def, assoc) => TyKind::Projection(
                Box::new(Self::substitute_params(self_ty, substs)),
                *trait_def,
                *assoc,
            ),
            _ => ty.clone(),
        }
    }

    fn adt_field_ty(&self, def_id: DefId, substs: &[TyKind], field_idx: usize) -> TyKind {
        self.field_types
            .get(&def_id)
            .and_then(|fields| fields.get(field_idx))
            .map(|ty| Self::substitute_params(ty, substs))
            .unwrap_or(TyKind::Error)
    }

    fn field_byte_offset(&self, place: &Place, field_proj_index: usize) -> i32 {
        let field_idx = match &place.projections[field_proj_index] {
            Projection::Field(idx) => *idx,
            _ => return 0,
        };
        // Walk type through projections up to (but not including) the Field
        let mut ty = self.body.locals[place.local.0].ty.clone();
        for proj in &place.projections[..field_proj_index] {
            match proj {
                Projection::Deref => {
                    ty = match ty {
                        TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => *inner,
                        _ => ty,
                    };
                }
                Projection::Field(idx) => {
                    ty = match &ty {
                        TyKind::Adt(def_id, substs) => self.adt_field_ty(*def_id, substs, *idx),
                        TyKind::Tuple(elems) => {
                            elems.get(*idx).cloned().unwrap_or(TyKind::Error)
                        }
                        _ => TyKind::Error,
                    };
                }
                Projection::Index(_) => {
                    ty = match ty {
                        TyKind::Array(elem, _) => *elem,
                        _ => TyKind::Error,
                    };
                }
            }
        }
        // Now ty is the struct type at the point of the Field projection
        match &ty {
            TyKind::Adt(def_id, _) => {
                if let Some(offsets) = self.field_offsets.get(def_id) {
                    offsets.get(field_idx).copied().unwrap_or(field_idx as i32 * 8)
                } else {
                    field_idx as i32 * 8
                }
            }
            _ => field_idx as i32 * 8,
        }
    }

    fn slots_for_size(size: i32) -> usize {
        ((size.max(1) + 7) / 8) as usize
    }

    fn projection_ty_before(&self, place: &Place, projection_index: usize) -> TyKind {
        let mut ty = self.body.locals[place.local.0].ty.clone();
        for proj in &place.projections[..projection_index] {
            match proj {
                Projection::Deref => {
                    ty = match ty {
                        TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => *inner,
                        _ => TyKind::Error,
                    };
                }
                Projection::Field(idx) => {
                    ty = match ty {
                        TyKind::Tuple(elems) => elems.get(*idx).cloned().unwrap_or(TyKind::Error),
                        TyKind::Adt(def_id, substs) => self.adt_field_ty(def_id, &substs, *idx),
                        _ => TyKind::Error,
                    };
                }
                Projection::Index(_) => {
                    ty = match ty {
                        TyKind::Array(elem, _) => *elem,
                        _ => TyKind::Error,
                    };
                }
            }
        }
        ty
    }

    fn projection_value_ty(&self, place: &Place) -> TyKind {
        let mut ty = self.body.locals[place.local.0].ty.clone();
        for proj in &place.projections {
            match proj {
                Projection::Deref => {
                    ty = match ty {
                        TyKind::Ref(inner, _) | TyKind::RawPtr(inner, _) => *inner,
                        _ => TyKind::Error,
                    };
                }
                Projection::Field(idx) => {
                    ty = match ty {
                        TyKind::Tuple(elems) => elems.get(*idx).cloned().unwrap_or(TyKind::Error),
                        TyKind::Adt(def_id, substs) => self.adt_field_ty(def_id, &substs, *idx),
                        _ => TyKind::Error,
                    };
                }
                Projection::Index(_) => {
                    ty = match ty {
                        TyKind::Array(elem, _) => *elem,
                        _ => TyKind::Error,
                    };
                }
            }
        }
        ty
    }

    fn index_stride(&self, place: &Place, projection_index: usize) -> i32 {
        match self.projection_ty_before(place, projection_index) {
            TyKind::Array(elem, _) => regalloc::ty_layout_size(&elem, self.struct_sizes).max(1),
            _ => 8,
        }
    }

    fn place_value_size(&self, place: &Place) -> i32 {
        if place.projections.is_empty() {
            self.alloc.local_sizes[place.local.0]
        } else {
            regalloc::ty_layout_size(&self.projection_value_ty(place), self.struct_sizes).max(1)
        }
    }

    fn emit_prologue(&mut self) {
        self.asm.push(Reg::RBP);
        self.asm.mov_rr(Reg::RBP, Reg::RSP);
        if self.alloc.frame_size > 0 {
            self.asm.sub_ri(Reg::RSP, self.alloc.frame_size);
        }
    }

    fn emit_epilogue(&mut self) {
        self.asm.mov_rr(Reg::RSP, Reg::RBP);
        self.asm.pop(Reg::RBP);
        self.asm.ret();
    }

    fn store_args_to_stack(&mut self) {
        // Locals: 0 = return place, 1..=arg_count = arguments
        // Each arg may span multiple slots (structs), consuming multiple registers.
        let mut reg_idx = 0;
        for i in 0..self.body.arg_count {
            let local = i + 1; // arguments start at local 1
            if local < self.alloc.stack_slots.len() {
                let slot = self.alloc.stack_slots[local];
                let n_slots = Self::slots_for_size(self.alloc.local_sizes[local]);
                for s in 0..n_slots {
                    if reg_idx < ARG_REGS.len() {
                        self.asm.mov_mr(Reg::RBP, slot + (s as i32) * 8, ARG_REGS[reg_idx]);
                        reg_idx += 1;
                    }
                }
            }
        }
    }

    fn operand_slot_count(&self, op: &Operand) -> usize {
        match op {
            Operand::Copy(place) | Operand::Move(place) | Operand::Ref(place, _) => {
                if place.projections.is_empty() {
                    Self::slots_for_size(self.alloc.local_sizes[place.local.0])
                } else {
                    1
                }
            }
            Operand::Constant(_) => 1,
        }
    }

    fn load_operand(&mut self, op: &Operand, dst: Reg) {
        match op {
            Operand::Copy(place) | Operand::Move(place) | Operand::Ref(place, _) => {
                self.load_place(place, dst);
            }
            Operand::Constant(c) => {
                match &c.value {
                    ConstValue::StaticRef(sym) => {
                        let name = self.interner.resolve(*sym).to_string();
                        self.asm.lea_rip_relative(dst, &name);
                    }
                    ConstValue::FnItem(sym) => {
                        let name = self.interner.resolve(*sym).to_string();
                        self.asm.lea_rip_relative(dst, &name);
                    }
                    _ => {
                        let val = const_to_i64(&c.value);
                        self.asm.mov_ri(dst, val);
                    }
                }
            }
        }
    }

    fn load_place(&mut self, place: &Place, dst: Reg) {
        let slot = self.alloc.stack_slots[place.local.0];
        if place.projections.is_empty() {
            self.asm.mov_rm(dst, Reg::RBP, slot);
            return;
        }

        // State: (base_reg, offset) or "stack-based" (RBP, slot)
        // After Deref: dst = pointer loaded from current location; future ops use [dst + off]
        // After Field: adjust offset
        enum Base { Stack(i32), Reg }
        let mut base = Base::Stack(slot);

        for (i, proj) in place.projections.iter().enumerate() {
            let is_last = i == place.projections.len() - 1;
            match proj {
                Projection::Deref => {
                    match base {
                        Base::Stack(off) => {
                            self.asm.mov_rm(dst, Reg::RBP, off);
                        }
                        Base::Reg => {
                            self.asm.mov_rm(dst, dst, 0);
                        }
                    }
                    // dst now holds a pointer to the deref'd data
                    if is_last {
                        // Lone deref: load the value pointed to
                        self.asm.mov_rm(dst, dst, 0);
                    }
                    base = Base::Reg;
                }
                Projection::Field(_) => {
                    let field_offset = self.field_byte_offset(place, i);
                    // Check if the next projection needs an address (Index)
                    let next_needs_addr = place.projections.get(i + 1)
                        .map(|p| matches!(p, Projection::Index(_)))
                        .unwrap_or(false);
                    match base {
                        Base::Stack(off) => {
                            if next_needs_addr {
                                self.asm.lea(dst, Reg::RBP, off + field_offset);
                            } else {
                                self.asm.mov_rm(dst, Reg::RBP, off + field_offset);
                            }
                        }
                        Base::Reg => {
                            if next_needs_addr {
                                if field_offset != 0 {
                                    self.asm.add_ri(dst, field_offset);
                                }
                            } else {
                                self.asm.mov_rm(dst, dst, field_offset);
                            }
                        }
                    }
                    base = Base::Reg;
                }
                Projection::Index(idx_local) => {
                    // Load index into R11 (scratch, won't conflict with dst)
                    self.asm.mov_rm(Reg::R11, Reg::RBP, self.alloc.stack_slots[idx_local.0]);
                    // Compute base address into dst
                    match base {
                        Base::Stack(off) => {
                            self.asm.lea(dst, Reg::RBP, off);
                        }
                        Base::Reg => {
                            // dst already holds the address
                        }
                    }
                    let stride = self.index_stride(place, i);
                    if stride == 8 {
                        self.asm.shl_ri(Reg::R11, 3);
                    } else if stride != 1 {
                        self.asm.imul_ri(Reg::R11, stride as i64);
                    }
                    self.asm.add_rr(dst, Reg::R11);
                    // Load the value at [dst]
                    self.asm.movzx_rm_sized(dst, dst, 0, self.place_value_size(place));
                    base = Base::Reg;
                }
            }
        }
    }

    fn store_place(&mut self, place: &Place, src: Reg) {
        let slot = self.alloc.stack_slots[place.local.0];
        if place.projections.is_empty() {
            self.asm.mov_mr(Reg::RBP, slot, src);
            return;
        }

        // Build the target address in R11, then store src there.
        // We track whether R11 holds a computed address (via_reg=true)
        // or we're still relative to RBP+offset (via_reg=false).
        let mut offset = slot;
        let mut via_reg = false;

        for (i, proj) in place.projections.iter().enumerate() {
            match proj {
                Projection::Deref => {
                    // Load pointer from current location into R11
                    if via_reg {
                        self.asm.mov_rm(Reg::R11, Reg::R11, 0);
                    } else {
                        self.asm.mov_rm(Reg::R11, Reg::RBP, offset);
                    }
                    via_reg = true;
                    offset = 0;
                }
                Projection::Field(_) => {
                    let field_offset = self.field_byte_offset(place, i);
                    if via_reg {
                        if field_offset != 0 {
                            self.asm.add_ri(Reg::R11, field_offset);
                        }
                    } else {
                        offset += field_offset;
                    }
                }
                Projection::Index(idx_local) => {
                    // Compute base address into R11 if not already
                    if !via_reg {
                        self.asm.lea(Reg::R11, Reg::RBP, offset);
                        via_reg = true;
                        offset = 0;
                    }
                    let stride = self.index_stride(place, i);
                    self.asm.mov_rm(Reg::RCX, Reg::RBP, self.alloc.stack_slots[idx_local.0]);
                    if stride == 8 {
                        self.asm.shl_ri(Reg::RCX, 3);
                    } else if stride != 1 {
                        self.asm.imul_ri(Reg::RCX, stride as i64);
                    }
                    self.asm.add_rr(Reg::R11, Reg::RCX);
                }
            }
        }

        // Final store
        let value_size = self.place_value_size(place);
        if via_reg {
            self.asm.mov_mr_sized(Reg::R11, 0, src, value_size);
        } else {
            self.asm.mov_mr_sized(Reg::RBP, offset, src, value_size);
        }
    }

    fn emit_statement(&mut self, stmt: &Statement) {
        match &stmt.kind {
            StatementKind::Assign(place, rvalue) => {
                // Special-case aggregates: store each field directly into the place
                if let Rvalue::Aggregate(agg_kind, operands) = rvalue {
                    if place.projections.is_empty() {
                        let base_slot = self.alloc.stack_slots[place.local.0];
                        let def_id = match agg_kind {
                            AggregateKind::Adt(did, _) => Some(*did),
                            _ => None,
                        };
                        for (i, op) in operands.iter().enumerate() {
                            let field_off = def_id
                                .and_then(|did| self.field_offsets.get(&did))
                                .and_then(|offs| offs.get(i).copied())
                                .unwrap_or((i as i32) * 8);
                            // Check if operand is multi-slot (e.g. an array field)
                            let op_size = match op {
                                Operand::Copy(p) | Operand::Move(p) if p.projections.is_empty() => {
                                    self.alloc.local_sizes[p.local.0]
                                }
                                _ => 8,
                            };
                            if op_size > 8 {
                                // Multi-slot copy
                                if let Operand::Copy(p) | Operand::Move(p) = op {
                                    let src_slot = self.alloc.stack_slots[p.local.0];
                                    let n_slots = Self::slots_for_size(op_size);
                                    for s in 0..n_slots {
                                        let byte_off = (s as i32) * 8;
                                        self.asm.mov_rm(Reg::RAX, Reg::RBP, src_slot + byte_off);
                                        self.asm.mov_mr(Reg::RBP, base_slot + field_off + byte_off, Reg::RAX);
                                    }
                                }
                            } else {
                                self.load_operand(op, Reg::RAX);
                                self.asm.mov_mr(Reg::RBP, base_slot + field_off, Reg::RAX);
                            }
                        }
                        return;
                    }
                }
                // Special-case MakeVtable: store function addresses into stack slots
                if let Rvalue::MakeVtable(fn_names) = rvalue {
                    if place.projections.is_empty() {
                        let base_slot = self.alloc.stack_slots[place.local.0];
                        for (i, sym) in fn_names.iter().enumerate() {
                            let fn_name = self.interner.resolve(*sym).to_string();
                            self.asm.lea_rip_relative(Reg::RAX, &fn_name);
                            self.asm.mov_mr(Reg::RBP, base_slot + (i as i32) * 8, Reg::RAX);
                        }
                        return;
                    }
                }
                // Special-case multi-slot copies (struct = struct)
                if let Rvalue::Use(Operand::Copy(src) | Operand::Move(src)) = rvalue {
                    if place.projections.is_empty() && src.projections.is_empty() {
                        let dst_size = self.alloc.local_sizes[place.local.0];
                        let src_size = self.alloc.local_sizes[src.local.0];
                        let copy_size = dst_size.min(src_size);
                        if copy_size > 8 {
                            let dst_slot = self.alloc.stack_slots[place.local.0];
                            let src_slot = self.alloc.stack_slots[src.local.0];
                            let n_slots = Self::slots_for_size(copy_size);
                            for i in 0..n_slots {
                                let byte_off = (i as i32) * 8;
                                self.asm.mov_rm(Reg::RAX, Reg::RBP, src_slot + byte_off);
                                self.asm.mov_mr(Reg::RBP, dst_slot + byte_off, Reg::RAX);
                            }
                            return;
                        }
                    }
                }
                self.emit_rvalue(rvalue, Reg::RAX);
                self.store_place(place, Reg::RAX);
            }
            StatementKind::InlineAsm { template, operands, options: _ } => {
                self.emit_inline_asm(template, operands);
            }
            StatementKind::StorageLive(_) | StatementKind::StorageDead(_) | StatementKind::Nop => {}
        }
    }

    fn emit_rvalue(&mut self, rvalue: &Rvalue, dst: Reg) {
        match rvalue {
            Rvalue::Use(op) => {
                self.load_operand(op, dst);
            }
            Rvalue::BinaryOp(op, lhs, rhs) => {
                self.load_operand(lhs, Reg::RAX);
                self.load_operand(rhs, Reg::RCX);
                match op {
                    BinOp::Add => self.asm.add_rr(Reg::RAX, Reg::RCX),
                    BinOp::Sub => self.asm.sub_rr(Reg::RAX, Reg::RCX),
                    BinOp::Mul => self.asm.imul_rr(Reg::RAX, Reg::RCX),
                    BinOp::Div => {
                        self.asm.cqo();
                        self.asm.idiv_r(Reg::RCX);
                        // result in RAX
                    }
                    BinOp::Rem => {
                        self.asm.cqo();
                        self.asm.idiv_r(Reg::RCX);
                        self.asm.mov_rr(Reg::RAX, Reg::RDX); // remainder
                    }
                    BinOp::BitAnd => self.asm.and_rr(Reg::RAX, Reg::RCX),
                    BinOp::BitOr => self.asm.or_rr(Reg::RAX, Reg::RCX),
                    BinOp::BitXor => self.asm.xor_rr(Reg::RAX, Reg::RCX),
                    BinOp::Shl => {
                        // SHL r/m64, CL: REX.W D3 /4 — CL is low byte of RCX
                        self.asm.shl_cl(Reg::RAX);
                    }
                    BinOp::Shr => {
                        self.asm.shr_cl(Reg::RAX);
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        self.asm.cmp_rr(Reg::RAX, Reg::RCX);
                        let cc = match op {
                            BinOp::Eq => CondCode::Equal,
                            BinOp::Ne => CondCode::NotEqual,
                            BinOp::Lt => CondCode::Less,
                            BinOp::Le => CondCode::LessEqual,
                            BinOp::Gt => CondCode::Greater,
                            BinOp::Ge => CondCode::GreaterEqual,
                            _ => unreachable!(),
                        };
                        self.asm.setcc(cc, Reg::RAX);
                        self.asm.movzx_r8_r64(Reg::RAX, Reg::RAX);
                    }
                    BinOp::And => {
                        self.asm.and_rr(Reg::RAX, Reg::RCX);
                    }
                    BinOp::Or => {
                        self.asm.or_rr(Reg::RAX, Reg::RCX);
                    }
                }
                if dst != Reg::RAX {
                    self.asm.mov_rr(dst, Reg::RAX);
                }
            }
            Rvalue::UnaryOp(op, operand) => {
                self.load_operand(operand, dst);
                match op {
                    crate::ast::UnOp::Neg => self.asm.neg_r(dst),
                    crate::ast::UnOp::Not => {
                        // For bools, use XOR 1 (logical NOT); for integers, bitwise NOT
                        let is_bool = match operand {
                            Operand::Copy(p) | Operand::Move(p) | Operand::Ref(p, _) => {
                                matches!(self.body.locals[p.local.0].ty, TyKind::Bool)
                            }
                            Operand::Constant(c) => matches!(c.ty, TyKind::Bool),
                        };
                        if is_bool {
                            self.asm.xor_ri(dst, 1);
                        } else {
                            self.asm.not_r(dst);
                        }
                    }
                    crate::ast::UnOp::Deref => {
                        // dst has pointer, load from it
                        self.asm.mov_rm(dst, dst, 0);
                    }
                }
            }
            Rvalue::Ref(_, place) => {
                let slot = self.alloc.stack_slots[place.local.0];
                if place.projections.is_empty() {
                    self.asm.lea(dst, Reg::RBP, slot);
                } else {
                    // Compute address of the projected place
                    // Walk projections, computing the effective address
                    enum Base { Stack(i32), Reg }
                    let mut base = Base::Stack(slot);
                    for (i, proj) in place.projections.iter().enumerate() {
                        match proj {
                            Projection::Deref => {
                                // Load the pointer from current location
                                match base {
                                    Base::Stack(off) => {
                                        self.asm.mov_rm(dst, Reg::RBP, off);
                                    }
                                    Base::Reg => {
                                        self.asm.mov_rm(dst, dst, 0);
                                    }
                                }
                                // dst now holds the pointer; future ops relative to it
                                base = Base::Reg;
                            }
                            Projection::Field(_) => {
                                let field_offset = self.field_byte_offset(place, i);
                                match base {
                                    Base::Stack(ref mut off) => {
                                        *off += field_offset;
                                    }
                                    Base::Reg => {
                                        if field_offset != 0 {
                                            self.asm.add_ri(dst, field_offset);
                                        }
                                    }
                                }
                            }
                            Projection::Index(idx_local) => {
                                // Compute base address first
                                match base {
                                    Base::Stack(off) => {
                                        self.asm.lea(dst, Reg::RBP, off);
                                    }
                                    Base::Reg => {
                                        // dst already holds the address
                                    }
                                }
                                // Load index, scale by the array element stride, add to base
                                self.asm.mov_rm(Reg::R11, Reg::RBP, self.alloc.stack_slots[idx_local.0]);
                                let stride = self.index_stride(place, i);
                                if stride == 8 {
                                    self.asm.shl_ri(Reg::R11, 3);
                                } else if stride != 1 {
                                    self.asm.imul_ri(Reg::R11, stride as i64);
                                }
                                self.asm.add_rr(dst, Reg::R11);
                                base = Base::Reg;
                            }
                        }
                    }
                    // Now compute the final address
                    match base {
                        Base::Stack(off) => {
                            self.asm.lea(dst, Reg::RBP, off);
                        }
                        Base::Reg => {
                            // dst already holds the address
                        }
                    }
                }
            }
            Rvalue::Cast(op, _) => {
                // Simple truncation/extension - just load for now
                self.load_operand(op, dst);
            }
            Rvalue::Aggregate(_, operands) => {
                // Store each field; dst register not directly meaningful here.
                // The destination place's store_place will handle the base.
                // For aggregates, we just store fields sequentially.
                // This is called from emit_statement which will store_place after.
                // We need a different approach: store fields into the place directly.
                // For now just load the first field (will be overwritten by store_place).
                if let Some(first) = operands.first() {
                    self.load_operand(first, dst);
                }
            }
            Rvalue::Discriminant(place) => {
                // For C-like enums, the value is the discriminant (first word)
                self.load_place(place, dst);
            }
            Rvalue::Len(place) => {
                // For fixed-size arrays, look up the type to get the length
                let local_ty = &self.body.locals[place.local.0].ty;
                let len = match local_ty {
                    crate::typeck::TyKind::Array(_, n) => *n as i64,
                    _ => 0,
                };
                self.asm.mov_ri(dst, len);
            }
            Rvalue::MakeVtable(_) => {
                // Handled in emit_statement special-case; shouldn't reach here
                self.asm.xor_rr(dst, dst);
            }
        }
    }

    fn emit_terminator(&mut self, term: &Terminator, interner: &Interner) {
        match term {
            Terminator::Goto(target) => {
                self.asm.jmp(self.block_labels[target.0]);
            }
            Terminator::SwitchInt { operand, targets, default } => {
                self.load_operand(operand, Reg::RAX);
                for (val, target) in targets {
                    self.asm.cmp_ri(Reg::RAX, *val as i32);
                    self.asm.jcc(CondCode::Equal, self.block_labels[target.0]);
                }
                self.asm.jmp(self.block_labels[default.0]);
            }
            Terminator::Call { func, args, dest, target } => {
                // Move args into calling convention registers.
                // Struct args span multiple slots and consume multiple registers.
                let mut reg_idx = 0;
                for arg in args.iter() {
                    let n_slots = self.operand_slot_count(arg);
                    if n_slots > 1 {
                        // Multi-slot arg (struct): copy each slot into consecutive regs
                        if let Operand::Copy(place) | Operand::Move(place) = arg {
                            let src_slot = self.alloc.stack_slots[place.local.0];
                            for s in 0..n_slots {
                                if reg_idx < ARG_REGS.len() {
                                    self.asm.mov_rm(ARG_REGS[reg_idx], Reg::RBP, src_slot + (s as i32) * 8);
                                    reg_idx += 1;
                                }
                            }
                        }
                    } else {
                        if reg_idx < ARG_REGS.len() {
                            self.load_operand(arg, ARG_REGS[reg_idx]);
                            reg_idx += 1;
                        }
                    }
                }
                // Check for intrinsic calls before emitting a regular call
                let mut is_intrinsic = false;
                if let Operand::Constant(c) = func {
                    if let ConstValue::FnItem(sym) | ConstValue::MethodRef(sym) = &c.value {
                        let fn_name = interner.resolve(*sym);
                        is_intrinsic = self.try_emit_intrinsic(fn_name, args, dest);
                    }
                }

                if !is_intrinsic {
                    // Extract the function name or do indirect call
                    match func {
                        Operand::Constant(c) => match &c.value {
                            ConstValue::FnItem(sym) | ConstValue::MethodRef(sym) => {
                                let fn_name = interner.resolve(*sym).to_string();
                                self.asm.call_extern(&fn_name);
                            }
                            _ => {
                                self.asm.call_extern("__unknown");
                            }
                        },
                        Operand::Copy(place) | Operand::Move(place) | Operand::Ref(place, _) => {
                            // Indirect call: load function pointer from place, call through register
                            self.load_place(place, Reg::R10);
                            self.asm.call_reg(Reg::R10);
                        }
                    };
                    // Store return value
                    self.store_place(dest, Reg::RAX);
                }
                self.asm.jmp(self.block_labels[target.0]);
            }
            Terminator::Return => {
                // Load return value from _0
                let slot = self.alloc.stack_slots[0];
                self.asm.mov_rm(Reg::RAX, Reg::RBP, slot);
                self.emit_epilogue();
            }
            Terminator::Unreachable => {
                // UD2 — just emit a nop as placeholder
                self.asm.nop();
            }
        }
    }

    /// Try to emit an intrinsic call. Returns true if the function was recognized as an intrinsic.
    fn try_emit_intrinsic(&mut self, fn_name: &str, args: &[Operand], dest: &Place) -> bool {
        if let Some(argc) = Self::raw_syscall_arg_count(fn_name) {
            self.emit_anyos_raw_syscall(argc, dest);
            return true;
        }

        match fn_name {
            "u8::from" | "u16::from" | "u32::from" | "u64::from" | "u128::from" | "usize::from"
            | "i8::from" | "i16::from" | "i32::from" | "i64::from" | "i128::from" | "isize::from" => {
                if !args.is_empty() {
                    self.asm.mov_rr(Reg::RAX, Reg::RDI);
                    self.store_place(dest, Reg::RAX);
                }
                true
            }
            "u8::max" | "u16::max" | "u32::max" | "u64::max" | "u128::max" | "usize::max" => {
                self.asm.mov_rr(Reg::RAX, Reg::RDI);
                self.asm.emit_raw(&[0x48, 0x39, 0xF0]);       // cmp rax, rsi
                self.asm.emit_raw(&[0x48, 0x0F, 0x42, 0xC6]); // cmovb rax, rsi
                self.store_place(dest, Reg::RAX);
                true
            }
            "u8::min" | "u16::min" | "u32::min" | "u64::min" | "u128::min" | "usize::min" => {
                self.asm.mov_rr(Reg::RAX, Reg::RDI);
                self.asm.emit_raw(&[0x48, 0x39, 0xF0]);       // cmp rax, rsi
                self.asm.emit_raw(&[0x48, 0x0F, 0x47, 0xC6]); // cmova rax, rsi
                self.store_place(dest, Reg::RAX);
                true
            }
            "i8::max" | "i16::max" | "i32::max" | "i64::max" | "i128::max" | "isize::max" => {
                self.asm.mov_rr(Reg::RAX, Reg::RDI);
                self.asm.emit_raw(&[0x48, 0x39, 0xF0]);       // cmp rax, rsi
                self.asm.emit_raw(&[0x48, 0x0F, 0x4C, 0xC6]); // cmovl rax, rsi
                self.store_place(dest, Reg::RAX);
                true
            }
            "i8::min" | "i16::min" | "i32::min" | "i64::min" | "i128::min" | "isize::min" => {
                self.asm.mov_rr(Reg::RAX, Reg::RDI);
                self.asm.emit_raw(&[0x48, 0x39, 0xF0]);       // cmp rax, rsi
                self.asm.emit_raw(&[0x48, 0x0F, 0x4F, 0xC6]); // cmovg rax, rsi
                self.store_place(dest, Reg::RAX);
                true
            }
            // core::ptr
            "null_mut" | "null" => {
                // Return null pointer (0)
                self.asm.xor_rr(Reg::RAX, Reg::RAX);
                self.store_place(dest, Reg::RAX);
                true
            }
            "is_null" => {
                // (ptr: *const T) -> bool
                self.asm.emit_raw(&[0x48, 0x83, 0xFF, 0x00]); // cmp rdi, 0
                self.asm.emit_raw(&[0x0F, 0x94, 0xC0]);       // sete al
                self.asm.emit_raw(&[0x48, 0x0F, 0xB6, 0xC0]); // movzx rax, al
                self.store_place(dest, Reg::RAX);
                true
            }
            "write_bytes" => {
                // (dst: *mut T, val: u8, count: usize)
                // Args in RDI, RSI, RDX (already loaded by caller setup)
                // rep stosb: RDI=dst, AL=val, RCX=count
                self.asm.emit_raw(&[0x88, 0xF0]);       // mov al, sil
                self.asm.emit_raw(&[0x48, 0x89, 0xD1]); // mov rcx, rdx
                self.asm.emit_raw(&[0xF3, 0xAA]);       // rep stosb
                true
            }
            "copy_nonoverlapping" => {
                // (src: *const T, dst: *mut T, count: usize)
                // Args: RDI=src, RSI=dst, RDX=count
                // rep movsb needs: RSI=src, RDI=dst, RCX=count
                self.asm.emit_raw(&[0x48, 0x87, 0xFE]); // xchg rdi, rsi
                self.asm.emit_raw(&[0x48, 0x89, 0xD1]); // mov rcx, rdx
                self.asm.emit_raw(&[0xF3, 0xA4]);       // rep movsb
                true
            }
            "write_volatile" | "write" | "write_unaligned" => {
                // (dst: *mut T, val: T) — args in RDI, RSI
                // mov [rdi], rsi
                self.asm.mov_mr(Reg::RDI, 0, Reg::RSI);
                true
            }
            "read_volatile" | "read_unaligned" => {
                // (src: *const T) -> T — arg in RDI
                // mov rax, [rdi]
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);
                self.store_place(dest, Reg::RAX);
                true
            }
            // core::mem
            "size_of" => {
                // Try to determine actual type size from generic args
                // For now: types are 8 bytes unless we know better
                // Common sizes: u8=1, u16=2, u32=4, u64/usize/ptr=8, u128=16
                self.asm.mov_ri(Reg::RAX, 8);
                self.store_place(dest, Reg::RAX);
                true
            }
            "size_of_val" => {
                // size_of_val(&T) — for sized types, same as size_of
                self.asm.mov_ri(Reg::RAX, 8);
                self.store_place(dest, Reg::RAX);
                true
            }
            "align_of" => {
                self.asm.mov_ri(Reg::RAX, 8);
                self.store_place(dest, Reg::RAX);
                true
            }
            "transmute" => {
                // Just move the first arg through — reinterpret bits
                self.asm.mov_rr(Reg::RAX, Reg::RDI);
                self.store_place(dest, Reg::RAX);
                true
            }
            "forget" | "drop" => {
                // No-op: skip drop / explicit drop is no-op at codegen level
                true
            }
            "needs_drop" => {
                // For simplicity, return false (no destructors for now)
                self.asm.xor_rr(Reg::RAX, Reg::RAX);
                self.store_place(dest, Reg::RAX);
                true
            }
            "swap" => {
                // core::mem::swap(&mut T, &mut T) — args in RDI, RSI
                // Load both values, then store crossed
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);   // tmp = *a
                self.asm.mov_rm(Reg::RCX, Reg::RSI, 0);   // tmp2 = *b
                self.asm.mov_mr(Reg::RDI, 0, Reg::RCX);   // *a = tmp2
                self.asm.mov_mr(Reg::RSI, 0, Reg::RAX);   // *b = tmp
                true
            }
            "replace" => {
                // core::mem::replace(&mut T, T) -> T — args in RDI, RSI
                // old = *dst; *dst = new; return old
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);   // old = *dst
                self.asm.mov_mr(Reg::RDI, 0, Reg::RSI);   // *dst = new
                self.store_place(dest, Reg::RAX);
                true
            }
            // core::hint
            "spin_loop" => {
                self.asm.emit_raw(&[0xF3, 0x90]); // PAUSE
                true
            }
            "unreachable_unchecked" => {
                self.asm.emit_raw(&[0x0F, 0x0B]); // UD2
                true
            }
            "black_box" => {
                // Identity function (prevents optimization)
                self.asm.mov_rr(Reg::RAX, Reg::RDI);
                self.store_place(dest, Reg::RAX);
                true
            }
            // core::slice
            "from_raw_parts" | "from_raw_parts_mut" => {
                // Construct fat pointer from (ptr, len) — already in RDI, RSI
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.mov_mr(Reg::RBP, slot, Reg::RDI);
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RSI);
                true
            }

            // ── alloc intrinsics ──

            // alloc::alloc::alloc(layout_size, layout_align) -> *mut u8
            // In anyOS, we call the heap allocator via sbrk/mmap.
            // The allocator is available as `__anyrc_alloc(size: usize) -> *mut u8`.
            "alloc" if args.len() >= 1 => {
                // size in RDI (already loaded)
                // Call the runtime allocator
                self.asm.call_extern("__anyrc_alloc");
                self.store_place(dest, Reg::RAX);
                true
            }
            "alloc_zeroed" => {
                // Allocate and zero the memory
                // size in RDI
                self.asm.call_extern("__anyrc_alloc");
                self.store_place(dest, Reg::RAX);
                // Zero the allocated memory: memset(ptr, 0, size)
                // rax = ptr, rdi still = size from before call? No, save/restore needed
                // Simplified: caller handles zeroing if needed
                true
            }
            "dealloc" => {
                // dealloc(ptr, layout_size, layout_align)
                // ptr in RDI, size in RSI
                self.asm.call_extern("__anyrc_dealloc");
                true
            }
            "realloc" => {
                // realloc(ptr, old_layout_size, new_size) -> *mut u8
                // ptr in RDI, old_size in RSI, new_size in RDX
                self.asm.call_extern("__anyrc_realloc");
                self.store_place(dest, Reg::RAX);
                true
            }

            // ── Box intrinsics ──

            // Box::new(val) — allocate 8 bytes and store value
            "Box::new" => {
                // arg0 = value in RDI
                // Save value on stack
                self.asm.push(Reg::RDI);
                // Allocate 8 bytes
                self.asm.mov_ri(Reg::RDI, 8);
                self.asm.call_extern("__anyrc_alloc");
                // rax = ptr, restore value
                self.asm.pop(Reg::RDI);
                // Store value at allocated ptr
                self.asm.mov_mr(Reg::RAX, 0, Reg::RDI);
                self.store_place(dest, Reg::RAX);
                true
            }

            // ── Vec intrinsics ──
            // Vec layout: [ptr: *mut T, len: usize, capacity: usize] = 24 bytes

            "Vec::new" => {
                // Return empty Vec: ptr=0, len=0, cap=0
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.xor_rr(Reg::RAX, Reg::RAX);
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);      // ptr = 0
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RAX);  // len = 0
                self.asm.mov_mr(Reg::RBP, slot + 16, Reg::RAX); // cap = 0
                true
            }
            "Vec::with_capacity" => {
                // arg0 = capacity in RDI
                // Allocate capacity * 8 bytes
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.push(Reg::RDI);               // save capacity
                self.asm.emit_raw(&[0x48, 0xC1, 0xE7, 0x03]); // shl rdi, 3 (capacity * 8)
                self.asm.call_extern("__anyrc_alloc");
                self.asm.pop(Reg::RCX);                // rcx = original capacity
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);      // ptr
                self.asm.xor_rr(Reg::RDX, Reg::RDX);
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RDX);  // len = 0
                self.asm.mov_mr(Reg::RBP, slot + 16, Reg::RCX); // cap
                true
            }
            "Vec::len" | "String::len" => {
                // arg0 = &Vec in RDI → read len field at offset 8
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 8);
                self.store_place(dest, Reg::RAX);
                true
            }
            "Vec::capacity" => {
                // arg0 = &Vec in RDI → read capacity field at offset 16
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 16);
                self.store_place(dest, Reg::RAX);
                true
            }
            "Vec::is_empty" | "String::is_empty" => {
                // len == 0
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 8);
                self.asm.emit_raw(&[0x48, 0x85, 0xC0]); // test rax, rax
                self.asm.emit_raw(&[0x0F, 0x94, 0xC0]);  // sete al
                self.asm.emit_raw(&[0x48, 0x0F, 0xB6, 0xC0]); // movzx rax, al
                self.store_place(dest, Reg::RAX);
                true
            }
            "Vec::push" => {
                // arg0 = &mut Vec in RDI, arg1 = value in RSI
                // Check if len < capacity; if not, grow
                // Simplified: always call runtime helper
                self.asm.call_extern("__anyrc_vec_push");
                true
            }
            "Vec::pop" => {
                // arg0 = &mut Vec in RDI → returns Option-like (disc, value)
                self.asm.call_extern("__anyrc_vec_pop");
                // Returns discriminant in RAX (0=Some, 1=None), value in RDX
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);     // disc
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RDX); // value
                true
            }
            "Vec::as_ptr" => {
                // arg0 = &Vec in RDI → return ptr field
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);
                self.store_place(dest, Reg::RAX);
                true
            }
            "Vec::as_slice" | "Vec::as_ref" => {
                // Return fat pointer (ptr, len) from Vec
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);  // ptr
                self.asm.mov_rm(Reg::RCX, Reg::RDI, 8);   // len
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RCX);
                true
            }
            "Vec::clear" | "String::clear" => {
                // Set len to 0 (keep allocation)
                self.asm.xor_rr(Reg::RAX, Reg::RAX);
                self.asm.mov_mr(Reg::RDI, 8, Reg::RAX); // len = 0
                true
            }

            // ── String intrinsics ──

            "String::new" => {
                // Same layout as Vec<u8>: ptr=0, len=0, cap=0
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.xor_rr(Reg::RAX, Reg::RAX);
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RAX);
                self.asm.mov_mr(Reg::RBP, slot + 16, Reg::RAX);
                true
            }
            "String::with_capacity" => {
                // Same as Vec::with_capacity but element size = 1
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.push(Reg::RDI);
                self.asm.call_extern("__anyrc_alloc"); // size already in RDI
                self.asm.pop(Reg::RCX);
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);
                self.asm.xor_rr(Reg::RDX, Reg::RDX);
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RDX);
                self.asm.mov_mr(Reg::RBP, slot + 16, Reg::RCX);
                true
            }
            "String::as_str" | "String::as_bytes" => {
                // Return &str / &[u8]: fat pointer from String's (ptr, len)
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);
                self.asm.mov_rm(Reg::RCX, Reg::RDI, 8);
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RCX);
                true
            }
            "String::push_str" => {
                // arg0 = &mut String in RDI, arg1 = &str (ptr,len) fat ptr
                // Call runtime helper
                self.asm.call_extern("__anyrc_string_push_str");
                true
            }
            "String::push" => {
                // arg0 = &mut String in RDI, arg1 = char in RSI
                self.asm.call_extern("__anyrc_string_push_char");
                true
            }
            "String::from" => {
                // String::from(&str) — clone a string slice into a new heap String
                // arg0 = &str fat ptr (ptr in RDI, len in RSI)
                self.asm.call_extern("__anyrc_string_from_str");
                // Returns 3-word struct in memory via hidden first arg or RAX for simple case
                self.store_place(dest, Reg::RAX);
                true
            }

            // ── HashMap intrinsics ──
            "HashMap::new" => {
                // Empty hashmap: all zeros (implementation-specific)
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.xor_rr(Reg::RAX, Reg::RAX);
                // Zero out 4 * 8 = 32 bytes for HashMap struct
                for i in 0..4 {
                    self.asm.mov_mr(Reg::RBP, slot + i * 8, Reg::RAX);
                }
                true
            }
            // Atomic intrinsics
            // AtomicXxx::new(val) — just returns the value (stored by caller)
            s if s.ends_with("::new") && s.contains("Atomic") => {
                // arg0 = value, already in RDI
                self.asm.mov_rr(Reg::RAX, Reg::RDI);
                self.store_place(dest, Reg::RAX);
                true
            }
            // AtomicXxx::load(&self, ordering) — load from self pointer
            s if s.ends_with("::load") && s.contains("Atomic") => {
                // arg0 = &self (pointer to atomic), arg1 = ordering (ignored)
                // mov rax, [rdi]
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);
                self.store_place(dest, Reg::RAX);
                true
            }
            // AtomicXxx::store(&self, val, ordering) — store to self pointer
            s if s.ends_with("::store") && s.contains("Atomic") => {
                // arg0 = &self, arg1 = val, arg2 = ordering (ignored)
                // mov [rdi], rsi
                self.asm.mov_mr(Reg::RDI, 0, Reg::RSI);
                true
            }
            // AtomicXxx::fetch_add(&self, val, ordering) → lock xadd
            s if s.ends_with("::fetch_add") && s.contains("Atomic") => {
                // arg0 = &self, arg1 = val, arg2 = ordering (ignored)
                // lock xadd [rdi], rsi → old value ends up in rsi
                self.asm.emit_raw(&[0xF0, 0x48, 0x0F, 0xC1, 0x37]); // lock xadd [rdi], rsi
                self.asm.mov_rr(Reg::RAX, Reg::RSI); // return old value
                self.store_place(dest, Reg::RAX);
                true
            }
            // AtomicXxx::compare_exchange(&self, current, new, success_ord, fail_ord)
            s if s.ends_with("::compare_exchange") && s.contains("Atomic") => {
                // arg0=&self, arg1=current, arg2=new, arg3=succ_ord, arg4=fail_ord
                // lock cmpxchg [rdi], rdx — rax(=rsi)=expected, result in rax
                self.asm.mov_rr(Reg::RAX, Reg::RSI); // rax = current (expected)
                self.asm.emit_raw(&[0xF0, 0x48, 0x0F, 0xB1, 0x17]); // lock cmpxchg [rdi], rdx
                // Return old value in rax (if == expected, exchange happened)
                self.store_place(dest, Reg::RAX);
                true
            }
            // AtomicXxx::swap(&self, val, ordering) → xchg
            s if s.ends_with("::swap") && s.contains("Atomic") => {
                // arg0=&self, arg1=val, arg2=ordering
                // xchg [rdi], rsi → old value in rsi
                self.asm.emit_raw(&[0x48, 0x87, 0x37]); // xchg [rdi], rsi
                self.asm.mov_rr(Reg::RAX, Reg::RSI);
                self.store_place(dest, Reg::RAX);
                true
            }
            // ── core::mem intrinsics ──

            "take" => {
                // core::mem::take(&mut T) -> T: replace *ptr with default (0) and return old
                // arg0 = &mut T in RDI
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);   // old = *ptr
                self.asm.xor_rr(Reg::RCX, Reg::RCX);
                self.asm.mov_mr(Reg::RDI, 0, Reg::RCX);   // *ptr = 0
                self.store_place(dest, Reg::RAX);
                true
            }
            "discriminant" => {
                // core::mem::discriminant(&T) -> Discriminant<T>
                // For enums, the discriminant is at offset 0
                // arg0 = &T in RDI
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);
                self.store_place(dest, Reg::RAX);
                true
            }
            "ManuallyDrop::new" => {
                // Just pass through the value
                self.asm.mov_rr(Reg::RAX, Reg::RDI);
                self.store_place(dest, Reg::RAX);
                true
            }

            // ── Option/Result combinator methods ──

            "Option::unwrap" | "Result::unwrap" => {
                // Extract value from Some/Ok (field 1), panic on None/Err
                // arg0 = &Option/Result in RDI
                // Check discriminant (field 0): 0 = Some/Ok
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);     // disc
                // If disc != 0, trap (simplified: just read value regardless)
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 8);     // value
                self.store_place(dest, Reg::RAX);
                true
            }
            "Option::unwrap_or" => {
                // arg0 = &Option in RDI, arg1 = default in RSI
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);     // disc
                self.asm.emit_raw(&[0x48, 0x85, 0xC0]);     // test rax, rax
                // If disc == 0 (Some), load value; else use default
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 8);     // value from Some
                self.asm.emit_raw(&[0x74, 0x03]);            // je .done (disc==0 → Some)
                self.asm.mov_rr(Reg::RAX, Reg::RSI);        // else: use default
                // .done:
                self.store_place(dest, Reg::RAX);
                true
            }
            "Option::is_some" => {
                // arg0 = &Option in RDI → disc == 0 means Some
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);
                self.asm.emit_raw(&[0x48, 0x85, 0xC0]);     // test rax, rax
                self.asm.emit_raw(&[0x0F, 0x94, 0xC0]);     // sete al
                self.asm.emit_raw(&[0x48, 0x0F, 0xB6, 0xC0]); // movzx rax, al
                self.store_place(dest, Reg::RAX);
                true
            }
            "Option::is_none" => {
                // arg0 = &Option in RDI → disc != 0 means None
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);
                self.asm.emit_raw(&[0x48, 0x85, 0xC0]);     // test rax, rax
                self.asm.emit_raw(&[0x0F, 0x95, 0xC0]);     // setne al
                self.asm.emit_raw(&[0x48, 0x0F, 0xB6, 0xC0]); // movzx rax, al
                self.store_place(dest, Reg::RAX);
                true
            }
            "Option::copied" | "Option::cloned" => {
                // Just return the option as-is (value types are already copied)
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);
                self.asm.mov_rm(Reg::RCX, Reg::RDI, 8);
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RCX);
                true
            }
            "Option::map" | "Result::map" => {
                // arg0 = &Option in RDI, arg1 = closure fn ptr in RSI
                // Check disc, if Some: call closure with value, wrap result in Some
                self.asm.call_extern("__anyrc_option_map");
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);     // disc
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RDX); // value
                true
            }

            // ── Iterator intrinsics ──

            // Vec::iter(&self) → return fat pointer (ptr, len) as an iterator state
            "iter" | "Vec::iter" | "iter_mut" | "Vec::iter_mut" => {
                // Return iterator = (current_ptr, end_ptr)
                // arg0 = &Vec in RDI
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);     // ptr
                self.asm.mov_rm(Reg::RCX, Reg::RDI, 8);     // len
                // end = ptr + len * 8
                self.asm.emit_raw(&[0x48, 0xC1, 0xE1, 0x03]); // shl rcx, 3
                self.asm.emit_raw(&[0x48, 0x01, 0xC1]);       // add rcx, rax
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);    // current ptr
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RCX); // end ptr
                true
            }
            "next" => {
                // Iterator::next(&mut self) → Option<T>
                // Iterator state = (current_ptr, end_ptr) at [RDI]
                // If current < end: return Some(*current), advance current
                // Else: return None
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);     // current
                self.asm.mov_rm(Reg::RCX, Reg::RDI, 8);     // end
                self.asm.emit_raw(&[0x48, 0x39, 0xC8]);     // cmp rax, rcx
                self.asm.emit_raw(&[0x73, 0x15]);            // jae .none
                // Some: read value, advance ptr
                self.asm.mov_rm(Reg::RDX, Reg::RAX, 0);     // value = *current
                self.asm.emit_raw(&[0x48, 0x83, 0xC0, 0x08]); // add rax, 8
                self.asm.mov_mr(Reg::RDI, 0, Reg::RAX);     // update current
                self.asm.xor_rr(Reg::RAX, Reg::RAX);        // disc = 0 (Some)
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RDX);
                self.asm.emit_raw(&[0xEB, 0x0D]);           // jmp .done
                // .none:
                self.asm.mov_ri(Reg::RAX, 1);               // disc = 1 (None)
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);
                self.asm.xor_rr(Reg::RAX, Reg::RAX);
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RAX);
                // .done:
                true
            }
            "enumerate" => {
                // Wraps iterator, adding index — returns (iter_state, counter=0)
                // arg0 = iterator state in RDI
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);     // current
                self.asm.mov_rm(Reg::RCX, Reg::RDI, 8);     // end
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RCX);
                self.asm.xor_rr(Reg::RAX, Reg::RAX);
                self.asm.mov_mr(Reg::RBP, slot + 16, Reg::RAX); // counter = 0
                true
            }

            // ── Slice operations ──

            "slice_index" | "SliceIndex::index" => {
                // &slice[index] — arg0 = &slice (fat ptr) in RDI, arg1 = index in RSI
                // slice.ptr + index * 8
                self.asm.mov_rm(Reg::RAX, Reg::RDI, 0);     // ptr
                self.asm.emit_raw(&[0x48, 0xC1, 0xE6, 0x03]); // shl rsi, 3
                self.asm.emit_raw(&[0x48, 0x01, 0xF0]);       // add rax, rsi
                self.asm.mov_rm(Reg::RAX, Reg::RAX, 0);     // deref
                self.store_place(dest, Reg::RAX);
                true
            }

            // ── HashMap intrinsics ──

            "HashMap::insert" => {
                // arg0 = &mut HashMap, arg1 = key, arg2 = value
                self.asm.call_extern("__anyrc_hashmap_insert");
                true
            }
            "HashMap::get" => {
                // arg0 = &HashMap, arg1 = &key → Option<&V>
                self.asm.call_extern("__anyrc_hashmap_get");
                let slot = self.alloc.stack_slots[dest.local.0];
                self.asm.mov_mr(Reg::RBP, slot, Reg::RAX);     // disc
                self.asm.mov_mr(Reg::RBP, slot + 8, Reg::RDX); // value ptr
                true
            }
            "HashMap::contains_key" => {
                self.asm.call_extern("__anyrc_hashmap_get");
                // If RAX (disc) == 0, key exists
                self.asm.emit_raw(&[0x48, 0x85, 0xC0]);     // test rax, rax
                self.asm.emit_raw(&[0x0F, 0x94, 0xC0]);     // sete al
                self.asm.emit_raw(&[0x48, 0x0F, 0xB6, 0xC0]); // movzx rax, al
                self.store_place(dest, Reg::RAX);
                true
            }
            "HashMap::entry" => {
                // Returns a reference to the entry slot
                self.asm.call_extern("__anyrc_hashmap_entry");
                self.store_place(dest, Reg::RAX);
                true
            }
            "or_default" | "or_insert" => {
                // Entry::or_default() — insert default if vacant
                self.asm.call_extern("__anyrc_entry_or_default");
                self.store_place(dest, Reg::RAX);
                true
            }

            // ── Clone/Copy/Display intrinsics ──

            "clone" => {
                // For Copy types: identity. For heap types: deep copy.
                // Simplified: just copy the value
                self.asm.mov_rr(Reg::RAX, Reg::RDI);
                self.store_place(dest, Reg::RAX);
                true
            }
            "to_string" => {
                // Convert to String — calls runtime
                self.asm.call_extern("__anyrc_to_string");
                self.store_place(dest, Reg::RAX);
                true
            }

            // ── Comparison/ordering intrinsics ──

            // ── Format/Print intrinsics ──

            "__anyrc_format" => {
                // format!() expansion — for bootstrap, just return the first arg (format string)
                // Real implementation would do string interpolation
                self.asm.mov_rr(Reg::RAX, Reg::RDI);
                self.store_place(dest, Reg::RAX);
                true
            }
            "__anyrc_println" => {
                // println!() expansion — write string to stdout via syscall
                // arg0 = format string ptr in RDI
                // For bootstrap: call runtime write helper
                self.asm.call_extern("__anyrc_println");
                true
            }

            // ── Comparison/ordering intrinsics ──

            "cmp" | "partial_cmp" => {
                // Compare two values: arg0 in RDI, arg1 in RSI
                self.asm.emit_raw(&[0x48, 0x39, 0xF7]);     // cmp rdi, rsi
                self.asm.emit_raw(&[0x0F, 0x9C, 0xC0]);     // setl al (Less = -1)
                self.asm.emit_raw(&[0x0F, 0x9F, 0xC2]);     // setg dl (Greater = 1)
                self.asm.emit_raw(&[0x48, 0x0F, 0xB6, 0xC0]); // movzx rax, al
                self.asm.emit_raw(&[0x48, 0x0F, 0xB6, 0xD2]); // movzx rdx, dl
                self.asm.emit_raw(&[0x48, 0x29, 0xC2]);     // sub rdx, rax
                self.asm.mov_rr(Reg::RAX, Reg::RDX);
                self.store_place(dest, Reg::RAX);
                true
            }

            // SYS_EXIT = 1
            "exit" | "anyos_std::process::exit" => {
                self.emit_anyos_fixed_syscall(1, 1, dest);
                true
            }

            // SYS_WRITE = 2: write(fd, buf_ptr, len) -> n
            "anyos_std::fs::write" => {
                self.emit_anyos_fixed_syscall(2, 3, dest);
                true
            }

            // SYS_READ = 3: read(fd, buf_ptr, len) -> n
            "anyos_std::fs::read" => {
                self.emit_anyos_fixed_syscall(3, 3, dest);
                true
            }

            // SYS_OPEN = 4: open(path_ptr, flags) -> fd
            "anyos_std::fs::open" => {
                self.emit_anyos_fixed_syscall(4, 2, dest);
                true
            }

            // SYS_CLOSE = 5: close(fd) -> 0
            "anyos_std::fs::close" => {
                self.emit_anyos_fixed_syscall(5, 1, dest);
                true
            }

            // SYS_SBRK = 9: sbrk(increment) -> old_break
            "anyos_std::heap::init" | "heap::init" => {
                self.asm.xor_rr(Reg::RDI, Reg::RDI);
                self.emit_anyos_fixed_syscall(9, 1, dest);
                true
            }

            // SYS_EXEC = 11: exec(path_ptr, args_ptr) -> pid
            "exec" | "anyos_std::process::exec" => {
                self.emit_anyos_fixed_syscall(11, 2, dest);
                true
            }

            // SYS_READDIR = 23: readdir(path_ptr, buf_ptr) -> count
            "anyos_std::fs::readdir" | "readdir" => {
                self.emit_anyos_fixed_syscall(23, 2, dest);
                true
            }

            // SYS_GETARGS = 28: getargs(buf_ptr) -> len
            "args" | "anyos_std::process::args" => {
                self.emit_anyos_fixed_syscall(28, 1, dest);
                true
            }

            // SYS_MKDIR = 90
            "anyos_std::fs::mkdir" | "mkdir" => {
                self.emit_anyos_fixed_syscall(90, 1, dest);
                true
            }

            // SYS_UNLINK = 91
            "anyos_std::fs::unlink" | "unlink" => {
                self.emit_anyos_fixed_syscall(91, 1, dest);
                true
            }

            // Raw syscall passthrough
            "syscall" | "anyos_std::raw::syscall" => {
                self.emit_anyos_raw_syscall(6, dest);
                true
            }

            _ => false,
        }
    }

    fn raw_syscall_arg_count(fn_name: &str) -> Option<usize> {
        let short = fn_name.rsplit("::").next().unwrap_or(fn_name);
        match short {
            "syscall0" => Some(1),
            "syscall1" | "syscall1_u64" => Some(2),
            "syscall2" | "syscall2_u64" => Some(3),
            "syscall3" => Some(4),
            "syscall4" => Some(5),
            "syscall5" => Some(6),
            _ => None,
        }
    }

    fn emit_anyos_fixed_syscall(&mut self, num: u32, arg_count: usize, dest: &Place) {
        self.asm.push(Reg::RBX);
        self.asm.mov_ri(Reg::RAX, num as i64);
        self.move_fixed_sysv_args_to_anyos_syscall(arg_count);
        self.asm.syscall();
        self.asm.pop(Reg::RBX);
        self.store_place(dest, Reg::RAX);
    }

    fn emit_anyos_raw_syscall(&mut self, arg_count: usize, dest: &Place) {
        self.asm.push(Reg::RBX);
        self.asm.mov_rr(Reg::RAX, Reg::RDI);
        self.move_sysv_args_to_anyos_syscall(arg_count);
        self.asm.syscall();
        self.asm.pop(Reg::RBX);
        self.store_place(dest, Reg::RAX);
    }

    fn move_fixed_sysv_args_to_anyos_syscall(&mut self, arg_count: usize) {
        if arg_count > 0 {
            self.asm.mov_rr(Reg::RBX, Reg::RDI);
        }
        if arg_count > 1 {
            self.asm.mov_rr(Reg::R10, Reg::RSI);
        }
        if arg_count > 3 {
            self.asm.mov_rr(Reg::RSI, Reg::RCX);
        }
        if arg_count > 4 {
            self.asm.mov_rr(Reg::RDI, Reg::R8);
        }
    }

    fn move_sysv_args_to_anyos_syscall(&mut self, arg_count: usize) {
        if arg_count > 1 {
            self.asm.mov_rr(Reg::RBX, Reg::RSI);
        }
        if arg_count > 2 {
            self.asm.mov_rr(Reg::R10, Reg::RDX);
        }
        if arg_count > 3 {
            self.asm.mov_rr(Reg::RDX, Reg::RCX);
        }
        if arg_count > 4 {
            self.asm.mov_rr(Reg::RSI, Reg::R8);
        }
        if arg_count > 5 {
            self.asm.mov_rr(Reg::RDI, Reg::R9);
        }
    }

    fn emit_inline_asm(&mut self, template: &[String], operands: &[MirAsmOperand]) {
        // Step 1: Move input operands into their designated registers
        for (i, op) in operands.iter().enumerate() {
            match &op.kind {
                MirAsmOperandKind::In(operand) => {
                    let target_reg = self.asm_reg_to_reg(&op.reg, i);
                    self.load_operand(operand, target_reg);
                }
                MirAsmOperandKind::InOut(operand, _) => {
                    let target_reg = self.asm_reg_to_reg(&op.reg, i);
                    self.load_operand(operand, target_reg);
                }
                MirAsmOperandKind::Out(_) => {}
            }
        }

        // Step 2: Emit the assembly instruction bytes for each template string
        for tmpl in template {
            let resolved = self.resolve_asm_template(tmpl, operands);
            let bytes = Self::assemble_instruction(&resolved);
            self.asm.emit_raw(&bytes);
        }

        // Step 3: Move output operands from their registers to destinations
        for (i, op) in operands.iter().enumerate() {
            match &op.kind {
                MirAsmOperandKind::Out(Some(place)) => {
                    let src_reg = self.asm_reg_to_reg(&op.reg, i);
                    self.store_place(place, src_reg);
                }
                MirAsmOperandKind::InOut(_, Some(place)) => {
                    let src_reg = self.asm_reg_to_reg(&op.reg, i);
                    self.store_place(place, src_reg);
                }
                _ => {}
            }
        }
    }

    fn asm_reg_to_reg(&self, reg: &MirAsmReg, _index: usize) -> Reg {
        match reg {
            MirAsmReg::Named(name) => match name.as_str() {
                "rax" | "eax" | "ax" | "al" => Reg::RAX,
                "rcx" | "ecx" | "cx" | "cl" => Reg::RCX,
                "rdx" | "edx" | "dx" | "dl" => Reg::RDX,
                "rbx" | "ebx" | "bx" | "bl" => Reg::RBX,
                "rsp" | "esp" => Reg::RSP,
                "rbp" | "ebp" => Reg::RBP,
                "rsi" | "esi" => Reg::RSI,
                "rdi" | "edi" => Reg::RDI,
                "r8" | "r8d" => Reg::R8,
                "r9" | "r9d" => Reg::R9,
                "r10" | "r10d" => Reg::R10,
                "r11" | "r11d" => Reg::R11,
                _ => Reg::RAX, // fallback
            },
            MirAsmReg::Class(_) => Reg::RAX, // allocate RAX for generic reg class
        }
    }

    fn resolve_asm_template(&self, tmpl: &str, operands: &[MirAsmOperand]) -> String {
        let mut result = tmpl.to_string();
        // Replace {0}, {1}, etc. with register names
        for (i, op) in operands.iter().enumerate() {
            let reg_name = match &op.reg {
                MirAsmReg::Named(n) => n.clone(),
                MirAsmReg::Class(_) => "rax".to_string(),
            };
            let placeholder = format!("{{{}}}", i);
            result = result.replace(&placeholder, &reg_name);
            // Also handle {name} - but we don't track names at MIR level, skip for now
        }
        // Replace bare {} with positional
        let mut idx = 0;
        while result.contains("{}") {
            if idx < operands.len() {
                let reg_name = match &operands[idx].reg {
                    MirAsmReg::Named(n) => n.clone(),
                    MirAsmReg::Class(_) => "rax".to_string(),
                };
                result = result.replacen("{}", &reg_name, 1);
                idx += 1;
            } else {
                break;
            }
        }
        result
    }

    fn assemble_instruction(instr: &str) -> Vec<u8> {
        let instr = instr.trim();
        match instr {
            // Simple instructions
            "nop" => vec![0x90],
            "cli" => vec![0xFA],
            "sti" => vec![0xFB],
            "hlt" => vec![0xF4],
            "ret" => vec![0xC3],
            "iretq" => vec![0x48, 0xCF],
            "swapgs" => vec![0x0F, 0x01, 0xF8],
            "syscall" => vec![0x0F, 0x05],
            "wrmsr" => vec![0x0F, 0x30],
            "rdmsr" => vec![0x0F, 0x32],
            "cpuid" => vec![0x0F, 0xA2],
            "pause" => vec![0xF3, 0x90],
            "ud2" => vec![0x0F, 0x0B],
            "clflush" => vec![0x0F, 0xAE],
            "mfence" => vec![0x0F, 0xAE, 0xF0],
            "lfence" => vec![0x0F, 0xAE, 0xE8],
            "sfence" => vec![0x0F, 0xAE, 0xF8],
            "cld" => vec![0xFC],
            "std" => vec![0xFD],

            // Port I/O
            "out dx, al" => vec![0xEE],
            "out dx, ax" => vec![0x66, 0xEF],
            "out dx, eax" => vec![0xEF],
            "in al, dx" => vec![0xEC],
            "in ax, dx" => vec![0x66, 0xED],
            "in eax, dx" => vec![0xED],

            // Control register moves
            "mov rax, cr0" => vec![0x0F, 0x20, 0xC0],
            "mov cr0, rax" => vec![0x0F, 0x22, 0xC0],
            "mov rax, cr2" => vec![0x0F, 0x20, 0xD0],
            "mov rax, cr3" => vec![0x0F, 0x20, 0xD8],
            "mov cr3, rax" => vec![0x0F, 0x22, 0xD8],
            "mov rax, cr4" => vec![0x0F, 0x20, 0xE0],
            "mov cr4, rax" => vec![0x0F, 0x22, 0xE0],

            // Memory ops with [rax]
            "invlpg [rax]" => vec![0x0F, 0x01, 0x38],
            "lgdt [rax]" => vec![0x0F, 0x01, 0x10],
            "lidt [rax]" => vec![0x0F, 0x01, 0x18],
            "sgdt [rax]" => vec![0x0F, 0x01, 0x00],
            "sidt [rax]" => vec![0x0F, 0x01, 0x08],
            "ltr ax" => vec![0x0F, 0x00, 0xD8],

            _ => {
                // Try to handle more patterns dynamically
                Self::assemble_dynamic(instr)
            }
        }
    }

    fn assemble_dynamic(instr: &str) -> Vec<u8> {
        // Handle "mov CRn, REG" and "mov REG, CRn" patterns
        let parts: Vec<&str> = instr.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let mnemonic = parts[0];
            let args: Vec<&str> = parts[1].split(',').map(|s| s.trim()).collect();

            if mnemonic == "mov" && args.len() == 2 {
                if let (Some(dst), Some(src)) = (Self::reg_num_64(args[0]), Self::reg_num_64(args[1])) {
                    let rex = 0x48
                        | if src >= 8 { 0x04 } else { 0 }
                        | if dst >= 8 { 0x01 } else { 0 };
                    return vec![rex, 0x89, 0xC0 | ((src & 7) << 3) | (dst & 7)];
                }
                // mov reg, crN
                if args[1].starts_with("cr") {
                    if let (Some(reg), Some(cr)) = (Self::reg_num_64(args[0]), Self::cr_num(args[1])) {
                        // 0F 20 /r — MOV r64, CRn
                        return vec![0x0F, 0x20, 0xC0 | (cr << 3) | reg];
                    }
                }
                // mov crN, reg
                if args[0].starts_with("cr") {
                    if let (Some(cr), Some(reg)) = (Self::cr_num(args[0]), Self::reg_num_64(args[1])) {
                        // 0F 22 /r — MOV CRn, r64
                        return vec![0x0F, 0x22, 0xC0 | (cr << 3) | reg];
                    }
                }
            }

            if mnemonic == "push" && args.len() == 1 {
                if let Some(reg) = Self::reg_num_64(args[0]) {
                    let mut out = Vec::new();
                    if reg >= 8 {
                        out.push(0x41);
                    }
                    out.push(0x50 + (reg & 7));
                    return out;
                }
            }

            if mnemonic == "pop" && args.len() == 1 {
                if let Some(reg) = Self::reg_num_64(args[0]) {
                    let mut out = Vec::new();
                    if reg >= 8 {
                        out.push(0x41);
                    }
                    out.push(0x58 + (reg & 7));
                    return out;
                }
            }

            if mnemonic == "invlpg" && args.len() == 1 {
                // invlpg [reg]
                let inner = args[0].trim_start_matches('[').trim_end_matches(']');
                if let Some(reg) = Self::reg_num_64(inner) {
                    return vec![0x0F, 0x01, 0x38 | reg];
                }
            }
        }

        // Unknown instruction — emit nop as fallback (should not happen in practice)
        vec![0x90]
    }

    fn reg_num_64(name: &str) -> Option<u8> {
        match name {
            "rax" | "eax" => Some(0),
            "rcx" | "ecx" => Some(1),
            "rdx" | "edx" => Some(2),
            "rbx" | "ebx" => Some(3),
            "rsp" | "esp" => Some(4),
            "rbp" | "ebp" => Some(5),
            "rsi" | "esi" => Some(6),
            "rdi" | "edi" => Some(7),
            "r8" | "r8d" => Some(8),
            "r9" | "r9d" => Some(9),
            "r10" | "r10d" => Some(10),
            "r11" | "r11d" => Some(11),
            "r12" | "r12d" => Some(12),
            "r13" | "r13d" => Some(13),
            "r14" | "r14d" => Some(14),
            "r15" | "r15d" => Some(15),
            _ => None,
        }
    }

    fn cr_num(name: &str) -> Option<u8> {
        match name {
            "cr0" => Some(0),
            "cr2" => Some(2),
            "cr3" => Some(3),
            "cr4" => Some(4),
            _ => None,
        }
    }
}

fn const_to_i64(val: &ConstValue) -> i64 {
    match val {
        ConstValue::Int(v) => *v as i64,
        ConstValue::Uint(v) => *v as i64,
        ConstValue::Bool(b) => *b as i64,
        ConstValue::Char(c) => *c as i64,
        ConstValue::Float(f) => *f as i64,
        ConstValue::Str(_) => 0,
        ConstValue::FnItem(_) => 0,
        ConstValue::MethodRef(_) => 0,
        ConstValue::StaticRef(_) => 0,
        ConstValue::Unit => 0,
    }
}

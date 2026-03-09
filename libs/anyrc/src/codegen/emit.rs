use crate::ast::BinOp;
use crate::codegen::regalloc::RegAlloc;
use crate::codegen::x86asm::{CondCode, Label, Reg, Relocation, X86Assembler};
use crate::intern::Interner;
use crate::mir::*;

/// System V AMD64 argument registers
const ARG_REGS: [Reg; 6] = [Reg::RDI, Reg::RSI, Reg::RDX, Reg::RCX, Reg::R8, Reg::R9];

pub struct CodeEmitter<'a> {
    asm: X86Assembler,
    alloc: &'a RegAlloc,
    body: &'a MirBody,
    block_labels: Vec<Label>,
}

impl<'a> CodeEmitter<'a> {
    pub fn emit_fn(
        body: &MirBody,
        alloc: &RegAlloc,
        interner: &Interner,
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
        for i in 0..self.body.arg_count.min(ARG_REGS.len()) {
            let local = i + 1; // arguments start at local 1
            if local < self.alloc.stack_slots.len() {
                let slot = self.alloc.stack_slots[local];
                self.asm.mov_mr(Reg::RBP, slot, ARG_REGS[i]);
            }
        }
    }

    fn load_operand(&mut self, op: &Operand, dst: Reg) {
        match op {
            Operand::Copy(place) | Operand::Move(place) => {
                self.load_place(place, dst);
            }
            Operand::Constant(c) => {
                let val = const_to_i64(&c.value);
                self.asm.mov_ri(dst, val);
            }
        }
    }

    fn load_place(&mut self, place: &Place, dst: Reg) {
        let slot = self.alloc.stack_slots[place.local.0];
        if place.projections.is_empty() {
            self.asm.mov_rm(dst, Reg::RBP, slot);
        } else {
            // Load base address into dst, then apply projections
            self.asm.mov_rm(dst, Reg::RBP, slot);
            for proj in &place.projections {
                match proj {
                    Projection::Field(idx) => {
                        // Each field is 8 bytes
                        let field_offset = (*idx as i32) * 8;
                        // dst currently has the value at the base slot.
                        // For aggregates on the stack, we need the address.
                        // Load from [rbp + slot + field_offset]
                        self.asm.mov_rm(dst, Reg::RBP, slot + field_offset);
                    }
                    Projection::Deref => {
                        // dst has a pointer, dereference it
                        self.asm.mov_rm(dst, dst, 0);
                    }
                    Projection::Index(idx_local) => {
                        // Load index into RCX, compute address
                        self.asm.mov_rm(Reg::RCX, Reg::RBP, self.alloc.stack_slots[idx_local.0]);
                        // dst + index*8
                        self.asm.imul_rr(Reg::RCX, Reg::RCX); // placeholder; not ideal
                        // simplified: just load from base for now
                    }
                }
            }
        }
    }

    fn store_place(&mut self, place: &Place, src: Reg) {
        let slot = self.alloc.stack_slots[place.local.0];
        if place.projections.is_empty() {
            self.asm.mov_mr(Reg::RBP, slot, src);
        } else {
            // For field projections on stack-allocated aggregates
            let mut offset = slot;
            for proj in &place.projections {
                match proj {
                    Projection::Field(idx) => {
                        offset += (*idx as i32) * 8;
                    }
                    Projection::Deref => {
                        // Load the pointer from [rbp+offset] into a scratch reg
                        self.asm.mov_rm(Reg::R11, Reg::RBP, offset);
                        self.asm.mov_mr(Reg::R11, 0, src);
                        return;
                    }
                    Projection::Index(_) => {
                        // Simplified: store to base
                    }
                }
            }
            self.asm.mov_mr(Reg::RBP, offset, src);
        }
    }

    fn emit_statement(&mut self, stmt: &Statement) {
        match &stmt.kind {
            StatementKind::Assign(place, rvalue) => {
                self.emit_rvalue(rvalue, Reg::RAX);
                self.store_place(place, Reg::RAX);
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
                        self.asm.shl_ri(Reg::RAX, 0); // placeholder for dynamic shift
                    }
                    BinOp::Shr => {
                        self.asm.shr_ri(Reg::RAX, 0); // placeholder for dynamic shift
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
                    crate::ast::UnOp::Not => self.asm.not_r(dst),
                    crate::ast::UnOp::Deref => {
                        // dst has pointer, load from it
                        self.asm.mov_rm(dst, dst, 0);
                    }
                }
            }
            Rvalue::Ref(_, place) => {
                let slot = self.alloc.stack_slots[place.local.0];
                self.asm.lea(dst, Reg::RBP, slot);
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
            Rvalue::Discriminant(_) | Rvalue::Len(_) => {
                // Stub: load 0
                self.asm.xor_rr(dst, dst);
            }
        }
    }

    fn emit_terminator(&mut self, term: &Terminator, _interner: &Interner) {
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
            Terminator::Call { func: _, args, dest, target } => {
                // Move args into calling convention registers
                for (i, arg) in args.iter().enumerate() {
                    if i < ARG_REGS.len() {
                        self.load_operand(arg, ARG_REGS[i]);
                    }
                }
                // For now, all calls go through extern relocations
                // The func operand would tell us which function, but we emit a placeholder call
                self.asm.call_extern("__placeholder");
                // Store return value
                self.store_place(dest, Reg::RAX);
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
}

fn const_to_i64(val: &ConstValue) -> i64 {
    match val {
        ConstValue::Int(v) => *v as i64,
        ConstValue::Uint(v) => *v as i64,
        ConstValue::Bool(b) => *b as i64,
        ConstValue::Char(c) => *c as i64,
        ConstValue::Float(f) => *f as i64,
        ConstValue::Str(_) => 0,
        ConstValue::Unit => 0,
    }
}

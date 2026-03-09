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
    interner: &'a Interner,
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
            interner,
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
        // Each arg may span multiple slots (structs), consuming multiple registers.
        let mut reg_idx = 0;
        for i in 0..self.body.arg_count {
            let local = i + 1; // arguments start at local 1
            if local < self.alloc.stack_slots.len() {
                let slot = self.alloc.stack_slots[local];
                let n_slots = (self.alloc.local_sizes[local] / 8) as usize;
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
            Operand::Copy(place) | Operand::Move(place) => {
                if place.projections.is_empty() {
                    (self.alloc.local_sizes[place.local.0] / 8) as usize
                } else {
                    1
                }
            }
            Operand::Constant(_) => 1,
        }
    }

    fn load_operand(&mut self, op: &Operand, dst: Reg) {
        match op {
            Operand::Copy(place) | Operand::Move(place) => {
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
                Projection::Field(idx) => {
                    let field_offset = (*idx as i32) * 8;
                    match base {
                        Base::Stack(off) => {
                            self.asm.mov_rm(dst, Reg::RBP, off + field_offset);
                        }
                        Base::Reg => {
                            self.asm.mov_rm(dst, dst, field_offset);
                        }
                    }
                    base = Base::Reg; // after loading a value, further projections from reg
                }
                Projection::Index(idx_local) => {
                    self.asm.mov_rm(Reg::RCX, Reg::RBP, self.alloc.stack_slots[idx_local.0]);
                    // simplified for now
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
                // Special-case aggregates: store each field directly into the place
                if let Rvalue::Aggregate(_, operands) = rvalue {
                    if place.projections.is_empty() {
                        let base_slot = self.alloc.stack_slots[place.local.0];
                        for (i, op) in operands.iter().enumerate() {
                            self.load_operand(op, Reg::RAX);
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
                            let n_slots = copy_size / 8;
                            for i in 0..n_slots {
                                self.asm.mov_rm(Reg::RAX, Reg::RBP, src_slot + i * 8);
                                self.asm.mov_mr(Reg::RBP, dst_slot + i * 8, Reg::RAX);
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
            Rvalue::Discriminant(place) => {
                // For C-like enums, the value is the discriminant (first word)
                self.load_place(place, dst);
            }
            Rvalue::Len(_) => {
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
                // Extract the function name or do indirect call
                match func {
                    Operand::Constant(c) => match &c.value {
                        ConstValue::FnItem(sym) => {
                            let fn_name = interner.resolve(*sym).to_string();
                            self.asm.call_extern(&fn_name);
                        }
                        _ => {
                            self.asm.call_extern("__unknown");
                        }
                    },
                    Operand::Copy(place) | Operand::Move(place) => {
                        // Indirect call: load function pointer from place, call through register
                        self.load_place(place, Reg::R10);
                        self.asm.call_reg(Reg::R10);
                    }
                };
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
        ConstValue::StaticRef(_) => 0,
        ConstValue::Unit => 0,
    }
}

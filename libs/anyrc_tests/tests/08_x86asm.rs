use anyrc::codegen::x86asm::{CondCode, Reg, X86Assembler};

#[test]
fn encode_mov_reg_reg() {
    let mut asm = X86Assembler::new();
    asm.mov_rr(Reg::RAX, Reg::RBX);
    assert_eq!(asm.code(), &[0x48, 0x89, 0xD8]);
}

#[test]
fn encode_mov_reg_imm() {
    let mut asm = X86Assembler::new();
    asm.mov_ri(Reg::RAX, 42);
    assert_eq!(asm.code().len(), 10);
    assert_eq!(asm.code()[0], 0x48);
    assert_eq!(asm.code()[1], 0xB8);
    assert_eq!(asm.code()[2], 42);
}

#[test]
fn encode_add_reg_reg() {
    let mut asm = X86Assembler::new();
    asm.add_rr(Reg::RAX, Reg::RCX);
    assert_eq!(asm.code(), &[0x48, 0x01, 0xC8]);
}

#[test]
fn encode_sub_reg_imm() {
    let mut asm = X86Assembler::new();
    asm.sub_ri(Reg::RSP, 32);
    assert_eq!(asm.code()[0], 0x48);
}

#[test]
fn encode_push_pop() {
    let mut asm = X86Assembler::new();
    asm.push(Reg::RBP);
    asm.pop(Reg::RBP);
    assert_eq!(asm.code(), &[0x55, 0x5D]);
}

#[test]
fn encode_ret() {
    let mut asm = X86Assembler::new();
    asm.ret();
    assert_eq!(asm.code(), &[0xC3]);
}

#[test]
fn encode_cmp_and_jcc() {
    let mut asm = X86Assembler::new();
    let label = asm.new_label();
    asm.cmp_rr(Reg::RAX, Reg::RBX);
    asm.jcc(CondCode::Equal, label);
    asm.nop();
    asm.bind_label(label);
    asm.resolve_fixups();
    assert_eq!(&asm.code()[0..3], &[0x48, 0x39, 0xD8]);
    assert_eq!(&asm.code()[3..5], &[0x0F, 0x84]);
}

#[test]
fn encode_jmp_forward() {
    let mut asm = X86Assembler::new();
    let label = asm.new_label();
    asm.jmp(label);
    asm.nop();
    asm.nop();
    asm.bind_label(label);
    asm.resolve_fixups();
    assert_eq!(asm.code()[0], 0xE9);
    assert_eq!(asm.code()[1], 2);
}

#[test]
fn encode_call() {
    let mut asm = X86Assembler::new();
    let label = asm.new_label();
    asm.bind_label(label);
    asm.call_rel(label);
    asm.resolve_fixups();
    assert_eq!(asm.code()[0], 0xE8);
}

#[test]
fn encode_mov_mem() {
    let mut asm = X86Assembler::new();
    asm.mov_rm(Reg::RAX, Reg::RBP, -8);
    assert_eq!(asm.code(), &[0x48, 0x8B, 0x45, 0xF8]);
}

#[test]
fn encode_mov_to_mem() {
    let mut asm = X86Assembler::new();
    asm.mov_mr(Reg::RBP, -16, Reg::RAX);
    assert_eq!(asm.code(), &[0x48, 0x89, 0x45, 0xF0]);
}

#[test]
fn encode_lea() {
    let mut asm = X86Assembler::new();
    asm.lea(Reg::RAX, Reg::RBP, -8);
    assert_eq!(asm.code(), &[0x48, 0x8D, 0x45, 0xF8]);
}

#[test]
fn encode_r8_to_r15() {
    let mut asm = X86Assembler::new();
    asm.mov_rr(Reg::R8, Reg::R15);
    assert_eq!(asm.code()[0] & 0xF0, 0x40);
}

#[test]
fn encode_xor_self_zero() {
    let mut asm = X86Assembler::new();
    asm.xor_rr(Reg::RAX, Reg::RAX);
    assert_eq!(asm.code(), &[0x48, 0x31, 0xC0]);
}

#[test]
fn encode_imul() {
    let mut asm = X86Assembler::new();
    asm.imul_rr(Reg::RAX, Reg::RCX);
    assert_eq!(asm.code(), &[0x48, 0x0F, 0xAF, 0xC1]);
}

#[test]
fn encode_neg() {
    let mut asm = X86Assembler::new();
    asm.neg_r(Reg::RAX);
    assert_eq!(asm.code(), &[0x48, 0xF7, 0xD8]);
}

#[test]
fn encode_syscall() {
    let mut asm = X86Assembler::new();
    asm.syscall();
    assert_eq!(asm.code(), &[0x0F, 0x05]);
}

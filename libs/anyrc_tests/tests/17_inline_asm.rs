use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};
use anyrc::parser::Parser;
use anyrc::intern::Interner;

// ── Parser tests ──

#[test]
fn parse_asm_simple() {
    let src = r#"fn f() { unsafe { asm!("nop"); } }"#;
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let krate = parser.parse_crate();
    // Should parse without panic
    assert_eq!(krate.items.len(), 1);
}

#[test]
fn parse_asm_with_operands() {
    let src = r#"fn f(port: u16, val: u8) { unsafe { asm!("out dx, al", in("dx") port, in("al") val); } }"#;
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let krate = parser.parse_crate();
    assert_eq!(krate.items.len(), 1);
}

#[test]
fn parse_asm_with_options() {
    let src = r#"fn f() { unsafe { asm!("cli", options(nostack, nomem)); } }"#;
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let krate = parser.parse_crate();
    assert_eq!(krate.items.len(), 1);
}

#[test]
fn parse_asm_with_output() {
    let src = r#"fn f() -> u64 { let val: u64; unsafe { asm!("mov {}, cr0", out(reg) val); } val }"#;
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let krate = parser.parse_crate();
    assert_eq!(krate.items.len(), 1);
}

#[test]
fn parse_asm_multiple_templates() {
    let src = r#"fn f() { unsafe { asm!("cli", "hlt"); } }"#;
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let krate = parser.parse_crate();
    assert_eq!(krate.items.len(), 1);
}

// ── Compilation tests ──

fn assert_compiles(src: &str) {
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Obj,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    compile(src, "test.rs", &options).expect("compilation failed");
}

#[test]
fn compile_asm_nop() {
    assert_compiles(r#"
        fn do_nop() {
            unsafe { asm!("nop"); }
        }
        fn main() -> i32 { do_nop(); 0 }
    "#);
}

#[test]
fn compile_asm_cli_sti() {
    assert_compiles(r#"
        fn disable_interrupts() {
            unsafe { asm!("cli"); }
        }
        fn enable_interrupts() {
            unsafe { asm!("sti"); }
        }
        fn main() -> i32 { 0 }
    "#);
}

#[test]
fn compile_asm_with_input() {
    assert_compiles(r#"
        fn outb(port: u16, val: u8) {
            unsafe { asm!("out dx, al", in("dx") port, in("al") val); }
        }
        fn main() -> i32 { 0 }
    "#);
}

#[test]
fn compile_asm_with_options() {
    assert_compiles(r#"
        fn cli() {
            unsafe { asm!("cli", options(nostack, nomem, preserves_flags)); }
        }
        fn main() -> i32 { 0 }
    "#);
}

#[test]
fn compile_asm_multiple_instructions() {
    assert_compiles(r#"
        fn halt_loop() {
            unsafe {
                asm!("cli");
                asm!("hlt");
            }
        }
        fn main() -> i32 { 0 }
    "#);
}

// ── Codegen verification tests ──

#[test]
fn codegen_nop_bytes() {
    let src = r#"
        fn do_nop() {
            unsafe { asm!("nop"); }
        }
        fn main() -> i32 { 0 }
    "#;
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Obj,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    let obj_bytes = compile(src, "test.rs", &options).expect("compilation failed");
    // The object file should contain a 0x90 (nop) byte somewhere in the .text section
    assert!(obj_bytes.windows(1).any(|w| w[0] == 0x90),
        "expected nop (0x90) in generated object code");
}

#[test]
fn codegen_cli_sti_bytes() {
    let src = r#"
        fn do_cli_sti() {
            unsafe {
                asm!("cli");
                asm!("sti");
            }
        }
        fn main() -> i32 { 0 }
    "#;
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Obj,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    let obj_bytes = compile(src, "test.rs", &options).expect("compilation failed");
    // Should contain CLI (0xFA) followed by STI (0xFB) somewhere
    assert!(obj_bytes.windows(2).any(|w| w[0] == 0xFA && w[1] == 0xFB),
        "expected cli;sti (0xFA 0xFB) in generated object code");
}

#[test]
fn codegen_port_io_bytes() {
    let src = r#"
        fn outb(port: u16, val: u8) {
            unsafe { asm!("out dx, al", in("dx") port, in("al") val); }
        }
        fn main() -> i32 { 0 }
    "#;
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Obj,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    let obj_bytes = compile(src, "test.rs", &options).expect("compilation failed");
    // Should contain OUT DX, AL (0xEE) somewhere
    assert!(obj_bytes.contains(&0xEE),
        "expected 'out dx, al' (0xEE) in generated object code");
}

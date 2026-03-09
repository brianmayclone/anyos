use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};

#[test]
fn compile_returns_ok() {
    let source = "fn main() -> i32 { 42 }";
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    let result = compile(source, "test.rs", &options);
    assert!(result.is_ok());
    let exe = result.unwrap();
    assert_eq!(&exe[0..4], &[0x7f, b'E', b'L', b'F']);
}

#[test]
fn compile_with_error_returns_err() {
    let source = "fn main() { let x: i32 = true; }";
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    let result = compile(source, "test.rs", &options);
    assert!(result.is_err());
}

#[test]
fn compile_emit_obj() {
    let source = "fn foo() -> i32 { 42 }";
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test.o".to_string(),
        emit: EmitKind::Obj,
        opt_level: 0,
        crate_type: CrateType::Lib,
        crate_name: None,
    };
    let result = compile(source, "test.rs", &options);
    assert!(result.is_ok());
    let obj = result.unwrap();
    assert_eq!(&obj[0..4], &[0x7f, b'E', b'L', b'F']);
    assert_eq!(obj[16], 1);  // ET_REL
}

#[test]
fn compile_complex_program() {
    let source = r#"
        fn add(a: i32, b: i32) -> i32 { a + b }

        fn main() -> i32 {
            let sum: i32 = add(10, 20);
            sum
        }
    "#;
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    let result = compile(source, "test.rs", &options);
    assert!(result.is_ok(), "compilation failed: {:?}", result.err().unwrap().iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn compile_with_optimization() {
    let source = "fn main() -> i32 { let x: i32 = 5; x }";
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 1,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    let result = compile(source, "test.rs", &options);
    assert!(result.is_ok());
}

#[test]
fn compile_enum_and_match() {
    let source = r#"
        enum Color { Red, Green, Blue }
        fn value(c: Color) -> i32 {
            match c {
                Color::Red => 1,
                Color::Green => 2,
                Color::Blue => 3,
            }
        }
        fn main() -> i32 { value(Color::Green) }
    "#;
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    let result = compile(source, "test.rs", &options);
    assert!(result.is_ok(), "failed: {:?}", result.err().unwrap().iter().map(|e| &e.message).collect::<Vec<_>>());
}

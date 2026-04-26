use anyrc::ast::Mutability;
use anyrc::codegen::emit::CodeEmitter;
use anyrc::codegen::regalloc;
use anyrc::diagnostics::Span;
use anyrc::hir_lower::LoweringContext;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::mir::{
    BasicBlock, BlockId, ConstValue, Constant, Local, LocalDecl, MirBody, Operand, Place, Rvalue,
    Statement, StatementKind, Terminator,
};
use anyrc::mir_build::MirBuilder;
use anyrc::parser::Parser;
use anyrc::resolve::Resolver;
use anyrc::typeck::TypeChecker;
use anyrc::typeck::{IntTy, TyKind};

fn compile_fn(src: &str) -> Vec<u8> {
    let (code, _) = compile_fn_with_relocs(src);
    code
}

fn compile_fn_with_relocs(src: &str) -> (Vec<u8>, Vec<anyrc::codegen::x86asm::Relocation>) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&mut interner);
    let resolve_result = resolver.resolve_crate(&hir);
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let typeck_result = checker.check_crate(&hir);
    let bodies = MirBuilder::build_crate(&mut interner, &resolve_result, &typeck_result, &hir);
    let body = &bodies[0];
    let struct_sizes = regalloc::StructSizes::new();
    let field_offsets = regalloc::StructFieldOffsets::new();
    let field_types = regalloc::StructFieldTypes::new();
    let alloc = regalloc::allocate(body, &struct_sizes);
    CodeEmitter::emit_fn(
        body,
        &alloc,
        &interner,
        &struct_sizes,
        &field_offsets,
        &field_types,
    )
}

#[test]
fn codegen_simple_return() {
    let code = compile_fn("fn foo() -> i32 { 42 }");
    assert!(!code.is_empty());
}

#[test]
fn codegen_binary_add() {
    let code = compile_fn("fn foo(a: i32, b: i32) -> i32 { a + b }");
    assert!(!code.is_empty());
}

#[test]
fn codegen_if_else() {
    let code = compile_fn("fn foo(x: bool) -> i32 { if x { 1 } else { 2 } }");
    assert!(!code.is_empty());
}

#[test]
fn codegen_loop_and_break() {
    let code = compile_fn(
        "fn foo() -> i32 { let mut i: i32 = 0; loop { i = i + 1; if i > 10 { break; } } i }",
    );
    assert!(!code.is_empty());
}

#[test]
fn codegen_fn_call() {
    let (code, _relocs) = compile_fn_with_relocs("fn bar() -> i32 { 0 } fn foo() -> i32 { bar() }");
    assert!(!code.is_empty());
}

#[test]
fn codegen_struct_and_field() {
    let code = compile_fn(
        r#"
        struct Point { x: i32, y: i32 }
        fn foo() -> i32 { let p = Point { x: 10, y: 20 }; p.x }
    "#,
    );
    assert!(!code.is_empty());
}

#[test]
fn regalloc_reuses_non_overlapping_single_block_stack_slots() {
    let i32_ty = TyKind::Int(IntTy::I32);
    let local = |name| LocalDecl {
        ty: i32_ty.clone(),
        mutability: Mutability::Immutable,
        name,
        span: Span::dummy(),
    };
    let int = |value| {
        Operand::Constant(Constant {
            ty: i32_ty.clone(),
            value: ConstValue::Int(value),
        })
    };

    let mut interner = Interner::new();
    let body = MirBody {
        basic_blocks: vec![BasicBlock {
            statements: vec![
                Statement {
                    kind: StatementKind::Assign(Place::local(Local(1)), Rvalue::Use(int(1))),
                    span: Span::dummy(),
                },
                Statement {
                    kind: StatementKind::Assign(Place::local(Local(2)), Rvalue::Use(int(2))),
                    span: Span::dummy(),
                },
                Statement {
                    kind: StatementKind::Assign(
                        Place::local(Local(0)),
                        Rvalue::Use(Operand::Copy(Place::local(Local(2)))),
                    ),
                    span: Span::dummy(),
                },
            ],
            terminator: Terminator::Return,
        }],
        locals: vec![local(None), local(None), local(None)],
        arg_count: 0,
        name: interner.intern("foo"),
        span: Span::dummy(),
        no_mangle: false,
    };

    let alloc = regalloc::allocate(&body, &regalloc::StructSizes::new());
    assert_eq!(alloc.frame_size, 16);
    assert_eq!(alloc.stack_slots[1], alloc.stack_slots[2]);
}

#[test]
fn regalloc_keeps_values_live_across_basic_block_edges() {
    let i32_ty = TyKind::Int(IntTy::I32);
    let local = || LocalDecl {
        ty: i32_ty.clone(),
        mutability: Mutability::Immutable,
        name: None,
        span: Span::dummy(),
    };
    let int = |value| {
        Operand::Constant(Constant {
            ty: i32_ty.clone(),
            value: ConstValue::Int(value),
        })
    };

    let mut interner = Interner::new();
    let body = MirBody {
        basic_blocks: vec![
            BasicBlock {
                statements: vec![Statement {
                    kind: StatementKind::Assign(Place::local(Local(1)), Rvalue::Use(int(5))),
                    span: Span::dummy(),
                }],
                terminator: Terminator::Goto(BlockId(1)),
            },
            BasicBlock {
                statements: vec![
                    Statement {
                        kind: StatementKind::Assign(Place::local(Local(2)), Rvalue::Use(int(9))),
                        span: Span::dummy(),
                    },
                    Statement {
                        kind: StatementKind::Assign(
                            Place::local(Local(0)),
                            Rvalue::Use(Operand::Copy(Place::local(Local(1)))),
                        ),
                        span: Span::dummy(),
                    },
                ],
                terminator: Terminator::Return,
            },
        ],
        locals: vec![local(), local(), local()],
        arg_count: 0,
        name: interner.intern("foo"),
        span: Span::dummy(),
        no_mangle: false,
    };

    let alloc = regalloc::allocate(&body, &regalloc::StructSizes::new());
    assert_ne!(alloc.stack_slots[1], alloc.stack_slots[2]);
}

#[test]
fn regalloc_reuses_slots_after_dead_predecessor_value() {
    let i32_ty = TyKind::Int(IntTy::I32);
    let local = || LocalDecl {
        ty: i32_ty.clone(),
        mutability: Mutability::Immutable,
        name: None,
        span: Span::dummy(),
    };
    let int = |value| {
        Operand::Constant(Constant {
            ty: i32_ty.clone(),
            value: ConstValue::Int(value),
        })
    };

    let mut interner = Interner::new();
    let body = MirBody {
        basic_blocks: vec![
            BasicBlock {
                statements: vec![
                    Statement {
                        kind: StatementKind::Assign(Place::local(Local(1)), Rvalue::Use(int(5))),
                        span: Span::dummy(),
                    },
                    Statement {
                        kind: StatementKind::Assign(
                            Place::local(Local(0)),
                            Rvalue::Use(Operand::Copy(Place::local(Local(1)))),
                        ),
                        span: Span::dummy(),
                    },
                ],
                terminator: Terminator::Goto(BlockId(1)),
            },
            BasicBlock {
                statements: vec![Statement {
                    kind: StatementKind::Assign(Place::local(Local(2)), Rvalue::Use(int(9))),
                    span: Span::dummy(),
                }],
                terminator: Terminator::Return,
            },
        ],
        locals: vec![local(), local(), local()],
        arg_count: 0,
        name: interner.intern("foo"),
        span: Span::dummy(),
        no_mangle: false,
    };

    let alloc = regalloc::allocate(&body, &regalloc::StructSizes::new());
    assert_eq!(alloc.stack_slots[1], alloc.stack_slots[2]);
}

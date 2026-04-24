use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::ast::*;

fn parse_expr(src: &str) -> Expr {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    parser.parse_expr()
}

#[test]
fn parse_integer_literal() {
    let expr = parse_expr("42");
    assert!(matches!(expr, Expr::Lit(Literal::Int(42), _)));
}

#[test]
fn parse_binary_op_precedence() {
    let expr = parse_expr("1 + 2 * 3");
    match expr {
        Expr::Binary(BinOp::Add, lhs, rhs, _) => {
            assert!(matches!(*lhs, Expr::Lit(Literal::Int(1), _)));
            match *rhs {
                Expr::Binary(BinOp::Mul, l, r, _) => {
                    assert!(matches!(*l, Expr::Lit(Literal::Int(2), _)));
                    assert!(matches!(*r, Expr::Lit(Literal::Int(3), _)));
                }
                _ => panic!("expected mul"),
            }
        }
        _ => panic!("expected add"),
    }
}

#[test]
fn parse_function_call() {
    let expr = parse_expr("foo(1, 2)");
    match expr {
        Expr::Call(func, args, _) => {
            assert!(matches!(*func, Expr::Path(_)));
            assert_eq!(args.len(), 2);
        }
        _ => panic!("expected call"),
    }
}

#[test]
fn parse_method_call() {
    let expr = parse_expr("x.push(5)");
    match expr {
        Expr::MethodCall(_, _, _, args, _) => {
            assert_eq!(args.len(), 1);
        }
        _ => panic!("expected method call"),
    }
}

#[test]
fn parse_if_else() {
    let expr = parse_expr("if true { 1 } else { 2 }");
    assert!(matches!(expr, Expr::If(_, _, Some(_), _)));
}

#[test]
fn parse_match_expr() {
    let expr = parse_expr("match x { 0 => 1, _ => 2 }");
    match expr {
        Expr::Match(_, arms, _) => assert_eq!(arms.len(), 2),
        _ => panic!("expected match"),
    }
}

#[test]
fn parse_closure() {
    let expr = parse_expr("|x, y| x + y");
    assert!(matches!(expr, Expr::Closure(_, _, _, false, _)));
}

#[test]
fn parse_reference() {
    let expr = parse_expr("&mut x");
    match expr {
        Expr::Ref(_, Mutability::Mut, _) => {}
        _ => panic!("expected &mut"),
    }
}

#[test]
fn parse_struct_literal() {
    let expr = parse_expr("Point { x: 1, y: 2 }");
    match expr {
        Expr::Struct(_, fields, _, _) => assert_eq!(fields.len(), 2),
        _ => panic!("expected struct literal"),
    }
}

#[test]
fn parse_struct_literal_after_question_statement_in_if_block() {
    let expr = parse_expr(
        r#"
        if meta.input.peek(Token![=]) {
            let lifetimes = parse_lit_into_lifetimes(cx, &meta)?;
            BorrowAttribute {
                path: meta.path.clone(),
                lifetimes: Some(lifetimes),
            }
        } else {
            BorrowAttribute {
                path: meta.path.clone(),
                lifetimes: None,
            }
        }
        "#,
    );
    assert!(matches!(expr, Expr::If(_, _, Some(_), _)));
}

#[test]
fn parse_if_struct_literal_initializer_after_question_statement() {
    let expr = parse_expr(
        r#"
        if meta.path == BORROW {
            let borrow_attribute = if meta.input.peek(Token![=]) {
                let lifetimes = parse_lit_into_lifetimes(cx, &meta)?;
                BorrowAttribute {
                    path: meta.path.clone(),
                    lifetimes: Some(lifetimes),
                }
            } else {
                BorrowAttribute {
                    path: meta.path.clone(),
                    lifetimes: None,
                }
            };
            borrow.set(&meta.path, borrow_attribute);
        } else {
            other();
        }
        "#,
    );
    assert!(matches!(expr, Expr::If(_, _, Some(_), _)));
}

#[test]
fn parse_index() {
    let expr = parse_expr("arr[0]");
    assert!(matches!(expr, Expr::Index(_, _, _)));
}

#[test]
fn parse_unary_neg() {
    let expr = parse_expr("-5");
    assert!(matches!(expr, Expr::Unary(UnOp::Neg, _, _)));
}

#[test]
fn parse_array() {
    let expr = parse_expr("[1, 2, 3]");
    match expr {
        Expr::Array(elems, _) => assert_eq!(elems.len(), 3),
        _ => panic!("expected array"),
    }
}

#[test]
fn parse_tuple() {
    let expr = parse_expr("(1, 2, 3)");
    match expr {
        Expr::Tuple(elems, _) => assert_eq!(elems.len(), 3),
        _ => panic!("expected tuple"),
    }
}

#[test]
fn parse_block_expr() {
    let expr = parse_expr("{ let x = 1; x + 1 }");
    assert!(matches!(expr, Expr::Block(_)));
}

#[test]
fn parse_while_loop() {
    let expr = parse_expr("while x > 0 { x = x - 1 }");
    assert!(matches!(expr, Expr::While(_, _, _, _)));
}

#[test]
fn parse_return_expr() {
    let expr = parse_expr("return 42");
    match expr {
        Expr::Return(Some(e), _) => {
            assert!(matches!(*e, Expr::Lit(Literal::Int(42), _)));
        }
        _ => panic!("expected return"),
    }
}

use anyrc::borrowck::{self, BorrowckResult};
use anyrc::hir_lower::LoweringContext;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::mir_build::MirBuilder;
use anyrc::parser::Parser;
use anyrc::resolve::Resolver;
use anyrc::typeck::TypeChecker;

fn build_and_check(src: &str) -> Vec<(String, BorrowckResult)> {
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
    bodies
        .iter()
        .map(|body| {
            (
                interner.resolve(body.name).to_string(),
                borrowck::check_borrows(
                    body,
                    &interner,
                    &typeck_result.struct_defs,
                    &typeck_result.enum_variants,
                    &typeck_result.copy_types,
                ),
            )
        })
        .collect()
}

fn assert_borrowck_ok(src: &str) {
    let results = build_and_check(src);
    for (body_name, result) in &results {
        assert!(
            result.errors.is_empty(),
            "unexpected borrow errors: {:?}",
            result
                .errors
                .iter()
                .map(|e| format!("in {}: {}", body_name, e.message))
                .collect::<Vec<_>>()
        );
    }
}

fn assert_borrowck_error(src: &str, expected_msg: &str) {
    let results = build_and_check(src);
    let all_errors: Vec<_> = results.iter().flat_map(|(_, r)| &r.errors).collect();
    assert!(
        !all_errors.is_empty(),
        "expected borrow error containing '{}'",
        expected_msg
    );
    assert!(
        all_errors.iter().any(|e| e
            .message
            .to_lowercase()
            .contains(&expected_msg.to_lowercase())),
        "expected error containing '{}', got: {:?}",
        expected_msg,
        all_errors.iter().map(|e| &e.message).collect::<Vec<_>>()
    );
}

#[test]
fn borrowck_simple_ref() {
    assert_borrowck_ok("fn foo() { let x: i32 = 5; let y: &i32 = &x; }");
}

#[test]
fn borrowck_simple_mut_ref() {
    assert_borrowck_ok("fn foo() { let mut x: i32 = 5; let y: &mut i32 = &mut x; }");
}

#[test]
fn borrowck_use_after_move() {
    assert_borrowck_error(
        r#"
        struct S { x: &mut i32 }
        fn take(s: S) {}
        fn foo() { let mut v: i32 = 1; let s = S { x: &mut v }; take(s); take(s); }
    "#,
        "moved",
    );
}

#[test]
fn borrowck_two_mut_borrows() {
    assert_borrowck_error(
        r#"
        fn foo() { let mut x: i32 = 5; let a: &mut i32 = &mut x; let b: &mut i32 = &mut x; }
    "#,
        "borrow",
    );
}

#[test]
fn borrowck_assign_while_borrowed() {
    assert_borrowck_error(
        r#"
        fn foo() { let mut x: i32 = 5; let r: &i32 = &x; x = 6; }
    "#,
        "borrow",
    );
}

#[test]
fn borrowck_reassign_from_shared_call_borrow_ok() {
    assert_borrowck_ok(
        r#"
        fn replace(old: &i32) -> i32 { *old + 1 }
        fn foo() {
            let mut current: i32 = 1;
            current = replace(&current);
        }
    "#,
    );
}

#[test]
fn borrowck_temporary_mut_method_borrow_allows_later_shared_arg() {
    assert_borrowck_ok(
        r#"
        struct Buffer { value: i32 }

        impl Buffer {
            fn as_mut_ptr(&mut self) -> *mut i32 { &mut self.value as *mut i32 }
            fn len(&self) -> usize { 1 }
        }

        fn sink(ptr: *mut i32, len: usize) {}

        fn foo(buf: &mut Buffer) {
            sink(buf.as_mut_ptr(), buf.len());
        }
    "#,
    );
}

#[test]
fn borrowck_copy_types_ok() {
    assert_borrowck_ok("fn foo() { let x: i32 = 5; let y: i32 = x; let z: i32 = x; }");
}

#[test]
fn borrowck_derived_copy_adt_can_be_reused() {
    assert_borrowck_ok(
        r#"
        #[derive(Copy, Clone)]
        struct Span {
            lo: usize,
            hi: usize,
        }

        fn take(span: Span) {}

        fn foo() {
            let span = Span { lo: 0, hi: 1 };
            take(span);
            take(span);
        }
    "#,
    );
}

#[test]
fn borrowck_match_scrutinee_can_be_used_in_wildcard_arm() {
    assert_borrowck_ok(
        r#"
        struct Token {}

        fn take(token: Token) {}

        fn foo(token: Token) {
            match token {
                _ => take(token),
            }
        }
    "#,
    );
}

#[test]
fn borrowck_mut_ref_argument_is_reborrowed_for_calls() {
    assert_borrowck_ok(
        r#"
        fn write(f: &mut i32) {}

        fn foo(f: &mut i32) {
            write(f);
            write(f);
        }
    "#,
    );
}

#[test]
fn borrowck_move_in_if_branch_does_not_poison_else_branch() {
    assert_borrowck_ok(
        r#"
        struct Token {}

        fn take(token: Token) {}
        fn cond() -> bool { true }

        fn foo(token: Token) {
            if cond() {
                take(token);
            } else {
                take(token);
            }
        }
    "#,
    );
}

#[test]
fn borrowck_move_in_match_arm_does_not_poison_sibling_arms() {
    assert_borrowck_ok(
        r#"
        struct Token {}

        enum Context {
            A { token: Token },
            B { token: Token },
            C,
        }

        fn take(token: Token) {}

        fn foo(context: Context) {
            match context {
                Context::A { token } => take(token),
                Context::B { token } => take(token),
                Context::C => {}
            }
        }
    "#,
    );
}

#[test]
fn borrowck_method_chain_consumes_each_receiver_once() {
    assert_borrowck_ok(
        r#"
        struct Iter {}
        struct Filtered {}
        struct Mapped {}
        struct Acc {}

        impl Iter {
            fn filter(self) -> Filtered { Filtered {} }
        }

        impl Filtered {
            fn map(self) -> Mapped { Mapped {} }
        }

        impl Mapped {
            fn fold(self, acc: Acc) -> Acc { acc }
        }

        fn foo(iter: Iter, acc: Acc) {
            let _result = iter.filter().map().fold(acc);
        }
    "#,
    );
}

#[test]
fn borrowck_enum_with_only_copy_fields_can_be_reused() {
    assert_borrowck_ok(
        r#"
        struct Name {}

        enum StructVariant<'a> {
            ExternallyTagged {
                variant_index: u32,
                variant_name: &'a Name,
            },
            InternallyTagged {
                tag: &'a str,
                variant_name: &'a Name,
            },
            Untagged,
        }

        fn consume(context: StructVariant) {}

        fn foo(context: StructVariant) {
            consume(context);
            match context {
                StructVariant::ExternallyTagged { variant_index, variant_name } => {
                    let _idx = variant_index;
                    let _name = variant_name;
                }
                StructVariant::InternallyTagged { tag, variant_name } => {
                    let _tag = tag;
                    let _name = variant_name;
                }
                StructVariant::Untagged => {}
            }
        }
    "#,
    );
}

#[test]
fn borrowck_no_error_after_scope() {
    assert_borrowck_ok("fn foo() { let mut x: i32 = 5; let y: i32 = x; x = 6; }");
}

#[test]
fn borrowck_slice_reference_can_be_reassigned_to_subslice() {
    assert_borrowck_ok("fn foo(mut buf: &[u8], n: usize) { buf = &buf[n..]; }");
}

#[test]
fn borrowck_str_slice_values_are_reusable() {
    assert_borrowck_ok(
        r#"
        struct String {}

        impl String {
            fn from(_: &str) -> String { String {} }
        }

        fn foo(segment: &str) {
            let cmd = segment.trim();
            let path = String::from(cmd);
            let failed = String::from(cmd);
        }
    "#,
    );
}

#[test]
fn borrowck_trait_default_copy_self_is_not_moved() {
    assert_borrowck_ok(
        r#"
        trait Copy {}

        trait ConditionallySelectable: Copy {
            fn conditional_select(a: &Self, b: &Self, choice: bool) -> Self;

            fn conditional_assign(&mut self, other: &Self, choice: bool) {
                *self = Self::conditional_select(self, other, choice);
            }

            fn conditional_swap(a: &mut Self, b: &mut Self, choice: bool) {
                let t: Self = *a;
                a.conditional_assign(&b, choice);
                b.conditional_assign(&t, choice);
            }
        }
    "#,
    );
}

#[test]
fn borrowck_mut_ref_call_arg_is_reborrowed_not_moved() {
    assert_borrowck_ok(
        r#"
        fn select(a: &i32, b: &i32) -> i32 { *a }

        fn assign_through_ref(self_ref: &mut i32, other: &i32) {
            *self_ref = select(self_ref, other);
        }
    "#,
    );
}

#[test]
fn borrowck_subtle_integer_conditional_assign_pattern() {
    assert_borrowck_ok(
        r#"
        struct Choice(u8);

        impl Choice {
            fn unwrap_u8(&self) -> u8 {
                self.0
            }
        }

        trait Copy {}

        trait ConditionallySelectable: Copy {
            fn conditional_select(a: &Self, b: &Self, choice: Choice) -> Self;

            fn conditional_assign(&mut self, other: &Self, choice: Choice) {
                *self = Self::conditional_select(self, other, choice);
            }
        }

        impl ConditionallySelectable for u8 {
            fn conditional_select(a: &Self, b: &Self, choice: Choice) -> Self {
                let mask = -(choice.unwrap_u8() as i8) as u8;
                a ^ (mask & (a ^ b))
            }

            fn conditional_assign(&mut self, other: &Self, choice: Choice) {
                let mask = -(choice.unwrap_u8() as i8) as u8;
                *self ^= mask & (*self ^ *other);
            }
        }
    "#,
    );
}

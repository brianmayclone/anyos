use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use anyrc::typeck::{TypeChecker, TypeckResult, TyKind, IntTy, UintTy, FloatTy};

fn typecheck(src: &str) -> (TypeckResult, Interner) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&mut interner);
    let resolve_result = resolver.resolve_crate(&hir);
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let result = checker.check_crate(&hir);
    (result, interner)
}

fn typecheck_with_cfg(src: &str, cfg_flags: &[&str]) -> (TypeckResult, Interner) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    let flags = cfg_flags.iter().map(|flag| flag.to_string()).collect::<Vec<_>>();
    let cfg_ctx = anyrc::cfg::CfgContext::from_flags(&flags);
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    expand_macros(&mut krate, &mut interner);
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    expand_macros(&mut krate, &mut interner);
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&mut interner);
    let resolve_result = resolver.resolve_crate(&hir);
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let result = checker.check_crate(&hir);
    (result, interner)
}

fn assert_type_ok(src: &str) {
    let (result, _) = typecheck(src);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

fn assert_type_error(src: &str, expected_msg: &str) {
    let (result, _) = typecheck(src);
    assert!(!result.errors.is_empty(), "expected error containing '{}'", expected_msg);
    assert!(result.errors.iter().any(|e| e.message.to_lowercase().contains(&expected_msg.to_lowercase())),
        "expected error containing '{}', got: {:?}", expected_msg,
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn infer_integer_literal() {
    assert_type_ok("fn main() { let x: i32 = 42; }");
}

#[test]
fn infer_from_annotation() {
    assert_type_ok("fn main() { let x: u64 = 42; }");
}

#[test]
fn infer_bool() {
    assert_type_ok("fn main() { let x: bool = true; }");
}

#[test]
fn primitive_assoc_from_is_typed() {
    assert_type_ok("fn main() { let b: u8 = 7; let x: u32 = u32::from(b); }");
}

#[test]
fn primitive_assoc_try_from_is_result_typed() {
    assert_type_ok("fn main() { let n: usize = 7; let x = u32::try_from(n); }");
}

#[test]
fn primitive_endian_assoc_methods_are_typed() {
    assert_type_ok("fn main() { let n: u16 = 7; let x: u16 = u16::from_le(n); }");
}

#[test]
fn primitive_float_epsilon_consts_are_typed() {
    assert_type_ok("fn main() { let x: f64 = f64::EPSILON; let y: f32 = f32::EPSILON; let z: f64 = f64::MIN_POSITIVE; }");
}

#[test]
fn primitive_float_from_bits_is_typed() {
    assert_type_ok("fn main() { let x: f64 = f64::from_bits(0); let y: f32 = f32::from_bits(0); }");
}

#[test]
fn primitive_char_is_whitespace_assoc_fn_is_typed() {
    assert_type_ok("fn main() { let x: bool = char::is_whitespace(' '); }");
}

#[test]
fn scoped_use_prefers_local_type_over_same_named_external_type() {
    assert_type_ok(r#"
        mod fs {
            #[derive(Clone, Copy)]
            pub struct DirEntry {
                pub name_len: usize,
            }

            impl DirEntry {
                pub const EMPTY: Self = Self { name_len: 0 };

                pub fn name_len(&self) -> usize {
                    self.name_len
                }
            }

            pub const MAX_ENTRIES: usize = 4;

            pub fn read_dir(out: &mut [DirEntry; MAX_ENTRIES]) -> usize {
                out[0] = DirEntry::EMPTY;
                1
            }
        }

        mod panel {
            use crate::fs::{self, DirEntry, MAX_ENTRIES};

            pub struct Panel {
                pub entries: [DirEntry; MAX_ENTRIES],
                pub entry_count: usize,
            }

            impl Panel {
                pub fn new() -> Self {
                    let mut panel = Self {
                        entries: [DirEntry::EMPTY; MAX_ENTRIES],
                        entry_count: 0,
                    };
                    panel.reload();
                    panel
                }

                pub fn reload(&mut self) {
                    self.entry_count = fs::read_dir(&mut self.entries);
                    let n = self.entries[0].name_len();
                    self.entry_count = self.entry_count + n;
                }
            }
        }

        mod anyos_std {
            pub mod fs {
                pub struct DirEntry {
                    pub name: String,
                    pub size: u32,
                }

                pub fn metadata_size(entry: DirEntry) -> u32 {
                    entry.size
                }
            }
        }

        mod dependency_prelude_like {
            use crate::anyos_std::fs::{self, DirEntry};

            pub fn keep_imports_live(entry: DirEntry) -> u32 {
                fs::metadata_size(entry)
            }
        }
    "#);
}

#[test]
fn nested_use_of_root_reexport_chases_alias_target() {
    assert_type_ok(r#"
        mod token {
            pub struct Type {}
        }

        mod ty {
            pub struct Type {}
        }

        pub use crate::ty::Type;

        mod parse_quote {
            use crate::Type;

            fn takes_ty(_: crate::ty::Type) {}

            fn parse(value: Type) {
                takes_ty(value);
            }
        }
    "#);
}

#[test]
fn trait_self_associated_type_projection_in_method_signature_is_typed() {
    assert_type_ok(r#"
        struct NonNull<T> {}
        struct DstLayout {}

        pub unsafe trait KnownLayout {
            fn only_derive_is_allowed_to_implement_this_trait()
            where
                Self: Sized;

            type PointerMetadata: PointerMetadata;
            type MaybeUninit: ?Sized + KnownLayout<PointerMetadata = Self::PointerMetadata>;
            const LAYOUT: DstLayout;
            fn raw_from_ptr_len(bytes: NonNull<u8>, meta: Self::PointerMetadata) -> NonNull<Self>;
            fn pointer_to_metadata(ptr: *mut Self) -> Self::PointerMetadata;
        }

        trait PointerMetadata {}
    "#);
}

#[test]
fn core_float_consts_are_typed() {
    assert_type_ok(r#"
        fn main() {
            let a: f64 = core::f64::consts::LN_2;
            let b: f64 = core::f64::consts::FRAC_PI_2;
            let c: f32 = core::f32::consts::PI;
        }
    "#);
}

#[test]
fn vec_push_pop_last_truncate_are_typed() {
    assert_type_ok(r#"
        struct Vec<T> {}
        impl<T> Vec<T> {
            fn new() -> Vec<T> { Vec {} }
            fn push(&mut self, value: T) {}
            fn pop(&mut self) -> Option<T> { None }
            fn last(&self) -> Option<&T> { None }
            fn truncate(&mut self, len: usize) {}
        }
        enum Option<T> { Some(T), None }

        fn main() {
            let mut values: Vec<u32> = Vec::new();
            values.push(1);
            let x: u32 = values.pop().unwrap_or(0);
            let y: Option<&u32> = values.last();
            values.truncate(0);
        }
    "#);
}

#[test]
fn vec_macro_integer_element_infers_from_later_push() {
    assert_type_ok(r#"
        struct Vec<T> {}
        impl<T> Vec<T> {
            fn push(&mut self, value: T) {}
        }
        impl<T> [T] {
            fn to_vec(&self) -> Vec<T> { loop {} }
        }

        fn main() {
            let mut values = vec![0];
            let offset: u32 = 1;
            values.push(offset);
        }
    "#);
}

#[test]
fn vec_new_element_infers_from_index_assignment() {
    assert_type_ok(r#"
        struct Vec<T> {}
        impl<T> Vec<T> {
            fn new() -> Self { loop {} }
        }

        fn main() {
            let mut code = Vec::new();
            code[0] = 1u8;
        }
    "#);
}

#[test]
fn cfg_strips_false_tail_block_so_true_tail_returns() {
    let (result, _) = typecheck_with_cfg(
        r#"
            fn arch() -> &'static str {
                #[cfg(target_arch = "x86_64")]  { "x86_64" }
                #[cfg(target_arch = "aarch64")] { "aarch64" }
            }
        "#,
        &["target_arch=\"x86_64\""],
    );
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>()
    );
}

#[test]
fn loop_tail_match_does_not_force_arm_values_to_function_return() {
    assert_type_ok(r#"
        enum Op { Value, Unit }

        fn value() -> u32 { 1 }

        fn run(op: Op) -> u32 {
            loop {
                match op {
                    Op::Value => value(),
                    Op::Unit => {},
                }
            }
        }
    "#);
}

#[test]
fn nested_match_expression_inside_loop_keeps_its_value_type() {
    assert_type_ok(r#"
        enum Op { Yes, No }

        fn run(op: Op) -> bool {
            loop {
                let value = {
                    match op {
                        Op::Yes => true,
                        Op::No => false,
                    }
                };
                if value {
                    return true;
                }
            }
        }
    "#);
}

#[test]
fn nested_match_inside_discarded_loop_tail_match_keeps_value_type() {
    assert_type_ok(r#"
        enum Outer { Run, Done }
        enum Inner { Yes, No }

        fn run(outer: Outer, inner: Inner) -> bool {
            loop {
                match outer {
                    Outer::Run => {
                        let value = {
                            match inner {
                                Inner::Yes => true,
                                Inner::No => false,
                            }
                        };
                        if value {
                            return true;
                        }
                    }
                    Outer::Done => return false,
                }
            }
        }
    "#);
}

#[test]
fn let_rebinding_from_if_branch_with_module_function_preserves_value_type() {
    assert_type_ok(r#"
        mod promise {
            use super::Value;

            pub fn resolve(value: Value) -> Value {
                value
            }
        }

        enum Value { Undefined, Object }

        fn run(is_async: bool, value: Value) -> Value {
            let ret = if is_async {
                promise::resolve(value)
            } else {
                value
            };
            return ret;
        }
    "#);
}

#[test]
fn constructor_return_style_if_block_keeps_tail_value_after_continue_guard() {
    assert_type_ok(r#"
        enum Value { Undefined, Object }

        impl Value {
            fn is_object(&self) -> bool { true }
        }

        struct Frame {
            this_val: Value,
            is_constructor: bool,
        }

        fn run(frame: Frame, val: Value) -> Value {
            let ret = if frame.is_constructor && !val.is_object() {
                let is_derived = true;
                if is_derived {
                    continue;
                }
                frame.this_val
            } else {
                val
            };
            return ret;
        }
    "#);
}

#[test]
fn break_inside_for_does_not_define_enclosing_loop_type() {
    assert_type_ok(r#"
        fn run() -> u32 {
            loop {
                for i in 0..4 {
                    break;
                }
            }
        }
    "#);
}

#[test]
fn multiline_match_guard_condition_is_typed_as_bool() {
    assert_type_ok(r#"
        enum Value {
            Object(u32),
            Other,
        }

        fn is_match(value: Value) -> bool {
            match value {
                Value::Object(ref inner)
                    if *inner
                        == 7 => true,
                _ => false,
            }
        }
    "#);
}

#[test]
fn match_arm_return_and_continue_paths_do_not_poison_numeric_arms() {
    assert_type_ok(r#"
        enum Option<T> { Some(T), None }

        fn parse_digit(bytes: &[u8], pos: &mut usize) -> Option<u32> {
            let d = match bytes[*pos] {
                b'0'..=b'9' => (bytes[*pos] - b'0') as u32,
                _ => return None,
            };
            *pos += 1;
            Some(d)
        }
    "#);
}

#[test]
fn ref_mut_tuple_variant_pattern_on_mut_ref_binds_inner_mut_ref() {
    assert_type_ok(r#"
        enum Op {
            Jump(i32),
            Other,
        }

        fn patch(op: &mut Op, target: i32) {
            match op {
                Op::Jump(ref mut off) => *off = target,
                _ => {}
            }
        }
    "#);
}

#[test]
fn string_like_to_string_and_deref_are_typed() {
    assert_type_ok(r#"
        struct String {}

        fn take_str(_: &str) {}

        fn main() {
            let s: &str = "hello";
            let owned: String = s.to_string();
            let owned_again: String = str::to_string(s);
            take_str(&*owned);
        }
    "#);
}

#[test]
fn string_like_as_str_returns_ref_str() {
    assert_type_ok(r#"
        struct String {}

        fn take_str(_: &str) {}

        fn main() {
            let owned = String {};
            let s: &str = owned.as_str();
            take_str(s.trim());
            match s {
                "solid" => {},
                _ => {},
            }
        }
    "#);
}

#[test]
fn string_range_index_trim_passes_ref_str() {
    assert_type_ok(r#"
        fn take_str(_: &str) {}

        fn main() {
            let kw: &str = "auto 16/9";
            let pos: usize = 7;
            let w = kw[..pos]
                .trim()
                .trim_start_matches("auto")
                .trim_end_matches("auto")
                .trim();
            take_str(w);
        }
    "#);
}

#[test]
fn string_like_comparisons_normalize_refs_and_slices() {
    assert_type_ok(r#"
        struct String {}

        fn main() {
            let owned = String {};
            let mut mutable = String {};
            let borrowed: &str = "auto";
            let _a: bool = owned.as_str() == "auto";
            let _b: bool = borrowed[..borrowed.len()] == "auto";
            let _c: bool = &mut mutable == "auto";
            let _d: &str = borrowed[..borrowed.len()];
        }
    "#);
}

#[test]
fn borrowed_field_ref_enum_string_pattern_matches_css_style_use() {
    assert_type_ok(r#"
        struct String {}

        enum Option<T> {
            Some(T),
            None,
        }

        enum CssValue {
            Keyword(String),
            Number(i32),
        }

        enum Property {
            AspectRatio,
        }

        struct Declaration {
            property: Property,
            value: CssValue,
        }

        fn try_parse_simple_float(_: &str) -> Option<i32> {
            Option::None
        }

        fn apply_declaration(decl: &Declaration) {
            match decl.property {
                Property::AspectRatio => {
                    if let CssValue::Keyword(ref kw) = decl.value {
                        if kw == "auto" {
                        } else if let Some(pos) = kw.find('/') {
                            let w_str = kw[..pos]
                                .trim()
                                .trim_start_matches("auto")
                                .trim_end_matches("auto")
                                .trim();
                            let h_str = kw[pos + 1..]
                                .trim()
                                .trim_start_matches("auto")
                                .trim_end_matches("auto")
                                .trim();
                            if let (Some(w), Some(h)) =
                                (try_parse_simple_float(w_str), try_parse_simple_float(h_str))
                            {
                                let _ratio = w * 100 / h;
                            }
                        } else if let Some(v) =
                            try_parse_simple_float(kw.trim().trim_start_matches("auto").trim())
                        {
                            let _ratio = v;
                        }
                    } else if let CssValue::Number(v) = decl.value {
                        let _ratio = v;
                    }
                }
            }
        }
    "#);
}

#[test]
fn string_match_assigns_enum_field_from_string_patterns() {
    assert_type_ok(r#"
        struct String {}

        enum CssValue {
            Keyword(String),
        }

        enum TextDecorationStyle {
            Solid,
            Double,
            Dotted,
            Dashed,
            Wavy,
        }

        struct ComputedStyle {
            text_decoration_style: TextDecorationStyle,
        }

        struct Declaration {
            value: CssValue,
        }

        fn apply_declaration(style: &mut ComputedStyle, decl: &Declaration) {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_decoration_style = match kw.as_str() {
                    "solid" => TextDecorationStyle::Solid,
                    "double" => TextDecorationStyle::Double,
                    "dotted" => TextDecorationStyle::Dotted,
                    "dashed" => TextDecorationStyle::Dashed,
                    "wavy" => TextDecorationStyle::Wavy,
                    _ => style.text_decoration_style,
                };
            }
        }
    "#);
}

#[test]
fn string_match_assigns_enum_field_through_glob_imported_style_type() {
    assert_type_ok(r#"
        struct String {}

        enum CssValue {
            Keyword(String),
        }

        struct Declaration {
            value: CssValue,
        }

        mod style {
            pub mod types {
                #[derive(Clone, Copy)]
                pub enum TextDecorationStyle {
                    Solid,
                    Double,
                    Dotted,
                    Dashed,
                    Wavy,
                }

                pub struct ComputedStyle {
                    pub text_decoration_style: TextDecorationStyle,
                }
            }

            pub mod engine {
                use super::types::*;
                use crate::{CssValue, Declaration};

                pub fn apply_declaration(style: &mut ComputedStyle, decl: &Declaration) {
                    if let CssValue::Keyword(ref kw) = decl.value {
                        style.text_decoration_style = match kw.as_str() {
                            "solid" => TextDecorationStyle::Solid,
                            "double" => TextDecorationStyle::Double,
                            "dotted" => TextDecorationStyle::Dotted,
                            "dashed" => TextDecorationStyle::Dashed,
                            "wavy" => TextDecorationStyle::Wavy,
                            _ => style.text_decoration_style,
                        };
                    }
                }
            }
        }
    "#);
}

#[test]
fn match_tuple_arms_coerce_function_items_to_fn_pointer_fields() {
    assert_type_ok(r#"
        enum WorkerClass {
            A,
            B,
        }

        fn worker_a() {}
        fn worker_b() {}

        fn ensure_worker(class: WorkerClass) {
            let (entry, thread_name): (fn(), &str) = match class {
                WorkerClass::A => (worker_a, "a"),
                WorkerClass::B => (worker_b, "b"),
            };
        }
    "#);
}

#[test]
fn for_ref_pattern_over_u8_slice_binds_u8() {
    assert_type_ok(r#"
        fn parse_decimal(b: &[u8]) -> u32 {
            let mut n: u32 = 0;
            for &c in b {
                if c >= b'0' && c <= b'9' {
                    n = n * 10 + (c - b'0') as u32;
                }
            }
            n
        }
    "#);
}

#[test]
fn intrinsic_box_preserves_type_argument_for_deref() {
    assert_type_ok(r#"
        fn take_boxed(value: Box<u8>) {
            let byte: u8 = *value;
        }

        fn box_new_preserves_inner_type() {
            let value = Box::new(7u8);
            let byte: u8 = *value;
        }
    "#);
}

#[test]
fn match_ref_enum_variant_fields_bind_by_reference() {
    assert_type_ok(r#"
        enum Pattern {
            Ident(Option<Box<Pattern>>),
            Or(Vec<Pattern>),
            Other,
        }

        fn take_pattern_ref(_: &Pattern) {}

        fn walk(pat: &Pattern) {
            match pat {
                Pattern::Ident(sub) => {
                    if let Some(sub) = sub {
                        take_pattern_ref(sub);
                    }
                }
                Pattern::Or(pats) => {
                    for pat in pats {
                        take_pattern_ref(pat);
                    }
                }
                Pattern::Other => {}
            }
        }
    "#);
}

#[test]
fn iterator_filter_preserves_borrowed_item_type() {
    assert_type_ok(r#"
        enum Pattern {
            Rest,
            Other,
        }

        fn take_pattern_ref(_: &Pattern) {}

        fn walk(pats: Vec<Pattern>) {
            let non_rest = pats.iter().filter(|p| true);
            for pat in non_rest {
                take_pattern_ref(pat);
            }
        }
    "#);
}

#[test]
fn vec_retain_on_mutably_borrowed_field_expects_bool_closure() {
    assert_type_ok(r#"
        struct Statement {}

        struct Block {
            statements: Vec<Statement>,
        }

        fn keep(stmt: &Statement) -> bool {
            true
        }

        fn optimize(block: &mut Block) {
            block.statements.retain(|stmt| {
                keep(stmt)
            });
        }
    "#);
}

#[test]
fn array_lengths_resolve_const_path_expressions() {
    assert_type_ok(r#"
        const TAG_SIZE: usize = 16;

        fn make_tag() -> [u8; TAG_SIZE] {
            [0u8; TAG_SIZE]
        }

        fn take_tag(tag: &mut [u8; TAG_SIZE]) {
            *tag = make_tag();
        }
    "#);
}

#[test]
fn module_array_lengths_resolve_local_const_path_expressions() {
    assert_type_ok(r#"
        mod crypto {
            pub const TAG_SIZE: usize = 16;

            pub fn make_tag() -> [u8; TAG_SIZE] {
                [0u8; TAG_SIZE]
            }
        }
    "#);
}

#[test]
fn nested_module_array_lengths_resolve_local_const_path_expressions() {
    assert_type_ok(r#"
        mod crypto {
            pub mod poly1305 {
                pub const TAG_SIZE: usize = 16;

pub fn make_tag() -> [u8; TAG_SIZE] {
                    [0u8; 16]
                }
            }
        }
    "#);
}

#[test]
fn imported_module_const_paths_work_in_array_lengths() {
    assert_type_ok(r#"
        mod crypto {
            pub mod sha256 {
                pub const DIGEST_SIZE: usize = 32;

                pub struct Sha256 {}

                impl Sha256 {
                    pub fn finalize(self) -> [u8; DIGEST_SIZE] {
                        [0u8; 32]
                    }
                }
            }

            pub mod hmac {
                use crate::crypto::sha256::{self, Sha256};

                pub fn finalize(ctx: Sha256) -> [u8; sha256::DIGEST_SIZE] {
                    ctx.finalize()
                }
            }
        }
    "#);
}

#[test]
fn imported_consts_work_in_array_repeat_lengths() {
    assert_type_ok(r#"
        mod panel {
            use crate::fs::{path_join, MAX_PATH};

            pub fn selected_paths() -> ([([u8; MAX_PATH], usize); 64], usize) {
                let mut result = [([0u8; MAX_PATH], 0usize); 64];
                result[0] = path_join();
                (result, 1usize)
            }
        }

        mod fs {
            pub const MAX_PATH: usize = 512;

            pub fn path_join() -> ([u8; MAX_PATH], usize) {
                let buf = [0u8; MAX_PATH];
                (buf, 0usize)
            }
        }
    "#);
}

#[test]
fn fn_items_coerce_to_fn_pointer_in_arrays_and_matches() {
    assert_type_ok(r#"
        fn a(_: u32) {}
        fn b(_: u32) {}
        fn c(_: u32) {}

        fn call(f: fn(u32)) {
            f(1);
        }

        fn run(which: u32) {
            let tests: [fn(u32); 3] = [a, b, c];
            tests[0](1);

            let selected = match which {
                0 => a,
                1 => b,
                _ => c,
            };
            call(selected);
        }
    "#);
}

#[test]
fn float_primitive_assoc_min_max_consts_typecheck() {
    assert_type_ok(r#"
        fn use_float_consts() {
            let a: f32 = f32::MAX;
            let b: f32 = f32::MIN;
            let c: f64 = f64::MAX;
            let d: f64 = f64::MIN;
        }
    "#);
}

#[test]
fn loop_expression_uses_break_value_type() {
    assert_type_ok(r#"
        fn recv() {
            let plaintext = loop {
                let pt: Vec<u8> = Vec::new();
                break pt;
            };
            let byte: u8 = plaintext[0];
        }
    "#);
}

#[test]
fn shift_mask_inherits_integer_type_from_bitand_lhs() {
    assert_type_ok(r#"
        fn read_bits(bit_buf: u32, count: u8) -> u32 {
            bit_buf & ((1 << count) - 1)
        }
    "#);
}

#[test]
fn shift_assign_allows_different_integer_rhs_type() {
    assert_type_ok(r#"
        fn read_bits(mut bit_buf: u32, count: u8) -> u32 {
            bit_buf >>= count;
            bit_buf
        }
    "#);
}

#[test]
fn struct_literal_prefers_resolved_local_use_over_same_name_global_type() {
    assert_type_ok(r#"
        mod object {
            pub struct Object {
                pub data: u8,
            }
        }

        mod anyos_std {
            pub mod json {
                pub struct Object {
                    pub json: u8,
                }
            }
        }

        mod transport {
            fn write_object(_: crate::object::Object) {}

            fn stream_parse_objects() {
                use crate::object::Object;

                let obj = Object { data: 1u8 };
                write_object(obj);
            }
        }
    "#);
}

#[test]
fn return_inside_retain_closure_targets_closure_return_type() {
    assert_type_ok(r#"
        struct Statement {
            keep: bool,
        }

        struct Block {
            statements: Vec<Statement>,
        }

        fn optimize(block: &mut Block) {
            block.statements.retain(|stmt| {
                if stmt.keep {
                    return true;
                }
                false
            });
        }
    "#);
}

#[test]
fn return_before_nested_item_keeps_block_diverging() {
    assert_type_ok(r#"
        struct Error {}
        struct Span {}
        struct String {}

        fn new_error(span: Span, message: String) -> Error {
            return make_error(span, message);

            fn make_error(span: Span, message: String) -> Error {
                Error {}
            }
        }
    "#);
}

#[test]
fn aliased_core_slice_iter_does_not_fall_back_to_local_iter() {
    assert_type_ok(r#"
        use core::slice;

        trait IntoIterator {
            type Item;
            type IntoIter;
            fn into_iter(self) -> Self::IntoIter;
        }

        struct Vec<T> {}
        impl<T> Vec<T> {
            fn iter(&self) -> slice::Iter<T> {
                loop {}
            }
        }

        struct ErrorMessage {}
        struct Error {
            messages: Vec<ErrorMessage>,
        }

        struct Iter<'a> {
            messages: slice::Iter<'a, ErrorMessage>,
        }

        impl<'a> IntoIterator for &'a Error {
            type Item = Error;
            type IntoIter = Iter<'a>;

            fn into_iter(self) -> Self::IntoIter {
                Iter {
                    messages: self.messages.iter(),
                }
            }
        }
    "#);
}

#[test]
fn ref_to_box_str_coerces_to_ref_str() {
    assert_type_ok(r#"
        struct Box<T> {}

        struct Repr {
            suffix: Box<str>,
        }

        struct Lit {
            repr: Box<Repr>,
        }

        impl Lit {
            fn suffix(&self) -> &str {
                &self.repr.suffix
            }
        }
    "#);
}

#[test]
fn qualified_anyos_std_vec_and_string_use_builtin_coercions() {
    assert_type_ok(r#"
        mod anyos_std {
            pub struct String {}

            pub struct Vec<T> {}

            impl<T> Vec<T> {
                pub fn new() -> Vec<T> {
                    Vec {}
                }

                pub fn push(&mut self, value: T) {}
            }
        }

        fn takes_str(value: &str) {}
        fn takes_slice(values: &[&str]) {}

        fn main() {
            let s = anyos_std::String {};
            takes_str(&s);

            let mut values: anyos_std::Vec<&str> = anyos_std::Vec::new();
            values.push("hello");
            takes_slice(&values);

            let first: &str = values[0];
        }
    "#);
}

#[test]
fn qualified_anyos_std_consts_resolve_from_module_interface() {
    assert_type_ok(r#"
        mod anyos_std {
            pub mod sys {
                pub const CON_MODE_HIDE_CURSOR: u32 = 1;
                pub const CON_MODE_NO_AUTOSCROLL: u32 = 2;
            }

            pub mod net {
                pub const NET_TRACE_ENTRY_SIZE: usize = 24;
                pub const NET_TRACE_DIR_RX: u8 = 0;
                pub const NET_TRACE_DIR_TX: u8 = 1;
            }
        }

        struct Entry {
            direction: u8,
        }

        fn main() {
            let flags: u32 =
                anyos_std::sys::CON_MODE_HIDE_CURSOR
                | anyos_std::sys::CON_MODE_NO_AUTOSCROLL;
            let mut raw = [0u8; anyos_std::net::NET_TRACE_ENTRY_SIZE * 4];
            let entry = Entry { direction: anyos_std::net::NET_TRACE_DIR_RX };
            let is_tx = entry.direction == anyos_std::net::NET_TRACE_DIR_TX;
        }
    "#);
}

#[test]
fn str_range_indexing_autoderefs_nested_refs() {
    assert_type_ok(r#"
        fn suffix(name: &&str, pos: usize) -> &str {
            &name[pos + 1..]
        }
    "#);
}

#[test]
fn question_operator_unwraps_option_for_tuple_patterns() {
    assert_type_ok(r#"
        enum Option<T> {
            Some(T),
            None,
        }

        struct Box<T> {}

        fn parse_inner(input: &str) -> Option<(Box<str>, Box<str>)> {
            loop {}
        }

        fn parse_outer(input: &str) -> Option<(Box<str>, Box<str>)> {
            let (value, suffix) = parse_inner(input)?;
            Some((value, suffix))
        }
    "#);
}

#[test]
fn primitive_integer_min_max_associated_functions_return_receiver_type() {
    assert_type_ok(r#"
        fn pick_usize(a: usize, b: usize) -> usize {
            usize::max(a, b)
        }

        fn pick_i32(a: i32, b: i32) -> i32 {
            i32::min(a, b)
        }
    "#);
}

#[test]
fn if_let_chain_binds_pattern_in_guard_and_body() {
    assert_type_ok(r#"
        enum Option<T> {
            Some(T),
            None,
        }

        struct Policy {
            max_files: Option<usize>,
        }

        fn check(policy: Policy, projected_files: usize) -> usize {
            if let Some(limit) = policy.max_files
                && projected_files > limit
            {
                limit
            } else {
                0
            }
        }
    "#);
}

#[test]
fn field_access_autoderefs_through_deref_target() {
    assert_type_ok(r#"
        trait Deref {
            type Target;
        }

        struct RawTable {
            table: usize,
        }

        struct ScopeGuard<T, F> {
            value: T,
            f: F,
        }

        impl<T, F> Deref for ScopeGuard<T, F> {
            type Target = T;
        }

        fn read_table<F>(guard: ScopeGuard<&mut RawTable, F>) -> usize {
            guard.table
        }
    "#);
}

#[test]
fn unary_deref_uses_deref_target_impl() {
    assert_type_ok(r#"
        trait Deref {
            type Target;
        }

        struct ScopeGuard<T, F> {
            value: T,
            f: F,
        }

        impl<T, F> Deref for ScopeGuard<T, F> {
            type Target = T;
        }

        fn consume<T>(value: T) {}

        fn use_guard<T, F>(guard: ScopeGuard<T, F>) {
            consume(*guard);
        }
    "#);
}

#[test]
fn ref_argument_coerces_through_deref_target() {
    assert_type_ok(r#"
        trait Deref {
            type Target;
        }

        struct Control {}
        struct TextField {}

        impl Deref for TextField {
            type Target = Control;
        }

        fn get_field_text(_: &Control) {}

        fn main() {
            let field = TextField {};
            get_field_text(&field);
        }
    "#);
}

#[test]
fn for_loop_uses_concrete_into_iterator_item_type() {
    assert_type_ok(r#"
        trait IntoIterator {
            type Item;
        }

        struct FullBucketsIndices {}

        impl IntoIterator for FullBucketsIndices {
            type Item = usize;
        }

        fn full_buckets_indices() -> FullBucketsIndices {
            FullBucketsIndices {}
        }

        fn takes_index(index: usize) {}

        fn resize_inner() {
            for full_byte_index in full_buckets_indices() {
                takes_index(full_byte_index);
            }
        }
    "#);
}

#[test]
fn for_loop_uses_iterator_item_type_through_blanket_into_iterator() {
    assert_type_ok(r#"
        trait Iterator {
            type Item;
        }

        struct FullBucketsIndices {}

        impl Iterator for FullBucketsIndices {
            type Item = usize;
        }

        fn full_buckets_indices() -> FullBucketsIndices {
            FullBucketsIndices {}
        }

        fn takes_index(index: usize) {}

        fn resize_inner() {
            for full_byte_index in full_buckets_indices() {
                takes_index(full_byte_index);
            }
        }
    "#);
}

#[test]
fn for_loop_can_infer_iterator_item_from_next_method() {
    assert_type_ok(r#"
        enum Option<T> {
            Some(T),
            None,
        }

        struct FullBucketsIndices {}

        impl FullBucketsIndices {
            fn next(&mut self) -> Option<usize> {
                None
            }
        }

        fn full_buckets_indices() -> FullBucketsIndices {
            FullBucketsIndices {}
        }

        fn takes_index(index: usize) {}

        fn resize_inner() {
            for full_byte_index in full_buckets_indices() {
                takes_index(full_byte_index);
            }
        }
    "#);
}

#[test]
fn syn_lit_style_option_parser_uses_question_on_string_get() {
    assert_type_ok(r#"
        enum Option<T> {
            Some(T),
            None,
        }

        struct Box<T> {}
        struct String {}

        impl String {
            fn new() -> Self { String {} }
            fn push(&mut self, ch: char) {}
            fn into_boxed_str(self) -> Box<str> { loop {} }
        }

        impl str {
            fn get(&self, range: core::ops::RangeFrom<usize>) -> Option<&str> { loop {} }
        }

        fn byte(s: &str, offset: usize) -> u8 { 0 }
        fn next_chr(s: &str) -> char { 'x' }

        fn parse_lit_str_cooked(mut s: &str) -> Option<(Box<str>, Box<str>)> {
            s = s.get(1..)?;

            let mut content = String::new();
            loop {
                let ch = match byte(s, 0) {
                    b'"' => break,
                    _ => {
                        let ch = next_chr(s);
                        s = s.get(1..)?;
                        ch
                    }
                };
                content.push(ch);
            }

            let content = content.into_boxed_str();
            let suffix = s.get(1..)?.to_owned().into_boxed_str();
            Some((content, suffix))
        }
    "#);
}

#[test]
fn option_some_and_none_in_loop_parser_infer_same_payload() {
    assert_type_ok(r#"
        enum Option<T> {
            Some(T),
            None,
        }

        struct Box<T> {}
        struct String {}

        impl String {
            fn new() -> Self { String {} }
            fn push(&mut self, ch: char) {}
            fn into_boxed_str(self) -> Box<str> { loop {} }
        }

        fn byte(s: &str, offset: usize) -> u8 { 0 }
        fn next_chr(s: &str) -> char { 'x' }

        fn parse_lit_str_cooked(mut s: &str) -> Option<(Box<str>, Box<str>)> {
            let mut content = String::new();
            loop {
                let ch = match byte(s, 0) {
                    b'"' => break,
                    b'\\' => {
                        let b = byte(s, 1);
                        match b {
                            b'n' => '\n',
                            _ => return None,
                        }
                    }
                    _ => {
                        let ch = next_chr(s);
                        ch
                    }
                };
                content.push(ch);
            }

            let content = content.into_boxed_str();
            let suffix = content;
            Some((content, suffix))
        }
    "#);
}

#[test]
fn assign_op_allows_adt_receiver_with_primitive_rhs() {
    assert_type_ok(r#"
        struct BigInt {}

        fn parse_lit_int() {
            let mut value = BigInt {};
            let digit: u8 = 7;
            value += digit;
        }
    "#);
}

#[test]
fn unreachable_macro_diverges_in_match_arm() {
    assert_type_ok(r#"
        enum Option<T> {
            Some(T),
            None,
        }

        struct Box<T> {}

        fn cooked(s: &str) -> Option<(Box<str>, Box<str>)> { loop {} }
        fn raw(s: &str) -> Option<(Box<str>, Box<str>)> { loop {} }
        fn byte(s: &str, offset: usize) -> u8 { 0 }

        fn parse_lit_str(s: &str) -> Option<(Box<str>, Box<str>)> {
            match byte(s, 0) {
                b'"' => cooked(s),
                b'r' => raw(s),
                _ => unreachable!(),
            }
        }
    "#);
}

#[test]
fn for_loop_over_token_stream_binds_token_tree_items() {
    assert_type_ok(r#"
        struct Span {}

        mod proc_macro2 {
            pub struct TokenStream {}
            pub enum TokenTree {
                Group,
                Ident,
            }
        }

        use proc_macro2::{TokenStream, TokenTree};

        trait IntoIterator {
            type Item;
            type IntoIter;
            fn into_iter(self) -> Self::IntoIter;
        }

        fn respan_token_tree(token: TokenTree, span: Span) -> TokenTree {
            token
        }

        fn parse_spanned(tokens: TokenStream, span: Span) {
            for token in tokens {
                respan_token_tree(token, span);
            }
        }
    "#);
}

#[test]
fn proc_macro2_intrinsic_enum_variants_have_parent_type() {
    assert_type_ok(r#"
        use proc_macro2::{Delimiter, Spacing};

        fn takes_spacing(_: Spacing) {}
        fn takes_delimiter(_: Delimiter) {}

        fn main() {
            takes_spacing(Spacing::Joint);
            takes_spacing(Spacing::Alone);
            takes_delimiter(Delimiter::Brace);
            takes_delimiter(Delimiter::None);
        }
    "#);
}

#[test]
fn quote_macros_typecheck_as_proc_macro2_token_stream() {
    assert_type_ok(r#"
        mod proc_macro2 {
            pub struct TokenStream {}
            impl TokenStream {
                pub fn new() -> TokenStream {
                    TokenStream {}
                }
            }
        }

        mod fragment {
            use crate::proc_macro2::TokenStream;

            pub enum Fragment {
                Block(TokenStream),
            }
        }

        macro_rules! quote_block {
            ($($tt:tt)*) => {
                crate::fragment::Fragment::Block(quote!($($tt)*))
            }
        }

        fn make(flag: bool) -> fragment::Fragment {
            if flag {
                quote_block! { struct Demo; }
            } else {
                quote_block! { impl Demo {} }
            }
        }

        fn make_spanned(span: i32) -> proc_macro2::TokenStream {
            quote_spanned!(span=> struct Demo;)
        }
    "#);
}

#[test]
fn unexpanded_expr_macro_does_not_force_integer_inference() {
    assert_type_ok(r#"
        mod proc_macro2 {
            pub struct TokenStream {}
            impl TokenStream {
                pub fn new() -> TokenStream {
                    TokenStream {}
                }
            }
        }

        mod fragment {
            use crate::proc_macro2::TokenStream;

            pub enum Fragment {
                Block(TokenStream),
            }
        }

        macro_rules! quote_block {
            ($($tt:tt)*) => {
                crate::fragment::Fragment::Block(quote!($($tt)*))
            }
        }

        fn make(flag: bool) -> fragment::Fragment {
            if flag {
                not_yet_supported_macro!()
            } else {
                quote_block! { impl Demo {} }
            }
        }
    "#);
}

#[test]
fn impl_method_body_resolves_self_enum_variants_against_impl_type() {
    assert_type_ok(r#"
        enum First {
            A,
            B,
        }

        enum Second {
            A,
            B,
        }

        impl First {
            fn from_u32(v: u32) -> Self {
                match v {
                    1 => Self::A,
                    _ => Self::B,
                }
            }
        }

        impl Second {
            fn from_u32(v: u32) -> Self {
                match v {
                    1 => Self::A,
                    _ => Self::B,
                }
            }
        }
    "#);
}

#[test]
fn self_enum_variant_resolution_wins_over_global_type_names() {
    assert_type_ok(r#"
        struct String {}

        enum SortType {
            String,
            Numeric,
        }

        impl SortType {
            fn from_u8(v: u8) -> Self {
                match v {
                    1 => Self::Numeric,
                    _ => Self::String,
                }
            }
        }
    "#);
}

#[test]
fn module_qualified_impl_self_resolves_enum_variants() {
    assert_type_ok(r#"
        mod error {
            pub enum DecodeError {
                OtherString(String),
            }
        }

        struct String {}

        impl crate::error::DecodeError {
            fn custom(msg: String) -> Self {
                Self::OtherString(msg)
            }
        }
    "#);
}

#[test]
fn function_arg_autoderefs_nested_slice_references() {
    assert_type_ok(r#"
        fn takes_slice(item: &[u8]) -> bool {
            true
        }

        fn main() {
            let items: [&[u8]; 1] = [b"x"];
            for item in &items {
                let ok = takes_slice(item);
            }
        }
    "#);
}

#[test]
fn vec_u8_compares_with_byte_string_arrays() {
    assert_type_ok(r#"
        fn main() {
            let t: Vec<u8> = Vec::new();
            let is_time = &t == b"time";
            let is_date = t == b"date";
        }
    "#);
}

#[test]
fn if_arms_allow_byte_string_literals_with_different_lengths_for_slice_use() {
    assert_type_ok(r#"
        fn takes_slice(item: &[u8]) {}

        fn main() {
            let flag = true;
            takes_slice(if flag { b"Folder" } else { b"File name" });
        }
    "#);
}

#[test]
fn shift_rhs_accepts_usize_without_forcing_lhs_type() {
    assert_type_ok(r#"
        fn main() {
            let index: usize = 3;
            let mask = 1u64 << index;
        }
    "#);
}

#[test]
fn binary_ops_autoderef_shared_primitive_refs() {
    assert_type_ok(r#"
        fn f(a: &u8, b: &u8) -> u8 {
            a ^ b
        }
    "#);
}

#[test]
fn binary_ops_autoderef_mixed_primitive_ref_and_value() {
    assert_type_ok(r#"
        fn f(a: &u8, b: u8) -> u8 {
            a ^ b
        }
    "#);
}

#[test]
fn impl_generic_params_are_visible_in_method_bodies() {
    assert_type_ok(r#"
        trait Add<Rhs> {
            type Output;
        }

        struct B0 {}
        struct UTerm {}
        struct UInt<U, B> { msb: U, lsb: B }

        impl<U, B> UInt<U, B> {
            fn new() -> Self {
                UInt { msb: UTerm, lsb: B0 }
            }
        }

        impl<U, B> Add<B0> for UInt<U, B> {
            type Output = UInt<U, B>;

            fn add(self, _: B0) -> Self::Output {
                UInt::new()
            }
        }
    "#);
}

#[test]
fn self_type_is_available_in_struct_fields() {
    assert_type_ok(r#"
        struct Block<T> { value: T }
        struct BlockCtx<BS> {
            block: Block<Self>,
            marker: BS,
        }
    "#);
}

#[test]
fn self_output_uses_current_impl_associated_type() {
    assert_type_ok(r#"
        trait Trait {
            type Output;
        }

        struct A {}
        struct B {}
        struct X {}
        struct Y {}

        impl Trait for A {
            type Output = X;

            fn get(self) -> Self::Output {
                X {}
            }
        }

        impl Trait for B {
            type Output = Y;

            fn get(self) -> Self::Output {
                Y {}
            }
        }
    "#);
}

#[test]
fn method_resolution_uses_argument_types_for_same_receiver_impls() {
    assert_type_ok(r#"
        trait BitOr<Rhs> {
            type Output;
            fn bitor(self, rhs: Rhs) -> Self::Output;
        }

        struct B0 {}
        struct B1 {}

        impl BitOr<B0> for B0 {
            type Output = B0;
            fn bitor(self, _: B0) -> Self::Output { B0 {} }
        }

        impl BitOr<B1> for B0 {
            type Output = B1;
            fn bitor(self, _: B1) -> Self::Output { B1 {} }
        }

        fn f(lhs: B0, rhs: B1) -> B1 {
            lhs.bitor(rhs)
        }
    "#);
}

#[test]
fn binary_operator_uses_trait_output_type() {
    assert_type_ok(r#"
        mod core {
            pub mod ops {
                pub trait Mul<Rhs> {
                    type Output;
                    fn mul(self, rhs: Rhs) -> Self::Output;
                }
            }
        }

        struct Aligned130 {}
        struct PrecomputedMultiplier {}
        struct Unreduced130 {}

        impl core::ops::Mul<PrecomputedMultiplier> for Aligned130 {
            type Output = Unreduced130;
            fn mul(self, rhs: PrecomputedMultiplier) -> Self::Output {
                Unreduced130 {}
            }
        }

        fn multiply(lhs: Aligned130, rhs: PrecomputedMultiplier) -> Unreduced130 {
            lhs * rhs
        }
    "#);
}

#[test]
fn comparison_operator_uses_partial_eq_impl() {
    assert_type_ok(r#"
        trait PartialEq<Rhs> {
            fn eq(&self, rhs: &Rhs) -> bool;
        }

        struct Wrapped<T> {
            value: usize,
            marker: T,
        }

        struct Big;

        impl<T> PartialEq<usize> for Wrapped<T> {
            fn eq(&self, rhs: &usize) -> bool {
                self.value == *rhs
            }
        }

        fn is_zero(value: Wrapped<Big>) -> bool {
            value == 0
        }
    "#);
}

#[test]
fn function_arguments_allow_from_impl_into_expected_type() {
    assert_type_ok(r#"
        trait From<T> {}

        struct TokenTree {}
        struct Ident {}

        impl From<Ident> for TokenTree {}

        fn append(token: TokenTree) {}

        fn main() {
            append(Ident {});
        }
    "#);
}

#[test]
fn comparison_operator_uses_generic_partial_eq_rhs() {
    assert_type_ok(r#"
        trait PartialEq<Rhs> {
            fn eq(&self, rhs: &Rhs) -> bool;
        }

        struct Ident {}

        impl<T> PartialEq<T> for Ident {
            fn eq(&self, rhs: &T) -> bool {
                true
            }
        }

        fn is_keyword(ident: Ident, token: &str) -> bool {
            ident == token
        }
    "#);
}

#[test]
fn module_alias_assoc_fn_resolves_same_named_type_in_target_module() {
    assert_type_ok(r#"
        mod internals {
            pub mod attr {
                use crate::internals::Ctxt;

                pub struct Container {}

                impl Container {
                    pub fn from_ast(cx: Ctxt, item: &Item) -> Self {
                        Container {}
                    }
                }

                pub struct Ctxt {}
                pub struct Item {}
                pub struct Field {}
            }

            pub mod ast {
                use crate::internals::{attr, Ctxt, Derive};

                pub struct Container<'a> {
                    attrs: attr::Container,
                    original: &'a attr::Item,
                }

                pub struct Field<'a> {
                    attrs: attr::Field,
                    original: &'a syn::Field,
                }

                impl<'a> Container<'a> {
                    pub fn from_ast(
                        cx: Ctxt,
                        item: &'a attr::Item,
                        derive: Derive,
                        private: Ident,
                    ) -> Self {
                        let attrs = attr::Container::from_ast(cx, item);
                        Container {
                            attrs,
                            original: item,
                        }
                    }
                }

                fn takes_attr_container(attrs: &attr::Container) {}

                pub fn use_attrs(cont: &Container) {
                    takes_attr_container(&cont.attrs);
                }

                pub struct Derive {}
                pub struct Ident {}
            }

            pub struct Ctxt {}
            pub struct Derive {}
        }

        mod de {
            use crate::internals::ast::Container;
            use crate::internals::attr;

            fn takes_attr_container(attrs: &attr::Container) {}

            fn deserialize_body(cont: &Container) {
                takes_attr_container(&cont.attrs);
            }
        }

        mod syn {
            pub mod punctuated {
                pub struct Punctuated<T, P> {
                    value: T,
                    punct: P,
                }
            }

            pub struct Comma {}

            pub mod data {
                pub struct Field {
                    pub ty: u8,
                }

                pub struct Variant {
                    pub fields: u8,
                }
            }

            pub mod derive {
                pub struct DeriveInput {
                    pub attrs: u8,
                    pub data: Data,
                }

                pub enum Data {
                    Enum(DataEnum),
                    Struct(DataStruct),
                }

                pub struct DataEnum {
                    pub variants: crate::syn::punctuated::Punctuated<
                        crate::syn::data::Variant,
                        crate::syn::Comma,
                    >,
                }

                pub struct DataStruct {}
            }

            pub use crate::syn::data::{Field, Variant};
            pub use crate::syn::derive::{Data, DeriveInput};
        }

        mod bound {
            use crate::internals::ast::Field;

            fn visit_field(field: &Field) -> u8 {
                field.original.ty
            }
        }

        mod deprecated {
            use crate::syn::{Data, DeriveInput};

            fn should_allow_deprecated(input: &DeriveInput) -> u8 {
                input.attrs
            }

            fn enum_variants(input: &DeriveInput) -> u8 {
                if let Data::Enum(data) = &input.data {
                    for variant in &data.variants {
                        return variant.fields;
                    }
                    0
                } else {
                    0
                }
            }
        }

        mod alias_user {
            use crate::syn::Variant;

            fn variant_fields(variant: &Variant) -> u8 {
                variant.fields
            }
        }

        mod wrong_names {
            pub struct DeriveInput {}
            pub struct Variant {}
        }
    "#);
}

#[test]
fn generic_tuple_struct_constructor_infers_adt_args_from_fields() {
    assert_type_ok(r#"
        struct PhantomData<T>;
        struct SendSyncPhantomData<T>(PhantomData<T>);
        struct Wrapper<T> {
            bytes: [u8; 2],
            marker: SendSyncPhantomData<T>,
        }

        impl<T> SendSyncPhantomData<T> {
            fn default() -> SendSyncPhantomData<T> {
                SendSyncPhantomData(PhantomData)
            }
        }

        fn make<T>() -> Wrapper<T> {
            Wrapper {
                bytes: [0u8; 2],
                marker: SendSyncPhantomData::default(),
            }
        }
    "#);
}

#[test]
fn generic_self_struct_literal_uses_impl_self_substs() {
    assert_type_ok(r#"
        struct PhantomData<T>;
        struct SendSyncPhantomData<T>(PhantomData<T>);
        struct AlignmentError<Src, Dst> {
            src: Src,
            dst: SendSyncPhantomData<Dst>,
        }

        impl<T> SendSyncPhantomData<T> {
            fn default() -> SendSyncPhantomData<T> {
                SendSyncPhantomData(PhantomData)
            }
        }

        impl<Src, Dst> AlignmentError<Src, Dst> {
            fn new(src: Src) -> Self {
                Self {
                    src,
                    dst: SendSyncPhantomData::default(),
                }
            }
        }

        fn make<U>() -> AlignmentError<(), U> {
            AlignmentError::new(())
        }
    "#);
}

#[test]
fn binary_operator_uses_reference_self_trait_output_type() {
    assert_type_ok(r#"
        mod core {
            pub mod ops {
                pub trait Mul<Rhs> {
                    type Output;
                    fn mul(self, rhs: Rhs) -> Self::Output;
                }
            }
        }

        struct Aligned4x130 {}
        struct PrecomputedMultiplier {}
        struct Unreduced4x130 {}

        impl core::ops::Mul<PrecomputedMultiplier> for &Aligned4x130 {
            type Output = Unreduced4x130;
            fn mul(self, rhs: PrecomputedMultiplier) -> Self::Output {
                Unreduced4x130 {}
            }
        }

        fn multiply(lhs: &Aligned4x130, rhs: PrecomputedMultiplier) -> Unreduced4x130 {
            lhs * rhs
        }
    "#);
}

#[test]
fn operator_impl_does_not_bind_unanchored_infer_lhs() {
    assert_type_ok(r#"
        mod core {
            pub mod ops {
                pub trait Sub<Rhs> {
                    type Output;
                    fn sub(self, rhs: Rhs) -> Self::Output;
                }
            }
        }

        struct U16<O> {}
        struct NonZeroUsize {}

        impl<O> core::ops::Sub<usize> for U16<O> {
            type Output = U16<O>;
            fn sub(self, rhs: usize) -> Self::Output {
                self
            }
        }

        fn padding(align: NonZeroUsize) -> usize {
            let mask = align.get() - 1;
            mask
        }
    "#);
}

#[test]
fn result_branches_with_unit_and_generic_error_align() {
    assert_type_ok(r#"
        mod core {
            pub mod mem {
                pub fn align_of<T>() -> usize { 1 }
            }
        }

        enum Result<T, E> {
            Ok(T),
            Err(E),
        }

        trait AsAddress {
            fn addr(self) -> usize;
        }

        struct PhantomData<T>;
        struct SendSyncPhantomData<T>(PhantomData<T>);
        struct AlignmentError<Src, Dst> {
            src: Src,
            marker: SendSyncPhantomData<Dst>,
        }

        impl<T> SendSyncPhantomData<T> {
            fn default() -> SendSyncPhantomData<T> {
                SendSyncPhantomData(PhantomData)
            }
        }

        impl<Src, Dst> AlignmentError<Src, Dst> {
            fn new_unchecked(src: Src) -> Self {
                Self { src, marker: SendSyncPhantomData::default() }
            }
        }

        fn validate<T: AsAddress, U>(t: T) -> Result<(), AlignmentError<(), U>> {
            let remainder = t.addr() % core::mem::align_of::<U>();
            if remainder == 0 {
                Ok(())
            } else {
                Err(AlignmentError::new_unchecked(()))
            }
        }
    "#);
}

#[test]
fn imported_nonzero_usize_get_stays_primitive_with_byteorder_usize_nearby() {
    assert_type_ok(r#"
        mod core {
            pub mod num {
                pub struct NonZeroUsize {}
            }
            pub mod ops {
                pub trait Sub<Rhs> {
                    type Output;
                    fn sub(self, rhs: Rhs) -> Self::Output;
                }
            }
        }

        use core::num::NonZeroUsize;

        struct Usize<O> {}
        impl<O> core::ops::Sub<Usize<O>> for usize {
            type Output = Usize<O>;
            fn sub(self, rhs: Usize<O>) -> Self::Output {
                rhs
            }
        }
        impl<O> core::ops::Sub<usize> for Usize<O> {
            type Output = Usize<O>;
            fn sub(self, rhs: usize) -> Self::Output {
                self
            }
        }

        fn padding(align: NonZeroUsize) -> usize {
            let mask = align.get() - 1;
            mask
        }
    "#);
}

#[test]
fn external_nonzero_usize_get_stays_primitive_with_byteorder_usize_nearby() {
    assert_type_ok(r#"
        use core::num::NonZeroUsize;

        mod byteorder {
            pub struct Usize<O> {}
            impl<O> core::ops::Sub<Usize<O>> for usize {
                type Output = Usize<O>;
                fn sub(self, rhs: Usize<O>) -> Self::Output {
                    rhs
                }
            }
            impl<O> core::ops::Sub<usize> for Usize<O> {
                type Output = Usize<O>;
                fn sub(self, rhs: usize) -> Self::Output {
                    self
                }
            }
        }

        mod util {
            use super::*;

            fn padding(align: NonZeroUsize) -> usize {
                let mask = align.get() - 1;
                mask
            }
        }
    "#);
}

#[test]
fn primitive_integer_methods_keep_receiver_type() {
    assert_type_ok(r#"
        fn padding_needed_for(len: usize, align: usize) -> usize {
            let mask = align - 1;
            !(len.wrapping_sub(1)) & mask
        }

        fn round_down(n: usize, align: usize) -> usize {
            let _ok: bool = align.is_power_of_two();
            let mask = !(align - 1);
            n & mask
        }

        fn checked(n: usize, align: usize) -> Option<usize> {
            n.checked_add(align)
        }
    "#);
}

#[test]
fn generic_param_associated_function_uses_trait_bound_signature() {
    assert_type_ok(r#"
        mod marker_traits {
            pub trait Bit {
                const U8: u8;
                fn to_u8() -> u8;
            }

            pub trait Unsigned {
                const U8: u8;
                const U16: u16;
                fn to_u8() -> u8;
                fn to_u16() -> u16;
            }
        }

        use crate::marker_traits::{Bit, Unsigned};

        struct B0 {}
        struct UTerm {}
        struct UInt<U, B> { msb: U, lsb: B }

        impl Bit for B0 {
            const U8: u8 = 0;
            fn to_u8() -> u8 { 0 }
        }

        impl Unsigned for UTerm {
            const U8: u8 = 0;
            const U16: u16 = 0;
            fn to_u8() -> u8 { 0 }
            fn to_u16() -> u16 { 0 }
        }

        impl<U: Unsigned, B: Bit> Unsigned for UInt<U, B> {
            const U8: u8 = B::U8 | U::U8 << 1;
            const U16: u16 = B::U8 as u16 | U::U16 << 1;

            fn to_u8() -> u8 {
                let bit: u8 = B::to_u8();
                let rest: u8 = U::to_u8();
                B::to_u8() | U::to_u8() << 1
            }

            fn to_u16() -> u16 {
                let bit: u8 = B::to_u8();
                let rest: u16 = U::to_u16();
                u16::from(B::to_u8()) | U::to_u16() << 1
            }
        }
    "#);
}

#[test]
fn generic_param_associated_const_mismatch_is_reported_from_bound() {
    assert_type_error(r#"
        mod bit {
            pub trait Bit {
                const U8: u8;
                fn to_u8() -> u8;
            }

            pub struct B0 {}

            impl Bit for B0 {
                const U8: u8 = 0;
                fn to_u8() -> u8 { 0 }
            }
        }

        mod marker_traits {
            pub trait Unsigned {
                const U16: u16;
                fn to_u16() -> u16;
            }
        }

        mod uint {
            use crate::{bit::{Bit, B0}, marker_traits::Unsigned};

            pub struct UTerm {}
            pub struct UInt<U, B> { msb: U, lsb: B }

            impl Unsigned for UTerm {
                const U16: u16 = 0;
                fn to_u16() -> u16 { 0 }
            }

            impl<U: Unsigned, B: Bit> UInt<U, B> {
                fn broken_const() -> u16 {
                    B::U8
                }
            }
        }
    "#, "mismatch");
}

#[test]
fn generic_param_associated_fn_mismatch_is_reported_from_bound() {
    assert_type_error(r#"
        mod bit {
            pub trait Bit {
                const U8: u8;
                fn to_u8() -> u8;
            }

            pub struct B0 {}

            impl Bit for B0 {
                const U8: u8 = 0;
                fn to_u8() -> u8 { 0 }
            }
        }

        mod marker_traits {
            pub trait Unsigned {
                const U16: u16;
                fn to_u16() -> u16;
            }
        }

        mod uint {
            use crate::{bit::{Bit, B0}, marker_traits::Unsigned};

            pub struct UTerm {}
            pub struct UInt<U, B> { msb: U, lsb: B }

            impl Unsigned for UTerm {
                const U16: u16 = 0;
                fn to_u16() -> u16 { 0 }
            }

            impl<U: Unsigned, B: Bit> UInt<U, B> {
                fn broken_fn() -> u16 {
                    B::to_u8()
                }
            }
        }
    "#, "mismatch");
}

#[test]
fn typenum_style_primitive_powi_impls_typecheck() {
    assert_type_ok(r#"
        trait Bit {}
        trait Unsigned {
            fn to_u32() -> u32;
        }

        trait Pow<Exp> {
            type Output;
            fn powi(self, exp: Exp) -> Self::Output;
        }

        struct UTerm {}
        struct UInt<U, B> { msb: U, lsb: B }
        struct B0 {}

        impl Bit for B0 {}

        impl Unsigned for UTerm {
            fn to_u32() -> u32 { 0 }
        }

        impl<U: Unsigned, B: Bit> Unsigned for UInt<U, B> {
            fn to_u32() -> u32 {
                U::to_u32() << 1
            }
        }

        impl Pow<UTerm> for f32 {
            type Output = f32;
            fn powi(self, _: UTerm) -> Self::Output {
                1.0
            }
        }

        impl<U: Unsigned, B: Bit> Pow<UInt<U, B>> for f32 {
            type Output = f32;
            fn powi(self, _: UInt<U, B>) -> Self::Output {
                let mut exp = <UInt<U, B> as Unsigned>::to_u32();
                let mut base = self;

                if exp == 0 {
                    return 1.0;
                }

                while exp & 1 == 0 {
                    base *= base;
                    exp >>= 1;
                }

                base
            }
        }
    "#);
}

#[test]
fn typenum_style_recursive_primitive_powi_macro_typechecks() {
    assert_type_ok(r#"
        trait Bit {}
        trait Unsigned {
            fn to_u32() -> u32;
        }

        trait Pow<Exp> {
            type Output;
            fn powi(self, exp: Exp) -> Self::Output;
        }

        struct UTerm {}
        struct UInt<U, B> { msb: U, lsb: B }
        struct B0 {}

        impl Bit for B0 {}

        impl Unsigned for UTerm {
            fn to_u32() -> u32 { 0 }
        }

        impl<U: Unsigned, B: Bit> Unsigned for UInt<U, B> {
            fn to_u32() -> u32 {
                U::to_u32() << 1
            }
        }

        macro_rules! impl_pow_i {
            () => ();
            ($t: ty $(, $tail:tt)*) => (
                impl Pow<UTerm> for $t {
                    type Output = $t;
                    fn powi(self, _: UTerm) -> Self::Output {
                        1
                    }
                }

                impl<U: Unsigned, B: Bit> Pow<UInt<U, B>> for $t {
                    type Output = $t;
                    fn powi(self, _: UInt<U, B>) -> Self::Output {
                        self.pow(<UInt<U, B> as Unsigned>::to_u32())
                    }
                }

                impl_pow_i!($($tail),*);
            );
        }

        impl_pow_i!(u8, u16, u32);
    "#);
}

#[test]
fn typenum_style_float_powi_macro_typechecks() {
    assert_type_ok(r#"
        trait Bit {}
        trait Unsigned {
            fn to_u32() -> u32;
        }

        trait Pow<Exp> {
            type Output;
            fn powi(self, exp: Exp) -> Self::Output;
        }

        struct UTerm {}
        struct UInt<U, B> { msb: U, lsb: B }
        struct B0 {}
        struct PInt<U> { inner: U }
        struct NInt<U> { inner: U }

        trait NonZero {}

        impl Bit for B0 {}

        impl PInt<UTerm> {
            fn new() -> Self { PInt { inner: UTerm {} } }
        }

        impl Unsigned for UTerm {
            fn to_u32() -> u32 { 0 }
        }

        impl<U: Unsigned, B: Bit> Unsigned for UInt<U, B> {
            fn to_u32() -> u32 {
                U::to_u32() << 1
            }
        }

        macro_rules! impl_pow_f {
            ($t:ty) => {
                impl Pow<UTerm> for $t {
                    type Output = $t;
                    fn powi(self, _: UTerm) -> Self::Output {
                        1.0
                    }
                }

                impl<U: Unsigned, B: Bit> Pow<UInt<U, B>> for $t {
                    type Output = $t;
                    fn powi(self, _: UInt<U, B>) -> Self::Output {
                        let mut exp = <UInt<U, B> as Unsigned>::to_u32();
                        let mut base = self;

                        if exp == 0 {
                            return 1.0;
                        }

                        while exp & 1 == 0 {
                            base *= base;
                            exp >>= 1;
                        }
                        if exp == 1 {
                            return base;
                        }

                        let mut acc = base.clone();
                        while exp > 1 {
                            exp >>= 1;
                            base *= base;
                            if exp & 1 == 1 {
                                acc *= base.clone();
                            }
                        }
                        acc
                    }
                }

                impl<U: Unsigned + NonZero> Pow<NInt<U>> for $t {
                    type Output = $t;
                    fn powi(self, _: NInt<U>) -> Self::Output {
                        <$t as Pow<PInt<U>>>::powi(self, PInt::new()).recip()
                    }
                }
            };
        }

        impl_pow_f!(f32);
        impl_pow_f!(f64);
    "#);
}

#[test]
fn typenum_style_generic_rem_uses_projection_not_concrete_operator_impl() {
    assert_type_ok(r#"
        trait Rem<Rhs = Self> {
            type Output;
            fn rem(self, rhs: Rhs) -> Self::Output;
        }

        trait Unsigned {}
        trait NonZero {}
        trait Bit {}

        mod bit {
            pub struct B0 {}
            impl crate::Bit for B0 {}
        }

        mod uint {
            use crate::bit::B0;
            use crate::{Bit, Rem, Unsigned};

            pub struct UTerm {}
            pub struct UInt<U: Unsigned, B: Bit> {
                pub msb: U,
                pub lsb: B,
            }

            impl Unsigned for UTerm {}

            impl<Ur: Unsigned, Br: Bit> Rem<UInt<Ur, Br>> for UTerm {
                type Output = UTerm;

                fn rem(self, rhs: UInt<Ur, Br>) -> Self::Output {
                    UTerm {}
                }
            }
        }

        mod consts {
            pub type U0 = crate::uint::UTerm;
        }

        mod int {
            use crate::consts::U0;
            use crate::uint::Unsigned;
            use crate::{NonZero, Rem};

            pub struct Z0;
            pub struct PInt<U: Unsigned + NonZero> {
                pub n: U,
            }

            pub trait PrivateRem<URem, Divisor> {
                type Output;
                fn private_rem(self, rem: URem, divisor: Divisor) -> Self::Output;
            }

            impl<I: NonZero> Rem<I> for Z0 {
                type Output = Z0;

                fn rem(self, rhs: I) -> Self::Output {
                    Z0
                }
            }

            impl<Ul: Unsigned + NonZero, Ur: Unsigned + NonZero> PrivateRem<U0, PInt<Ur>> for PInt<Ul> {
                type Output = Z0;

                fn private_rem(self, rem: U0, rhs: PInt<Ur>) -> Self::Output {
                    Z0
                }
            }

            impl<Ul: Unsigned + NonZero, Ur: Unsigned + NonZero> Rem<PInt<Ur>> for PInt<Ul>
            where
                Ul: Rem<Ur>,
                PInt<Ul>: PrivateRem<<Ul as Rem<Ur>>::Output, PInt<Ur>>,
            {
                type Output = <PInt<Ul> as PrivateRem<<Ul as Rem<Ur>>::Output, PInt<Ur>>>::Output;

                fn rem(self, rhs: PInt<Ur>) -> Self::Output {
                    self.private_rem(self.n % rhs.n, rhs)
                }
            }
        }
    "#);
}

#[test]
fn qualified_associated_type_projection_is_not_global_impl_output() {
    assert_type_ok(r#"
        trait Trait {
            type Output;
        }

        struct B0 {}
        struct UTerm {}
        struct Wrap<T> { value: T }

        impl<T> Trait for Wrap<T> {
            type Output = T;
        }

        impl Trait for B0 {
            type Output = B0;
        }

        type Out<T> = <Wrap<T> as Trait>::Output;

        fn f(x: UTerm) -> Out<UTerm> {
            x
        }
    "#);
}

#[test]
fn qualified_module_types_do_not_collide_by_leaf_name() {
    assert_type_ok(r#"
        mod fallback {
            pub struct TokenStream {}
            pub fn make() -> TokenStream { TokenStream {} }
        }

        use crate::fallback as imp;

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        fn wrap(inner: fallback::TokenStream) -> TokenStream {
            TokenStream { inner }
        }
    "#);
}

#[test]
fn qualified_assoc_fns_do_not_collide_by_leaf_name() {
    assert_type_ok(r#"
        mod fallback {
            pub struct TokenStream {}
            impl TokenStream {
                pub fn new() -> TokenStream { TokenStream {} }
            }
        }

        use crate::fallback as imp;

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        fn wrap() -> TokenStream {
            TokenStream { inner: imp::TokenStream::new() }
        }
    "#);
}

#[test]
fn assoc_from_selects_matching_impl_on_module_alias() {
    assert_type_ok(r#"
        mod fallback {
            pub struct TokenTree {}
            pub struct TokenStream {}

            impl TokenStream {
                pub fn from(tree: TokenTree) -> TokenStream {
                    TokenStream {}
                }
            }
        }

        use crate::fallback as imp;

        fn wrap(tree: fallback::TokenTree) -> fallback::TokenStream {
            imp::TokenStream::from(tree)
        }
    "#);
}

#[test]
fn assoc_from_identity_uses_target_type() {
    assert_type_ok(r#"
        mod fallback {
            pub struct TokenStream {}
        }

        use crate::fallback as imp;

        fn wrap(inner: fallback::TokenStream) -> fallback::TokenStream {
            imp::TokenStream::from(inner)
        }
    "#);
}

#[test]
fn assoc_from_identity_wins_when_single_impl_does_not_match() {
    assert_type_ok(r#"
        mod fallback {
            pub struct Foreign {}
            pub struct TokenStream {}

            impl TokenStream {
                pub fn from(value: Foreign) -> TokenStream {
                    TokenStream {}
                }
            }
        }

        use crate::fallback as imp;

        fn wrap(inner: fallback::TokenStream) -> fallback::TokenStream {
            imp::TokenStream::from(inner)
        }
    "#);
}

#[test]
fn cfg_selects_module_alias_before_wrapper_module() {
    let (result, _) = typecheck_with_cfg(r#"
        mod fallback {
            pub struct TokenStream {}
        }

        #[cfg(not(wrap_proc_macro))]
        use crate::fallback as imp;

        #[cfg(wrap_proc_macro)]
        mod imp {
            pub struct TokenStream {}
        }

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        impl TokenStream {
            fn _new_fallback(inner: fallback::TokenStream) -> Self {
                TokenStream {
                    inner: imp::TokenStream::from(inner),
                }
            }
        }
    "#, &[]);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn cfg_wrapper_module_assoc_from_uses_wrapper_impl() {
    let (result, _) = typecheck_with_cfg(r#"
        trait From<T> {
            fn from(value: T) -> Self;
        }

        mod fallback {
            pub struct TokenStream {}
        }

        #[cfg(not(wrap_proc_macro))]
        use crate::fallback as imp;

        #[cfg(wrap_proc_macro)]
        mod imp {
            use crate::fallback;

            pub struct TokenStream {}

            impl crate::From<fallback::TokenStream> for TokenStream {
                fn from(inner: fallback::TokenStream) -> TokenStream {
                    TokenStream {}
                }
            }
        }

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        impl TokenStream {
            fn _new_fallback(inner: fallback::TokenStream) -> Self {
                TokenStream {
                    inner: imp::TokenStream::from(inner),
                }
            }
        }
    "#, &["wrap_proc_macro"]);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn proc_macro2_style_fallback_wrappers_typecheck() {
    let (result, _) = typecheck_with_cfg(r#"
        trait From<T> {
            fn from(value: T) -> Self;
        }

        pub enum TokenTree {
            Group,
        }

        pub mod fallback {
            use crate::TokenTree;

            pub struct TokenStream {}
            pub struct Span {}
            pub struct Ident {}

            impl TokenStream {
                pub fn new() -> TokenStream { TokenStream {} }
            }

            impl From<TokenTree> for TokenStream {
                fn from(token: TokenTree) -> TokenStream {
                    TokenStream {}
                }
            }

            impl Ident {
                pub fn new_checked(string: &str, span: Span) -> Ident {
                    Ident {}
                }
            }
        }

        #[cfg(not(wrap_proc_macro))]
        use crate::fallback as imp;

        #[cfg(wrap_proc_macro)]
        mod imp {
            pub struct TokenStream {}
            pub struct Span {}
            pub struct Ident {}
        }

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        impl TokenStream {
            fn _new(inner: imp::TokenStream) -> Self {
                TokenStream { inner }
            }

            pub fn new() -> Self {
                TokenStream::_new(imp::TokenStream::new())
            }
        }

        impl From<TokenTree> for TokenStream {
            fn from(token: TokenTree) -> Self {
                TokenStream::_new(imp::TokenStream::from(token))
            }
        }

        pub struct Span {
            inner: imp::Span,
        }

        pub struct Ident {
            inner: imp::Ident,
        }

        impl Ident {
            fn _new(inner: imp::Ident) -> Self {
                Ident { inner }
            }

            pub fn new(string: &str, span: Span) -> Self {
                Ident::_new(imp::Ident::new_checked(string, span.inner))
            }
        }
    "#, &[]);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn proc_macro2_style_wrapper_from_overloads_typecheck() {
    let (result, _) = typecheck_with_cfg(r#"
        trait From<T> {
            fn from(value: T) -> Self;
        }

        pub enum TokenTree {
            Group,
        }

        pub mod fallback {
            use crate::TokenTree;

            pub struct TokenStream {}

            impl From<TokenTree> for TokenStream {
                fn from(token: TokenTree) -> TokenStream {
                    TokenStream {}
                }
            }
        }

        #[cfg(not(wrap_proc_macro))]
        use crate::fallback as imp;

        #[cfg(wrap_proc_macro)]
        mod imp {
            use crate::{fallback, TokenTree};

            pub struct TokenStream {}
            pub struct CompilerTokenStream {}

            impl From<CompilerTokenStream> for TokenStream {
                fn from(inner: CompilerTokenStream) -> TokenStream {
                    TokenStream {}
                }
            }

            impl From<fallback::TokenStream> for TokenStream {
                fn from(inner: fallback::TokenStream) -> TokenStream {
                    TokenStream {}
                }
            }

            impl From<TokenTree> for TokenStream {
                fn from(token: TokenTree) -> TokenStream {
                    TokenStream::from(fallback::TokenStream::from(token))
                }
            }
        }

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        impl TokenStream {
            fn _new(inner: imp::TokenStream) -> Self {
                TokenStream { inner }
            }

            fn _new_fallback(inner: fallback::TokenStream) -> Self {
                TokenStream {
                    inner: imp::TokenStream::from(inner),
                }
            }
        }

        impl From<TokenTree> for TokenStream {
            fn from(token: TokenTree) -> Self {
                TokenStream::_new(imp::TokenStream::from(token))
            }
        }
    "#, &["wrap_proc_macro"]);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn proc_macro2_style_scoped_fallback_imports_do_not_pick_root_wrappers() {
    let (result, _) = typecheck_with_cfg(r#"
        pub mod fallback {
            pub struct TokenStream {}
            pub struct TokenStreamBuilder {}
            pub struct Group {}
            pub struct Span {}
            pub struct Literal {}

            impl TokenStreamBuilder {
                pub fn new() -> TokenStreamBuilder { TokenStreamBuilder {} }
                pub fn build(self) -> TokenStream { TokenStream {} }
                pub fn push_token_from_parser(&mut self, token: crate::TokenTree) {}
            }

            impl Group {
                pub fn new() -> Group { Group {} }
                pub fn span(&self) -> Span { Span {} }
            }

            impl Span {
                pub fn call_site() -> Span { Span {} }
            }

            impl Literal {
                pub fn new() -> Literal { Literal {} }
            }
        }

        #[cfg(not(wrap_proc_macro))]
        use crate::fallback as imp;

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        pub struct Group {
            inner: imp::Group,
        }

        pub struct Span {
            inner: imp::Span,
        }

        pub struct Literal {
            inner: imp::Literal,
        }

        pub enum TokenTree {
            Group(Group),
            Literal(Literal),
        }

        impl TokenStream {
            fn _new_fallback(inner: fallback::TokenStream) -> Self {
                TokenStream { inner: imp::TokenStream::from(inner) }
            }
        }

        impl Group {
            fn _new_fallback(inner: fallback::Group) -> Self {
                Group { inner }
            }
        }

        impl Span {
            fn _new_fallback(inner: fallback::Span) -> Self {
                Span { inner }
            }
        }

        impl Literal {
            fn _new_fallback(inner: fallback::Literal) -> Self {
                Literal { inner }
            }
        }

        mod parse {
            use crate::fallback::{
                self, Group, Literal, Span, TokenStream, TokenStreamBuilder,
            };
            use crate::TokenTree;

            fn token_stream() -> TokenStream {
                let mut tokens = TokenStreamBuilder::new();
                let g = Group::new();
                tokens.push_token_from_parser(TokenTree::Group(crate::Group::_new_fallback(g)));
                tokens.build()
            }

            fn leaf_token() -> TokenTree {
                let literal = Literal::new();
                TokenTree::Literal(crate::Literal::_new_fallback(literal))
            }

            fn doc_comment(tokens: &mut TokenStreamBuilder) {
                let span = crate::Span::_new_fallback(Span::call_site());
            }
        }
    "#, &[]);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn proc_macro2_style_qualified_enum_variant_pattern_binds_payload_type() {
    assert_type_ok(r#"
        pub mod fallback {
            pub struct Span {}
            pub struct Group {}

            impl Group {
                pub fn span(&self) -> Span { Span {} }
            }
        }

        pub mod imp {
            use crate::fallback;

            pub enum Group {
                Fallback(fallback::Group),
            }

            pub enum Span {
                Fallback(fallback::Span),
            }
        }

        mod extra {
            use crate::fallback;
            use crate::imp;

            enum DelimSpanEnum {
                Fallback(fallback::Span),
            }

            fn new(group: &imp::Group) -> DelimSpanEnum {
                match group {
                    imp::Group::Fallback(group) => DelimSpanEnum::Fallback(group.span()),
                }
            }
        }
    "#);
}

#[test]
fn proc_macro_intrinsic_enum_variant_constructors_have_parent_type() {
    assert_type_ok(r#"
        fn into_compiler_token(g: proc_macro::Group, p: proc_macro::Punct) -> proc_macro::TokenTree {
            if true {
                proc_macro::TokenTree::Group(g)
            } else {
                proc_macro::TokenTree::Punct(p)
            }
        }
    "#);
}

#[test]
fn proc_macro2_style_extend_maps_generic_into_iterator_item() {
    assert_type_ok(r#"
        trait IntoIterator {
            type Item;
            type IntoIter;
            fn into_iter(self) -> Self::IntoIter;
        }

        trait Extend<A> {
            fn extend<T: IntoIterator<Item = A>>(&mut self, iter: T);
        }

        struct Map<I, F> {
            iter: I,
            f: F,
        }

        struct Ident {}

        mod imp {
            pub enum TokenTree {
                Ident(crate::Ident),
            }

            pub struct TokenStream {}

            impl Extend<TokenTree> for TokenStream {
                fn extend<I: IntoIterator<Item = TokenTree>>(&mut self, tokens: I) {}
            }
        }

        struct TokenStream {
            inner: imp::TokenStream,
        }

        impl Extend<Ident> for TokenStream {
            fn extend<I: IntoIterator<Item = Ident>>(&mut self, tokens: I) {
                self.inner.extend(tokens.into_iter().map(imp::TokenTree::Ident));
            }
        }
    "#);
}

#[test]
fn proc_macro2_style_wrapper_into_iter_preserves_fallback_variant_field_type() {
    assert_type_ok(r#"
        trait IntoIterator {
            type Item;
            type IntoIter;
            fn into_iter(self) -> Self::IntoIter;
        }

        struct RcVecIntoIter<T> {
            item: T,
        }

        enum TokenTree {
            Ident,
        }

        mod fallback {
            use crate::{RcVecIntoIter, TokenTree};

            pub struct TokenStream {}
            pub type TokenTreeIter = RcVecIntoIter<TokenTree>;

            impl IntoIterator for TokenStream {
                type Item = TokenTree;
                type IntoIter = TokenTreeIter;

                fn into_iter(self) -> TokenTreeIter {
                    RcVecIntoIter { item: TokenTree::Ident }
                }
            }
        }

        mod imp {
            use crate::fallback;
            use crate::TokenTree;

            pub enum TokenStream {
                Fallback(fallback::TokenStream),
            }

            pub enum TokenTreeIter {
                Fallback(fallback::TokenTreeIter),
            }

            impl IntoIterator for TokenStream {
                type Item = TokenTree;
                type IntoIter = TokenTreeIter;

                fn into_iter(self) -> TokenTreeIter {
                    match self {
                        TokenStream::Fallback(tts) => TokenTreeIter::Fallback(tts.into_iter()),
                    }
                }
            }
        }
    "#);
}

#[test]
fn proc_macro2_style_into_iter_struct_name_wins_over_assoc_type_in_return_annotation() {
    assert_type_ok(r#"
        trait IntoIterator {
            type Item;
            type IntoIter;
            fn into_iter(self) -> Self::IntoIter;
        }

        enum TokenTree { Ident }

        mod token_stream {
            use crate::TokenTree;

            pub struct TokenStream {}

            pub struct IntoIter {
                item: TokenTree,
            }

            impl IntoIterator for TokenStream {
                type Item = TokenTree;
                type IntoIter = IntoIter;

                fn into_iter(self) -> IntoIter {
                    IntoIter { item: TokenTree::Ident }
                }
            }
        }
    "#);
}

#[test]
fn field_access_autoderefs_nested_references() {
    assert_type_ok(r#"
        struct Package {
            name: String,
            version: String,
        }

        fn read(pkg: &&Package) -> String {
            pkg.name
        }
    "#);
}

#[test]
fn sort_by_vec_of_refs_binds_double_ref_closure_params() {
    assert_type_ok(r#"
        struct Ordering {}
        struct Package {
            name: String,
            version: String,
        }

        impl String {
            fn cmp(&self, other: &String) -> Ordering {
                Ordering {}
            }
        }

        impl Ordering {
            fn then(self, other: Ordering) -> Ordering {
                other
            }
        }

        fn sort(sorted: &mut Vec<&Package>) {
            sorted.sort_by(|a, b| a.name.cmp(&b.name).then(a.version.cmp(&b.version)));
        }
    "#);
}

#[test]
fn generic_array_style_index_uses_deref_slice_target() {
    assert_type_ok(r#"
        trait Deref {
            type Target;
        }

        trait Unsigned {}
        unsafe trait ArrayLength<T>: Unsigned {
            type ArrayType;
        }

        struct UTerm {}
        impl Unsigned for UTerm {}
        unsafe impl<T> ArrayLength<T> for UTerm {
            type ArrayType = [T; 0];
        }

        struct GenericArray<T, N: ArrayLength<T>> {
            marker: T,
            len: N,
        }

        impl<T, N: ArrayLength<T>> Deref for GenericArray<T, N> {
            type Target = [T];
        }

        fn fmt<N: ArrayLength<u8>>(array: GenericArray<u8, N>) {
            let byte: u8 = array[0];
            let bytes = array[..1];
        }
    "#);
}

#[test]
fn raw_pointer_add_result_can_initialize_local() {
    assert_type_ok(r#"
        struct Header {
            buckets: *const u32,
        }

        fn f(hash_ptr: *const u32) -> Header {
            let buckets = unsafe { hash_ptr.add(2) };
            Header { buckets }
        }
    "#);
}

#[test]
fn referenced_raw_struct_array_field_can_be_indexed() {
    assert_type_ok(r#"
        struct Elf64Ehdr {
            e_ident: [u8; 16],
        }

        fn magic(base: u64) -> bool {
            let ehdr = unsafe { &*(base as *const Elf64Ehdr) };
            ehdr.e_ident[0] == 0x7F
                || ehdr.e_ident[1] == b'E'
                || ehdr.e_ident[2] == b'L'
                || ehdr.e_ident[3] == b'F'
        }
    "#);
}

#[test]
fn str_as_bytes_matches_slice_parameter_after_pointer_add() {
    assert_type_ok(r#"
        struct Elf64Sym {
            st_name: u32,
        }

        struct Handle {
            strtab: *const u8,
        }

        fn cstr_eq(cstr: *const u8, name: &[u8]) -> bool {
            let _ = cstr;
            let _ = name;
            true
        }

        fn probe(handle: &Handle, sym: &Elf64Sym, name: &str) -> bool {
            unsafe { cstr_eq(handle.strtab.add(sym.st_name as usize), name.as_bytes()) }
        }
    "#);
}

#[test]
fn builtin_dll_exports_generated_loader_typechecks() {
    assert_type_ok(r#"
        mod core {
            pub mod mem {
                pub fn transmute_copy<T, U>(src: &T) -> U { loop {} }
            }
        }

        mod anyos_std {
            pub mod dll {
                pub fn dll_load(path: &str) -> usize {
                    let _ = path;
                    0
                }
            }
        }

        enum Option<T> {
            Some(T),
            None,
        }

        impl<T> Option<T> {
            fn as_ref(&self) -> Option<&T> { loop {} }
        }

        impl u64 {
            const MAX: u64 = 0;
        }

        dynlink::dll_exports! {
            lib_path: "/Libraries/libm.so",
            lib_struct: MathLib,
            symbols: {
                math_sqrt(x: f64) -> f64,
            }
        }
    "#);
}

#[test]
fn slice_iter_enumerate_binds_index_and_referenced_item() {
    assert_type_ok(r#"
        fn sum(bytes: &[u8]) -> usize {
            let mut out: usize = 0;
            for (i, &b) in bytes.iter().enumerate() {
                out = out + i + b as usize;
            }
            out
        }
    "#);
}

#[test]
fn for_loop_over_mut_vec_reference_binds_mut_referenced_tuple_fields() {
    assert_type_ok(r#"
        struct String;

        impl String {
            fn from(s: &str) -> String {
                let _ = s;
                String
            }
        }

        fn set(attrs: &mut Vec<(String, String)>, name: &str, value: &str) {
            for (k, v) in attrs {
                if k == name {
                    *v = String::from(value);
                    return;
                }
            }
        }
    "#);
}

#[test]
fn core_index_impl_supplies_index_expression_output() {
    assert_type_ok(r#"
        mod core {
            pub mod ops {
                pub trait Index<Idx> {
                    type Output;
                    fn index(&self, index: Idx) -> &Self::Output;
                }
            }
        }

        struct Value;

        impl core::ops::Index<&str> for Value {
            type Output = Value;
            fn index(&self, key: &str) -> &Value {
                let _ = key;
                self
            }
        }

        impl Value {
            fn as_bool(&self) -> bool { true }
        }

        fn get(v: Value) -> bool {
            v["enabled"].as_bool()
        }
    "#);
}

#[test]
fn root_unqualified_types_do_not_collide_with_nested_leaf_names() {
    assert_type_ok(r#"
        mod fallback {
            pub struct Span {}
            impl Span {
                pub fn call_site() -> Span { Span {} }
            }
        }

        use crate::fallback as imp;

        pub struct Span {
            inner: imp::Span,
        }

        impl Span {
            fn _new(inner: imp::Span) -> Self {
                Span { inner }
            }

            pub fn call_site() -> Self {
                Span::_new(imp::Span::call_site())
            }
        }

        fn get() -> Span {
            Span::call_site()
        }
    "#);
}

#[test]
fn struct_like_enum_variant_construction_resolves_parent_enum() {
    assert_type_ok(r#"
        mod helper {
            pub trait Sized {}
        }

        enum SizeInfo {
            Sized { size: usize },
            Other,
        }

        fn make() -> SizeInfo {
            SizeInfo::Sized { size: 0 }
        }

        fn read(info: SizeInfo) -> usize {
            match info {
                SizeInfo::Sized { size } => size,
                SizeInfo::Other => 0,
            }
        }
    "#);
}

#[test]
fn panic_macro_diverges_in_match_arm() {
    assert_type_ok(r#"
        enum SizeInfo {
            Sized { size: usize },
            Slice,
        }

        fn unpack(info: SizeInfo) -> (usize, SizeInfo) {
            match info {
                SizeInfo::Slice => panic!("cannot unpack"),
                SizeInfo::Sized { size } => (size, SizeInfo::Sized { size }),
            }
        }
    "#);
}

#[test]
fn cfg_attributed_panic_macro_is_expanded_after_strip() {
    let (result, _) = typecheck_with_cfg(r#"
        enum SizeInfo {
            Sized { size: usize },
            Slice,
        }

        macro_rules! const_panic {
            ($($arg:tt)+) => {{
                #[cfg(not(no_panic))]
                panic!($($arg)+);
                #[cfg(no_panic)]
                ()
            }};
        }

        fn unpack(info: SizeInfo) -> (usize, SizeInfo) {
            match info {
                SizeInfo::Slice => const_panic!("cannot unpack"),
                SizeInfo::Sized { size } => (size, SizeInfo::Sized { size }),
            }
        }
    "#, &[]);

    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn cfg_false_strips_call_arguments() {
    let (result, _) = typecheck_with_cfg(r#"
        fn one(_: i32) -> i32 { 1 }
        fn missing() -> bool { true }

        fn main() {
            let _: i32 = one(
                1,
                #[cfg(feature = "full")]
                missing(),
            );
        }
    "#, &[]);

    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn cfg_false_strips_function_parameters() {
    let (result, _) = typecheck_with_cfg(r#"
        struct Missing;

        fn one(
            value: i32,
            #[cfg(feature = "full")]
            missing: Missing,
        ) -> i32 {
            value
        }

        fn main() {
            let _: i32 = one(1);
        }
    "#, &[]);

    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn from_impl_allows_into_parameter_coercion() {
    let (result, _) = typecheck(r#"
        trait From<T> {}
        struct TokenTree {}
        struct Ident {}

        impl From<Ident> for TokenTree {}

        fn append(_: TokenTree) {}

        fn main() {
            append(Ident {});
        }
    "#);

    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn generic_partial_eq_impl_allows_different_rhs() {
    let (result, _) = typecheck(r#"
        trait PartialEq<Rhs> {}
        struct Ident {}

        impl<T> PartialEq<T> for Ident {}

        fn main() {
            let ident = Ident {};
            let text: &str = "x";
            if ident == text {}
        }
    "#);

    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn trait_path_self_receiver_infers_self_from_first_argument() {
    let (result, _) = typecheck(r#"
        trait AsAddress {
            fn addr(self) -> usize;
        }

        struct Unit;

        impl<T> AsAddress for *const T {
            fn addr(self) -> usize {
                0
            }
        }

        fn display_verbose_extras(addr: *const Unit) -> usize {
            AsAddress::addr(addr)
        }
    "#);

    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn infer_fn_return() {
    assert_type_ok("fn foo() -> i32 { 42 } fn main() { let x: i32 = foo(); }");
}

#[test]
fn infer_binary_op() {
    assert_type_ok("fn main() { let x: i32 = 1 + 2; }");
}

#[test]
fn error_type_mismatch() {
    assert_type_error("fn main() { let x: u32 = true; }", "mismatch");
}

#[test]
fn infer_reference() {
    assert_type_ok("fn main() { let x: i32 = 5; let y: &i32 = &x; }");
}

#[test]
fn infer_if_else() {
    assert_type_ok("fn main() { let x: i32 = if true { 1 } else { 2 }; }");
}

#[test]
fn error_if_else_mismatch() {
    assert_type_error("fn main() { let x: i32 = if true { 1 } else { true }; }", "mismatch");
}

#[test]
fn infer_block_type() {
    assert_type_ok("fn main() { let x: i32 = { let y = 5; y }; }");
}

#[test]
fn infer_struct_construction() {
    assert_type_ok(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: 1, y: 2 }; }
    "#);
}

#[test]
fn infer_field_access() {
    assert_type_ok(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: 1, y: 2 }; let v: i32 = p.x; }
    "#);
}

#[test]
fn check_fn_args() {
    assert_type_ok(r#"
        fn add(a: i32, b: i32) -> i32 { a + b }
        fn main() { let x: i32 = add(1, 2); }
    "#);
}

#[test]
fn error_wrong_arg_type() {
    assert_type_error(r#"
        fn add(a: i32, b: i32) -> i32 { a + b }
        fn main() { add(true, 2); }
    "#, "mismatch");
}

#[test]
fn error_wrong_arg_count() {
    assert_type_error(r#"
        fn add(a: i32, b: i32) -> i32 { a + b }
        fn main() { add(1); }
    "#, "argument");
}

#[test]
fn check_return_type() {
    assert_type_ok("fn foo() -> i32 { return 42; }");
}

#[test]
fn error_wrong_return_type() {
    assert_type_error("fn foo() -> i32 { return true; }", "mismatch");
}

#[test]
fn check_assign_types() {
    assert_type_ok("fn main() { let mut x: i32 = 0; x = 5; }");
}

#[test]
fn check_comparison_returns_bool() {
    assert_type_ok("fn main() { let x: bool = 1 < 2; }");
}

#[test]
fn check_logical_ops() {
    assert_type_ok("fn main() { let x: bool = true && false || true; }");
}

#[test]
fn infer_enum_variant() {
    assert_type_ok(r#"
        enum Color { Red, Green, Blue }
        fn main() { let c = Color::Red; }
    "#);
}

#[test]
fn block_like_statement_does_not_absorb_following_deref_expr() {
    assert_type_ok(r#"
        unsafe fn cstr_eq_sym(strtab: *const u8, offset: usize, name: &[u8]) -> bool {
            let s = strtab.add(offset);
            for (i, &b) in name.iter().enumerate() {
                if *s.add(i) != b { return false; }
            }
            *s.add(name.len()) == 0
        }
    "#);
}

#[test]
fn vec_new_infers_element_type_through_slice_coercion() {
    assert_type_ok(r#"
        fn main() {
            let data = Vec::new();
            let _text = core::str::from_utf8(&data);
        }
    "#);
}

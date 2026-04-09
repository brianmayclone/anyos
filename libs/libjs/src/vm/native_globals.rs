//! Global functions and type constructors:
//! parseInt, parseFloat, isNaN, isFinite, encodeURIComponent,
//! decodeURIComponent, Object, Array, String, Number, Boolean.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::Vm;
use crate::ast::*;
use crate::compiler::Compiler;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::value::*;

// ═══════════════════════════════════════════════════════════
// Global functions
// ═══════════════════════════════════════════════════════════

pub fn global_parse_int(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let radix = args.get(1).map(|v| v.to_number() as u32).unwrap_or(0);
    let s = s.trim();

    if s.is_empty() {
        return JsValue::Number(f64::NAN);
    }

    let (negative, s) = if s.starts_with('-') {
        (true, &s[1..])
    } else if s.starts_with('+') {
        (false, &s[1..])
    } else {
        (false, s)
    };

    let actual_radix = if radix == 0 {
        if s.starts_with("0x") || s.starts_with("0X") {
            16
        } else {
            10
        }
    } else {
        radix
    };

    let digits = if actual_radix == 16 && (s.starts_with("0x") || s.starts_with("0X")) {
        &s[2..]
    } else {
        s
    };

    if actual_radix < 2 || actual_radix > 36 {
        return JsValue::Number(f64::NAN);
    }

    let mut result: f64 = 0.0;
    let mut found = false;
    for b in digits.bytes() {
        let digit = match b {
            b'0'..=b'9' => (b - b'0') as u32,
            b'a'..=b'z' => (b - b'a' + 10) as u32,
            b'A'..=b'Z' => (b - b'A' + 10) as u32,
            _ => break,
        };
        if digit >= actual_radix {
            break;
        }
        result = result * actual_radix as f64 + digit as f64;
        found = true;
    }

    if !found {
        return JsValue::Number(f64::NAN);
    }
    JsValue::Number(if negative { -result } else { result })
}

pub fn global_parse_float(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    JsValue::Number(parse_js_float(&s))
}

pub fn global_is_nan(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Bool(n.is_nan())
}

pub fn global_is_finite(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Bool(n.is_finite())
}

pub fn global_encode_uri_component(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let mut result = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => {
                result.push(b as char);
            }
            _ => {
                result.push('%');
                result.push(hex_digit(b >> 4));
                result.push(hex_digit(b & 0x0F));
            }
        }
    }
    JsValue::String(result)
}

pub fn global_decode_uri_component(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let bytes = s.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                result.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    JsValue::String(String::from_utf8(result).unwrap_or_default())
}

// ═══════════════════════════════════════════════════════════
// Type constructors
// ═══════════════════════════════════════════════════════════

/// `Object()` / `new Object()` — returns an empty object or wraps a value.
pub fn ctor_object(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(val @ JsValue::Object(_)) => val.clone(),
        Some(val @ JsValue::Array(_)) => val.clone(),
        None | Some(JsValue::Undefined) | Some(JsValue::Null) => JsValue::new_object(),
        _ => JsValue::new_object(),
    }
}

/// `Array(len)` / `Array(...items)` / `new Array(...)`.
pub fn ctor_array(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if args.len() == 1 {
        if let JsValue::Number(n) = &args[0] {
            let len = *n as usize;
            // Sparse: only set the logical length, allocate nothing.
            let mut arr = JsArray::new();
            arr.length = len;
            return JsValue::Array(Rc::new(RefCell::new(arr)));
        }
    }
    JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(args.to_vec()))))
}

/// `String(value)` — converts to string.
pub fn ctor_string(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args
        .first()
        .map(|v| {
            let prim = match v {
                JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) => {
                    vm.to_primitive_for_op(v.clone(), "string")
                }
                _ => v.clone(),
            };
            if super::is_symbol_value(&prim) {
                super::native_string::symbol_display_name(&prim.to_js_string())
            } else {
                prim.to_js_string()
            }
        })
        .unwrap_or_default();
    // Only constructor calls entered via `new` may create wrapper objects.
    if vm.is_in_constructor_call() {
        if let JsValue::Object(obj) = vm.current_this.clone() {
            let mut o = obj.borrow_mut();
            o.internal_tag = Some(String::from("__string__"));
            o.primitive_value = Some(Box::new(JsValue::String(s)));
            drop(o);
            return JsValue::Object(obj);
        }
    }
    // Called as plain function: return primitive
    JsValue::String(s)
}

/// `Number(value)` — converts to number.
pub fn ctor_number(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(0.0);
    // Only constructor calls entered via `new` may create wrapper objects.
    if vm.is_in_constructor_call() {
        if let JsValue::Object(obj) = vm.current_this.clone() {
            let mut o = obj.borrow_mut();
            o.internal_tag = Some(String::from("__number__"));
            o.primitive_value = Some(Box::new(JsValue::Number(n)));
            drop(o);
            return JsValue::Object(obj);
        }
    }
    // Called as plain function: return primitive
    JsValue::Number(n)
}

fn strict_code_contains_with(stmts: &[Stmt], inherited_strict: bool) -> bool {
    let mut is_strict = inherited_strict;
    if let Some(Stmt::Expr(Expr::String(s))) = stmts.first() {
        if s == "use strict" {
            is_strict = true;
        }
    }
    stmts.iter().any(|stmt| stmt_contains_strict_with(stmt, is_strict))
}

fn stmt_contains_strict_with(stmt: &Stmt, is_strict: bool) -> bool {
    match stmt {
        Stmt::With { .. } => is_strict,
        Stmt::Block(stmts) => strict_code_contains_with(stmts, is_strict),
        Stmt::If {
            consequent,
            alternate,
            ..
        } => {
            stmt_contains_strict_with(consequent, is_strict)
                || alternate
                    .as_ref()
                    .map(|alt| stmt_contains_strict_with(alt, is_strict))
                    .unwrap_or(false)
        }
        Stmt::While { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::Labeled { body, .. } => stmt_contains_strict_with(body, is_strict),
        Stmt::For { body, .. }
        | Stmt::ForIn { body, .. }
        | Stmt::ForOf { body, .. } => stmt_contains_strict_with(body, is_strict),
        Stmt::Switch { cases, .. } => cases
            .iter()
            .flat_map(|c| c.consequent.iter())
            .any(|s| stmt_contains_strict_with(s, is_strict)),
        Stmt::Try {
            block,
            catch,
            finally,
        } => {
            strict_code_contains_with(block, is_strict)
                || catch
                    .as_ref()
                    .map(|clause| strict_code_contains_with(&clause.body, is_strict))
                    .unwrap_or(false)
                || finally
                    .as_ref()
                    .map(|body| strict_code_contains_with(body, is_strict))
                    .unwrap_or(false)
        }
        Stmt::FunctionDecl { body, .. } => strict_code_contains_with(body, is_strict),
        Stmt::VarDecl { decls, .. } => decls.iter().any(|d| {
            d.init
                .as_ref()
                .map(|e| expr_contains_strict_with(e, is_strict))
                .unwrap_or(false)
        }),
        Stmt::Expr(expr) => expr_contains_strict_with(expr, is_strict),
        _ => false,
    }
}

fn expr_contains_strict_with(expr: &Expr, is_strict: bool) -> bool {
    match expr {
        Expr::FunctionExpr { body, .. } | Expr::Arrow { body: ArrowBody::Block(body), .. } => {
            strict_code_contains_with(body, is_strict)
        }
        Expr::Arrow {
            body: ArrowBody::Expr(expr),
            ..
        } => expr_contains_strict_with(expr, is_strict),
        Expr::Assign { left, right, .. } => {
            expr_contains_strict_with(left, is_strict)
                || expr_contains_strict_with(right, is_strict)
        }
        Expr::Binary { left, right, .. }
        | Expr::Logical { left, right, .. }
        | Expr::Index {
            object: left,
            index: right,
        } => {
            expr_contains_strict_with(left, is_strict)
                || expr_contains_strict_with(right, is_strict)
        }
        Expr::Unary { argument: expr, .. }
        | Expr::Await(expr)
        | Expr::Yield(Some(expr))
        | Expr::YieldDelegate(expr)
        | Expr::Spread(expr)
        | Expr::Void(expr)
        | Expr::Typeof(expr)
        | Expr::Delete(expr) => expr_contains_strict_with(expr, is_strict),
        Expr::Call { callee, arguments }
        | Expr::New { callee, arguments } => {
            expr_contains_strict_with(callee, is_strict)
                || arguments
                    .iter()
                    .any(|arg| expr_contains_strict_with(arg, is_strict))
        }
        Expr::Member { object, .. } | Expr::OptionalChain { object, .. } => {
            expr_contains_strict_with(object, is_strict)
        }
        Expr::OptionalCall { callee, arguments } => {
            expr_contains_strict_with(callee, is_strict)
                || arguments
                    .iter()
                    .any(|arg| expr_contains_strict_with(arg, is_strict))
        }
        Expr::Array(items) => items
            .iter()
            .flatten()
            .any(|e| expr_contains_strict_with(e, is_strict)),
        Expr::Object(props) => props.iter().any(|p| {
            let key_has_with = match &p.key {
                PropKey::Computed(expr) => expr_contains_strict_with(expr, is_strict),
                _ => false,
            };
            key_has_with || expr_contains_strict_with(&p.value, is_strict)
        }),
        Expr::Conditional {
            test,
            consequent,
            alternate,
        } => {
            expr_contains_strict_with(test, is_strict)
                || expr_contains_strict_with(consequent, is_strict)
                || expr_contains_strict_with(alternate, is_strict)
        }
        Expr::Sequence(exprs) => exprs.iter().any(|e| expr_contains_strict_with(e, is_strict)),
        Expr::ClassExpr { body, .. } => body.iter().any(|member| {
            let key_has_with = match &member.key {
                PropKey::Computed(expr) => expr_contains_strict_with(expr, true),
                _ => false,
            };
            key_has_with
                || match &member.kind {
                    ClassMemberKind::Method { body, .. }
                    | ClassMemberKind::Constructor { body, .. }
                    | ClassMemberKind::Getter { body }
                    | ClassMemberKind::Setter { body, .. }
                    | ClassMemberKind::StaticBlock { body } => {
                        strict_code_contains_with(body, true)
                    }
                    ClassMemberKind::Property { value } => value
                        .as_ref()
                        .map(|expr| expr_contains_strict_with(expr, true))
                        .unwrap_or(false),
                }
        }),
        Expr::TaggedTemplate { tag, .. } => expr_contains_strict_with(tag, is_strict),
        _ => false,
    }
}

/// `Function([arg1[, arg2[, ...argN]],] body)` — dynamically compiles a function.
///
/// This is a deliberately small but real implementation because many modern
/// sites use `new Function(code)()` for deferred actions or configuration hooks.
pub fn ctor_function(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let body = args
        .last()
        .map(|v| v.to_js_string())
        .unwrap_or_else(String::new);
    let params = if args.len() > 1 {
        args[..args.len() - 1]
            .iter()
            .map(|v| v.to_js_string())
            .collect::<Vec<_>>()
            .join(",")
    } else {
        String::new()
    };

    let source = if params.is_empty() {
        alloc::format!("(function anonymous() {{\n{}\n}})", body)
    } else {
        alloc::format!("(function anonymous({}) {{\n{}\n}})", params, body)
    };

    let tokens = Lexer::tokenize(&source);
    let mut parser = Parser::new(tokens);
    let program = parser.parse_program();
    if !parser.errors.is_empty() {
        let err = vm.make_syntax_error(&parser.errors[0]);
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    if strict_code_contains_with(&program.body, false) {
        let err = vm.make_syntax_error("Strict mode code may not include a with statement");
        vm.throw_native(err);
        return JsValue::Undefined;
    }

    let mut compiler = Compiler::new();
    let chunk = compiler.compile_eval(&program);

    let prev_this = vm.current_this.clone();
    vm.current_this = JsValue::Undefined;
    let result = vm.execute(chunk);
    vm.current_this = prev_this;
    result
}

/// `Boolean(value)` — converts to boolean, or creates a wrapper object when called as `new`.
///
/// When called as `new Boolean(x)`, `vm.current_this` is the freshly allocated
/// object (set by `new_object()`).  We tag it with `__bool_data__` and return it
/// so the caller receives the wrapper.  When called as a plain function,
/// `current_this` is `undefined` and we return the primitive bool.
pub fn ctor_boolean(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let b = args.first().map(|v| v.to_boolean()).unwrap_or(false);
    if vm.is_in_constructor_call() {
        if let JsValue::Object(obj) = vm.current_this.clone() {
            let mut o = obj.borrow_mut();
            o.internal_tag = Some(String::from("__boolean__"));
            // Store the bool both as [[PrimitiveValue]] (for abstract equality) and
            // as a named property (for backward compatibility with extract_bool_this).
            o.primitive_value = Some(Box::new(JsValue::Bool(b)));
            o.set(String::from("__bool_data__"), JsValue::Bool(b));
            drop(o);
            return vm.current_this.clone();
        }
    }
    JsValue::Bool(b)
}

// ═══════════════════════════════════════════════════════════
// Boolean.prototype methods
// ═══════════════════════════════════════════════════════════

/// `Boolean.prototype.valueOf()` — returns the boolean primitive value.
/// Throws TypeError when called on a non-Boolean `this`.
pub fn boolean_value_of(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    match extract_bool_this(vm) {
        Some(v) => v,
        None => {
            let err = vm.make_type_error("Boolean.prototype.valueOf called on non-Boolean");
            vm.throw_native(err);
            JsValue::Undefined
        }
    }
}

/// `Boolean.prototype.toString()` — returns "true" or "false".
/// Throws TypeError when called on a non-Boolean `this`.
pub fn boolean_to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    match extract_bool_this(vm) {
        Some(JsValue::Bool(true)) => JsValue::String(String::from("true")),
        Some(JsValue::Bool(false)) => JsValue::String(String::from("false")),
        Some(_) => JsValue::String(String::from("false")),
        None => {
            let err = vm.make_type_error("Boolean.prototype.toString called on non-Boolean");
            vm.throw_native(err);
            JsValue::Undefined
        }
    }
}

/// Try to extract the boolean value from `this`.
/// Returns `Some(Bool)` for Boolean primitives and `Boolean` wrapper objects.
/// Returns `None` for any other type (caller should throw TypeError).
fn extract_bool_this(vm: &Vm) -> Option<JsValue> {
    match &vm.current_this {
        JsValue::Bool(_) => Some(vm.current_this.clone()),
        JsValue::Object(obj) => {
            let o = obj.borrow();
            if o.internal_tag.as_deref() == Some("__boolean__") {
                if let Some(prop) = o.properties.get("__bool_data__") {
                    Some(prop.value.clone())
                } else {
                    Some(JsValue::Bool(false))
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════
// Number static methods
// ═══════════════════════════════════════════════════════════

/// `Number.isNaN(value)` — strict NaN check (no coercion).
pub fn number_is_nan(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Number(n)) => JsValue::Bool(n.is_nan()),
        _ => JsValue::Bool(false),
    }
}

/// `Number.isFinite(value)` — strict finite check (no coercion).
pub fn number_is_finite(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Number(n)) => JsValue::Bool(n.is_finite()),
        _ => JsValue::Bool(false),
    }
}

/// `Number.isInteger(value)` — true if value is a finite integer.
pub fn number_is_integer(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Number(n)) => JsValue::Bool(n.is_finite() && *n % 1.0 == 0.0),
        _ => JsValue::Bool(false),
    }
}

// ═══════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'A' + n - 10) as char,
        _ => '0',
    }
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

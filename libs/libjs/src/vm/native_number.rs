//! Number.prototype methods.

use alloc::string::String;
use alloc::vec::Vec;

use super::native_math::{floor_f64, ln_approx, trunc_f64};
use super::Vm;
use crate::value::*;

fn this_number_value(vm: &mut Vm) -> Option<f64> {
    match &vm.current_this {
        JsValue::Number(n) => Some(*n),
        JsValue::Object(obj) => {
            let o = obj.borrow();
            match o.primitive_value.as_deref() {
                Some(JsValue::Number(n)) => Some(*n),
                _ => {
                    let err = vm.make_type_error("Number.prototype method called on incompatible receiver");
                    drop(o);
                    vm.throw_native(err);
                    None
                }
            }
        }
        _ => {
            let err = vm.make_type_error("Number.prototype method called on incompatible receiver");
            vm.throw_native(err);
            None
        }
    }
}

fn to_number_arg(vm: &mut Vm, value: &JsValue) -> Option<f64> {
    let n = crate::vm::native_array::to_number_vm(vm, value);
    if vm.pending_exception.is_some() {
        None
    } else {
        Some(n)
    }
}

fn integer_arg_in_range(
    vm: &mut Vm,
    args: &[JsValue],
    default: Option<usize>,
    min: usize,
    max: usize,
    name: &str,
) -> Option<Option<usize>> {
    match args.first() {
        None => Some(default),
        Some(JsValue::Undefined) => Some(default),
        Some(v) => {
            let raw = to_number_arg(vm, v)?;
            let int = trunc_f64(raw);
            if !int.is_finite() || int < min as f64 || int > max as f64 {
                let err = vm.make_range_error(name);
                vm.throw_native(err);
                return None;
            }
            Some(Some(int as usize))
        }
    }
}

// ═══════════════════════════════════════════════════════════
// Number.prototype methods
// ═══════════════════════════════════════════════════════════

pub fn number_to_string(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = match this_number_value(vm) {
        Some(n) => n,
        None => return JsValue::Undefined,
    };
    let radix = match args.first() {
        None | Some(JsValue::Undefined) => 10,
        Some(v) => {
            let r = match to_number_arg(vm, v) {
                Some(n) => n as u32,
                None => return JsValue::Undefined,
            };
            if r < 2 || r > 36 {
                let err = vm.make_range_error("toString() radix must be between 2 and 36");
                vm.throw_native(err);
                return JsValue::Undefined;
            }
            r
        }
    };

    if radix == 10 {
        return JsValue::String(format_number(n));
    }

    if n.is_nan() {
        return JsValue::String(String::from("NaN"));
    }
    if n.is_infinite() {
        return JsValue::String(if n > 0.0 {
            String::from("Infinity")
        } else {
            String::from("-Infinity")
        });
    }

    // Integer radix conversion
    let negative = n < 0.0;
    let mut value = if negative { -n } else { n } as u64;

    if value == 0 {
        return JsValue::String(String::from("0"));
    }

    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut buf = Vec::new();
    while value > 0 {
        buf.push(digits[(value % radix as u64) as usize]);
        value /= radix as u64;
    }
    if negative {
        buf.push(b'-');
    }
    buf.reverse();
    // SAFETY: buf contains only ASCII
    JsValue::String(unsafe { String::from_utf8_unchecked(buf) })
}

pub fn number_value_of(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    match this_number_value(vm) {
        Some(n) => JsValue::Number(n),
        None => JsValue::Undefined,
    }
}

pub fn number_to_fixed(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = match this_number_value(vm) {
        Some(n) => n,
        None => return JsValue::Undefined,
    };
    let digits = match integer_arg_in_range(
        vm,
        args,
        Some(0),
        0,
        100,
        "toFixed() digits must be between 0 and 100",
    ) {
        Some(Some(d)) => d,
        Some(None) => 0,
        None => return JsValue::Undefined,
    };

    if n.is_nan() {
        return JsValue::String(String::from("NaN"));
    }
    if n.is_infinite() {
        return JsValue::String(if n > 0.0 {
            String::from("Infinity")
        } else {
            String::from("-Infinity")
        });
    }

    let negative = n < 0.0;
    let abs = if negative { -n } else { n };

    // Multiply by 10^digits, round, then format
    let factor = pow10_usize(digits);
    let rounded = super::native_math::floor_f64(abs * factor + 0.5) as u64;

    let int_part = rounded / (factor as u64);
    let frac_part = rounded % (factor as u64);

    let mut result = String::new();
    if negative && (int_part > 0 || frac_part > 0) {
        result.push('-');
    }

    // Integer part
    result.push_str(&format_u64(int_part));

    if digits > 0 {
        result.push('.');
        // Pad fractional part with leading zeros
        let frac_str = format_u64(frac_part);
        for _ in 0..digits.saturating_sub(frac_str.len()) {
            result.push('0');
        }
        result.push_str(&frac_str);
    }

    JsValue::String(result)
}

/// `Number.prototype.toPrecision(precision)`
pub fn number_to_precision(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = match this_number_value(vm) {
        Some(n) => n,
        None => return JsValue::Undefined,
    };
    if args.is_empty() || matches!(args.first(), Some(JsValue::Undefined)) {
        return JsValue::String(format_number(n));
    }
    let prec = match integer_arg_in_range(
        vm,
        args,
        None,
        1,
        100,
        "toPrecision() precision must be between 1 and 100",
    ) {
        Some(Some(p)) => p,
        Some(None) => 1,
        None => return JsValue::Undefined,
    };

    if n.is_nan() {
        return JsValue::String(String::from("NaN"));
    }
    if n.is_infinite() {
        return JsValue::String(if n > 0.0 {
            String::from("Infinity")
        } else {
            String::from("-Infinity")
        });
    }

    let negative = n < 0.0;
    let abs = if negative { -n } else { n };

    // Use exponential notation if needed
    if abs == 0.0 {
        let mut s = String::from("0");
        if prec > 1 {
            s.push('.');
            for _ in 0..prec - 1 {
                s.push('0');
            }
        }
        if negative {
            let mut r = String::from("-");
            r.push_str(&s);
            return JsValue::String(r);
        }
        return JsValue::String(s);
    }

    let e = floor_f64(log10_approx(abs)) as i32;
    let factor = pow10_usize(prec.saturating_sub(1).max(0));
    let shifted = floor_f64(abs / pow10_i32(e - prec as i32 + 1) + 0.5) as u64;
    let digits_str = format_u64(shifted);

    let mut result = String::new();
    if negative {
        result.push('-');
    }

    if e >= 0 && (e as usize) < prec {
        // Fixed notation
        let int_digits = (e + 1) as usize;
        for (i, ch) in digits_str.chars().enumerate() {
            if i == int_digits && i < digits_str.len() {
                result.push('.');
            }
            result.push(ch);
        }
        // Pad with zeros if needed
        while result.replace('-', "").replace('.', "").len() < prec {
            if !result.contains('.') {
                result.push('.');
            }
            result.push('0');
        }
    } else {
        // Exponential notation
        let mut chars = digits_str.chars();
        if let Some(first) = chars.next() {
            result.push(first);
        }
        let rest: String = chars.collect();
        if !rest.is_empty() {
            result.push('.');
            result.push_str(&rest);
        }
        result.push('e');
        if e >= 0 {
            result.push('+');
        }
        let e_str = format_i32(e);
        result.push_str(&e_str);
    }

    JsValue::String(result)
}

/// `Number.prototype.toExponential(fractionDigits)`
pub fn number_to_exponential(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = match this_number_value(vm) {
        Some(n) => n,
        None => return JsValue::Undefined,
    };
    if n.is_nan() {
        return JsValue::String(String::from("NaN"));
    }
    if n.is_infinite() {
        return JsValue::String(if n > 0.0 {
            String::from("Infinity")
        } else {
            String::from("-Infinity")
        });
    }

    let negative = n < 0.0;
    let abs = if negative { -n } else { n };

    let frac_digits = if args.is_empty() || matches!(args.first(), Some(JsValue::Undefined)) {
        None
    } else {
        match integer_arg_in_range(
            vm,
            args,
            None,
            0,
            100,
            "toExponential() fractionDigits must be between 0 and 100",
        ) {
            Some(v) => v,
            None => return JsValue::Undefined,
        }
    };

    if abs == 0.0 {
        let mut s = String::from("0");
        if let Some(fd) = frac_digits {
            if fd > 0 {
                s.push('.');
                for _ in 0..fd {
                    s.push('0');
                }
            }
        }
        s.push_str("e+0");
        if negative {
            let mut r = String::from("-");
            r.push_str(&s);
            return JsValue::String(r);
        }
        return JsValue::String(s);
    }

    let e = floor_f64(log10_approx(abs)) as i32;
    let prec = frac_digits.unwrap_or_else(|| {
        // Determine minimum significant digits needed
        let mut p = 1;
        while p < 20 {
            let factor = pow10_i32(e - p as i32);
            let shifted = floor_f64(abs / factor + 0.5);
            if (shifted * factor - abs).abs() < 1e-10 {
                break;
            }
            p += 1;
        }
        p
    });

    let factor = pow10_i32(e - prec as i32);
    let shifted = floor_f64(abs / factor + 0.5) as u64;
    let digits_str = format_u64(shifted);

    let mut result = String::new();
    if negative {
        result.push('-');
    }
    let mut chars = digits_str.chars();
    if let Some(first) = chars.next() {
        result.push(first);
    }
    let rest: String = chars.collect();
    if !rest.is_empty() || frac_digits.map_or(false, |fd| fd > 0) {
        result.push('.');
        result.push_str(&rest);
        // Pad with zeros
        if let Some(fd) = frac_digits {
            while result.len() - result.find('.').unwrap_or(result.len()) - 1 < fd {
                result.push('0');
            }
        }
    }
    result.push('e');
    if e >= 0 {
        result.push('+');
    }
    result.push_str(&format_i32(e));

    JsValue::String(result)
}

/// `Number.isSafeInteger(value)`
pub fn number_is_safe_integer(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Number(n)) => JsValue::Bool(
            !n.is_nan() && !n.is_infinite() && *n == trunc_f64(*n) && n.abs() <= 9007199254740991.0,
        ),
        _ => JsValue::Bool(false),
    }
}

/// `Number.parseFloat(string)`
pub fn number_parse_float(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    super::native_globals::global_parse_float(vm, args)
}

/// `Number.parseInt(string, radix)`
pub fn number_parse_int(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    super::native_globals::global_parse_int(vm, args)
}

// ── Helpers ──

fn pow10_usize(n: usize) -> f64 {
    let mut r = 1.0;
    for _ in 0..n {
        r *= 10.0;
    }
    r
}

fn log10_approx(x: f64) -> f64 {
    ln_approx(x) / core::f64::consts::LN_10
}

fn pow10_i32(n: i32) -> f64 {
    if n >= 0 {
        let mut r = 1.0;
        for _ in 0..n {
            r *= 10.0;
        }
        r
    } else {
        let mut r = 1.0;
        for _ in 0..-n {
            r /= 10.0;
        }
        r
    }
}

fn format_i32(n: i32) -> String {
    if n < 0 {
        let mut s = String::from("-");
        s.push_str(&format_u64((-n) as u64));
        s
    } else {
        format_u64(n as u64)
    }
}

fn format_u64(mut n: u64) -> String {
    if n == 0 {
        return String::from("0");
    }
    let mut buf = Vec::new();
    while n > 0 {
        buf.push(b'0' + (n % 10) as u8);
        n /= 10;
    }
    buf.reverse();
    unsafe { String::from_utf8_unchecked(buf) }
}

//! Number.prototype methods.

use alloc::string::String;
use alloc::vec::Vec;

use crate::value::*;
use super::Vm;

// ═══════════════════════════════════════════════════════════
// Number.prototype methods
// ═══════════════════════════════════════════════════════════

pub fn number_to_string(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = vm.current_this.to_number();
    let radix = args.first().map(|v| v.to_number() as u32).unwrap_or(10);

    if radix == 10 || radix < 2 || radix > 36 {
        return JsValue::String(format_number(n));
    }

    if n.is_nan() { return JsValue::String(String::from("NaN")); }
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
    JsValue::Number(vm.current_this.to_number())
}

pub fn number_to_fixed(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = vm.current_this.to_number();
    let digits = args.first().map(|v| v.to_number() as usize).unwrap_or(0).min(100);

    if n.is_nan() { return JsValue::String(String::from("NaN")); }
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

// ── Helpers ──

fn pow10_usize(n: usize) -> f64 {
    let mut r = 1.0;
    for _ in 0..n { r *= 10.0; }
    r
}

fn format_u64(mut n: u64) -> String {
    if n == 0 { return String::from("0"); }
    let mut buf = Vec::new();
    while n > 0 {
        buf.push(b'0' + (n % 10) as u8);
        n /= 10;
    }
    buf.reverse();
    unsafe { String::from_utf8_unchecked(buf) }
}

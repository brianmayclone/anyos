//! console.log / console.warn / console.error

use alloc::string::String;

use super::Vm;
use crate::value::*;

// ═══════════════════════════════════════════════════════════
// console methods
// ═══════════════════════════════════════════════════════════

pub fn console_log(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let msg = format_args_to_string(args);
    vm.console_output.push(msg);
    JsValue::Undefined
}

pub fn console_warn(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut msg = String::from("[WARN] ");
    msg.push_str(&format_args_to_string(args));
    vm.console_output.push(msg);
    JsValue::Undefined
}

pub fn console_error(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut msg = String::from("[ERROR] ");
    msg.push_str(&format_args_to_string(args));
    // Add call stack for debugging
    let mut stack = String::from(" [at: ");
    for (fi, frame) in vm.frames.iter().rev().take(6).enumerate() {
        let fname = frame.chunk.name.as_deref().unwrap_or("(anon)");
        if fi > 0 {
            stack.push_str(" <- ");
        }
        stack.push_str(fname);
    }
    stack.push(']');
    msg.push_str(&stack);
    vm.console_output.push(msg);
    JsValue::Undefined
}

// ═══════════════════════════════════════════════════════════
// Helper
// ═══════════════════════════════════════════════════════════

fn format_args_to_string(args: &[JsValue]) -> String {
    let mut out = String::new();
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        match arg {
            JsValue::String(s) => out.push_str(s),
            JsValue::Undefined => out.push_str("undefined"),
            JsValue::Null => out.push_str("null"),
            JsValue::Bool(true) => out.push_str("true"),
            JsValue::Bool(false) => out.push_str("false"),
            JsValue::Number(n) => out.push_str(&format_number(*n)),
            JsValue::Array(a) => {
                let arr = a.borrow();
                out.push('[');
                let mut first = true;
                for (_, el) in arr.iter_entries() {
                    if !first {
                        out.push_str(", ");
                    }
                    first = false;
                    out.push_str(&el.to_js_string());
                }
                out.push(']');
            }
            JsValue::Object(_) => out.push_str("[object Object]"),
            JsValue::Function(f) => {
                let func = f.borrow();
                out.push_str("[Function");
                if let Some(ref name) = func.name {
                    out.push_str(": ");
                    out.push_str(name);
                }
                out.push(']');
            }
            JsValue::BigInt(bi) => {
                out.push_str(&bi.to_string_radix(10));
                out.push('n');
            }
        }
    }
    out
}

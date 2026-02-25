//! console.log / console.warn / console.error

use alloc::string::String;

use crate::value::*;
use super::Vm;

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
    vm.console_output.push(msg);
    JsValue::Undefined
}

// ═══════════════════════════════════════════════════════════
// Helper
// ═══════════════════════════════════════════════════════════

fn format_args_to_string(args: &[JsValue]) -> String {
    let mut out = String::new();
    for (i, arg) in args.iter().enumerate() {
        if i > 0 { out.push(' '); }
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
                for (j, el) in arr.elements.iter().enumerate() {
                    if j > 0 { out.push_str(", "); }
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
        }
    }
    out
}

//! Symbol — ES6+ unique identifiers.
//!
//! Symbols are represented as JsValue::String with a unique
//! prefix `__symbol__<id>__` that makes them distinguishable.
//! Well-known symbols (Symbol.iterator, Symbol.toPrimitive, etc.)
//! are pre-defined with fixed names.

use alloc::string::String;
use alloc::format;

use crate::value::*;
use super::{Vm, native_fn};

// Monotonically increasing symbol counter.
static mut NEXT_SYMBOL_ID: u64 = 1;

// ═══════════════════════════════════════════════════════════
// Symbol constructor
// ═══════════════════════════════════════════════════════════

/// `Symbol(description)` — creates a unique symbol value.
/// Calling with `new Symbol()` should throw, but we allow it
/// and return a string-based symbol representation.
pub fn ctor_symbol(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let desc = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let id = unsafe {
        let id = NEXT_SYMBOL_ID;
        NEXT_SYMBOL_ID += 1;
        id
    };
    JsValue::String(format!("__symbol__{}_{}", id, desc))
}

// ═══════════════════════════════════════════════════════════
// Symbol static methods / well-known symbols
// ═══════════════════════════════════════════════════════════

/// `Symbol.for(key)` — returns a globally shared symbol for the key.
/// We implement a simple global registry via a naming convention.
pub fn symbol_for(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    JsValue::String(format!("__symbol_global__{}", key))
}

/// `Symbol.keyFor(sym)` — reverse lookup of Symbol.for.
pub fn symbol_key_for(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(JsValue::String(s)) = args.first() {
        if let Some(rest) = s.strip_prefix("__symbol_global__") {
            return JsValue::String(String::from(rest));
        }
    }
    JsValue::Undefined
}

/// Install well-known symbols on the Symbol constructor object.
pub fn install_well_known_symbols(symbol_ctor: &JsValue) {
    symbol_ctor.set_property(String::from("iterator"), JsValue::String(String::from("Symbol.iterator")));
    symbol_ctor.set_property(String::from("toPrimitive"), JsValue::String(String::from("Symbol.toPrimitive")));
    symbol_ctor.set_property(String::from("toStringTag"), JsValue::String(String::from("Symbol.toStringTag")));
    symbol_ctor.set_property(String::from("hasInstance"), JsValue::String(String::from("Symbol.hasInstance")));
    symbol_ctor.set_property(String::from("isConcatSpreadable"), JsValue::String(String::from("Symbol.isConcatSpreadable")));
    symbol_ctor.set_property(String::from("species"), JsValue::String(String::from("Symbol.species")));
    symbol_ctor.set_property(String::from("match"), JsValue::String(String::from("Symbol.match")));
    symbol_ctor.set_property(String::from("replace"), JsValue::String(String::from("Symbol.replace")));
    symbol_ctor.set_property(String::from("search"), JsValue::String(String::from("Symbol.search")));
    symbol_ctor.set_property(String::from("split"), JsValue::String(String::from("Symbol.split")));
    symbol_ctor.set_property(String::from("unscopables"), JsValue::String(String::from("Symbol.unscopables")));
    symbol_ctor.set_property(String::from("asyncIterator"), JsValue::String(String::from("Symbol.asyncIterator")));

    // Static methods
    symbol_ctor.set_property(String::from("for"), native_fn("for", symbol_for));
    symbol_ctor.set_property(String::from("keyFor"), native_fn("keyFor", symbol_key_for));
}

//! Symbol — ES6+ unique identifiers.
//!
//! Symbols are represented as JsValue::String with a unique
//! prefix `__symbol__<id>__` that makes them distinguishable.
//! Well-known symbols (Symbol.iterator, Symbol.toPrimitive, etc.)
//! are pre-defined with fixed names.

use alloc::format;
use alloc::string::String;

use super::{native_fn, Vm};
use crate::value::*;

// Well-known symbol keys (use __symbol__0_ prefix so is_symbol_value() recognizes them)
pub const WELL_KNOWN_ITERATOR: &str = "__symbol__0_Symbol.iterator";
pub const WELL_KNOWN_TO_PRIMITIVE: &str = "__symbol__0_Symbol.toPrimitive";
pub const WELL_KNOWN_TO_STRING_TAG: &str = "__symbol__0_Symbol.toStringTag";
pub const WELL_KNOWN_HAS_INSTANCE: &str = "__symbol__0_Symbol.hasInstance";
pub const WELL_KNOWN_MATCH: &str = "__symbol__0_Symbol.match";
pub const WELL_KNOWN_IS_CONCAT_SPREADABLE: &str = "__symbol__0_Symbol.isConcatSpreadable";
pub const WELL_KNOWN_SPECIES: &str = "__symbol__0_Symbol.species";
pub const WELL_KNOWN_UNSCOPABLES: &str = "__symbol__0_Symbol.unscopables";

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
    symbol_ctor.set_property(
        String::from("iterator"),
        JsValue::String(String::from(WELL_KNOWN_ITERATOR)),
    );
    symbol_ctor.set_property(
        String::from("toPrimitive"),
        JsValue::String(String::from(WELL_KNOWN_TO_PRIMITIVE)),
    );
    symbol_ctor.set_property(
        String::from("toStringTag"),
        JsValue::String(String::from(WELL_KNOWN_TO_STRING_TAG)),
    );
    symbol_ctor.set_property(
        String::from("hasInstance"),
        JsValue::String(String::from(WELL_KNOWN_HAS_INSTANCE)),
    );
    symbol_ctor.set_property(
        String::from("isConcatSpreadable"),
        JsValue::String(String::from(WELL_KNOWN_IS_CONCAT_SPREADABLE)),
    );
    symbol_ctor.set_property(
        String::from("species"),
        JsValue::String(String::from(WELL_KNOWN_SPECIES)),
    );
    symbol_ctor.set_property(
        String::from("match"),
        JsValue::String(String::from(WELL_KNOWN_MATCH)),
    );
    symbol_ctor.set_property(
        String::from("replace"),
        JsValue::String(String::from("__symbol__0_Symbol.replace")),
    );
    symbol_ctor.set_property(
        String::from("search"),
        JsValue::String(String::from("__symbol__0_Symbol.search")),
    );
    symbol_ctor.set_property(
        String::from("split"),
        JsValue::String(String::from("__symbol__0_Symbol.split")),
    );
    symbol_ctor.set_property(
        String::from("unscopables"),
        JsValue::String(String::from(WELL_KNOWN_UNSCOPABLES)),
    );
    symbol_ctor.set_property(
        String::from("asyncIterator"),
        JsValue::String(String::from("__symbol__0_Symbol.asyncIterator")),
    );

    // Static methods
    symbol_ctor.set_property(String::from("for"), native_fn("for", symbol_for));
    symbol_ctor.set_property(String::from("keyFor"), native_fn("keyFor", symbol_key_for));
}

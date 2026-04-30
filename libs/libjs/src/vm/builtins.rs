//! Prototype initialization and global object setup.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;

use super::native_array;
use super::native_console;
use super::native_date;
use super::native_error;
use super::native_es2024;
use super::native_function;
use super::native_generator;
use super::native_globals;
use super::native_json;
use super::native_map;
use super::native_math;
use super::native_number;
use super::native_object;
use super::native_promise;
use super::native_proxy;
use super::native_regexp;
use super::native_string;
use super::native_symbol;
use super::native_timer;
use super::native_typed_array;
use super::native_weakref;
use super::iter;
use super::{native_ctor_fn, native_fn, native_fn_with_length, Vm};
use crate::value::*;

impl Vm {
    /// Populate all built-in prototypes with their methods.
    pub fn init_prototypes(&mut self) {
        self.log_engine("[libjs] initializing prototypes");

        // ── Object.prototype ──
        {
            let mut p = self.object_proto.borrow_mut();
            p.set_hidden(
                String::from("hasOwnProperty"),
                native_fn_with_length("hasOwnProperty", native_object::object_has_own_property, 1),
            );
            p.set_hidden(
                String::from("propertyIsEnumerable"),
                native_fn_with_length(
                    "propertyIsEnumerable",
                    native_object::object_property_is_enumerable,
                    1,
                ),
            );
            p.set_hidden(
                String::from("isPrototypeOf"),
                native_fn_with_length("isPrototypeOf", native_object::object_is_prototype_of, 1),
            );
            p.set_hidden(
                String::from("toString"),
                native_fn_with_length("toString", native_object::object_to_string, 0),
            );
            p.set_hidden(
                String::from("toLocaleString"),
                native_fn_with_length("toLocaleString", native_object::object_to_locale_string, 0),
            );
            p.set_hidden(
                String::from("valueOf"),
                native_fn_with_length("valueOf", native_object::object_value_of, 0),
            );
            p.set_hidden(
                String::from("keys"),
                native_fn("keys", native_object::object_keys_method),
            );
        }

        // ── Iterator.prototype ──
        iter::init_iterator_proto(self);

        // ── Array.prototype ──
        {
            let mut p = self.array_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            let values_fn = native_fn_with_length("values", native_array::array_values, 0);
            p.set_hidden(
                String::from("push"),
                native_fn_with_length("push", native_array::array_push, 1),
            );
            p.set_hidden(
                String::from("pop"),
                native_fn_with_length("pop", native_array::array_pop, 0),
            );
            p.set_hidden(
                String::from("shift"),
                native_fn_with_length("shift", native_array::array_shift, 0),
            );
            p.set_hidden(
                String::from("unshift"),
                native_fn_with_length("unshift", native_array::array_unshift, 1),
            );
            p.set_hidden(
                String::from("indexOf"),
                native_fn_with_length("indexOf", native_array::array_index_of, 1),
            );
            p.set_hidden(
                String::from("lastIndexOf"),
                native_fn_with_length("lastIndexOf", native_array::array_last_index_of, 1),
            );
            p.set_hidden(
                String::from("includes"),
                native_fn_with_length("includes", native_array::array_includes, 1),
            );
            p.set_hidden(
                String::from("join"),
                native_fn_with_length("join", native_array::array_join, 1),
            );
            p.set_hidden(
                String::from("slice"),
                native_fn_with_length("slice", native_array::array_slice, 2),
            );
            p.set_hidden(
                String::from("splice"),
                native_fn_with_length("splice", native_array::array_splice, 2),
            );
            p.set_hidden(
                String::from("concat"),
                native_fn_with_length("concat", native_array::array_concat, 1),
            );
            p.set_hidden(
                String::from("reverse"),
                native_fn_with_length("reverse", native_array::array_reverse, 0),
            );
            p.set_hidden(
                String::from("sort"),
                native_fn_with_length("sort", native_array::array_sort, 1),
            );
            p.set_hidden(
                String::from("map"),
                native_fn_with_length("map", native_array::array_map, 1),
            );
            p.set_hidden(
                String::from("filter"),
                native_fn_with_length("filter", native_array::array_filter, 1),
            );
            p.set_hidden(
                String::from("forEach"),
                native_fn_with_length("forEach", native_array::array_for_each, 1),
            );
            p.set_hidden(
                String::from("reduce"),
                native_fn_with_length("reduce", native_array::array_reduce, 1),
            );
            p.set_hidden(
                String::from("reduceRight"),
                native_fn_with_length("reduceRight", native_array::array_reduce_right, 1),
            );
            p.set_hidden(
                String::from("find"),
                native_fn_with_length("find", native_array::array_find, 1),
            );
            p.set_hidden(
                String::from("findIndex"),
                native_fn_with_length("findIndex", native_array::array_find_index, 1),
            );
            p.set_hidden(
                String::from("some"),
                native_fn_with_length("some", native_array::array_some, 1),
            );
            p.set_hidden(
                String::from("every"),
                native_fn_with_length("every", native_array::array_every, 1),
            );
            p.set_hidden(
                String::from("flat"),
                native_fn_with_length("flat", native_array::array_flat, 0),
            );
            p.set_hidden(
                String::from("flatMap"),
                native_fn_with_length("flatMap", native_array::array_flat_map, 1),
            );
            p.set_hidden(
                String::from("fill"),
                native_fn_with_length("fill", native_array::array_fill, 1),
            );
            p.set_hidden(
                String::from("copyWithin"),
                native_fn_with_length("copyWithin", native_array::array_copy_within, 2),
            );
            p.set_hidden(
                String::from("entries"),
                native_fn_with_length("entries", native_array::array_entries, 0),
            );
            p.set_hidden(
                String::from("keys"),
                native_fn_with_length("keys", native_array::array_keys, 0),
            );
            p.set_hidden(
                String::from("values"),
                values_fn.clone(),
            );
            p.set_hidden(
                String::from("at"),
                native_fn_with_length("at", native_array::array_at, 1),
            );
            p.set_hidden(
                String::from("toString"),
                native_fn_with_length("toString", native_array::array_to_string, 0),
            );
            // Symbol.iterator — returns an array iterator
            p.set_hidden(
                String::from(native_symbol::WELL_KNOWN_ITERATOR),
                values_fn,
            );
            let unscopables = JsValue::new_object();
            for key in [
                "at",
                "copyWithin",
                "entries",
                "fill",
                "find",
                "findIndex",
                "findLast",
                "findLastIndex",
                "flat",
                "flatMap",
                "includes",
                "keys",
                "toReversed",
                "toSorted",
                "toSpliced",
                "values",
            ] {
                unscopables.set_property(String::from(key), JsValue::Bool(true));
            }
            p.properties.insert(
                String::from(native_symbol::WELL_KNOWN_UNSCOPABLES),
                crate::value::Property {
                    value: unscopables,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    getter: None,
                    setter: None,
                },
            );
            // ES2023+
            p.set_hidden(
                String::from("findLast"),
                native_fn_with_length("findLast", native_es2024::array_find_last, 1),
            );
            p.set_hidden(
                String::from("findLastIndex"),
                native_fn_with_length("findLastIndex", native_es2024::array_find_last_index, 1),
            );
            p.set_hidden(
                String::from("toReversed"),
                native_fn_with_length("toReversed", native_es2024::array_to_reversed, 0),
            );
            p.set_hidden(
                String::from("toSorted"),
                native_fn_with_length("toSorted", native_es2024::array_to_sorted, 1),
            );
            p.set_hidden(
                String::from("toSpliced"),
                native_fn_with_length("toSpliced", native_es2024::array_to_spliced, 2),
            );
            p.set_hidden(
                String::from("with"),
                native_fn_with_length("with", native_es2024::array_with, 2),
            );
        }

        // ── String.prototype ──
        {
            let mut p = self.string_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.internal_tag = Some(String::from("__string__"));
            p.primitive_value = Some(Box::new(JsValue::String(String::new())));
            p.set_hidden(
                String::from("charAt"),
                native_fn_with_length("charAt", native_string::string_char_at, 1),
            );
            p.set_hidden(
                String::from("charCodeAt"),
                native_fn_with_length("charCodeAt", native_string::string_char_code_at, 1),
            );
            p.set_hidden(
                String::from("codePointAt"),
                native_fn_with_length("codePointAt", native_string::string_code_point_at, 1),
            );
            p.set_hidden(
                String::from("indexOf"),
                native_fn_with_length("indexOf", native_string::string_index_of, 1),
            );
            p.set_hidden(
                String::from("lastIndexOf"),
                native_fn_with_length("lastIndexOf", native_string::string_last_index_of, 1),
            );
            p.set_hidden(
                String::from("includes"),
                native_fn_with_length("includes", native_string::string_includes, 1),
            );
            p.set_hidden(
                String::from("startsWith"),
                native_fn_with_length("startsWith", native_string::string_starts_with, 1),
            );
            p.set_hidden(
                String::from("endsWith"),
                native_fn_with_length("endsWith", native_string::string_ends_with, 1),
            );
            p.set_hidden(
                String::from("slice"),
                native_fn_with_length("slice", native_string::string_slice, 2),
            );
            p.set_hidden(
                String::from("substring"),
                native_fn_with_length("substring", native_string::string_substring, 2),
            );
            p.set_hidden(
                String::from("substr"),
                native_fn_with_length("substr", native_string::string_substr, 2),
            );
            p.set_hidden(
                String::from("toLowerCase"),
                native_fn("toLowerCase", native_string::string_to_lower_case),
            );
            p.set_hidden(
                String::from("toUpperCase"),
                native_fn("toUpperCase", native_string::string_to_upper_case),
            );
            p.set_hidden(
                String::from("trim"),
                native_fn("trim", native_string::string_trim),
            );
            p.set_hidden(
                String::from("trimStart"),
                native_fn("trimStart", native_string::string_trim_start),
            );
            p.set_hidden(
                String::from("trimEnd"),
                native_fn("trimEnd", native_string::string_trim_end),
            );
            p.set_hidden(
                String::from("split"),
                native_fn_with_length("split", native_string::string_split, 2),
            );
            p.set_hidden(
                String::from("replace"),
                native_fn_with_length("replace", native_string::string_replace, 2),
            );
            p.set_hidden(
                String::from("replaceAll"),
                native_fn_with_length("replaceAll", native_string::string_replace_all, 2),
            );
            p.set_hidden(
                String::from("repeat"),
                native_fn_with_length("repeat", native_string::string_repeat, 1),
            );
            p.set_hidden(
                String::from("padStart"),
                native_fn_with_length("padStart", native_string::string_pad_start, 1),
            );
            p.set_hidden(
                String::from("padEnd"),
                native_fn_with_length("padEnd", native_string::string_pad_end, 1),
            );
            p.set_hidden(
                String::from("at"),
                native_fn_with_length("at", native_string::string_at, 1),
            );
            p.set_hidden(
                String::from("concat"),
                native_fn_with_length("concat", native_string::string_concat, 1),
            );
            p.set_hidden(
                String::from("toString"),
                native_fn_with_length("toString", native_string::string_to_string, 0),
            );
            p.set_hidden(
                String::from("valueOf"),
                native_fn_with_length("valueOf", native_string::string_to_string, 0),
            );
            p.set_hidden(
                String::from("normalize"),
                native_fn("normalize", native_string::string_normalize),
            );
            p.set_hidden(
                String::from("localeCompare"),
                native_fn_with_length("localeCompare", native_string::string_locale_compare, 1),
            );
            p.set_hidden(
                String::from("match"),
                native_fn_with_length("match", native_regexp::string_match, 1),
            );
            p.set_hidden(
                String::from("matchAll"),
                native_fn_with_length("matchAll", native_regexp::string_match_all, 1),
            );
            p.set_hidden(
                String::from("search"),
                native_fn_with_length("search", native_regexp::string_search, 1),
            );
            // toLocaleLowerCase / toLocaleUpperCase = same as toLowerCase/toUpperCase
            p.set_hidden(
                String::from("toLocaleLowerCase"),
                native_fn("toLocaleLowerCase", native_string::string_to_lower_case),
            );
            p.set_hidden(
                String::from("toLocaleUpperCase"),
                native_fn("toLocaleUpperCase", native_string::string_to_upper_case),
            );
            // Symbol.iterator — returns a string character iterator
            p.set_hidden(
                String::from(native_symbol::WELL_KNOWN_ITERATOR),
                native_fn_with_length("[Symbol.iterator]", string_symbol_iterator, 0),
            );
            // ES2024
            p.set_hidden(
                String::from("isWellFormed"),
                native_fn("isWellFormed", native_es2024::string_is_well_formed),
            );
            p.set_hidden(
                String::from("toWellFormed"),
                native_fn("toWellFormed", native_es2024::string_to_well_formed),
            );
        }

        // ── Number.prototype ──
        // Per spec: Number.prototype is itself a Number object with [[NumberData]] = +0.
        {
            let mut p = self.number_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.internal_tag = Some(String::from("__number__"));
            p.primitive_value = Some(Box::new(JsValue::Number(0.0)));
            p.set_hidden(
                String::from("toString"),
                native_fn_with_length("toString", native_number::number_to_string, 1),
            );
            p.set_hidden(
                String::from("valueOf"),
                native_fn_with_length("valueOf", native_number::number_value_of, 0),
            );
            p.set_hidden(
                String::from("toFixed"),
                native_fn_with_length("toFixed", native_number::number_to_fixed, 1),
            );
            p.set_hidden(
                String::from("toPrecision"),
                native_fn_with_length("toPrecision", native_number::number_to_precision, 1),
            );
            p.set_hidden(
                String::from("toExponential"),
                native_fn_with_length("toExponential", native_number::number_to_exponential, 1),
            );
            p.set_hidden(
                String::from("toLocaleString"),
                native_fn_with_length("toLocaleString", native_number::number_to_string, 0),
            );
        }

        // ── Boolean.prototype ──
        // Per the spec: Boolean.prototype is itself a Boolean object with [[BooleanData]] = false.
        {
            let mut p = self.boolean_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.internal_tag = Some(String::from("__boolean__"));
            p.primitive_value = Some(Box::new(JsValue::Bool(false)));
            p.set_hidden(String::from("__bool_data__"), JsValue::Bool(false));
            p.set_hidden(
                String::from("toString"),
                native_fn("toString", native_globals::boolean_to_string),
            );
            p.set_hidden(
                String::from("valueOf"),
                native_fn("valueOf", native_globals::boolean_value_of),
            );
        }

        // ── Function.prototype ──
        {
            let mut p = self.function_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.set_hidden(
                String::from("call"),
                native_fn_with_length("call", native_function::function_call, 1),
            );
            p.set_hidden(
                String::from("apply"),
                native_fn_with_length("apply", native_function::function_apply, 2),
            );
            p.set_hidden(
                String::from("bind"),
                native_fn_with_length("bind", native_function::function_bind, 1),
            );
            p.set_hidden(
                String::from("toString"),
                native_fn_with_length("toString", native_function::function_to_string, 0),
            );
            // ES2023 §10.2.4: "caller" and "arguments" are poison-pill accessor
            // properties on %Function.prototype% that throw TypeError.
            let thrower = native_fn("ThrowTypeError", |vm: &mut Vm, _args: &[JsValue]| {
                vm.pending_exception = Some(vm.make_type_error(
                    "'caller', 'arguments', and 'callee' are restricted function properties and cannot be accessed in this context",
                ));
                JsValue::Undefined
            });
            p.properties.insert(
                String::from("caller"),
                Property {
                    value: JsValue::Undefined,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    getter: Some(thrower.clone()),
                    setter: Some(thrower.clone()),
                },
            );
            p.properties.insert(
                String::from("arguments"),
                Property {
                    value: JsValue::Undefined,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    getter: Some(thrower.clone()),
                    setter: Some(thrower),
                },
            );
        }

        // ── Error.prototype ──
        {
            let mut p = self.error_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.set_hidden(String::from("name"), JsValue::String(String::from("Error")));
            p.set_hidden(String::from("message"), JsValue::String(String::new()));
            p.set_hidden(
                String::from("toString"),
                native_fn("toString", native_error::error_to_string),
            );
        }

        // ── RegExp.prototype ──
        {
            let mut p = self.regexp_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.set_hidden(
                String::from("test"),
                native_fn_with_length("test", native_regexp::regexp_test, 1),
            );
            p.set_hidden(
                String::from("exec"),
                native_fn_with_length("exec", native_regexp::regexp_exec, 1),
            );
            p.set_hidden(
                String::from("toString"),
                native_fn_with_length("toString", native_regexp::regexp_to_string, 0),
            );
        }

        // ── Generator.prototype ──
        {
            let mut p = self.generator_proto.borrow_mut();
            p.prototype = Some(self.iterator_proto.clone());
            p.set(
                String::from("next"),
                native_fn("next", native_generator::generator_next),
            );
            p.set(
                String::from("return"),
                native_fn("return", native_generator::generator_return),
            );
            p.set(
                String::from("throw"),
                native_fn("throw", native_generator::generator_throw),
            );
            // Generators are iterable: Symbol.iterator returns `this`
            p.set(
                String::from(native_symbol::WELL_KNOWN_ITERATOR),
                native_fn("[Symbol.iterator]", |vm: &mut Vm, _args: &[JsValue]| {
                    vm.current_this.clone()
                }),
            );
        }

        // ── TypedArray.prototype ──
        {
            let mut p = self.typed_array_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.set(
                String::from("set"),
                native_fn("set", native_typed_array::typed_array_set),
            );
            p.set(
                String::from("subarray"),
                native_fn("subarray", native_typed_array::typed_array_subarray),
            );
            p.set(
                String::from("slice"),
                native_fn("slice", native_typed_array::typed_array_slice),
            );
            p.set(
                String::from("fill"),
                native_fn("fill", native_typed_array::typed_array_fill),
            );
            p.set(
                String::from("indexOf"),
                native_fn("indexOf", native_typed_array::typed_array_index_of),
            );
            p.set(
                String::from("forEach"),
                native_fn("forEach", native_typed_array::typed_array_for_each),
            );
        }
    }

    /// Install global functions and objects (console, Math, JSON, etc.).
    pub fn init_globals(&mut self) {
        self.log_engine("[libjs] initializing globals");

        // ── Global functions ──
        self.set_global(
            "parseInt",
            native_fn("parseInt", native_globals::global_parse_int),
        );
        self.set_global(
            "parseFloat",
            native_fn("parseFloat", native_globals::global_parse_float),
        );
        self.set_global("isNaN", native_fn("isNaN", native_globals::global_is_nan));
        self.set_global(
            "isFinite",
            native_fn("isFinite", native_globals::global_is_finite),
        );
        self.set_global(
            "encodeURIComponent",
            native_fn(
                "encodeURIComponent",
                native_globals::global_encode_uri_component,
            ),
        );
        self.set_global(
            "decodeURIComponent",
            native_fn(
                "decodeURIComponent",
                native_globals::global_decode_uri_component,
            ),
        );
        self.set_global("eval", native_fn("eval", global_eval));

        // ── Constructors ──
        self.set_global("Object", native_ctor_fn("Object", native_globals::ctor_object));
        // Wire Object.prototype to the VM's object_proto so `instanceof` works.
        if let JsValue::Function(f) = self.globals.borrow().get("Object") {
            f.borrow_mut().prototype = Some(self.object_proto.clone());
        }
        self.set_global("Array", native_ctor_fn("Array", native_globals::ctor_array));
        // Wire Array.prototype to the VM's array_proto so `instanceof` works.
        if let JsValue::Function(f) = self.globals.borrow().get("Array") {
            f.borrow_mut().prototype = Some(self.array_proto.clone());
        }
        self.set_global("String", native_ctor_fn("String", native_globals::ctor_string));
        self.set_global("Number", native_ctor_fn("Number", native_globals::ctor_number));
        self.set_global(
            "Boolean",
            native_ctor_fn("Boolean", native_globals::ctor_boolean),
        );
        // `new Function([params], body)` — dynamically compiles JS source via
        // the parser and compiler (see native_globals::ctor_function).
        self.set_global(
            "Function",
            native_ctor_fn("Function", native_globals::ctor_function),
        );
        self.set_global("Error", native_ctor_fn("Error", native_error::ctor_error));
        self.set_global(
            "TypeError",
            native_ctor_fn("TypeError", native_error::ctor_type_error),
        );
        self.set_global(
            "RangeError",
            native_ctor_fn("RangeError", native_error::ctor_range_error),
        );
        self.set_global(
            "ReferenceError",
            native_ctor_fn("ReferenceError", native_error::ctor_reference_error),
        );
        self.set_global(
            "SyntaxError",
            native_ctor_fn("SyntaxError", native_error::ctor_syntax_error),
        );
        self.set_global(
            "URIError",
            native_ctor_fn("URIError", native_error::ctor_uri_error),
        );
        self.set_global(
            "EvalError",
            native_ctor_fn("EvalError", native_error::ctor_eval_error),
        );
        self.set_global(
            "AggregateError",
            native_ctor_fn("AggregateError", native_error::ctor_aggregate_error),
        );

        // ── console ──
        self.init_console();

        // ── Math ──
        self.init_math();

        // ── JSON ──
        self.init_json();

        // ── Object static methods ──
        self.init_object_statics();

        // ── Set constructor properties on prototypes (must happen AFTER globals) ──
        self.array_proto
            .borrow_mut()
            .set_hidden(String::from("constructor"), self.globals.borrow().get("Array"));
        self.string_proto
            .borrow_mut()
            .set_hidden(String::from("constructor"), self.globals.borrow().get("String"));
        self.number_proto
            .borrow_mut()
            .set_hidden(String::from("constructor"), self.globals.borrow().get("Number"));
        self.object_proto
            .borrow_mut()
            .set_hidden(String::from("constructor"), self.globals.borrow().get("Object"));
        self.boolean_proto
            .borrow_mut()
            .set_hidden(String::from("constructor"), self.globals.borrow().get("Boolean"));

        // ── Array static methods ──
        self.init_array_statics();

        // ── String static methods + prototype link ──
        if let JsValue::Function(f) = self.globals.borrow().get("String") {
            let ctor = JsValue::Function(f.clone());
            if let JsValue::Function(func_rc) = &ctor {
                func_rc.borrow_mut().own_props.insert(
                    String::from("prototype"),
                    JsValue::Object(self.string_proto.clone()),
                );
                func_rc.borrow_mut().own_props.insert(
                    String::from("__desc_writable_prototype"),
                    JsValue::Bool(false),
                );
                func_rc.borrow_mut().own_props.insert(
                    String::from("__desc_enumerable_prototype"),
                    JsValue::Bool(false),
                );
                func_rc.borrow_mut().own_props.insert(
                    String::from("__desc_configurable_prototype"),
                    JsValue::Bool(false),
                );
            }
            ctor.set_property(
                String::from("fromCharCode"),
                native_fn("fromCharCode", native_string::string_from_char_code),
            );
            ctor.set_property(
                String::from("fromCodePoint"),
                native_fn("fromCodePoint", native_string::string_from_code_point),
            );
            ctor.set_property(
                String::from("raw"),
                native_fn("raw", native_string::string_raw),
            );
        }

        // ── Number static methods ──
        self.init_number_statics();

        // ── Error prototype link ──
        self.init_error_statics();

        // ── Boolean prototype link ──
        self.init_boolean_statics();

        // ── Function prototype link ──
        self.init_function_statics();

        // ── Promise ──
        {
            let ctor = native_ctor_fn("Promise", native_promise::ctor_promise);
            let proto = JsValue::new_object();
            proto.set_property(
                String::from("then"),
                native_fn("then", native_promise::promise_then),
            );
            proto.set_property(
                String::from("catch"),
                native_fn("catch", native_promise::promise_catch),
            );
            proto.set_property(
                String::from("finally"),
                native_fn("finally", native_promise::promise_finally),
            );
            ctor.set_property(String::from("prototype"), proto);
            self.set_global("Promise", ctor);
        }
        self.init_promise_statics();

        // ── Map ──
        {
            let ctor = native_ctor_fn("Map", native_map::ctor_map);
            let proto = JsValue::new_object();
            proto.set_property(String::from("set"), native_fn("set", native_map::map_set));
            proto.set_property(String::from("get"), native_fn("get", native_map::map_get));
            proto.set_property(String::from("has"), native_fn("has", native_map::map_has));
            proto.set_property(
                String::from("delete"),
                native_fn("delete", native_map::map_delete),
            );
            proto.set_property(
                String::from("clear"),
                native_fn("clear", native_map::map_clear),
            );
            proto.set_property(
                String::from("keys"),
                native_fn("keys", native_map::map_keys),
            );
            proto.set_property(
                String::from("values"),
                native_fn("values", native_map::map_values),
            );
            proto.set_property(
                String::from("entries"),
                native_fn("entries", native_map::map_entries),
            );
            proto.set_property(
                String::from("forEach"),
                native_fn("forEach", native_map::map_for_each),
            );
            // Symbol.iterator → entries (ES2023 §24.1.3.12)
            proto.set_property(
                String::from(native_symbol::WELL_KNOWN_ITERATOR),
                native_fn("[Symbol.iterator]", native_map::map_entries),
            );
            if let JsValue::Object(proto_obj) = &proto {
                proto_obj.borrow_mut().properties.insert(
                    String::from("size"),
                    Property::accessor(Some(native_fn("get size", native_map::map_size_get)), None),
                );
            }
            ctor.set_property(String::from("prototype"), proto);
            self.set_global("Map", ctor);
        }
        // ── Set ──
        {
            let ctor = native_ctor_fn("Set", native_map::ctor_set);
            let proto = JsValue::new_object();
            proto.set_property(String::from("add"), native_fn("add", native_map::set_add));
            proto.set_property(String::from("has"), native_fn("has", native_map::set_has));
            proto.set_property(
                String::from("delete"),
                native_fn("delete", native_map::set_delete),
            );
            proto.set_property(
                String::from("clear"),
                native_fn("clear", native_map::set_clear),
            );
            proto.set_property(
                String::from("keys"),
                native_fn("keys", native_map::set_values),
            );
            proto.set_property(
                String::from("values"),
                native_fn("values", native_map::set_values),
            );
            proto.set_property(
                String::from("entries"),
                native_fn("entries", native_map::set_entries),
            );
            proto.set_property(
                String::from("forEach"),
                native_fn("forEach", native_map::set_for_each),
            );
            // Symbol.iterator → values (ES2023 §24.2.3.10)
            proto.set_property(
                String::from(native_symbol::WELL_KNOWN_ITERATOR),
                native_fn("[Symbol.iterator]", native_map::set_values),
            );
            // ES2025 Set methods
            proto.set_property(
                String::from("union"),
                native_fn("union", native_map::set_union),
            );
            proto.set_property(
                String::from("intersection"),
                native_fn("intersection", native_map::set_intersection),
            );
            proto.set_property(
                String::from("difference"),
                native_fn("difference", native_map::set_difference),
            );
            proto.set_property(
                String::from("symmetricDifference"),
                native_fn("symmetricDifference", native_map::set_symmetric_difference),
            );
            proto.set_property(
                String::from("isSubsetOf"),
                native_fn("isSubsetOf", native_map::set_is_subset_of),
            );
            proto.set_property(
                String::from("isSupersetOf"),
                native_fn("isSupersetOf", native_map::set_is_superset_of),
            );
            proto.set_property(
                String::from("isDisjointFrom"),
                native_fn("isDisjointFrom", native_map::set_is_disjoint_from),
            );
            if let JsValue::Object(proto_obj) = &proto {
                proto_obj.borrow_mut().properties.insert(
                    String::from("size"),
                    Property::accessor(Some(native_fn("get size", native_map::set_size_get)), None),
                );
            }
            ctor.set_property(String::from("prototype"), proto);
            self.set_global("Set", ctor);
        }

        // ── Date ──
        self.set_global("Date", native_ctor_fn("Date", native_date::ctor_date));
        self.init_date_statics();

        // ── Timers ──
        self.set_global(
            "setTimeout",
            native_fn("setTimeout", native_timer::set_timeout),
        );
        self.set_global(
            "setInterval",
            native_fn("setInterval", native_timer::set_interval),
        );
        self.set_global(
            "clearTimeout",
            native_fn("clearTimeout", native_timer::clear_timeout),
        );
        self.set_global(
            "clearInterval",
            native_fn("clearInterval", native_timer::clear_interval),
        );

        // ── Symbol ──
        let symbol_ctor = native_fn("Symbol", native_symbol::ctor_symbol);
        native_symbol::install_well_known_symbols(&symbol_ctor);
        self.set_global("Symbol", symbol_ctor);

        // ── Proxy ──
        let proxy_ctor = native_ctor_fn("Proxy", native_proxy::ctor_proxy);
        proxy_ctor.set_property(
            String::from("revocable"),
            native_fn("revocable", native_proxy::proxy_revocable),
        );
        self.set_global("Proxy", proxy_ctor);

        // ── Reflect ──
        native_proxy::install_reflect(self);

        // ── WeakMap ──
        {
            let wm = native_ctor_fn("WeakMap", native_weakref::ctor_weakmap);
            let proto = JsValue::new_object();
            proto.set_property(
                String::from("set"),
                native_fn("set", native_weakref::weakmap_set),
            );
            proto.set_property(
                String::from("get"),
                native_fn("get", native_weakref::weakmap_get),
            );
            proto.set_property(
                String::from("has"),
                native_fn("has", native_weakref::weakmap_has),
            );
            proto.set_property(
                String::from("delete"),
                native_fn("delete", native_weakref::weakmap_delete),
            );
            wm.set_property(String::from("prototype"), proto);
            self.set_global("WeakMap", wm);
        }
        // ── WeakSet ──
        {
            let ws = native_ctor_fn("WeakSet", native_weakref::ctor_weakset);
            let proto = JsValue::new_object();
            proto.set_property(
                String::from("add"),
                native_fn("add", native_weakref::weakset_add),
            );
            proto.set_property(
                String::from("has"),
                native_fn("has", native_weakref::weakset_has),
            );
            proto.set_property(
                String::from("delete"),
                native_fn("delete", native_weakref::weakset_delete),
            );
            ws.set_property(String::from("prototype"), proto);
            self.set_global("WeakSet", ws);
        }
        // ── WeakRef ──
        {
            let wr = native_ctor_fn("WeakRef", native_weakref::ctor_weakref);
            let proto = JsValue::new_object();
            proto.set_property(
                String::from("deref"),
                native_fn("deref", native_weakref::weakref_deref),
            );
            wr.set_property(String::from("prototype"), proto);
            self.set_global("WeakRef", wr);
        }
        // ── FinalizationRegistry ──
        {
            let fr = native_ctor_fn(
                "FinalizationRegistry",
                native_weakref::ctor_finalization_registry,
            );
            let proto = JsValue::new_object();
            proto.set_property(
                String::from("register"),
                native_fn("register", native_weakref::fr_register),
            );
            proto.set_property(
                String::from("unregister"),
                native_fn("unregister", native_weakref::fr_unregister),
            );
            fr.set_property(String::from("prototype"), proto);
            self.set_global("FinalizationRegistry", fr);
        }
        // ── structuredClone ──
        self.set_global(
            "structuredClone",
            native_fn("structuredClone", native_es2024::structured_clone),
        );

        // ── RegExp ──
        let regexp_ctor = native_ctor_fn("RegExp", native_regexp::regexp_constructor);
        regexp_ctor.set_property(
            String::from("prototype"),
            JsValue::Object(self.regexp_proto.clone()),
        );
        self.set_global("RegExp", regexp_ctor);

        // ── ArrayBuffer ──
        self.set_global(
            "ArrayBuffer",
            native_ctor_fn("ArrayBuffer", native_typed_array::ctor_arraybuffer),
        );

        // ── DataView ──
        self.set_global(
            "DataView",
            native_ctor_fn("DataView", native_typed_array::ctor_dataview),
        );

        // ── TypedArrays ──
        self.set_global(
            "Int8Array",
            native_ctor_fn("Int8Array", native_typed_array::ctor_int8array),
        );
        self.set_global(
            "Uint8Array",
            native_ctor_fn("Uint8Array", native_typed_array::ctor_uint8array),
        );
        self.set_global(
            "Uint8ClampedArray",
            native_ctor_fn(
                "Uint8ClampedArray",
                native_typed_array::ctor_uint8clampedarray,
            ),
        );
        self.set_global(
            "Int16Array",
            native_ctor_fn("Int16Array", native_typed_array::ctor_int16array),
        );
        self.set_global(
            "Uint16Array",
            native_ctor_fn("Uint16Array", native_typed_array::ctor_uint16array),
        );
        self.set_global(
            "Int32Array",
            native_ctor_fn("Int32Array", native_typed_array::ctor_int32array),
        );
        self.set_global(
            "Uint32Array",
            native_ctor_fn("Uint32Array", native_typed_array::ctor_uint32array),
        );
        self.set_global(
            "Float32Array",
            native_ctor_fn("Float32Array", native_typed_array::ctor_float32array),
        );
        self.set_global(
            "Float64Array",
            native_ctor_fn("Float64Array", native_typed_array::ctor_float64array),
        );

        // ── queueMicrotask ──
        self.set_global(
            "queueMicrotask",
            native_fn("queueMicrotask", queue_microtask_fn),
        );

        // ── ES Modules runtime ──
        self.set_global(
            "__import__",
            native_fn("__import__", module_import_fn),
        );
        self.set_global("__exports__", JsValue::new_object());

        // ── BigInt constructor ──
        self.set_global("BigInt", native_fn("BigInt", bigint_constructor));

        // ── Number constants ──
        self.set_global("Infinity", JsValue::Number(f64::INFINITY));
        self.set_global("NaN", JsValue::Number(f64::NAN));
        self.set_global("undefined", JsValue::Undefined);

        self.log_engine("[libjs] globals initialized OK");
    }

    fn init_console(&mut self) {
        let console = JsValue::new_object();
        console.set_property(
            String::from("log"),
            native_fn("log", native_console::console_log),
        );
        console.set_property(
            String::from("warn"),
            native_fn("warn", native_console::console_warn),
        );
        console.set_property(
            String::from("error"),
            native_fn("error", native_console::console_error),
        );
        console.set_property(
            String::from("info"),
            native_fn("info", native_console::console_log),
        );
        console.set_property(
            String::from("debug"),
            native_fn("debug", native_console::console_log),
        );
        console.set_property(
            String::from("dir"),
            native_fn("dir", native_console::console_log),
        );
        self.set_global("console", console);
    }

    fn init_math(&mut self) {
        let math_rc = Rc::new(RefCell::new(JsObject::new()));
        {
            let mut m = math_rc.borrow_mut();
            m.prototype = Some(self.object_proto.clone());
            m.internal_tag = Some(String::from("__math__"));
            m.set_hidden(String::from("PI"), JsValue::Number(core::f64::consts::PI));
            m.set_hidden(String::from("E"), JsValue::Number(core::f64::consts::E));
            m.set_hidden(
                String::from("LN2"),
                JsValue::Number(core::f64::consts::LN_2),
            );
            m.set_hidden(
                String::from("LN10"),
                JsValue::Number(core::f64::consts::LN_10),
            );
            m.set_hidden(
                String::from("LOG2E"),
                JsValue::Number(core::f64::consts::LOG2_E),
            );
            m.set_hidden(
                String::from("LOG10E"),
                JsValue::Number(core::f64::consts::LOG10_E),
            );
            m.set_hidden(
                String::from("SQRT2"),
                JsValue::Number(core::f64::consts::SQRT_2),
            );
            m.set_hidden(
                String::from("SQRT1_2"),
                JsValue::Number(core::f64::consts::FRAC_1_SQRT_2),
            );
        }
        {
            let mut m = math_rc.borrow_mut();
            m.set_hidden(
                String::from("abs"),
                native_fn_with_length("abs", native_math::math_abs, 1),
            );
            m.set_hidden(
                String::from("floor"),
                native_fn_with_length("floor", native_math::math_floor, 1),
            );
            m.set_hidden(
                String::from("ceil"),
                native_fn_with_length("ceil", native_math::math_ceil, 1),
            );
            m.set_hidden(
                String::from("round"),
                native_fn_with_length("round", native_math::math_round, 1),
            );
            m.set_hidden(
                String::from("trunc"),
                native_fn_with_length("trunc", native_math::math_trunc, 1),
            );
            m.set_hidden(
                String::from("max"),
                native_fn_with_length("max", native_math::math_max, 2),
            );
            m.set_hidden(
                String::from("min"),
                native_fn_with_length("min", native_math::math_min, 2),
            );
            m.set_hidden(
                String::from("pow"),
                native_fn_with_length("pow", native_math::math_pow, 2),
            );
            m.set_hidden(
                String::from("sqrt"),
                native_fn_with_length("sqrt", native_math::math_sqrt, 1),
            );
            m.set_hidden(
                String::from("cbrt"),
                native_fn_with_length("cbrt", native_math::math_cbrt, 1),
            );
            m.set_hidden(
                String::from("sign"),
                native_fn_with_length("sign", native_math::math_sign, 1),
            );
            m.set_hidden(
                String::from("log"),
                native_fn_with_length("log", native_math::math_log_fn, 1),
            );
            m.set_hidden(
                String::from("log2"),
                native_fn_with_length("log2", native_math::math_log2, 1),
            );
            m.set_hidden(
                String::from("log10"),
                native_fn_with_length("log10", native_math::math_log10, 1),
            );
            m.set_hidden(
                String::from("sin"),
                native_fn_with_length("sin", native_math::math_sin, 1),
            );
            m.set_hidden(
                String::from("cos"),
                native_fn_with_length("cos", native_math::math_cos, 1),
            );
            m.set_hidden(
                String::from("tan"),
                native_fn_with_length("tan", native_math::math_tan, 1),
            );
            m.set_hidden(
                String::from("atan2"),
                native_fn_with_length("atan2", native_math::math_atan2, 2),
            );
            m.set_hidden(
                String::from("hypot"),
                native_fn_with_length("hypot", native_math::math_hypot, 2),
            );
            m.set_hidden(
                String::from("clz32"),
                native_fn_with_length("clz32", native_math::math_clz32, 1),
            );
            m.set_hidden(
                String::from("fround"),
                native_fn_with_length("fround", native_math::math_fround, 1),
            );
            m.set_hidden(
                String::from("random"),
                native_fn_with_length("random", native_math::math_random, 0),
            );
            m.set_hidden(
                String::from("exp"),
                native_fn_with_length("exp", native_math::math_exp, 1),
            );
            m.set_hidden(
                String::from("expm1"),
                native_fn_with_length("expm1", native_math::math_expm1, 1),
            );
            m.set_hidden(
                String::from("log1p"),
                native_fn_with_length("log1p", native_math::math_log1p, 1),
            );
            m.set_hidden(
                String::from("asin"),
                native_fn_with_length("asin", native_math::math_asin, 1),
            );
            m.set_hidden(
                String::from("acos"),
                native_fn_with_length("acos", native_math::math_acos, 1),
            );
            m.set_hidden(
                String::from("atan"),
                native_fn_with_length("atan", native_math::math_atan, 1),
            );
            m.set_hidden(
                String::from("sinh"),
                native_fn_with_length("sinh", native_math::math_sinh, 1),
            );
            m.set_hidden(
                String::from("cosh"),
                native_fn_with_length("cosh", native_math::math_cosh, 1),
            );
            m.set_hidden(
                String::from("tanh"),
                native_fn_with_length("tanh", native_math::math_tanh, 1),
            );
            m.set_hidden(
                String::from("acosh"),
                native_fn_with_length("acosh", native_math::math_acosh, 1),
            );
            m.set_hidden(
                String::from("asinh"),
                native_fn_with_length("asinh", native_math::math_asinh, 1),
            );
            m.set_hidden(
                String::from("atanh"),
                native_fn_with_length("atanh", native_math::math_atanh, 1),
            );
            m.set_hidden(
                String::from("imul"),
                native_fn_with_length("imul", native_math::math_imul, 2),
            );
        }
        self.set_global("Math", JsValue::Object(math_rc));
    }

    fn init_json(&mut self) {
        let json_rc = Rc::new(RefCell::new(JsObject::new()));
        {
            let mut j = json_rc.borrow_mut();
            j.prototype = Some(self.object_proto.clone());
            j.internal_tag = Some(String::from("__json__"));
            j.set_hidden(
                String::from("parse"),
                native_fn_with_length("parse", native_json::json_parse, 2),
            );
            j.set_hidden(
                String::from("stringify"),
                native_fn_with_length("stringify", native_json::json_stringify, 3),
            );
        }
        self.set_global("JSON", JsValue::Object(json_rc));
    }

    fn init_object_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.borrow().get("Object") {
            let obj_ctor = JsValue::Function(f.clone());
            obj_ctor.set_property(
                String::from("keys"),
                native_fn("keys", native_object::object_keys),
            );
            obj_ctor.set_property(
                String::from("values"),
                native_fn("values", native_object::object_values),
            );
            obj_ctor.set_property(
                String::from("entries"),
                native_fn("entries", native_object::object_entries),
            );
            obj_ctor.set_property(
                String::from("assign"),
                native_fn("assign", native_object::object_assign),
            );
            obj_ctor.set_property(
                String::from("freeze"),
                native_fn("freeze", native_object::object_freeze),
            );
            obj_ctor.set_property(
                String::from("create"),
                native_fn("create", native_object::object_create),
            );
            obj_ctor.set_property(
                String::from("defineProperty"),
                native_fn("defineProperty", native_object::object_define_property),
            );
            obj_ctor.set_property(
                String::from("defineProperties"),
                native_fn("defineProperties", native_object::object_define_properties),
            );
            obj_ctor.set_property(
                String::from("getPrototypeOf"),
                native_fn("getPrototypeOf", native_object::object_get_prototype_of),
            );
            obj_ctor.set_property(
                String::from("setPrototypeOf"),
                native_fn("setPrototypeOf", native_object::object_set_prototype_of),
            );
            obj_ctor.set_property(
                String::from("fromEntries"),
                native_fn("fromEntries", native_object::object_from_entries),
            );
            obj_ctor.set_property(
                String::from("is"),
                native_fn("is", native_object::object_is),
            );
            obj_ctor.set_property(
                String::from("getOwnPropertyNames"),
                native_fn(
                    "getOwnPropertyNames",
                    native_object::object_get_own_property_names,
                ),
            );
            obj_ctor.set_property(
                String::from("getOwnPropertyDescriptor"),
                native_fn(
                    "getOwnPropertyDescriptor",
                    native_object::object_get_own_property_descriptor,
                ),
            );
            obj_ctor.set_property(
                String::from("getOwnPropertyDescriptors"),
                native_fn(
                    "getOwnPropertyDescriptors",
                    native_object::object_get_own_property_descriptors,
                ),
            );
            obj_ctor.set_property(
                String::from("getOwnPropertySymbols"),
                native_fn(
                    "getOwnPropertySymbols",
                    native_object::object_get_own_property_symbols,
                ),
            );
            obj_ctor.set_property(
                String::from("preventExtensions"),
                native_fn(
                    "preventExtensions",
                    native_object::object_prevent_extensions,
                ),
            );
            obj_ctor.set_property(
                String::from("isExtensible"),
                native_fn("isExtensible", native_object::object_is_extensible),
            );
            obj_ctor.set_property(
                String::from("seal"),
                native_fn("seal", native_object::object_seal),
            );
            obj_ctor.set_property(
                String::from("isSealed"),
                native_fn("isSealed", native_object::object_is_sealed),
            );
            obj_ctor.set_property(
                String::from("isFrozen"),
                native_fn("isFrozen", native_object::object_is_frozen),
            );
            // ES2022+
            obj_ctor.set_property(
                String::from("hasOwn"),
                native_fn("hasOwn", native_object::object_has_own),
            );
            obj_ctor.set_property(
                String::from("groupBy"),
                native_fn("groupBy", native_es2024::object_group_by),
            );
            // Expose object_proto as Object.prototype own_prop so that
            // `Object.hasOwnProperty("prototype")` is true and
            // `Object.prototype.isPrototypeOf(x)` resolves correctly.
            obj_ctor.set_property(
                String::from("prototype"),
                JsValue::Object(self.object_proto.clone()),
            );
        }
    }

    fn init_array_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.borrow().get("Array") {
            let arr_ctor = JsValue::Function(f.clone());
            arr_ctor.set_property(
                String::from("isArray"),
                native_fn("isArray", native_array::array_is_array),
            );
            arr_ctor.set_property(
                String::from("from"),
                native_fn("from", native_array::array_from),
            );
            arr_ctor.set_property(String::from("of"), native_fn("of", native_array::array_of));
            // Link Array.prototype so `Array.prototype.slice.call(...)` works.
            arr_ctor.set_property(
                String::from("prototype"),
                JsValue::Object(self.array_proto.clone()),
            );
        }
    }

    fn init_number_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.borrow().get("Number") {
            let num_ctor = JsValue::Function(f.clone());
            num_ctor.set_property(
                String::from("prototype"),
                JsValue::Object(self.number_proto.clone()),
            );
            num_ctor.set_property(
                String::from("isNaN"),
                native_fn("isNaN", native_globals::number_is_nan),
            );
            num_ctor.set_property(
                String::from("isFinite"),
                native_fn("isFinite", native_globals::number_is_finite),
            );
            num_ctor.set_property(
                String::from("isInteger"),
                native_fn("isInteger", native_globals::number_is_integer),
            );
            num_ctor.set_property(
                String::from("isSafeInteger"),
                native_fn("isSafeInteger", native_number::number_is_safe_integer),
            );
            num_ctor.set_property(
                String::from("parseFloat"),
                native_fn("parseFloat", native_number::number_parse_float),
            );
            num_ctor.set_property(
                String::from("parseInt"),
                native_fn("parseInt", native_number::number_parse_int),
            );
            num_ctor.set_property(
                String::from("MAX_SAFE_INTEGER"),
                JsValue::Number(9007199254740991.0),
            );
            num_ctor.set_property(
                String::from("MIN_SAFE_INTEGER"),
                JsValue::Number(-9007199254740991.0),
            );
            num_ctor.set_property(String::from("EPSILON"), JsValue::Number(f64::EPSILON));
            num_ctor.set_property(String::from("MAX_VALUE"), JsValue::Number(f64::MAX));
            num_ctor.set_property(
                String::from("MIN_VALUE"),
                JsValue::Number(f64::MIN_POSITIVE),
            );
            num_ctor.set_property(
                String::from("POSITIVE_INFINITY"),
                JsValue::Number(f64::INFINITY),
            );
            num_ctor.set_property(
                String::from("NEGATIVE_INFINITY"),
                JsValue::Number(f64::NEG_INFINITY),
            );
            num_ctor.set_property(String::from("NaN"), JsValue::Number(f64::NAN));
        }
    }

    fn init_error_statics(&mut self) {
        // Link Error.prototype so that `new Error()` gets error_proto as its prototype.
        for name in [
            "Error",
            "TypeError",
            "RangeError",
            "ReferenceError",
            "SyntaxError",
            "URIError",
            "EvalError",
            "AggregateError",
        ] {
            if let JsValue::Function(f) = self.globals.borrow().get(name) {
                let ctor = JsValue::Function(f.clone());
                ctor.set_property(
                    String::from("prototype"),
                    JsValue::Object(self.error_proto.clone()),
                );
            }
        }
    }

    fn init_promise_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.borrow().get("Promise") {
            let ctor = JsValue::Function(f.clone());
            ctor.set_property(
                String::from("resolve"),
                native_fn("resolve", native_promise::promise_resolve),
            );
            ctor.set_property(
                String::from("reject"),
                native_fn("reject", native_promise::promise_reject),
            );
            ctor.set_property(
                String::from("all"),
                native_fn("all", native_promise::promise_all),
            );
            ctor.set_property(
                String::from("allSettled"),
                native_fn("allSettled", native_promise::promise_all_settled),
            );
            ctor.set_property(
                String::from("race"),
                native_fn("race", native_promise::promise_race),
            );
            ctor.set_property(
                String::from("any"),
                native_fn("any", native_promise::promise_any),
            );
        }
    }

    fn init_date_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.borrow().get("Date") {
            let ctor = JsValue::Function(f.clone());
            let proto = Rc::new(RefCell::new(JsObject::new()));
            proto.borrow_mut().prototype = Some(self.object_proto.clone());
            proto.borrow_mut()
                .set(String::from("constructor"), ctor.clone());
            native_date::install_date_instance_methods(&mut proto.borrow_mut());
            ctor.set_property(String::from("prototype"), JsValue::Object(proto));
            ctor.set_property(String::from("now"), native_fn("now", native_date::date_now));
            ctor.set_property(
                String::from("parse"),
                native_fn("parse", native_date::date_parse),
            );
            ctor.set_property(
                String::from("UTC"),
                native_fn("UTC", native_date::date_utc),
            );
        }
    }

    /// Install `Boolean.prototype` as an own property on the Boolean constructor
    /// so that `Boolean.hasOwnProperty("prototype")` is `true`, and wire back
    /// `Boolean.prototype.constructor = Boolean`.  Also sets `Boolean.length = 1`.
    fn init_boolean_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.borrow().get("Boolean") {
            let ctor = JsValue::Function(f.clone());
            // Boolean.prototype → boolean_proto (own_prop so hasOwnProperty works)
            ctor.set_property(
                String::from("prototype"),
                JsValue::Object(self.boolean_proto.clone()),
            );
            // Boolean.length = 1 (accepts one parameter)
            ctor.set_property(String::from("length"), JsValue::Number(1.0));
            // Boolean.prototype.constructor → Boolean
            self.boolean_proto
                .borrow_mut()
                .set(String::from("constructor"), ctor);
        }
    }

    /// Install `Function.prototype` as an own property on the Function constructor.
    fn init_function_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.borrow().get("Function") {
            let ctor = JsValue::Function(f.clone());
            // Function.prototype → function_proto (own_prop for hasOwnProperty + isPrototypeOf)
            ctor.set_property(
                String::from("prototype"),
                JsValue::Object(self.function_proto.clone()),
            );
            // Function.prototype.constructor → Function
            self.function_proto
                .borrow_mut()
                .set(String::from("constructor"), ctor);
        }
    }
}

/// `queueMicrotask(callback)`
fn queue_microtask_fn(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(callback) = args.first() {
        if callback.is_function() {
            vm.enqueue_microtask(callback.clone(), alloc::vec::Vec::new());
        }
    }
    JsValue::Undefined
}

/// `eval(source)` — evaluate JavaScript source code in the current VM context.
fn global_eval(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let src = match args.first() {
        Some(v) => {
            // Non-string arguments are returned as-is per spec
            if let JsValue::String(s) = v {
                s.clone()
            } else {
                return v.clone();
            }
        }
        None => return JsValue::Undefined,
    };

    let tokens = crate::lexer::Lexer::tokenize(&src);
    let mut parser = crate::parser::Parser::new(tokens);
    let program = parser.parse_program();
    if !parser.errors.is_empty() {
        let err = vm.make_syntax_error(&parser.errors[0]);
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    let mut compiler = crate::compiler::Compiler::new();
    if vm.frames.last().map(|f| f.chunk.strict).unwrap_or(false) {
        compiler.is_strict = true;
    }
    let chunk = compiler.compile_eval(&program);

    // Run inline in the current VM
    let result = vm.call_value(
        &JsValue::Function(alloc::rc::Rc::new(core::cell::RefCell::new(
            crate::value::JsFunction {
                name: Some(String::from("eval")),
                params: alloc::vec::Vec::new(),
                kind: crate::value::FnKind::Bytecode(alloc::rc::Rc::new(chunk)),
                object_proto: None,
                this_binding: None,
                bound_args: alloc::vec::Vec::new(),
                upvalues: alloc::vec::Vec::new(),
                with_scopes: alloc::vec::Vec::new(),
                prototype: None,
                own_props: alloc::collections::BTreeMap::new(),
                arity: None,
                super_class: None,
            },
        ))),
        &[],
        JsValue::Undefined,
    );
    result
}

/// `__import__(specifier)` — ES Module loader.
///
/// Resolves a module specifier:
/// 1. If already in `module_registry` → return cached namespace object.
/// 2. If source text is in `module_sources` → compile and execute it,
///    cache the resulting `__exports__` object, and return it.
/// 3. Otherwise → throw an error.
/// `BigInt(value)` — converts a value to BigInt (ES2020 §21.2.1.1).
fn bigint_constructor(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let val = args.first().cloned().unwrap_or(JsValue::Undefined);
    match &val {
        JsValue::BigInt(_) => val,
        JsValue::Number(n) => {
            let n = *n;
            if n.is_nan() || n.is_infinite() || n != (n as i64 as f64) {
                let err =
                    vm.make_range_error("The number is not safe to convert to a BigInt");
                vm.throw_native(err);
                return JsValue::Undefined;
            }
            JsValue::BigInt(JsBigInt::from_i64(n as i64))
        }
        JsValue::String(s) => match JsBigInt::from_str(s) {
            Some(bi) => JsValue::BigInt(bi),
            None => {
                let err = vm.make_syntax_error("Cannot convert string to BigInt");
                vm.throw_native(err);
                JsValue::Undefined
            }
        },
        JsValue::Bool(true) => JsValue::BigInt(JsBigInt::from_i64(1)),
        JsValue::Bool(false) => JsValue::BigInt(JsBigInt::from_i64(0)),
        _ => {
            let err = vm.make_type_error("Cannot convert value to BigInt");
            vm.throw_native(err);
            JsValue::Undefined
        }
    }
}

fn module_import_fn(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let specifier = match args.first() {
        Some(JsValue::String(s)) => s.clone(),
        _ => {
            vm.throw_native(vm.make_type_error("import() requires a string specifier"));
            return JsValue::Undefined;
        }
    };

    // 1. Check cached registry.
    if let Some(ns) = vm.module_registry.get(&specifier) {
        return ns.clone();
    }

    // 2. Check source registry — compile, execute, cache.
    if let Some(source) = vm.module_sources.get(&specifier).cloned() {
        // Save current __exports__ and install a fresh one for this module.
        let prev_exports = vm.get_global("__exports__");
        let module_exports = JsValue::new_object();
        vm.set_global("__exports__", module_exports.clone());

        // Compile the module source.
        let tokens = crate::lexer::Lexer::tokenize(&source);
        let mut parser = crate::parser::Parser::new(tokens);
        let program = parser.parse_program();
        if !parser.errors.is_empty() {
            vm.set_global("__exports__", prev_exports);
            let err = vm.make_syntax_error(&parser.errors[0]);
            vm.throw_native(err);
            return JsValue::Undefined;
        }
        let mut compiler = crate::compiler::Compiler::new();
        let chunk = compiler.compile_eval(&program);

        // Execute the module body.
        let module_fn = JsValue::Function(alloc::rc::Rc::new(core::cell::RefCell::new(
            crate::value::JsFunction {
                name: Some(alloc::format!("<module:{}>", specifier)),
                params: alloc::vec::Vec::new(),
                kind: crate::value::FnKind::Bytecode(alloc::rc::Rc::new(chunk)),
                object_proto: None,
                this_binding: None,
                bound_args: alloc::vec::Vec::new(),
                upvalues: alloc::vec::Vec::new(),
                with_scopes: alloc::vec::Vec::new(),
                prototype: None,
                own_props: alloc::collections::BTreeMap::new(),
                arity: None,
                super_class: None,
            },
        )));
        vm.call_value(&module_fn, &[], JsValue::Undefined);

        // Collect exports and restore previous __exports__.
        let final_exports = vm.get_global("__exports__");
        vm.set_global("__exports__", prev_exports);

        // Also copy any global-scope declarations that `export let/const/function`
        // placed into the global scope.  ExportDecl::Decl compiles to global stores.
        // (Named exports via `export { a, b }` already wrote to __exports__.)

        // Cache the module namespace.
        vm.module_registry
            .insert(specifier, final_exports.clone());
        return final_exports;
    }

    // 3. Module not found.
    let msg = alloc::format!("Cannot find module '{}'", specifier);
    let err = vm.make_error(&msg);
    vm.throw_native(err);
    JsValue::Undefined
}

// ═══════════════════════════════════════════════════════
// Symbol.iterator implementations
// ═══════════════════════════════════════════════════════

/// `Array.prototype[Symbol.iterator]()` — returns an array iterator.
/// The iterator has a `.next()` method that yields `{ value, done }`.
fn array_symbol_iterator(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    // Collect elements from the array
    let items: alloc::vec::Vec<JsValue> = match &this {
        JsValue::Array(arr) => arr.borrow().to_dense_vec(),
        _ => alloc::vec::Vec::new(),
    };
    make_value_iterator(vm, items)
}

/// `String.prototype[Symbol.iterator]()` — returns a string character iterator.
fn string_symbol_iterator(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let s = match &this {
        JsValue::Null | JsValue::Undefined => {
            let err = vm.make_type_error("Cannot convert undefined or null to object");
            vm.throw_native(err);
            return JsValue::Undefined;
        }
        JsValue::Object(obj) => {
            if let Some(prim) = obj.borrow().primitive_value.as_deref() {
                prim.to_js_string()
            } else {
                let prim = vm.to_primitive_for_op(this.clone(), "string");
                if vm.pending_exception.is_some() {
                    return JsValue::Undefined;
                }
                prim.to_js_string()
            }
        }
        JsValue::Array(_) | JsValue::Function(_) => {
            let prim = vm.to_primitive_for_op(this.clone(), "string");
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            prim.to_js_string()
        }
        _ => this.to_js_string(),
    };
    let items: alloc::vec::Vec<JsValue> = s
        .chars()
        .map(|c| {
            let mut buf = alloc::string::String::new();
            buf.push(c);
            JsValue::String(buf)
        })
        .collect();
    make_value_iterator(vm, items)
}

/// Create a spec-compliant iterator object with a `.next()` method.
/// The iterator yields `{ value, done }` result objects.
fn make_value_iterator(vm: &Vm, items: alloc::vec::Vec<JsValue>) -> JsValue {
    let items_arr = JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(items))));
    let mut iter = JsObject::with_tag("__iterator__");
    iter.prototype = Some(vm.iterator_proto.clone());
    iter.set(alloc::string::String::from("__items__"), items_arr);
    iter.set(
        alloc::string::String::from("__index__"),
        JsValue::Number(0.0),
    );
    JsValue::Object(Rc::new(RefCell::new(iter)))
}

/// `Iterator.prototype.next()` — advances the iterator.
fn iterator_next(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    if let JsValue::Object(obj) = &this {
        let mut o = obj.borrow_mut();
        let index = o.get("__index__").to_number() as usize;
        let items = o.get("__items__");
        if let JsValue::Array(arr) = &items {
            let a = arr.borrow();
            if index < a.length {
                let val = a.get(index);
                o.properties.insert(
                    alloc::string::String::from("__index__"),
                    Property::data(JsValue::Number((index + 1) as f64)),
                );
                drop(o);
                // Return { value, done: false }
                let result = JsValue::new_object();
                result.set_property(alloc::string::String::from("value"), val);
                result.set_property(alloc::string::String::from("done"), JsValue::Bool(false));
                return result;
            }
        }
    }
    // Done
    let result = JsValue::new_object();
    result.set_property(alloc::string::String::from("value"), JsValue::Undefined);
    result.set_property(alloc::string::String::from("done"), JsValue::Bool(true));
    result
}

/// `iterator[Symbol.iterator]()` — returns itself.
fn iterator_self(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}

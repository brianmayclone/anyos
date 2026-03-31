//! Prototype initialization and global object setup.

use alloc::boxed::Box;
use alloc::string::String;

use crate::value::*;
use super::{Vm, native_fn};
use super::native_array;
use super::native_string;
use super::native_object;
use super::native_number;
use super::native_function;
use super::native_console;
use super::native_error;
use super::native_globals;
use super::native_math;
use super::native_json;
use super::native_promise;
use super::native_map;
use super::native_date;
use super::native_timer;
use super::native_symbol;
use super::native_proxy;
use super::native_regexp;
use super::native_generator;
use super::native_typed_array;
use super::native_weakref;
use super::native_es2024;

impl Vm {
    /// Populate all built-in prototypes with their methods.
    pub fn init_prototypes(&mut self) {
        self.log_engine("[libjs] initializing prototypes");

        // ── Object.prototype ──
        {
            let mut p = self.object_proto.borrow_mut();
            p.set(String::from("hasOwnProperty"), native_fn("hasOwnProperty", native_object::object_has_own_property));
            p.set(String::from("isPrototypeOf"), native_fn("isPrototypeOf", native_object::object_is_prototype_of));
            p.set(String::from("toString"), native_fn("toString", native_object::object_to_string));
            p.set(String::from("valueOf"), native_fn("valueOf", native_object::object_value_of));
            p.set(String::from("keys"), native_fn("keys", native_object::object_keys_method));
        }

        // ── Array.prototype ──
        {
            let mut p = self.array_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.set(String::from("push"), native_fn("push", native_array::array_push));
            p.set(String::from("pop"), native_fn("pop", native_array::array_pop));
            p.set(String::from("shift"), native_fn("shift", native_array::array_shift));
            p.set(String::from("unshift"), native_fn("unshift", native_array::array_unshift));
            p.set(String::from("indexOf"), native_fn("indexOf", native_array::array_index_of));
            p.set(String::from("lastIndexOf"), native_fn("lastIndexOf", native_array::array_last_index_of));
            p.set(String::from("includes"), native_fn("includes", native_array::array_includes));
            p.set(String::from("join"), native_fn("join", native_array::array_join));
            p.set(String::from("slice"), native_fn("slice", native_array::array_slice));
            p.set(String::from("splice"), native_fn("splice", native_array::array_splice));
            p.set(String::from("concat"), native_fn("concat", native_array::array_concat));
            p.set(String::from("reverse"), native_fn("reverse", native_array::array_reverse));
            p.set(String::from("sort"), native_fn("sort", native_array::array_sort));
            p.set(String::from("map"), native_fn("map", native_array::array_map));
            p.set(String::from("filter"), native_fn("filter", native_array::array_filter));
            p.set(String::from("forEach"), native_fn("forEach", native_array::array_for_each));
            p.set(String::from("reduce"), native_fn("reduce", native_array::array_reduce));
            p.set(String::from("reduceRight"), native_fn("reduceRight", native_array::array_reduce_right));
            p.set(String::from("find"), native_fn("find", native_array::array_find));
            p.set(String::from("findIndex"), native_fn("findIndex", native_array::array_find_index));
            p.set(String::from("some"), native_fn("some", native_array::array_some));
            p.set(String::from("every"), native_fn("every", native_array::array_every));
            p.set(String::from("flat"), native_fn("flat", native_array::array_flat));
            p.set(String::from("flatMap"), native_fn("flatMap", native_array::array_flat_map));
            p.set(String::from("fill"), native_fn("fill", native_array::array_fill));
            p.set(String::from("copyWithin"), native_fn("copyWithin", native_array::array_copy_within));
            p.set(String::from("entries"), native_fn("entries", native_array::array_entries));
            p.set(String::from("keys"), native_fn("keys", native_array::array_keys));
            p.set(String::from("values"), native_fn("values", native_array::array_values));
            p.set(String::from("at"), native_fn("at", native_array::array_at));
            p.set(String::from("toString"), native_fn("toString", native_array::array_to_string));
            // ES2023+
            p.set(String::from("findLast"), native_fn("findLast", native_es2024::array_find_last));
            p.set(String::from("findLastIndex"), native_fn("findLastIndex", native_es2024::array_find_last_index));
            p.set(String::from("toReversed"), native_fn("toReversed", native_es2024::array_to_reversed));
            p.set(String::from("toSorted"), native_fn("toSorted", native_es2024::array_to_sorted));
            p.set(String::from("toSpliced"), native_fn("toSpliced", native_es2024::array_to_spliced));
            p.set(String::from("with"), native_fn("with", native_es2024::array_with));
        }

        // ── String.prototype ──
        {
            let mut p = self.string_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.set(String::from("charAt"), native_fn("charAt", native_string::string_char_at));
            p.set(String::from("charCodeAt"), native_fn("charCodeAt", native_string::string_char_code_at));
            p.set(String::from("codePointAt"), native_fn("codePointAt", native_string::string_code_point_at));
            p.set(String::from("indexOf"), native_fn("indexOf", native_string::string_index_of));
            p.set(String::from("lastIndexOf"), native_fn("lastIndexOf", native_string::string_last_index_of));
            p.set(String::from("includes"), native_fn("includes", native_string::string_includes));
            p.set(String::from("startsWith"), native_fn("startsWith", native_string::string_starts_with));
            p.set(String::from("endsWith"), native_fn("endsWith", native_string::string_ends_with));
            p.set(String::from("slice"), native_fn("slice", native_string::string_slice));
            p.set(String::from("substring"), native_fn("substring", native_string::string_substring));
            p.set(String::from("toLowerCase"), native_fn("toLowerCase", native_string::string_to_lower_case));
            p.set(String::from("toUpperCase"), native_fn("toUpperCase", native_string::string_to_upper_case));
            p.set(String::from("trim"), native_fn("trim", native_string::string_trim));
            p.set(String::from("trimStart"), native_fn("trimStart", native_string::string_trim_start));
            p.set(String::from("trimEnd"), native_fn("trimEnd", native_string::string_trim_end));
            p.set(String::from("split"), native_fn("split", native_string::string_split));
            p.set(String::from("replace"), native_fn("replace", native_string::string_replace));
            p.set(String::from("replaceAll"), native_fn("replaceAll", native_string::string_replace_all));
            p.set(String::from("repeat"), native_fn("repeat", native_string::string_repeat));
            p.set(String::from("padStart"), native_fn("padStart", native_string::string_pad_start));
            p.set(String::from("padEnd"), native_fn("padEnd", native_string::string_pad_end));
            p.set(String::from("at"), native_fn("at", native_string::string_at));
            p.set(String::from("concat"), native_fn("concat", native_string::string_concat));
            p.set(String::from("toString"), native_fn("toString", native_string::string_to_string));
            p.set(String::from("valueOf"), native_fn("valueOf", native_string::string_to_string));
            p.set(String::from("match"), native_fn("match", native_regexp::string_match));
            p.set(String::from("matchAll"), native_fn("matchAll", native_regexp::string_match_all));
            p.set(String::from("search"), native_fn("search", native_regexp::string_search));
            // ES2024
            p.set(String::from("isWellFormed"), native_fn("isWellFormed", native_es2024::string_is_well_formed));
            p.set(String::from("toWellFormed"), native_fn("toWellFormed", native_es2024::string_to_well_formed));
        }

        // ── Number.prototype ──
        {
            let mut p = self.number_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.set(String::from("toString"), native_fn("toString", native_number::number_to_string));
            p.set(String::from("valueOf"), native_fn("valueOf", native_number::number_value_of));
            p.set(String::from("toFixed"), native_fn("toFixed", native_number::number_to_fixed));
        }

        // ── Boolean.prototype ──
        // Per the spec: Boolean.prototype is itself a Boolean object with [[BooleanData]] = false.
        {
            let mut p = self.boolean_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.internal_tag = Some(String::from("__boolean__"));
            p.primitive_value = Some(Box::new(JsValue::Bool(false)));
            p.set(String::from("__bool_data__"), JsValue::Bool(false));
            p.set(String::from("toString"), native_fn("toString", native_globals::boolean_to_string));
            p.set(String::from("valueOf"), native_fn("valueOf", native_globals::boolean_value_of));
        }

        // ── Function.prototype ──
        {
            let mut p = self.function_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.set(String::from("call"), native_fn("call", native_function::function_call));
            p.set(String::from("apply"), native_fn("apply", native_function::function_apply));
            p.set(String::from("bind"), native_fn("bind", native_function::function_bind));
            p.set(String::from("toString"), native_fn("toString", native_function::function_to_string));
        }

        // ── Error.prototype ──
        {
            let mut p = self.error_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.set(String::from("name"), JsValue::String(String::from("Error")));
            p.set(String::from("message"), JsValue::String(String::new()));
            p.set(String::from("toString"), native_fn("toString", native_error::error_to_string));
        }

        // ── RegExp.prototype ──
        {
            let mut p = self.regexp_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.set(String::from("test"), native_fn("test", native_regexp::regexp_test));
            p.set(String::from("exec"), native_fn("exec", native_regexp::regexp_exec));
            p.set(String::from("toString"), native_fn("toString", native_regexp::regexp_to_string));
        }

        // ── Generator.prototype ──
        {
            let mut p = self.generator_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.set(String::from("next"), native_fn("next", native_generator::generator_next));
            p.set(String::from("return"), native_fn("return", native_generator::generator_return));
            p.set(String::from("throw"), native_fn("throw", native_generator::generator_throw));
        }

        // ── TypedArray.prototype ──
        {
            let mut p = self.typed_array_proto.borrow_mut();
            p.prototype = Some(self.object_proto.clone());
            p.set(String::from("set"), native_fn("set", native_typed_array::typed_array_set));
            p.set(String::from("subarray"), native_fn("subarray", native_typed_array::typed_array_subarray));
            p.set(String::from("slice"), native_fn("slice", native_typed_array::typed_array_slice));
            p.set(String::from("fill"), native_fn("fill", native_typed_array::typed_array_fill));
            p.set(String::from("indexOf"), native_fn("indexOf", native_typed_array::typed_array_index_of));
            p.set(String::from("forEach"), native_fn("forEach", native_typed_array::typed_array_for_each));
        }
    }

    /// Install global functions and objects (console, Math, JSON, etc.).
    pub fn init_globals(&mut self) {
        self.log_engine("[libjs] initializing globals");

        // ── Global functions ──
        self.set_global("parseInt", native_fn("parseInt", native_globals::global_parse_int));
        self.set_global("parseFloat", native_fn("parseFloat", native_globals::global_parse_float));
        self.set_global("isNaN", native_fn("isNaN", native_globals::global_is_nan));
        self.set_global("isFinite", native_fn("isFinite", native_globals::global_is_finite));
        self.set_global("encodeURIComponent", native_fn("encodeURIComponent", native_globals::global_encode_uri_component));
        self.set_global("decodeURIComponent", native_fn("decodeURIComponent", native_globals::global_decode_uri_component));

        // ── Constructors ──
        self.set_global("Object", native_fn("Object", native_globals::ctor_object));
        self.set_global("Array", native_fn("Array", native_globals::ctor_array));
        self.set_global("String", native_fn("String", native_globals::ctor_string));
        self.set_global("Number", native_fn("Number", native_globals::ctor_number));
        self.set_global("Boolean", native_fn("Boolean", native_globals::ctor_boolean));
        // Function constructor stub — creates an empty no-op function. Full source
        // evaluation is not implemented; this satisfies `new Function()` being callable
        // and truthy, and makes Function.prototype.isPrototypeOf(Boolean) work.
        self.set_global("Function", native_fn("Function", native_globals::ctor_function));
        self.set_global("Error", native_fn("Error", native_error::ctor_error));
        self.set_global("TypeError", native_fn("TypeError", native_error::ctor_error));
        self.set_global("RangeError", native_fn("RangeError", native_error::ctor_error));
        self.set_global("ReferenceError", native_fn("ReferenceError", native_error::ctor_error));
        self.set_global("SyntaxError", native_fn("SyntaxError", native_error::ctor_error));

        // ── console ──
        self.init_console();

        // ── Math ──
        self.init_math();

        // ── JSON ──
        self.init_json();

        // ── Object static methods ──
        self.init_object_statics();

        // ── Array static methods ──
        self.init_array_statics();

        // ── Number static methods ──
        self.init_number_statics();

        // ── Error prototype link ──
        self.init_error_statics();

        // ── Boolean prototype link ──
        self.init_boolean_statics();

        // ── Function prototype link ──
        self.init_function_statics();

        // ── Promise ──
        self.set_global("Promise", native_fn("Promise", native_promise::ctor_promise));
        self.init_promise_statics();

        // ── Map & Set ──
        self.set_global("Map", native_fn("Map", native_map::ctor_map));
        self.set_global("Set", native_fn("Set", native_map::ctor_set));

        // ── Date ──
        self.set_global("Date", native_fn("Date", native_date::ctor_date));
        self.init_date_statics();

        // ── Timers ──
        self.set_global("setTimeout", native_fn("setTimeout", native_timer::set_timeout));
        self.set_global("setInterval", native_fn("setInterval", native_timer::set_interval));
        self.set_global("clearTimeout", native_fn("clearTimeout", native_timer::clear_timeout));
        self.set_global("clearInterval", native_fn("clearInterval", native_timer::clear_interval));

        // ── Symbol ──
        let symbol_ctor = native_fn("Symbol", native_symbol::ctor_symbol);
        native_symbol::install_well_known_symbols(&symbol_ctor);
        self.set_global("Symbol", symbol_ctor);

        // ── Proxy ──
        let proxy_ctor = native_fn("Proxy", native_proxy::ctor_proxy);
        proxy_ctor.set_property(String::from("revocable"), native_fn("revocable", native_proxy::proxy_revocable));
        self.set_global("Proxy", proxy_ctor);

        // ── Reflect ──
        native_proxy::install_reflect(self);

        // ── WeakMap ──
        {
            let wm = native_fn("WeakMap", native_weakref::ctor_weakmap);
            let proto = JsValue::new_object();
            proto.set_property(String::from("set"), native_fn("set", native_weakref::weakmap_set));
            proto.set_property(String::from("get"), native_fn("get", native_weakref::weakmap_get));
            proto.set_property(String::from("has"), native_fn("has", native_weakref::weakmap_has));
            proto.set_property(String::from("delete"), native_fn("delete", native_weakref::weakmap_delete));
            wm.set_property(String::from("prototype"), proto);
            self.set_global("WeakMap", wm);
        }
        // ── WeakSet ──
        {
            let ws = native_fn("WeakSet", native_weakref::ctor_weakset);
            let proto = JsValue::new_object();
            proto.set_property(String::from("add"), native_fn("add", native_weakref::weakset_add));
            proto.set_property(String::from("has"), native_fn("has", native_weakref::weakset_has));
            proto.set_property(String::from("delete"), native_fn("delete", native_weakref::weakset_delete));
            ws.set_property(String::from("prototype"), proto);
            self.set_global("WeakSet", ws);
        }
        // ── WeakRef ──
        {
            let wr = native_fn("WeakRef", native_weakref::ctor_weakref);
            let proto = JsValue::new_object();
            proto.set_property(String::from("deref"), native_fn("deref", native_weakref::weakref_deref));
            wr.set_property(String::from("prototype"), proto);
            self.set_global("WeakRef", wr);
        }
        // ── FinalizationRegistry ──
        {
            let fr = native_fn("FinalizationRegistry", native_weakref::ctor_finalization_registry);
            let proto = JsValue::new_object();
            proto.set_property(String::from("register"), native_fn("register", native_weakref::fr_register));
            proto.set_property(String::from("unregister"), native_fn("unregister", native_weakref::fr_unregister));
            fr.set_property(String::from("prototype"), proto);
            self.set_global("FinalizationRegistry", fr);
        }
        // ── structuredClone ──
        self.set_global("structuredClone", native_fn("structuredClone", native_es2024::structured_clone));

        // ── RegExp ──
        let regexp_ctor = native_fn("RegExp", native_regexp::regexp_constructor);
        regexp_ctor.set_property(String::from("prototype"), JsValue::Object(self.regexp_proto.clone()));
        self.set_global("RegExp", regexp_ctor);

        // ── ArrayBuffer ──
        self.set_global("ArrayBuffer", native_fn("ArrayBuffer", native_typed_array::ctor_arraybuffer));

        // ── DataView ──
        self.set_global("DataView", native_fn("DataView", native_typed_array::ctor_dataview));

        // ── TypedArrays ──
        self.set_global("Int8Array", native_fn("Int8Array", native_typed_array::ctor_int8array));
        self.set_global("Uint8Array", native_fn("Uint8Array", native_typed_array::ctor_uint8array));
        self.set_global("Uint8ClampedArray", native_fn("Uint8ClampedArray", native_typed_array::ctor_uint8clampedarray));
        self.set_global("Int16Array", native_fn("Int16Array", native_typed_array::ctor_int16array));
        self.set_global("Uint16Array", native_fn("Uint16Array", native_typed_array::ctor_uint16array));
        self.set_global("Int32Array", native_fn("Int32Array", native_typed_array::ctor_int32array));
        self.set_global("Uint32Array", native_fn("Uint32Array", native_typed_array::ctor_uint32array));
        self.set_global("Float32Array", native_fn("Float32Array", native_typed_array::ctor_float32array));
        self.set_global("Float64Array", native_fn("Float64Array", native_typed_array::ctor_float64array));

        // ── queueMicrotask ──
        self.set_global("queueMicrotask", native_fn("queueMicrotask", queue_microtask_fn));

        // ── Number constants ──
        self.set_global("Infinity", JsValue::Number(f64::INFINITY));
        self.set_global("NaN", JsValue::Number(f64::NAN));
        self.set_global("undefined", JsValue::Undefined);

        self.log_engine("[libjs] globals initialized OK");
    }

    fn init_console(&mut self) {
        let console = JsValue::new_object();
        console.set_property(String::from("log"), native_fn("log", native_console::console_log));
        console.set_property(String::from("warn"), native_fn("warn", native_console::console_warn));
        console.set_property(String::from("error"), native_fn("error", native_console::console_error));
        console.set_property(String::from("info"), native_fn("info", native_console::console_log));
        console.set_property(String::from("debug"), native_fn("debug", native_console::console_log));
        self.set_global("console", console);
    }

    fn init_math(&mut self) {
        let math = JsValue::new_object();
        // Math.[[Prototype]] = Object.prototype (per spec)
        if let JsValue::Object(obj) = &math {
            obj.borrow_mut().prototype = Some(self.object_proto.clone());
        }
        math.set_property(String::from("PI"), JsValue::Number(core::f64::consts::PI));
        math.set_property(String::from("E"), JsValue::Number(core::f64::consts::E));
        math.set_property(String::from("LN2"), JsValue::Number(core::f64::consts::LN_2));
        math.set_property(String::from("LN10"), JsValue::Number(core::f64::consts::LN_10));
        math.set_property(String::from("LOG2E"), JsValue::Number(core::f64::consts::LOG2_E));
        math.set_property(String::from("LOG10E"), JsValue::Number(core::f64::consts::LOG10_E));
        math.set_property(String::from("SQRT2"), JsValue::Number(core::f64::consts::SQRT_2));
        math.set_property(String::from("SQRT1_2"), JsValue::Number(core::f64::consts::FRAC_1_SQRT_2));
        math.set_property(String::from("abs"), native_fn("abs", native_math::math_abs));
        math.set_property(String::from("floor"), native_fn("floor", native_math::math_floor));
        math.set_property(String::from("ceil"), native_fn("ceil", native_math::math_ceil));
        math.set_property(String::from("round"), native_fn("round", native_math::math_round));
        math.set_property(String::from("trunc"), native_fn("trunc", native_math::math_trunc));
        // Math.max.length = 2, Math.min.length = 2 per spec
        let max_fn = native_fn("max", native_math::math_max);
        max_fn.set_property(String::from("length"), JsValue::Number(2.0));
        math.set_property(String::from("max"), max_fn);
        let min_fn = native_fn("min", native_math::math_min);
        min_fn.set_property(String::from("length"), JsValue::Number(2.0));
        math.set_property(String::from("min"), min_fn);
        math.set_property(String::from("pow"), native_fn("pow", native_math::math_pow));
        math.set_property(String::from("sqrt"), native_fn("sqrt", native_math::math_sqrt));
        math.set_property(String::from("cbrt"), native_fn("cbrt", native_math::math_cbrt));
        math.set_property(String::from("sign"), native_fn("sign", native_math::math_sign));
        math.set_property(String::from("log"), native_fn("log", native_math::math_log_fn));
        math.set_property(String::from("log2"), native_fn("log2", native_math::math_log2));
        math.set_property(String::from("log10"), native_fn("log10", native_math::math_log10));
        math.set_property(String::from("sin"), native_fn("sin", native_math::math_sin));
        math.set_property(String::from("cos"), native_fn("cos", native_math::math_cos));
        math.set_property(String::from("tan"), native_fn("tan", native_math::math_tan));
        math.set_property(String::from("atan2"), native_fn("atan2", native_math::math_atan2));
        math.set_property(String::from("hypot"), native_fn("hypot", native_math::math_hypot));
        math.set_property(String::from("clz32"), native_fn("clz32", native_math::math_clz32));
        math.set_property(String::from("fround"), native_fn("fround", native_math::math_fround));
        math.set_property(String::from("random"), native_fn("random", native_math::math_random));
        math.set_property(String::from("exp"),    native_fn("exp",    native_math::math_exp));
        math.set_property(String::from("expm1"),  native_fn("expm1",  native_math::math_expm1));
        math.set_property(String::from("log1p"),  native_fn("log1p",  native_math::math_log1p));
        math.set_property(String::from("asin"),   native_fn("asin",   native_math::math_asin));
        math.set_property(String::from("acos"),   native_fn("acos",   native_math::math_acos));
        math.set_property(String::from("atan"),   native_fn("atan",   native_math::math_atan));
        math.set_property(String::from("sinh"),   native_fn("sinh",   native_math::math_sinh));
        math.set_property(String::from("cosh"),   native_fn("cosh",   native_math::math_cosh));
        math.set_property(String::from("tanh"),   native_fn("tanh",   native_math::math_tanh));
        math.set_property(String::from("acosh"),  native_fn("acosh",  native_math::math_acosh));
        math.set_property(String::from("asinh"),  native_fn("asinh",  native_math::math_asinh));
        math.set_property(String::from("atanh"),  native_fn("atanh",  native_math::math_atanh));
        math.set_property(String::from("imul"),   native_fn("imul",   native_math::math_imul));
        self.set_global("Math", math);
    }

    fn init_json(&mut self) {
        let json = JsValue::new_object();
        json.set_property(String::from("parse"), native_fn("parse", native_json::json_parse));
        json.set_property(String::from("stringify"), native_fn("stringify", native_json::json_stringify));
        self.set_global("JSON", json);
    }

    fn init_object_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.get("Object") {
            let obj_ctor = JsValue::Function(f.clone());
            obj_ctor.set_property(String::from("keys"), native_fn("keys", native_object::object_keys));
            obj_ctor.set_property(String::from("values"), native_fn("values", native_object::object_values));
            obj_ctor.set_property(String::from("entries"), native_fn("entries", native_object::object_entries));
            obj_ctor.set_property(String::from("assign"), native_fn("assign", native_object::object_assign));
            obj_ctor.set_property(String::from("freeze"), native_fn("freeze", native_object::object_freeze));
            obj_ctor.set_property(String::from("create"), native_fn("create", native_object::object_create));
            obj_ctor.set_property(String::from("defineProperty"), native_fn("defineProperty", native_object::object_define_property));
            obj_ctor.set_property(String::from("getPrototypeOf"), native_fn("getPrototypeOf", native_object::object_get_prototype_of));
            // ES2022+
            obj_ctor.set_property(String::from("hasOwn"), native_fn("hasOwn", native_es2024::object_has_own));
            obj_ctor.set_property(String::from("groupBy"), native_fn("groupBy", native_es2024::object_group_by));
            // Expose object_proto as Object.prototype own_prop so that
            // `Object.hasOwnProperty("prototype")` is true and
            // `Object.prototype.isPrototypeOf(x)` resolves correctly.
            obj_ctor.set_property(String::from("prototype"), JsValue::Object(self.object_proto.clone()));
        }
    }

    fn init_array_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.get("Array") {
            let arr_ctor = JsValue::Function(f.clone());
            arr_ctor.set_property(String::from("isArray"), native_fn("isArray", native_array::array_is_array));
            arr_ctor.set_property(String::from("from"), native_fn("from", native_array::array_from));
            arr_ctor.set_property(String::from("of"), native_fn("of", native_array::array_of));
            // Link Array.prototype so `Array.prototype.slice.call(...)` works.
            arr_ctor.set_property(String::from("prototype"), JsValue::Object(self.array_proto.clone()));
        }
    }

    fn init_number_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.get("Number") {
            let num_ctor = JsValue::Function(f.clone());
            num_ctor.set_property(String::from("isNaN"), native_fn("isNaN", native_globals::number_is_nan));
            num_ctor.set_property(String::from("isFinite"), native_fn("isFinite", native_globals::number_is_finite));
            num_ctor.set_property(String::from("isInteger"), native_fn("isInteger", native_globals::number_is_integer));
            num_ctor.set_property(String::from("MAX_SAFE_INTEGER"), JsValue::Number(9007199254740991.0));
            num_ctor.set_property(String::from("MIN_SAFE_INTEGER"), JsValue::Number(-9007199254740991.0));
            num_ctor.set_property(String::from("EPSILON"), JsValue::Number(f64::EPSILON));
            num_ctor.set_property(String::from("MAX_VALUE"), JsValue::Number(f64::MAX));
            num_ctor.set_property(String::from("MIN_VALUE"), JsValue::Number(f64::MIN_POSITIVE));
            num_ctor.set_property(String::from("POSITIVE_INFINITY"), JsValue::Number(f64::INFINITY));
            num_ctor.set_property(String::from("NEGATIVE_INFINITY"), JsValue::Number(f64::NEG_INFINITY));
            num_ctor.set_property(String::from("NaN"), JsValue::Number(f64::NAN));
        }
    }

    fn init_error_statics(&mut self) {
        // Link Error.prototype so that `new Error()` gets error_proto as its prototype.
        for name in ["Error", "TypeError", "RangeError", "ReferenceError", "SyntaxError"] {
            if let JsValue::Function(f) = self.globals.get(name) {
                let ctor = JsValue::Function(f.clone());
                ctor.set_property(String::from("prototype"), JsValue::Object(self.error_proto.clone()));
            }
        }
    }

    fn init_promise_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.get("Promise") {
            let ctor = JsValue::Function(f.clone());
            ctor.set_property(String::from("resolve"), native_fn("resolve", native_promise::promise_resolve));
            ctor.set_property(String::from("reject"), native_fn("reject", native_promise::promise_reject));
            ctor.set_property(String::from("all"), native_fn("all", native_promise::promise_all));
            ctor.set_property(String::from("allSettled"), native_fn("allSettled", native_promise::promise_all_settled));
            ctor.set_property(String::from("race"), native_fn("race", native_promise::promise_race));
        }
    }

    fn init_date_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.get("Date") {
            let ctor = JsValue::Function(f.clone());
            ctor.set_property(String::from("now"), native_fn("now", native_date::date_now));
            ctor.set_property(String::from("parse"), native_fn("parse", native_date::date_parse));
        }
    }

    /// Install `Boolean.prototype` as an own property on the Boolean constructor
    /// so that `Boolean.hasOwnProperty("prototype")` is `true`, and wire back
    /// `Boolean.prototype.constructor = Boolean`.  Also sets `Boolean.length = 1`.
    fn init_boolean_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.get("Boolean") {
            let ctor = JsValue::Function(f.clone());
            // Boolean.prototype → boolean_proto (own_prop so hasOwnProperty works)
            ctor.set_property(String::from("prototype"), JsValue::Object(self.boolean_proto.clone()));
            // Boolean.length = 1 (accepts one parameter)
            ctor.set_property(String::from("length"), JsValue::Number(1.0));
            // Boolean.prototype.constructor → Boolean
            self.boolean_proto.borrow_mut().set(String::from("constructor"), ctor);
        }
    }

    /// Install `Function.prototype` as an own property on the Function constructor.
    fn init_function_statics(&mut self) {
        if let JsValue::Function(f) = self.globals.get("Function") {
            let ctor = JsValue::Function(f.clone());
            // Function.prototype → function_proto (own_prop for hasOwnProperty + isPrototypeOf)
            ctor.set_property(String::from("prototype"), JsValue::Object(self.function_proto.clone()));
            // Function.prototype.constructor → Function
            self.function_proto.borrow_mut().set(String::from("constructor"), ctor);
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

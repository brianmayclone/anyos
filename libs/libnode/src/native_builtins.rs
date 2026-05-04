use libjs::JsEngine;

use crate::modules;
use crate::options::NativeModulePolicy;

struct NativeBuiltin {
    aliases: &'static [&'static str],
    factory: fn(&NativeModulePolicy) -> libjs::value::JsValue,
}

const BUILTINS: &[NativeBuiltin] = &[
    NativeBuiltin {
        aliases: &["@anyos/ffi", "node:ffi", "node:anyos-ffi"],
        factory: modules::ffi_module,
    },
    NativeBuiltin {
        aliases: &["@anyos/anyui", "node:anyui"],
        factory: modules::anyui_module,
    },
    NativeBuiltin {
        aliases: &["@anyos/image", "node:anyos-image"],
        factory: modules::image_module,
    },
    NativeBuiltin {
        aliases: &["@anyos/confd", "confd", "node:confd", "node:anyos-confd"],
        factory: |_| modules::confd_module(),
    },
    NativeBuiltin {
        aliases: &["@anyos/db", "db", "node:db", "node:anyos-db"],
        factory: |_| modules::db_module(),
    },
    NativeBuiltin {
        aliases: &["@anyos/gl", "gl", "node:gl", "node:anyos-gl"],
        factory: |_| modules::gl_module(),
    },
];

pub fn install(engine: &mut JsEngine, policy: &NativeModulePolicy) {
    for builtin in BUILTINS {
        let module = (builtin.factory)(policy);
        for alias in builtin.aliases {
            engine.register_module_object(alias, module.clone());
        }
    }
}

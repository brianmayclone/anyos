use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use crate::resolver;

use super::process::current_dir;
use super::util::object;

pub fn module() -> JsValue {
    module_with_flavor(false)
}

pub fn posix_module() -> JsValue {
    module_with_flavor(false)
}

pub fn win32_module() -> JsValue {
    module_with_flavor(true)
}

fn module_with_flavor(win32: bool) -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("join"), native_fn("join", join));
    module.set(String::from("resolve"), native_fn("resolve", resolve));
    module.set(String::from("normalize"), native_fn("normalize", normalize));
    module.set(
        String::from("isAbsolute"),
        native_fn("isAbsolute", is_absolute),
    );
    module.set(String::from("relative"), native_fn("relative", relative));
    module.set(String::from("dirname"), native_fn("dirname", dirname));
    module.set(String::from("basename"), native_fn("basename", basename));
    module.set(String::from("extname"), native_fn("extname", extname));
    module.set(String::from("parse"), native_fn("parse", parse));
    module.set(String::from("format"), native_fn("format", format_path));
    module.set(
        String::from("sep"),
        JsValue::String(String::from(if win32 { "\\" } else { "/" })),
    );
    module.set(
        String::from("delimiter"),
        JsValue::String(String::from(if win32 { ";" } else { ":" })),
    );
    object(module)
}

fn join(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut parts: Vec<String> = Vec::new();
    for arg in args {
        let part = arg.to_js_string();
        if !part.is_empty() {
            parts.push(part);
        }
    }
    let mut out = String::from(".");
    for part in parts {
        out = resolver::join_path(&out, &part);
    }
    JsValue::String(out)
}

fn resolve(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut out = current_dir();
    for arg in args {
        out = resolver::join_path(&out, &arg.to_js_string());
    }
    JsValue::String(resolver::normalize_path(&out))
}

fn normalize(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::String(resolver::normalize_path(
        &args
            .first()
            .map(|value| value.to_js_string())
            .unwrap_or_else(|| String::from(".")),
    ))
}

fn is_absolute(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Bool(
        args.first()
            .map(|value| value.to_js_string().starts_with('/'))
            .unwrap_or(false),
    )
}

fn relative(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let from = resolver::normalize_path(
        &args
            .first()
            .map(|value| value.to_js_string())
            .unwrap_or_default(),
    );
    let to = resolver::normalize_path(
        &args
            .get(1)
            .map(|value| value.to_js_string())
            .unwrap_or_default(),
    );
    if from == to {
        return JsValue::String(String::new());
    }
    let from_parts: Vec<&str> = from.split('/').filter(|part| !part.is_empty()).collect();
    let to_parts: Vec<&str> = to.split('/').filter(|part| !part.is_empty()).collect();
    let mut common = 0usize;
    while common < from_parts.len()
        && common < to_parts.len()
        && from_parts[common] == to_parts[common]
    {
        common += 1;
    }
    let mut out: Vec<String> = Vec::new();
    for _ in common..from_parts.len() {
        out.push(String::from(".."));
    }
    for part in &to_parts[common..] {
        out.push((*part).to_string());
    }
    if out.is_empty() {
        JsValue::String(String::new())
    } else {
        JsValue::String(out.join("/"))
    }
}

fn dirname(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::String(resolver::dirname(
        &args
            .first()
            .map(|value| value.to_js_string())
            .unwrap_or_default(),
    ))
}

fn basename(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::String(resolver::basename(
        &args
            .first()
            .map(|value| value.to_js_string())
            .unwrap_or_default(),
    ))
}

fn extname(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::String(resolver::extname(
        &args
            .first()
            .map(|value| value.to_js_string())
            .unwrap_or_default(),
    ))
}

fn parse(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let path = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let dir = resolver::dirname(&path);
    let base = resolver::basename(&path);
    let ext = resolver::extname(&path);
    let name = if ext.is_empty() || !base.ends_with(&ext) {
        base.clone()
    } else {
        String::from(&base[..base.len().saturating_sub(ext.len())])
    };
    let root = if path.starts_with('/') { "/" } else { "" };
    let out = JsValue::new_object();
    out.set_property(String::from("root"), JsValue::String(String::from(root)));
    out.set_property(String::from("dir"), JsValue::String(dir));
    out.set_property(String::from("base"), JsValue::String(base));
    out.set_property(String::from("ext"), JsValue::String(ext));
    out.set_property(String::from("name"), JsValue::String(name));
    out
}

fn format_path(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(input) = args.first() else {
        return JsValue::String(String::new());
    };
    let dir = input.get_property("dir").to_js_string();
    let root = input.get_property("root").to_js_string();
    let base = input.get_property("base").to_js_string();
    let name = input.get_property("name").to_js_string();
    let ext = input.get_property("ext").to_js_string();
    let file = if !base.is_empty() {
        base
    } else {
        let mut s = name;
        s.push_str(&ext);
        s
    };
    if !dir.is_empty() {
        JsValue::String(resolver::join_path(&dir, &file))
    } else if !root.is_empty() {
        JsValue::String(resolver::join_path(&root, &file))
    } else {
        JsValue::String(file)
    }
}

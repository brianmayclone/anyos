use alloc::format;
use alloc::string::{String, ToString};
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("parse"), native_fn("parse", parse));
    module.set(String::from("format"), native_fn("format", format_url));
    module.set(String::from("resolve"), native_fn("resolve", resolve));
    module.set(String::from("URL"), native_fn("URL", url_constructor));
    module.set(
        String::from("URLSearchParams"),
        native_fn("URLSearchParams", search_params_constructor),
    );
    object(module)
}

fn parse(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let input = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let parse_query = args.get(1).map(|value| value.to_boolean()).unwrap_or(false);
    parsed_url_object(&input, parse_query)
}

fn format_url(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(input) = args.first() else {
        return JsValue::String(String::new());
    };
    if !matches!(input, JsValue::Object(_)) {
        return JsValue::String(input.to_js_string());
    }
    let protocol = clean_undefined(input.get_property("protocol").to_js_string());
    let slashes = input.get_property("slashes").to_boolean();
    let auth = clean_undefined(input.get_property("auth").to_js_string());
    let host = clean_undefined(input.get_property("host").to_js_string());
    let hostname = clean_undefined(input.get_property("hostname").to_js_string());
    let port = clean_undefined(input.get_property("port").to_js_string());
    let pathname = clean_undefined(input.get_property("pathname").to_js_string());
    let search = clean_undefined(input.get_property("search").to_js_string());
    let hash = clean_undefined(input.get_property("hash").to_js_string());
    let mut out = String::new();
    if !protocol.is_empty() {
        out.push_str(&protocol);
    }
    let authority = if !host.is_empty() {
        host
    } else if !hostname.is_empty() && !port.is_empty() {
        format!("{}:{}", hostname, port)
    } else {
        hostname
    };
    if slashes || !authority.is_empty() {
        out.push_str("//");
    }
    if !auth.is_empty() {
        out.push_str(&auth);
        out.push('@');
    }
    out.push_str(&authority);
    if !pathname.is_empty() {
        if !pathname.starts_with('/') && !authority.is_empty() {
            out.push('/');
        }
        out.push_str(&pathname);
    }
    if !search.is_empty() {
        if search.starts_with('?') {
            out.push_str(&search);
        } else {
            out.push('?');
            out.push_str(&search);
        }
    }
    if !hash.is_empty() {
        if hash.starts_with('#') {
            out.push_str(&hash);
        } else {
            out.push('#');
            out.push_str(&hash);
        }
    }
    JsValue::String(out)
}

fn resolve(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let from = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let to = args
        .get(1)
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    if has_scheme(&to) {
        return JsValue::String(to);
    }
    let base = split_url(&from);
    if to.starts_with('/') {
        return JsValue::String(format!(
            "{}{}{}",
            base.protocol,
            if base.slashes { "//" } else { "" },
            base.host + &to
        ));
    }
    let dir = match base.pathname.rfind('/') {
        Some(idx) => String::from(&base.pathname[..idx + 1]),
        None => String::from("/"),
    };
    JsValue::String(format!(
        "{}{}{}{}{}",
        base.protocol,
        if base.slashes { "//" } else { "" },
        base.host,
        dir,
        to
    ))
}

fn url_constructor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let input = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    parsed_url_object(&input, false)
}

fn search_params_constructor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let raw = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let params = JsValue::new_object();
    params.set_property(
        String::from("toString"),
        native_fn("toString", search_params_to_string),
    );
    params.set_property(
        String::from("__query"),
        JsValue::String(raw.trim_start_matches('?').to_string()),
    );
    params
}

fn search_params_to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.get_property("__query")
}

fn parsed_url_object(input: &str, parse_query: bool) -> JsValue {
    let parts = split_url(input);
    let out = JsValue::new_object();
    out.set_property(
        String::from("protocol"),
        maybe_string(parts.protocol.clone()),
    );
    out.set_property(String::from("slashes"), JsValue::Bool(parts.slashes));
    out.set_property(String::from("auth"), maybe_string(parts.auth.clone()));
    out.set_property(String::from("host"), maybe_string(parts.host.clone()));
    out.set_property(String::from("hostname"), maybe_string(parts.hostname));
    out.set_property(String::from("port"), maybe_string(parts.port));
    out.set_property(
        String::from("pathname"),
        maybe_string(parts.pathname.clone()),
    );
    out.set_property(String::from("path"), maybe_string(parts.path));
    out.set_property(String::from("href"), JsValue::String(String::from(input)));
    out.set_property(String::from("search"), maybe_string(parts.search.clone()));
    out.set_property(
        String::from("query"),
        if parse_query {
            query_object(&parts.query)
        } else {
            maybe_string(parts.query)
        },
    );
    out.set_property(String::from("hash"), maybe_string(parts.hash));
    out
}

fn query_object(query: &str) -> JsValue {
    let out = JsValue::new_object();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        out.set_property(String::from(key), JsValue::String(String::from(value)));
    }
    out
}

fn maybe_string(value: String) -> JsValue {
    if value.is_empty() {
        JsValue::Null
    } else {
        JsValue::String(value)
    }
}

fn clean_undefined(value: String) -> String {
    if value == "undefined" || value == "null" {
        String::new()
    } else {
        value
    }
}

fn has_scheme(value: &str) -> bool {
    value
        .find(':')
        .map(|idx| value[..idx].bytes().all(|byte| byte.is_ascii_alphabetic()))
        .unwrap_or(false)
}

struct UrlParts {
    protocol: String,
    slashes: bool,
    auth: String,
    host: String,
    hostname: String,
    port: String,
    pathname: String,
    path: String,
    search: String,
    query: String,
    hash: String,
}

fn split_url(input: &str) -> UrlParts {
    let mut rest = input;
    let mut protocol = String::new();
    if let Some(idx) = rest.find(':') {
        if rest[..idx].bytes().all(|byte| byte.is_ascii_alphabetic()) {
            protocol = String::from(&rest[..=idx]);
            rest = &rest[idx + 1..];
        }
    }
    let mut slashes = false;
    if rest.starts_with("//") {
        slashes = true;
        rest = &rest[2..];
    }
    let mut hash = String::new();
    if let Some(idx) = rest.find('#') {
        hash = String::from(&rest[idx..]);
        rest = &rest[..idx];
    }
    let mut search = String::new();
    let mut query = String::new();
    if let Some(idx) = rest.find('?') {
        search = String::from(&rest[idx..]);
        query = String::from(&rest[idx + 1..]);
        rest = &rest[..idx];
    }
    let (authority, pathname) = if slashes {
        match rest.find('/') {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, "/"),
        }
    } else {
        ("", rest)
    };
    let (auth, host) = authority
        .rsplit_once('@')
        .map(|(auth, host)| (String::from(auth), String::from(host)))
        .unwrap_or_else(|| (String::new(), String::from(authority)));
    let (hostname, port) = host
        .rsplit_once(':')
        .map(|(hostname, port)| (String::from(hostname), String::from(port)))
        .unwrap_or_else(|| (host.clone(), String::new()));
    let path = if search.is_empty() {
        String::from(pathname)
    } else {
        format!("{}{}", pathname, search)
    };
    UrlParts {
        protocol,
        slashes,
        auth,
        host,
        hostname,
        port,
        pathname: String::from(pathname),
        path,
        search,
        query,
        hash,
    }
}

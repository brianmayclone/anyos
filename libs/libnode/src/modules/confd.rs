use alloc::format;
use alloc::string::{String, ToString};
use libconf::{
    ConfAuditEntry, ConfClient, ConfError, ConfItem, ConfTarget, ConfValue, NodeKind, RegistryScope,
};
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

const CLIENT_NAME_KEY: &str = "__confd_client_name__";
const TARGET_SCOPE_KEY: &str = "__confd_scope__";
const TARGET_UID_KEY: &str = "__confd_uid__";

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    install_methods(&mut module);
    module.set(
        String::from("createClient"),
        native_fn("createClient", create_client),
    );
    module.set(String::from("client"), native_fn("client", create_client));
    module.set(
        String::from("SCOPE_SYSTEM"),
        JsValue::String(String::from("system")),
    );
    module.set(
        String::from("SCOPE_USER"),
        JsValue::String(String::from("user")),
    );
    object(module)
}

fn install_methods(module: &mut JsObject) {
    module.set(String::from("ping"), native_fn("ping", ping));
    module.set(
        String::from("isAvailable"),
        native_fn("isAvailable", is_available),
    );
    module.set(String::from("mkdir"), native_fn("mkdir", mkdir));
    module.set(String::from("set"), native_fn("set", set));
    module.set(
        String::from("setString"),
        native_fn("setString", set_string),
    );
    module.set(String::from("setInt"), native_fn("setInt", set_int));
    module.set(String::from("setBool"), native_fn("setBool", set_bool));
    module.set(
        String::from("setExternalRef"),
        native_fn("setExternalRef", set_external_ref),
    );
    module.set(String::from("get"), native_fn("get", get));
    module.set(String::from("getValue"), native_fn("getValue", get_value));
    module.set(
        String::from("getString"),
        native_fn("getString", get_string),
    );
    module.set(String::from("getInt"), native_fn("getInt", get_int));
    module.set(String::from("getBool"), native_fn("getBool", get_bool));
    module.set(String::from("delete"), native_fn("delete", del));
    module.set(String::from("del"), native_fn("del", del));
    module.set(String::from("rm"), native_fn("rm", del));
    module.set(String::from("list"), native_fn("list", list));
    module.set(
        String::from("listChildren"),
        native_fn("listChildren", list_children),
    );
    module.set(
        String::from("list_children"),
        native_fn("list_children", list_children),
    );
    module.set(String::from("audit"), native_fn("audit", audit));
}

fn create_client(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let options = args.first().cloned().unwrap_or(JsValue::Undefined);
    let config = ClientConfig::from_options(&options, ClientConfig::default());
    let mut client = JsObject::new();
    install_methods(&mut client);
    client.set_hidden(
        String::from(CLIENT_NAME_KEY),
        JsValue::String(config.client_name),
    );
    client.set_hidden(
        String::from(TARGET_SCOPE_KEY),
        JsValue::String(match config.target {
            ConfTarget::Scope(RegistryScope::System) => String::from("system"),
            ConfTarget::Scope(RegistryScope::User) | ConfTarget::User(_) => String::from("user"),
        }),
    );
    if let ConfTarget::User(uid) = config.target {
        client.set_hidden(String::from(TARGET_UID_KEY), JsValue::Number(uid as f64));
    }
    object(client)
}

fn ping(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    with_client(vm, args, |client, _target| {
        client.ping()?;
        Ok(JsValue::Bool(true))
    })
}

fn is_available(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let config = ClientConfig::from_args(&JsValue::Undefined, args, 0);
    match ConfClient::connect(&config.client_name).and_then(|mut client| client.ping()) {
        Ok(()) => JsValue::Bool(true),
        Err(_) => JsValue::Bool(false),
    }
}

fn mkdir(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = required_path(vm, args) else {
        return JsValue::Undefined;
    };
    with_client(vm, args, |client, target| {
        client.mkdir_target(target, &path).map(item_to_js)
    })
}

fn set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = required_path(vm, args) else {
        return JsValue::Undefined;
    };
    let Some(raw) = args.get(1) else {
        vm.pending_exception = Some(vm.make_type_error("confd.set requires a value"));
        return JsValue::Undefined;
    };
    let Some(value) = js_to_conf_value(raw) else {
        vm.pending_exception =
            Some(vm.make_type_error("confd.set supports string, integer and boolean values"));
        return JsValue::Undefined;
    };
    with_client(vm, args, |client, target| {
        client
            .set_target(target, &path, value.clone())
            .map(item_to_js)
    })
}

fn set_string(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    set_typed(vm, args, |value| ConfValue::String(value.to_js_string()))
}

fn set_int(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args
        .get(1)
        .map(|value| value.to_number())
        .unwrap_or(f64::NAN);
    if !is_i64_number(value) {
        vm.pending_exception = Some(vm.make_type_error("confd.setInt requires an integer value"));
        return JsValue::Undefined;
    }
    set_typed(vm, args, |_| ConfValue::Int(value as i64))
}

fn set_bool(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    set_typed(vm, args, |value| ConfValue::Bool(value.to_boolean()))
}

fn set_external_ref(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    set_typed(vm, args, |value| {
        ConfValue::ExternalRef(value.to_js_string())
    })
}

fn set_typed<F>(vm: &mut Vm, args: &[JsValue], make_value: F) -> JsValue
where
    F: FnOnce(&JsValue) -> ConfValue,
{
    let Some(path) = required_path(vm, args) else {
        return JsValue::Undefined;
    };
    let Some(raw) = args.get(1) else {
        vm.pending_exception = Some(vm.make_type_error("confd setter requires a value"));
        return JsValue::Undefined;
    };
    let value = make_value(raw);
    with_client(vm, args, |client, target| {
        client
            .set_target(target, &path, value.clone())
            .map(item_to_js)
    })
}

fn get(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = required_path(vm, args) else {
        return JsValue::Undefined;
    };
    with_client(vm, args, |client, target| {
        client.get_target(target, &path).map(item_to_js)
    })
}

fn get_value(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    get_item_value(vm, args, |value| value_to_js(&value))
}

fn get_string(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    get_item_value(vm, args, |value| {
        JsValue::String(conf_value_to_string(&value))
    })
}

fn get_int(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    get_item_value(vm, args, |value| match value {
        ConfValue::Int(value) => JsValue::Number(value as f64),
        _ => JsValue::Null,
    })
}

fn get_bool(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    get_item_value(vm, args, |value| match value {
        ConfValue::Bool(value) => JsValue::Bool(value),
        _ => JsValue::Null,
    })
}

fn get_item_value<F>(vm: &mut Vm, args: &[JsValue], convert: F) -> JsValue
where
    F: FnOnce(ConfValue) -> JsValue,
{
    let Some(path) = required_path(vm, args) else {
        return JsValue::Undefined;
    };
    with_client(vm, args, |client, target| {
        let item = client.get_target(target, &path)?;
        Ok(item.value.map(convert).unwrap_or(JsValue::Null))
    })
}

fn del(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = required_path(vm, args) else {
        return JsValue::Undefined;
    };
    with_client(vm, args, |client, target| {
        client.del_target(target, &path).map(item_to_js)
    })
}

fn list(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let path = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    with_client(vm, args, |client, target| {
        client
            .list_target(target, &path)
            .map(|items| JsValue::new_array(items.into_iter().map(item_to_js).collect()))
    })
}

fn list_children(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let path = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    with_client(vm, args, |client, target| {
        client
            .list_children_target(target, &path)
            .map(|items| JsValue::new_array(items.into_iter().map(item_to_js).collect()))
    })
}

fn audit(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let path = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let limit = args
        .get(1)
        .map(|value| value.to_number() as u32)
        .unwrap_or(50);
    with_client(vm, args, |client, target| {
        client
            .audit_target(target, &path, limit)
            .map(|entries| JsValue::new_array(entries.into_iter().map(audit_entry_to_js).collect()))
    })
}

fn with_client<F>(vm: &mut Vm, args: &[JsValue], action: F) -> JsValue
where
    F: FnOnce(&mut ConfClient, ConfTarget) -> Result<JsValue, ConfError>,
{
    let config = ClientConfig::from_args(&vm.current_this, args, 0);
    match ConfClient::connect(&config.client_name).and_then(|mut client| {
        let target = config.target;
        action(&mut client, target)
    }) {
        Ok(value) => value,
        Err(err) => {
            vm.pending_exception =
                Some(vm.make_type_error(&format!("confd: {}", error_message(err))));
            JsValue::Undefined
        }
    }
}

fn required_path(vm: &mut Vm, args: &[JsValue]) -> Option<String> {
    let Some(path) = args.first().map(|value| value.to_js_string()) else {
        vm.pending_exception = Some(vm.make_type_error("confd path is required"));
        return None;
    };
    if path == "undefined" || path.starts_with('/') || path.ends_with('/') || path.contains("//") {
        vm.pending_exception =
            Some(vm.make_type_error("confd path must be a relative registry path"));
        None
    } else {
        Some(path)
    }
}

#[derive(Clone)]
struct ClientConfig {
    client_name: String,
    target: ConfTarget,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            client_name: String::from("node-confd"),
            target: ConfTarget::Scope(RegistryScope::User),
        }
    }
}

impl ClientConfig {
    fn from_args(this_value: &JsValue, args: &[JsValue], options_index: usize) -> Self {
        let mut config = Self::from_this(this_value);
        if let Some(options) = args.get(options_index) {
            config = Self::from_options(options, config);
        }
        if let Some(options) = args.last() {
            config = Self::from_options(options, config);
        }
        config
    }

    fn from_this(this_value: &JsValue) -> Self {
        let mut config = Self::default();
        let client_name = this_value.get_property(CLIENT_NAME_KEY).to_js_string();
        if !client_name.is_empty() && client_name != "undefined" {
            config.client_name = client_name;
        }
        let scope = this_value.get_property(TARGET_SCOPE_KEY).to_js_string();
        if scope == "system" {
            config.target = ConfTarget::Scope(RegistryScope::System);
        } else if scope == "user" {
            config.target = ConfTarget::Scope(RegistryScope::User);
        }
        let uid = this_value.get_property(TARGET_UID_KEY);
        if matches!(uid, JsValue::Number(_)) {
            config.target = ConfTarget::User(uid.to_number() as u16);
        }
        config
    }

    fn from_options(options: &JsValue, mut config: Self) -> Self {
        if !matches!(options, JsValue::Object(_)) {
            return config;
        }
        let client_name = options.get_property("clientName").to_js_string();
        if !client_name.is_empty() && client_name != "undefined" {
            config.client_name = client_name;
        }
        let name = options.get_property("name").to_js_string();
        if !name.is_empty() && name != "undefined" {
            config.client_name = name;
        }
        let target = options.get_property("target").to_js_string();
        if target.starts_with("user@") {
            let uid = target[5..].parse::<u16>().ok().unwrap_or(0);
            config.target = ConfTarget::User(uid);
        } else if target == "system" {
            config.target = ConfTarget::Scope(RegistryScope::System);
        } else if target == "user" {
            config.target = ConfTarget::Scope(RegistryScope::User);
        }
        let scope = options.get_property("scope").to_js_string();
        if scope == "system" {
            config.target = ConfTarget::Scope(RegistryScope::System);
        } else if scope == "user" {
            config.target = ConfTarget::Scope(RegistryScope::User);
        }
        let uid = options.get_property("uid");
        if matches!(uid, JsValue::Number(_)) && uid.to_number().is_finite() {
            config.target = ConfTarget::User(uid.to_number() as u16);
        }
        config
    }
}

fn js_to_conf_value(value: &JsValue) -> Option<ConfValue> {
    match value {
        JsValue::String(value) => Some(ConfValue::String(value.clone())),
        JsValue::Bool(value) => Some(ConfValue::Bool(*value)),
        JsValue::Number(value) if is_i64_number(*value) => Some(ConfValue::Int(*value as i64)),
        JsValue::BigInt(value) => Some(ConfValue::Int(value.to_f64() as i64)),
        _ => None,
    }
}

fn is_i64_number(value: f64) -> bool {
    value.is_finite() && (value as i64) as f64 == value
}

fn item_to_js(item: ConfItem) -> JsValue {
    let mut out = JsObject::new();
    out.set(
        String::from("scope"),
        JsValue::String(match item.scope {
            RegistryScope::System => String::from("system"),
            RegistryScope::User => String::from("user"),
        }),
    );
    out.set(String::from("path"), JsValue::String(item.path));
    out.set(
        String::from("kind"),
        JsValue::String(match item.kind {
            NodeKind::Directory => String::from("dir"),
            NodeKind::Value => String::from("value"),
        }),
    );
    let value_type = item
        .value
        .as_ref()
        .map(conf_value_type)
        .unwrap_or("none")
        .to_string();
    out.set(String::from("type"), JsValue::String(value_type));
    out.set(
        String::from("value"),
        item.value
            .map(|value| value_to_js(&value))
            .unwrap_or(JsValue::Null),
    );
    out.set(
        String::from("version"),
        JsValue::Number(item.version as f64),
    );
    out.set(
        String::from("updatedAt"),
        JsValue::Number(item.updated_at as f64),
    );
    object(out)
}

fn audit_entry_to_js(entry: ConfAuditEntry) -> JsValue {
    let mut out = JsObject::new();
    out.set(String::from("seq"), JsValue::Number(entry.seq as f64));
    out.set(
        String::from("actorUid"),
        JsValue::Number(entry.actor_uid as f64),
    );
    out.set(
        String::from("ownerUid"),
        JsValue::Number(entry.owner_uid as f64),
    );
    out.set(String::from("actorName"), JsValue::String(entry.actor_name));
    out.set(String::from("tid"), JsValue::Number(entry.tid as f64));
    out.set(String::from("action"), JsValue::String(entry.action));
    out.set(
        String::from("scope"),
        JsValue::String(match entry.scope {
            RegistryScope::System => String::from("system"),
            RegistryScope::User => String::from("user"),
        }),
    );
    out.set(String::from("path"), JsValue::String(entry.path));
    out.set(String::from("status"), JsValue::String(entry.status));
    out.set(String::from("detail"), JsValue::String(entry.detail));
    out.set(
        String::from("version"),
        JsValue::Number(entry.version as f64),
    );
    out.set(String::from("atMs"), JsValue::Number(entry.at_ms as f64));
    object(out)
}

fn value_to_js(value: &ConfValue) -> JsValue {
    match value {
        ConfValue::String(value) | ConfValue::ExternalRef(value) => JsValue::String(value.clone()),
        ConfValue::Int(value) => JsValue::Number(*value as f64),
        ConfValue::Bool(value) => JsValue::Bool(*value),
    }
}

fn conf_value_to_string(value: &ConfValue) -> String {
    match value {
        ConfValue::String(value) | ConfValue::ExternalRef(value) => value.clone(),
        ConfValue::Int(value) => value.to_string(),
        ConfValue::Bool(true) => String::from("true"),
        ConfValue::Bool(false) => String::from("false"),
    }
}

fn conf_value_type(value: &ConfValue) -> &'static str {
    match value {
        ConfValue::String(_) => "string",
        ConfValue::Int(_) => "int",
        ConfValue::Bool(_) => "bool",
        ConfValue::ExternalRef(_) => "external_ref",
    }
}

fn error_message(err: ConfError) -> String {
    match err {
        ConfError::NotRunning => String::from("confd is not running"),
        ConfError::PipeCreateFailed => String::from("could not create reply pipe"),
        ConfError::Disconnected => String::from("connection disconnected"),
        ConfError::Timeout => String::from("request timed out"),
        ConfError::Protocol(message) => format!("protocol error: {}", message),
        ConfError::Remote(message) => message,
        ConfError::InvalidArgument(message) => String::from(message),
    }
}

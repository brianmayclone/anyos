use alloc::string::String;
#[cfg(feature = "host")]
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

const EXIT_CODE_KEY: &str = "__node_child_exit_code__";
const ERROR_KEY: &str = "__node_child_error__";
const KILLED_KEY: &str = "__node_child_killed__";

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("spawn"), native_fn("spawn", spawn));
    object(module)
}

fn spawn(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let command = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let argv = args.get(1).map(array_to_strings).unwrap_or_default();

    let mut child = JsObject::new();
    child.set(String::from("pid"), JsValue::Number(0.0));
    child.set(String::from("killed"), JsValue::Bool(false));
    child.set(String::from("kill"), native_fn("kill", child_kill));
    child.set(String::from("on"), native_fn("on", child_on));

    let (exit_code, error) = spawn_process(&command, &argv);
    child.set_hidden(
        String::from(EXIT_CODE_KEY),
        exit_code
            .map(|code| JsValue::Number(code as f64))
            .unwrap_or(JsValue::Undefined),
    );
    child.set_hidden(
        String::from(ERROR_KEY),
        error.map(JsValue::String).unwrap_or(JsValue::Undefined),
    );
    child.set_hidden(String::from(KILLED_KEY), JsValue::Bool(false));
    object(child)
}

fn child_on(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if !matches!(callback, JsValue::Function(_)) {
        return vm.current_this.clone();
    }

    match event.as_str() {
        "error" => {
            let error = vm.current_this.get_property(ERROR_KEY);
            if !matches!(error, JsValue::Undefined) {
                let err = vm.make_type_error(&error.to_js_string());
                vm.call_value(&callback, &[err], vm.current_this.clone());
            }
        }
        "exit" | "close" => {
            let error = vm.current_this.get_property(ERROR_KEY);
            if matches!(error, JsValue::Undefined) {
                let code = vm.current_this.get_property(EXIT_CODE_KEY);
                let code = if matches!(code, JsValue::Undefined) {
                    JsValue::Number(1.0)
                } else {
                    code
                };
                vm.call_value(&callback, &[code, JsValue::Null], vm.current_this.clone());
            }
        }
        _ => {}
    }
    vm.current_this.clone()
}

fn child_kill(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this
        .set_property(String::from("killed"), JsValue::Bool(true));
    vm.current_this
        .set_hidden_property(String::from(KILLED_KEY), JsValue::Bool(true));
    JsValue::Bool(true)
}

fn array_to_strings(value: &JsValue) -> Vec<String> {
    match value {
        JsValue::Array(array) => {
            let array = array.borrow();
            (0..array.length)
                .map(|idx| array.get(idx).to_js_string())
                .collect()
        }
        JsValue::Undefined | JsValue::Null => Vec::new(),
        other => vec![other.to_js_string()],
    }
}

#[cfg(feature = "host")]
fn spawn_process(command: &str, args: &[String]) -> (Option<i32>, Option<String>) {
    let mut child = std::process::Command::new(command);
    child.args(args);
    child.stdin(std::process::Stdio::inherit());
    child.stdout(std::process::Stdio::inherit());
    child.stderr(std::process::Stdio::inherit());
    match child.status() {
        Ok(status) => (Some(status.code().unwrap_or(1)), None),
        Err(err) => (None, Some(err.to_string())),
    }
}

#[cfg(not(feature = "host"))]
fn spawn_process(command: &str, args: &[String]) -> (Option<i32>, Option<String>) {
    let args = args.join(" ");
    let tid = anyos_std::process::spawn(command, &args);
    if tid == u32::MAX {
        return (
            None,
            Some(alloc::format!("spawn failed: {} {}", command, args)),
        );
    }
    let code = anyos_std::process::waitpid(tid);
    (Some(code as i32), None)
}

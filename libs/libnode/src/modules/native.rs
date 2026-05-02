use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_ctor_fn, native_fn, Vm};

use crate::options::NativeModulePolicy;

use super::util::{object, string_array};

pub fn ffi_module(policy: &NativeModulePolicy) -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("enabled"), JsValue::Bool(policy.allow_ffi));
    module.set(String::from("open"), native_fn("open", ffi_open));
    module.set(String::from("call"), native_fn("call", ffi_call));
    module.set(
        String::from("allowedLibraries"),
        string_array(&policy.allowed_libraries),
    );
    object(module)
}

pub fn anyui_module(_policy: &NativeModulePolicy) -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("library"),
        JsValue::String(String::from("libanyui.so")),
    );
    module.set(
        String::from("createApp"),
        native_fn("createApp", anyui_create_app),
    );
    module.set(String::from("run"), native_fn("run", anyui_run));
    module.set(String::from("theme"), anyui_theme_module());
    module.set(
        String::from("Window"),
        native_ctor_fn("Window", anyui_window_ctor),
    );
    for name in [
        "View",
        "Button",
        "PlainButton",
        "IconButton",
        "ImageButton",
        "Label",
        "LinkLabel",
        "TextField",
        "TextArea",
        "TextEditor",
        "SearchField",
        "Checkbox",
        "RadioButton",
        "RadioGroup",
        "ComboBox",
        "DropDown",
        "ListBox",
        "TreeView",
        "DataGrid",
        "TableView",
        "ColorWell",
        "DatePicker",
        "DateTimePicker",
        "TimePicker",
        "Divider",
        "Expander",
        "FlowPanel",
        "GroupBox",
        "ImageView",
        "NavigationBar",
        "ProgressBar",
        "ScrollView",
        "SegmentedControl",
        "Slider",
        "Spinner",
        "SplitView",
        "StackPanel",
        "StatusIndicator",
        "Stepper",
        "TabBar",
        "TableLayout",
        "Tag",
        "Toggle",
        "Toolbar",
        "Tooltip",
        "Alert",
        "Badge",
        "Canvas",
        "Card",
    ] {
        module.set(String::from(name), native_ctor_fn(name, anyui_control_ctor));
    }
    module.set(String::from("DOCK_NONE"), JsValue::Number(0.0));
    module.set(String::from("DOCK_TOP"), JsValue::Number(1.0));
    module.set(String::from("DOCK_BOTTOM"), JsValue::Number(2.0));
    module.set(String::from("DOCK_LEFT"), JsValue::Number(3.0));
    module.set(String::from("DOCK_RIGHT"), JsValue::Number(4.0));
    module.set(String::from("DOCK_FILL"), JsValue::Number(5.0));
    module.set(String::from("ORIENTATION_VERTICAL"), JsValue::Number(0.0));
    module.set(String::from("ORIENTATION_HORIZONTAL"), JsValue::Number(1.0));
    object(module)
}

pub fn image_module(_policy: &NativeModulePolicy) -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("library"),
        JsValue::String(String::from("libimage.so")),
    );
    module.set(String::from("load"), native_fn("load", native_pending));
    object(module)
}

fn ffi_open(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.pending_exception = Some(vm.make_type_error("Native FFI is disabled by policy"));
    JsValue::Undefined
}

fn ffi_call(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.pending_exception = Some(vm.make_type_error("Native FFI calls are not available yet"));
    JsValue::Undefined
}

fn native_pending(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.pending_exception = Some(vm.make_type_error("Native module binding is not linked yet"));
    JsValue::Undefined
}

fn anyui_create_app(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    make_ui_object("Application")
}

fn anyui_run(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

fn anyui_window_ctor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let obj = make_ui_object("Window");
    obj.set_property(
        String::from("title"),
        args.first()
            .cloned()
            .unwrap_or(JsValue::String(String::new())),
    );
    obj
}

fn anyui_control_ctor(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let _ = vm;
    make_ui_object("Control")
}

fn anyui_theme_module() -> JsValue {
    let mut theme = JsObject::new();
    theme.set(
        String::from("colors"),
        native_fn("colors", anyui_theme_colors),
    );
    object(theme)
}

fn anyui_theme_colors(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let mut colors = JsObject::new();
    colors.set(String::from("editorBg"), JsValue::Number(0x202020 as f64));
    colors.set(String::from("text"), JsValue::Number(0xFFFFFFFFu32 as f64));
    colors.set(
        String::from("accent"),
        JsValue::Number(0xFF007ACCu32 as f64),
    );
    object(colors)
}

fn make_ui_object(kind: &str) -> JsValue {
    let mut obj = JsObject::new();
    obj.set(
        String::from("__anyuiKind"),
        JsValue::String(String::from(kind)),
    );
    for name in [
        "add",
        "setPosition",
        "setSize",
        "setColor",
        "setText",
        "setDock",
        "setMargin",
        "setPadding",
        "setOrientation",
        "setState",
        "onClick",
        "onDoubleClick",
        "onTextChanged",
        "onSelectionChanged",
        "onActiveChanged",
        "onCheckedChanged",
        "onValueChanged",
        "onChanged",
        "onColorSelected",
        "onSubmit",
        "onEnter",
    ] {
        obj.set(String::from(name), native_fn(name, anyui_chain));
    }
    object(obj)
}

fn anyui_chain(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}

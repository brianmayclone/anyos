#![cfg_attr(not(feature = "anyui"), allow(dead_code, unused_variables))]

#[cfg(feature = "anyui")]
use alloc::boxed::Box;
use alloc::string::String;
#[cfg(feature = "anyui")]
use libanyui_client as anyui;
#[cfg(feature = "anyui")]
use libanyui_client::Widget;
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
    module.set(
        String::from("View"),
        native_ctor_fn("View", anyui_view_ctor),
    );
    module.set(
        String::from("Button"),
        native_ctor_fn("Button", anyui_button_ctor),
    );
    module.set(
        String::from("PlainButton"),
        native_ctor_fn("PlainButton", anyui_plain_button_ctor),
    );
    module.set(
        String::from("IconButton"),
        native_ctor_fn("IconButton", anyui_icon_button_ctor),
    );
    module.set(
        String::from("ImageButton"),
        native_ctor_fn("ImageButton", anyui_image_button_ctor),
    );
    module.set(
        String::from("Label"),
        native_ctor_fn("Label", anyui_label_ctor),
    );
    module.set(
        String::from("LinkLabel"),
        native_ctor_fn("LinkLabel", anyui_link_label_ctor),
    );
    module.set(
        String::from("TextField"),
        native_ctor_fn("TextField", anyui_text_field_ctor),
    );
    module.set(
        String::from("TextArea"),
        native_ctor_fn("TextArea", anyui_text_area_ctor),
    );
    module.set(
        String::from("TextEditor"),
        native_ctor_fn("TextEditor", anyui_text_editor_ctor),
    );
    module.set(
        String::from("SearchField"),
        native_ctor_fn("SearchField", anyui_search_field_ctor),
    );
    module.set(
        String::from("AutoCompleteTextField"),
        native_ctor_fn("AutoCompleteTextField", anyui_auto_complete_text_field_ctor),
    );
    module.set(
        String::from("Checkbox"),
        native_ctor_fn("Checkbox", anyui_checkbox_ctor),
    );
    module.set(
        String::from("RadioButton"),
        native_ctor_fn("RadioButton", anyui_radio_button_ctor),
    );
    module.set(
        String::from("RadioGroup"),
        native_ctor_fn("RadioGroup", anyui_radio_group_ctor),
    );
    module.set(
        String::from("ComboBox"),
        native_ctor_fn("ComboBox", anyui_combo_box_ctor),
    );
    module.set(
        String::from("DropDown"),
        native_ctor_fn("DropDown", anyui_drop_down_ctor),
    );
    module.set(
        String::from("ListBox"),
        native_ctor_fn("ListBox", anyui_list_box_ctor),
    );
    module.set(
        String::from("TreeView"),
        native_ctor_fn("TreeView", anyui_tree_view_ctor),
    );
    module.set(
        String::from("DataGrid"),
        native_ctor_fn("DataGrid", anyui_data_grid_ctor),
    );
    module.set(
        String::from("TableView"),
        native_ctor_fn("TableView", anyui_table_view_ctor),
    );
    module.set(
        String::from("ColorWell"),
        native_ctor_fn("ColorWell", anyui_color_well_ctor),
    );
    module.set(
        String::from("DatePicker"),
        native_ctor_fn("DatePicker", anyui_date_picker_ctor),
    );
    module.set(
        String::from("DateTimePicker"),
        native_ctor_fn("DateTimePicker", anyui_date_time_picker_ctor),
    );
    module.set(
        String::from("TimePicker"),
        native_ctor_fn("TimePicker", anyui_time_picker_ctor),
    );
    module.set(
        String::from("Divider"),
        native_ctor_fn("Divider", anyui_divider_ctor),
    );
    module.set(
        String::from("Expander"),
        native_ctor_fn("Expander", anyui_expander_ctor),
    );
    module.set(
        String::from("FlowPanel"),
        native_ctor_fn("FlowPanel", anyui_flow_panel_ctor),
    );
    module.set(
        String::from("GroupBox"),
        native_ctor_fn("GroupBox", anyui_group_box_ctor),
    );
    module.set(
        String::from("ImageView"),
        native_ctor_fn("ImageView", anyui_image_view_ctor),
    );
    module.set(
        String::from("NavigationBar"),
        native_ctor_fn("NavigationBar", anyui_navigation_bar_ctor),
    );
    module.set(
        String::from("ProgressBar"),
        native_ctor_fn("ProgressBar", anyui_progress_bar_ctor),
    );
    module.set(
        String::from("ScrollView"),
        native_ctor_fn("ScrollView", anyui_scroll_view_ctor),
    );
    module.set(
        String::from("SegmentedControl"),
        native_ctor_fn("SegmentedControl", anyui_segmented_control_ctor),
    );
    module.set(
        String::from("Slider"),
        native_ctor_fn("Slider", anyui_slider_ctor),
    );
    module.set(
        String::from("Spinner"),
        native_ctor_fn("Spinner", anyui_spinner_ctor),
    );
    module.set(
        String::from("SplitView"),
        native_ctor_fn("SplitView", anyui_split_view_ctor),
    );
    module.set(
        String::from("StackPanel"),
        native_ctor_fn("StackPanel", anyui_stack_panel_ctor),
    );
    module.set(
        String::from("StatusIndicator"),
        native_ctor_fn("StatusIndicator", anyui_status_indicator_ctor),
    );
    module.set(
        String::from("Stepper"),
        native_ctor_fn("Stepper", anyui_stepper_ctor),
    );
    module.set(
        String::from("TabBar"),
        native_ctor_fn("TabBar", anyui_tab_bar_ctor),
    );
    module.set(
        String::from("TableLayout"),
        native_ctor_fn("TableLayout", anyui_table_layout_ctor),
    );
    module.set(String::from("Tag"), native_ctor_fn("Tag", anyui_tag_ctor));
    module.set(
        String::from("Toggle"),
        native_ctor_fn("Toggle", anyui_toggle_ctor),
    );
    module.set(
        String::from("Toolbar"),
        native_ctor_fn("Toolbar", anyui_toolbar_ctor),
    );
    module.set(
        String::from("Tooltip"),
        native_ctor_fn("Tooltip", anyui_tooltip_ctor),
    );
    module.set(
        String::from("Alert"),
        native_ctor_fn("Alert", anyui_alert_ctor),
    );
    module.set(
        String::from("Badge"),
        native_ctor_fn("Badge", anyui_badge_ctor),
    );
    module.set(
        String::from("Canvas"),
        native_ctor_fn("Canvas", anyui_canvas_ctor),
    );
    module.set(
        String::from("Card"),
        native_ctor_fn("Card", anyui_card_ctor),
    );
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
    #[cfg(feature = "anyui")]
    {
        let _ = anyui::init();
    }
    make_ui_object("Application", None)
}

fn anyui_run(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    anyui::run();
    JsValue::Undefined
}

fn anyui_window_ctor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let title = arg_string(args, 0, "");
    let native_id = create_anyui_window(&title, args);
    let obj = make_ui_object("Window", native_id);
    obj.set_property(
        String::from("title"),
        args.first()
            .cloned()
            .unwrap_or(JsValue::String(String::new())),
    );
    obj
}

macro_rules! anyui_ctor {
    ($fn_name:ident, $kind:literal) => {
        fn $fn_name(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
            make_ui_object($kind, create_anyui_control($kind, args))
        }
    };
}

anyui_ctor!(anyui_view_ctor, "View");
anyui_ctor!(anyui_button_ctor, "Button");
anyui_ctor!(anyui_plain_button_ctor, "PlainButton");
anyui_ctor!(anyui_icon_button_ctor, "IconButton");
anyui_ctor!(anyui_image_button_ctor, "ImageButton");
anyui_ctor!(anyui_label_ctor, "Label");
anyui_ctor!(anyui_link_label_ctor, "LinkLabel");
anyui_ctor!(anyui_text_field_ctor, "TextField");
anyui_ctor!(anyui_text_area_ctor, "TextArea");
anyui_ctor!(anyui_text_editor_ctor, "TextEditor");
anyui_ctor!(anyui_search_field_ctor, "SearchField");
anyui_ctor!(anyui_auto_complete_text_field_ctor, "AutoCompleteTextField");
anyui_ctor!(anyui_checkbox_ctor, "Checkbox");
anyui_ctor!(anyui_radio_button_ctor, "RadioButton");
anyui_ctor!(anyui_radio_group_ctor, "RadioGroup");
anyui_ctor!(anyui_combo_box_ctor, "ComboBox");
anyui_ctor!(anyui_drop_down_ctor, "DropDown");
anyui_ctor!(anyui_list_box_ctor, "ListBox");
anyui_ctor!(anyui_tree_view_ctor, "TreeView");
anyui_ctor!(anyui_data_grid_ctor, "DataGrid");
anyui_ctor!(anyui_table_view_ctor, "TableView");
anyui_ctor!(anyui_color_well_ctor, "ColorWell");
anyui_ctor!(anyui_date_picker_ctor, "DatePicker");
anyui_ctor!(anyui_date_time_picker_ctor, "DateTimePicker");
anyui_ctor!(anyui_time_picker_ctor, "TimePicker");
anyui_ctor!(anyui_divider_ctor, "Divider");
anyui_ctor!(anyui_expander_ctor, "Expander");
anyui_ctor!(anyui_flow_panel_ctor, "FlowPanel");
anyui_ctor!(anyui_group_box_ctor, "GroupBox");
anyui_ctor!(anyui_image_view_ctor, "ImageView");
anyui_ctor!(anyui_navigation_bar_ctor, "NavigationBar");
anyui_ctor!(anyui_progress_bar_ctor, "ProgressBar");
anyui_ctor!(anyui_scroll_view_ctor, "ScrollView");
anyui_ctor!(anyui_segmented_control_ctor, "SegmentedControl");
anyui_ctor!(anyui_slider_ctor, "Slider");
anyui_ctor!(anyui_spinner_ctor, "Spinner");
anyui_ctor!(anyui_split_view_ctor, "SplitView");
anyui_ctor!(anyui_stack_panel_ctor, "StackPanel");
anyui_ctor!(anyui_status_indicator_ctor, "StatusIndicator");
anyui_ctor!(anyui_stepper_ctor, "Stepper");
anyui_ctor!(anyui_tab_bar_ctor, "TabBar");
anyui_ctor!(anyui_table_layout_ctor, "TableLayout");
anyui_ctor!(anyui_tag_ctor, "Tag");
anyui_ctor!(anyui_toggle_ctor, "Toggle");
anyui_ctor!(anyui_toolbar_ctor, "Toolbar");
anyui_ctor!(anyui_tooltip_ctor, "Tooltip");
anyui_ctor!(anyui_alert_ctor, "Alert");
anyui_ctor!(anyui_badge_ctor, "Badge");
anyui_ctor!(anyui_canvas_ctor, "Canvas");
anyui_ctor!(anyui_card_ctor, "Card");

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

fn make_ui_object(kind: &str, native_id: Option<u32>) -> JsValue {
    let mut obj = JsObject::new();
    obj.set(
        String::from("__anyuiKind"),
        JsValue::String(String::from(kind)),
    );
    if let Some(id) = native_id {
        obj.set(String::from("__anyuiId"), JsValue::Number(id as f64));
    }
    for (name, handler) in [
        ("add", anyui_add as fn(&mut Vm, &[JsValue]) -> JsValue),
        ("setPosition", anyui_set_position),
        ("setSize", anyui_set_size),
        ("setColor", anyui_set_color),
        ("setText", anyui_set_text),
        ("setDock", anyui_set_dock),
        ("setMargin", anyui_set_margin),
        ("setPadding", anyui_set_padding),
        ("setOrientation", anyui_set_orientation),
        ("setAutoSize", anyui_set_auto_size),
        ("setMinSize", anyui_set_min_size),
        ("setMaxSize", anyui_set_max_size),
        ("setState", anyui_set_state),
        ("getState", anyui_get_state),
        ("getText", anyui_get_text),
        ("getPosition", anyui_get_position),
        ("getSize", anyui_get_size),
        ("setVisible", anyui_set_visible),
        ("setEnabled", anyui_set_enabled),
        ("setFontSize", anyui_set_font_size),
        ("setTextColor", anyui_set_text_color),
        ("setStyle", anyui_set_style),
        ("setTooltip", anyui_set_tooltip),
        ("setTabIndex", anyui_set_tab_index),
        ("setPlaceholder", anyui_set_placeholder),
        ("setPasswordMode", anyui_set_password_mode),
        ("setReadOnly", anyui_set_read_only),
        ("selectAll", anyui_select_all),
        ("setCursor", anyui_set_cursor),
        ("setSelection", anyui_set_selection),
        ("setMaxLength", anyui_set_max_length),
        ("setItems", anyui_set_items),
        ("setSelectedIndex", anyui_set_selected_index),
        ("setSuggestions", anyui_set_suggestions),
        ("setEditable", anyui_set_editable),
        ("setSplitRatio", anyui_set_split_ratio),
        ("setScrollOffsets", anyui_set_scroll_offsets),
        ("setSelectedColor", anyui_set_selected_color),
        ("setDraggable", anyui_set_draggable),
        ("setDropTarget", anyui_set_drop_target),
        ("openPopup", anyui_open_popup),
        ("remove", anyui_remove),
        ("focus", anyui_focus),
        ("bringToFront", anyui_bring_to_front),
    ] {
        obj.set(String::from(name), native_fn(name, handler));
    }
    for (name, handler) in [
        (
            "onClick",
            anyui_on_click as fn(&mut Vm, &[JsValue]) -> JsValue,
        ),
        ("onDoubleClick", anyui_on_double_click),
        ("onFocus", anyui_on_focus),
        ("onBlur", anyui_on_blur),
        ("onContextMenu", anyui_on_context_menu),
        ("onMouseEnter", anyui_on_mouse_enter),
        ("onMouseLeave", anyui_on_mouse_leave),
        ("onMouseDown", anyui_on_mouse_down),
        ("onMouseUp", anyui_on_mouse_up),
        ("onDragStart", anyui_on_drag_start),
        ("onDragEnter", anyui_on_drag_enter),
        ("onDragLeave", anyui_on_drag_leave),
        ("onDrop", anyui_on_drop),
        ("onDragEnd", anyui_on_drag_end),
        ("onTextChanged", anyui_on_change),
        ("onSelectionChanged", anyui_on_change),
        ("onActiveChanged", anyui_on_change),
        ("onCheckedChanged", anyui_on_change),
        ("onValueChanged", anyui_on_change),
        ("onChanged", anyui_on_change),
        ("onColorSelected", anyui_on_change),
        ("onSubmit", anyui_on_submit),
        ("onEnter", anyui_on_submit),
    ] {
        obj.set(String::from(name), native_fn(name, handler));
    }
    object(obj)
}

#[cfg(all(feature = "anyui", not(feature = "host")))]
fn create_anyui_window(title: &str, args: &[JsValue]) -> Option<u32> {
    let _ = anyui::init();
    Some(
        anyui::Window::new(
            title,
            arg_i32(args, 1, -1),
            arg_i32(args, 2, -1),
            arg_u32(args, 3, 960),
            arg_u32(args, 4, 640),
        )
        .id(),
    )
}

#[cfg(all(feature = "anyui", feature = "host"))]
fn create_anyui_window(title: &str, args: &[JsValue]) -> Option<u32> {
    let _ = anyui::init();
    Some(
        anyui::Window::new(
            title,
            arg_i32(args, 1, -1),
            arg_i32(args, 2, -1),
            arg_u32(args, 3, 960),
            arg_u32(args, 4, 640),
        )
        .id(),
    )
}

#[cfg(not(feature = "anyui"))]
fn create_anyui_window(_title: &str, _args: &[JsValue]) -> Option<u32> {
    None
}

#[cfg(all(feature = "anyui", not(feature = "host")))]
fn create_anyui_control(kind: &str, args: &[JsValue]) -> Option<u32> {
    let _ = anyui::init();
    let text = arg_string(args, 0, "");
    let items = arg_string(args, 0, "");
    let w = arg_u32(args, 0, 320);
    let h = arg_u32(args, 1, 200);
    let id = match kind {
        "View" => anyui::View::new().id(),
        "Button" => anyui::Button::new(&text).id(),
        "PlainButton" => anyui::PlainButton::new(&text).id(),
        "IconButton" => anyui::IconButton::new(&text).id(),
        "ImageButton" => anyui::ImageButton::new(w, h).id(),
        "Label" => anyui::Label::new(&text).id(),
        "LinkLabel" => anyui::LinkLabel::new(&text).id(),
        "TextField" => anyui::TextField::new().id(),
        "TextArea" => anyui::TextArea::new().id(),
        "TextEditor" => anyui::TextEditor::new(w, h).id(),
        "SearchField" => anyui::SearchField::new().id(),
        "AutoCompleteTextField" => anyui::AutoCompleteTextField::new().id(),
        "Checkbox" => anyui::Checkbox::new(&text).id(),
        "RadioButton" => anyui::RadioButton::new(&text).id(),
        "RadioGroup" => anyui::RadioGroup::new().id(),
        "ComboBox" => anyui::ComboBox::new().id(),
        "DropDown" => anyui::DropDown::new(&items).id(),
        "ListBox" => anyui::ListBox::new(&items).id(),
        "TreeView" => anyui::TreeView::new(w, h).id(),
        "DataGrid" => anyui::DataGrid::new(w, h).id(),
        "TableView" => anyui::TableView::new().id(),
        "ColorWell" => anyui::ColorWell::new().id(),
        "DatePicker" => anyui::DatePicker::new().id(),
        "DateTimePicker" => anyui::DateTimePicker::new().id(),
        "TimePicker" => anyui::TimePicker::new().id(),
        "Divider" => anyui::Divider::new().id(),
        "Expander" => anyui::Expander::new(&text).id(),
        "FlowPanel" => anyui::FlowPanel::new().id(),
        "GroupBox" => anyui::GroupBox::new(&text).id(),
        "ImageView" => anyui::ImageView::new(w, h).id(),
        "NavigationBar" => anyui::NavigationBar::new(&text).id(),
        "ProgressBar" => anyui::ProgressBar::new(arg_u32(args, 0, 0)).id(),
        "ScrollView" => anyui::ScrollView::new().id(),
        "SegmentedControl" => anyui::SegmentedControl::new(&items).id(),
        "Slider" => anyui::Slider::new(arg_u32(args, 0, 0)).id(),
        "Spinner" => anyui::Spinner::new().id(),
        "SplitView" => anyui::SplitView::new().id(),
        "StackPanel" => anyui::StackPanel::new(arg_u32(args, 0, anyui::ORIENTATION_VERTICAL)).id(),
        "StatusIndicator" => anyui::StatusIndicator::new(&text).id(),
        "Stepper" => anyui::Stepper::new().id(),
        "TabBar" => anyui::TabBar::new(&items).id(),
        "TableLayout" => anyui::TableLayout::new(arg_u32(args, 0, 2)).id(),
        "Tag" => anyui::Tag::new(&text).id(),
        "Toggle" => anyui::Toggle::new(arg_bool(args, 0, false)).id(),
        "Toolbar" => anyui::Toolbar::new().id(),
        "Tooltip" => anyui::Tooltip::new(&text).id(),
        "Alert" => anyui::Alert::new(&text).id(),
        "Badge" => anyui::Badge::new(&text).id(),
        "Canvas" => anyui::Canvas::new(w, h).id(),
        "Card" => anyui::Card::new().id(),
        _ => anyui::View::new().id(),
    };
    Some(id)
}

#[cfg(all(feature = "anyui", feature = "host"))]
fn create_anyui_control(kind: &str, args: &[JsValue]) -> Option<u32> {
    let _ = anyui::init();
    let text = arg_string(args, 0, "");
    let id = match kind {
        "View" => anyui::View::new().id(),
        "Button" | "PlainButton" => anyui::Button::new(&text).id(),
        "IconButton" => anyui::IconButton::new(&text).id(),
        "Label" => anyui::Label::new(&text).id(),
        "LinkLabel" => anyui::LinkLabel::new(&text).id(),
        "TextField" => anyui::TextField::new().id(),
        "TextArea" => anyui::TextArea::new().id(),
        "SearchField" => anyui::SearchField::new().id(),
        "Checkbox" => anyui::Checkbox::new(&text).id(),
        "RadioButton" => anyui::RadioButton::new(&text).id(),
        "DropDown" => anyui::DropDown::new(&arg_string(args, 0, "")).id(),
        "ListBox" => anyui::ListBox::new(&arg_string(args, 0, "")).id(),
        "ColorWell" => anyui::ColorWell::new().id(),
        "DatePicker" => anyui::DatePicker::new().id(),
        "DateTimePicker" => anyui::DateTimePicker::new().id(),
        "TimePicker" => anyui::TimePicker::new().id(),
        "ProgressBar" => anyui::ProgressBar::new(arg_u32(args, 0, 0)).id(),
        "ScrollView" => anyui::ScrollView::new().id(),
        "Slider" => anyui::Slider::new(arg_u32(args, 0, 0)).id(),
        "Spinner" => anyui::Spinner::new().id(),
        "TabBar" => anyui::TabBar::new().id(),
        "Toolbar" => anyui::Toolbar::new().id(),
        "Canvas" => anyui::Canvas::new(arg_u32(args, 0, 320), arg_u32(args, 1, 200)).id(),
        _ => anyui::View::new().id(),
    };
    Some(id)
}

#[cfg(not(feature = "anyui"))]
fn create_anyui_control(_kind: &str, _args: &[JsValue]) -> Option<u32> {
    None
}

fn anyui_id(value: &JsValue) -> Option<u32> {
    let n = value.get_property("__anyuiId").to_number();
    if n.is_finite() && n > 0.0 {
        Some(n as u32)
    } else {
        None
    }
}

fn this_anyui_id(vm: &Vm) -> Option<u32> {
    anyui_id(&vm.current_this)
}

fn this_anyui_kind(vm: &Vm) -> String {
    vm.current_this.get_property("__anyuiKind").to_js_string()
}

fn anyui_number_object(first_name: &str, first: f64, second_name: &str, second: f64) -> JsValue {
    let mut obj = JsObject::new();
    obj.set(String::from(first_name), JsValue::Number(first));
    obj.set(String::from(second_name), JsValue::Number(second));
    object(obj)
}

fn anyui_add(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let (Some(parent), Some(child)) = (this_anyui_id(vm), args.first().and_then(anyui_id)) {
        anyui::Control::from_id(parent).add_child(child);
    }
    vm.current_this.clone()
}

fn anyui_set_position(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_position(arg_i32(args, 0, 0), arg_i32(args, 1, 0));
    }
    vm.current_this.clone()
}

fn anyui_set_size(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_size(arg_u32(args, 0, 0), arg_u32(args, 1, 0));
    }
    vm.current_this.clone()
}

fn anyui_set_color(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_color(arg_color(args, 0, 0x00000000));
    }
    vm.current_this.clone()
}

fn anyui_set_text(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_text(&arg_string(args, 0, ""));
    }
    vm.current_this.clone()
}

fn anyui_set_dock(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_dock(arg_u32(args, 0, anyui::DOCK_NONE));
    }
    vm.current_this.clone()
}

fn anyui_set_margin(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_margin(
            arg_i32(args, 0, 0),
            arg_i32(args, 1, 0),
            arg_i32(args, 2, 0),
            arg_i32(args, 3, 0),
        );
    }
    vm.current_this.clone()
}

fn anyui_set_padding(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_padding(
            arg_i32(args, 0, 0),
            arg_i32(args, 1, 0),
            arg_i32(args, 2, 0),
            arg_i32(args, 3, 0),
        );
    }
    vm.current_this.clone()
}

fn anyui_set_orientation(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_orientation(arg_u32(args, 0, anyui::ORIENTATION_VERTICAL));
    }
    vm.current_this.clone()
}

fn anyui_set_auto_size(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_auto_size(arg_bool(args, 0, true));
    }
    vm.current_this.clone()
}

fn anyui_set_min_size(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_min_size(arg_u32(args, 0, 0), arg_u32(args, 1, 0));
    }
    vm.current_this.clone()
}

fn anyui_set_max_size(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_max_size(arg_u32(args, 0, 0), arg_u32(args, 1, 0));
    }
    vm.current_this.clone()
}

fn anyui_set_state(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_state(arg_u32(args, 0, 0));
    }
    vm.current_this.clone()
}

fn anyui_get_state(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        return JsValue::Number(anyui::Control::from_id(id).get_state() as f64);
    }
    JsValue::Number(0.0)
}

fn anyui_get_text(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        let mut buf = [0u8; 4096];
        let len = anyui::Control::from_id(id).get_text(&mut buf) as usize;
        let text = core::str::from_utf8(&buf[..len.min(buf.len())]).unwrap_or("");
        return JsValue::String(String::from(text));
    }
    JsValue::String(String::new())
}

fn anyui_get_position(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        let (x, y) = anyui::Control::from_id(id).get_position();
        return anyui_number_object("x", x as f64, "y", y as f64);
    }
    anyui_number_object("x", 0.0, "y", 0.0)
}

fn anyui_get_size(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        let (width, height) = anyui::Control::from_id(id).get_size();
        return anyui_number_object("width", width as f64, "height", height as f64);
    }
    anyui_number_object("width", 0.0, "height", 0.0)
}

fn anyui_set_visible(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_visible(arg_bool(args, 0, true));
    }
    vm.current_this.clone()
}

fn anyui_set_enabled(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_enabled(arg_bool(args, 0, true));
    }
    vm.current_this.clone()
}

fn anyui_set_font_size(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_font_size(arg_u32(args, 0, 14));
    }
    vm.current_this.clone()
}

fn anyui_set_text_color(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_text_color(arg_color(args, 0, 0xFFFFFFFF));
    }
    vm.current_this.clone()
}

fn anyui_set_style(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_style(arg_u32(args, 0, 0), arg_u32(args, 1, 0));
    }
    vm.current_this.clone()
}

fn anyui_set_tooltip(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_tooltip(&arg_string(args, 0, ""));
    }
    vm.current_this.clone()
}

fn anyui_set_tab_index(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_tab_index(arg_u32(args, 0, 0));
    }
    vm.current_this.clone()
}

fn anyui_set_placeholder(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        let control = anyui::Control::from_id(id);
        let text = arg_string(args, 0, "");
        if this_anyui_kind(vm) == "ComboBox" {
            control.set_combobox_placeholder(&text);
        } else {
            control.set_textfield_placeholder(&text);
        }
    }
    vm.current_this.clone()
}

fn anyui_set_password_mode(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_textfield_password_mode(arg_bool(args, 0, true));
    }
    vm.current_this.clone()
}

fn anyui_set_read_only(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        let control = anyui::Control::from_id(id);
        if this_anyui_kind(vm) == "TextArea" {
            control.set_textarea_read_only(arg_bool(args, 0, true));
        } else {
            control.set_textfield_read_only(arg_bool(args, 0, true));
        }
    }
    vm.current_this.clone()
}

fn anyui_select_all(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        let control = anyui::Control::from_id(id);
        if this_anyui_kind(vm) == "TextArea" {
            control.textarea_select_all();
        } else {
            control.textfield_select_all();
        }
    }
    vm.current_this.clone()
}

fn anyui_set_cursor(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        let control = anyui::Control::from_id(id);
        if this_anyui_kind(vm) == "TextArea" {
            control.set_textarea_cursor(arg_u32(args, 0, 0));
        } else {
            control.set_textfield_cursor(arg_u32(args, 0, 0));
        }
    }
    vm.current_this.clone()
}

fn anyui_set_selection(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        let control = anyui::Control::from_id(id);
        if this_anyui_kind(vm) == "TextArea" {
            control.set_textarea_selection(arg_u32(args, 0, 0), arg_u32(args, 1, 0));
        } else {
            control.set_textfield_selection(arg_u32(args, 0, 0), arg_u32(args, 1, 0));
        }
    }
    vm.current_this.clone()
}

fn anyui_set_max_length(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        let control = anyui::Control::from_id(id);
        if this_anyui_kind(vm) == "TextArea" {
            control.set_textarea_max_length(arg_u32(args, 0, 0));
        } else {
            control.set_textfield_max_length(arg_u32(args, 0, 0));
        }
    }
    vm.current_this.clone()
}

fn anyui_set_items(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        let items = arg_string(args, 0, "");
        let control = anyui::Control::from_id(id);
        if this_anyui_kind(vm) == "ComboBox" {
            control.set_combobox_items(&items);
        } else {
            control.set_text(&items);
        }
    }
    vm.current_this.clone()
}

fn anyui_set_selected_index(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        let control = anyui::Control::from_id(id);
        if this_anyui_kind(vm) == "ComboBox" {
            let index = if matches!(
                args.first(),
                Some(JsValue::Null) | Some(JsValue::Undefined) | None
            ) {
                None
            } else {
                Some(arg_u32(args, 0, 0))
            };
            control.set_combobox_selected_index(index);
        } else {
            control.set_state(arg_u32(args, 0, 0));
        }
    }
    vm.current_this.clone()
}

fn anyui_set_suggestions(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_autocomplete_suggestions(&arg_string(args, 0, ""));
    }
    vm.current_this.clone()
}

fn anyui_set_editable(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_combobox_editable(arg_bool(args, 0, true));
    }
    vm.current_this.clone()
}

fn anyui_set_split_ratio(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_split_ratio(arg_u32(args, 0, 50));
    }
    vm.current_this.clone()
}

fn anyui_set_scroll_offsets(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_scroll_offsets(arg_i32(args, 0, 0), arg_i32(args, 1, 0));
    }
    vm.current_this.clone()
}

fn anyui_set_selected_color(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_state(arg_color(args, 0, 0xFFFFFFFF));
    }
    vm.current_this.clone()
}

fn anyui_set_draggable(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_draggable(arg_bool(args, 0, true));
    }
    vm.current_this.clone()
}

fn anyui_set_drop_target(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).set_drop_target(arg_bool(args, 0, true));
    }
    vm.current_this.clone()
}

fn anyui_open_popup(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).open_popup();
    }
    vm.current_this.clone()
}

fn anyui_remove(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).remove();
    }
    vm.current_this.clone()
}

fn anyui_focus(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).focus();
    }
    vm.current_this.clone()
}

fn anyui_bring_to_front(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        anyui::Control::from_id(id).bring_to_front();
    }
    vm.current_this.clone()
}

fn anyui_on_click(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::Click)
}

fn anyui_on_double_click(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::DoubleClick)
}

fn anyui_on_focus(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::Focus)
}

fn anyui_on_blur(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::Blur)
}

fn anyui_on_context_menu(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::ContextMenu)
}

fn anyui_on_mouse_enter(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::MouseEnter)
}

fn anyui_on_mouse_leave(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::MouseLeave)
}

fn anyui_on_mouse_down(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::MouseDown)
}

fn anyui_on_mouse_up(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::MouseUp)
}

fn anyui_on_drag_start(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::DragStart)
}

fn anyui_on_drag_enter(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::DragEnter)
}

fn anyui_on_drag_leave(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::DragLeave)
}

fn anyui_on_drop(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::Drop)
}

fn anyui_on_drag_end(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::DragEnd)
}

fn anyui_on_change(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::Change)
}

fn anyui_on_submit(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    anyui_bind_event(vm, args, AnyuiEventBinding::Submit)
}

#[derive(Clone, Copy)]
enum AnyuiEventBinding {
    Click,
    DoubleClick,
    Focus,
    Blur,
    ContextMenu,
    MouseEnter,
    MouseLeave,
    MouseDown,
    MouseUp,
    DragStart,
    DragEnter,
    DragLeave,
    Drop,
    DragEnd,
    Change,
    Submit,
}

fn anyui_bind_event(vm: &mut Vm, args: &[JsValue], event: AnyuiEventBinding) -> JsValue {
    let Some(callback) = args.first() else {
        vm.pending_exception = Some(vm.make_type_error("Event handler must be a function"));
        return JsValue::Undefined;
    };
    if !callback.is_function() {
        vm.pending_exception = Some(vm.make_type_error("Event handler must be a function"));
        return JsValue::Undefined;
    }

    #[cfg(feature = "anyui")]
    if let Some(id) = this_anyui_id(vm) {
        let handler = Box::new(AnyuiJsEventHandler {
            vm: vm as *mut Vm,
            this_obj: vm.current_this.clone(),
            callback: callback.clone(),
        });
        let userdata = Box::into_raw(handler) as u64;
        let control = anyui::Control::from_id(id);
        match event {
            AnyuiEventBinding::Click => control.on_click_raw(anyui_js_event_thunk, userdata),
            AnyuiEventBinding::DoubleClick => {
                control.on_double_click_raw(anyui_js_event_thunk, userdata)
            }
            AnyuiEventBinding::Focus => control.on_focus_raw(anyui_js_event_thunk, userdata),
            AnyuiEventBinding::Blur => control.on_blur_raw(anyui_js_event_thunk, userdata),
            AnyuiEventBinding::ContextMenu => {
                control.on_context_menu_raw(anyui_js_event_thunk, userdata)
            }
            AnyuiEventBinding::MouseEnter => {
                control.on_mouse_enter_raw(anyui_js_event_thunk, userdata)
            }
            AnyuiEventBinding::MouseLeave => {
                control.on_mouse_leave_raw(anyui_js_event_thunk, userdata)
            }
            AnyuiEventBinding::MouseDown => {
                control.on_mouse_down_raw(anyui_js_event_thunk, userdata)
            }
            AnyuiEventBinding::MouseUp => control.on_mouse_up_raw(anyui_js_event_thunk, userdata),
            AnyuiEventBinding::DragStart => {
                control.on_drag_start_raw(anyui_js_event_thunk, userdata)
            }
            AnyuiEventBinding::DragEnter => {
                control.on_drag_enter_raw(anyui_js_event_thunk, userdata)
            }
            AnyuiEventBinding::DragLeave => {
                control.on_drag_leave_raw(anyui_js_event_thunk, userdata)
            }
            AnyuiEventBinding::Drop => control.on_drop_raw(anyui_js_event_thunk, userdata),
            AnyuiEventBinding::DragEnd => control.on_drag_end_raw(anyui_js_event_thunk, userdata),
            AnyuiEventBinding::Change => control.on_change_raw(anyui_js_event_thunk, userdata),
            AnyuiEventBinding::Submit => control.on_submit_raw(anyui_js_event_thunk, userdata),
        }
    }

    vm.current_this.clone()
}

#[cfg(feature = "anyui")]
struct AnyuiJsEventHandler {
    vm: *mut Vm,
    this_obj: JsValue,
    callback: JsValue,
}

#[cfg(feature = "anyui")]
extern "C" fn anyui_js_event_thunk(control_id: u32, event_type: u32, userdata: u64) {
    if userdata == 0 {
        return;
    }

    // The handler is leaked intentionally for the lifetime of the native UI
    // control. libanyui's raw callback ABI does not currently expose unbind.
    unsafe {
        let handler = &mut *(userdata as *mut AnyuiJsEventHandler);
        let Some(vm) = handler.vm.as_mut() else {
            return;
        };
        let mut event = JsObject::new();
        event.set(String::from("id"), JsValue::Number(control_id as f64));
        event.set(
            String::from("controlId"),
            JsValue::Number(control_id as f64),
        );
        event.set(String::from("type"), JsValue::Number(event_type as f64));
        let _ = vm.call_value(
            &handler.callback,
            &[object(event)],
            handler.this_obj.clone(),
        );
        vm.drain_microtasks();
    }
}

fn arg_string(args: &[JsValue], index: usize, default: &str) -> String {
    match args.get(index) {
        Some(JsValue::Undefined) | Some(JsValue::Null) | None => String::from(default),
        Some(value) => value.to_js_string(),
    }
}

fn arg_u32(args: &[JsValue], index: usize, default: u32) -> u32 {
    let Some(value) = args.get(index) else {
        return default;
    };
    let n = value.to_number();
    if n.is_finite() && n >= 0.0 {
        n as u32
    } else {
        default
    }
}

fn arg_i32(args: &[JsValue], index: usize, default: i32) -> i32 {
    let Some(value) = args.get(index) else {
        return default;
    };
    let n = value.to_number();
    if n.is_finite() {
        n as i32
    } else {
        default
    }
}

fn arg_bool(args: &[JsValue], index: usize, default: bool) -> bool {
    args.get(index).map(JsValue::to_boolean).unwrap_or(default)
}

fn arg_color(args: &[JsValue], index: usize, default: u32) -> u32 {
    match args.get(index) {
        Some(JsValue::String(s)) => parse_argb(s).unwrap_or(default),
        Some(value) => {
            let n = value.to_number();
            if n.is_finite() && n >= 0.0 {
                n as u32
            } else {
                default
            }
        }
        None => default,
    }
}

fn parse_argb(value: &str) -> Option<u32> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() == 8 {
        u32::from_str_radix(hex, 16).ok()
    } else if hex.len() == 6 {
        u32::from_str_radix(hex, 16)
            .ok()
            .map(|rgb| 0xFF000000 | rgb)
    } else {
        None
    }
}

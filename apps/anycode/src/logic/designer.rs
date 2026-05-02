use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
pub enum DesignerControlKind {
    Alert,
    AutoCompleteTextField,
    Badge,
    Button,
    Canvas,
    Card,
    Label,
    LinkLabel,
    PlainButton,
    TextField,
    TextArea,
    TextEditor,
    SearchField,
    CheckBox,
    RadioButton,
    RadioGroup,
    ComboBox,
    DropDown,
    ListBox,
    TreeView,
    DataGrid,
    TableView,
    ColorWell,
    DatePicker,
    DateTimePicker,
    TimePicker,
    Divider,
    Expander,
    FlowPanel,
    GroupBox,
    IconButton,
    ImageButton,
    ImageView,
    NavigationBar,
    Panel,
    ProgressBar,
    ScrollView,
    SegmentedControl,
    Slider,
    Spinner,
    SplitView,
    StackPanel,
    StatusIndicator,
    Stepper,
    TabBar,
    TableLayout,
    Tag,
    Toggle,
    Toolbar,
    Tooltip,
}

const TEXTUAL_PROPERTIES: &[&str] = &[
    "Text",
    "X",
    "Y",
    "Width",
    "Height",
    "Enabled",
    "Visible",
    "Tooltip",
    "FontSize",
    "TextColor",
    "BackgroundColor",
];
const LABEL_PROPERTIES: &[&str] = &[
    "Text",
    "X",
    "Y",
    "Width",
    "Height",
    "Enabled",
    "Visible",
    "Tooltip",
    "TextAlign",
    "FontSize",
    "FontWeight",
    "TextColor",
    "BackgroundColor",
];
const TEXT_INPUT_PROPERTIES: &[&str] = &[
    "Text",
    "X",
    "Y",
    "Width",
    "Height",
    "Enabled",
    "Visible",
    "Tooltip",
    "Placeholder",
    "ReadOnly",
    "Password",
    "MaxLength",
    "PrefixIcon",
    "PostfixIcon",
];
const CHOICE_PROPERTIES: &[&str] = &[
    "Text",
    "Items",
    "SelectedIndex",
    "X",
    "Y",
    "Width",
    "Height",
    "Enabled",
    "Visible",
    "Tooltip",
];
const PAGED_PROPERTIES: &[&str] = &[
    "Text",
    "Items",
    "SelectedIndex",
    "ActivePage",
    "PageHeight",
    "X",
    "Y",
    "Width",
    "Height",
    "Enabled",
    "Visible",
    "Tooltip",
    "Dock",
    "Margin",
];
const VALUE_PROPERTIES: &[&str] = &[
    "Value", "Min", "Max", "Step", "X", "Y", "Width", "Height", "Enabled", "Visible", "Tooltip",
];
const DATA_PROPERTIES: &[&str] = &[
    "Columns",
    "Rows",
    "SelectedIndex",
    "SelectionMode",
    "RowHeight",
    "HeaderHeight",
    "IndentWidth",
    "X",
    "Y",
    "Width",
    "Height",
    "Enabled",
    "Visible",
    "Tooltip",
];
const CONTAINER_PROPERTIES: &[&str] = &[
    "X",
    "Y",
    "Width",
    "Height",
    "Enabled",
    "Visible",
    "Dock",
    "Padding",
    "Margin",
    "Orientation",
    "Spacing",
    "BackgroundColor",
    "BorderColor",
];
const MEDIA_PROPERTIES: &[&str] = &[
    "Source",
    "ScaleMode",
    "Interactive",
    "X",
    "Y",
    "Width",
    "Height",
    "Enabled",
    "Visible",
    "Tooltip",
];
const CONTROL_EVENTS: &[&str] = &["OnClick", "OnDoubleClick", "OnChanged", "OnSubmit"];
const FORM_EVENTS: &[&str] = &["OnLoad", "OnShown", "OnClosing", "OnClosed"];
const FORM_PROPERTIES: &[&str] = &["Name", "Title", "Width", "Height", "BackgroundColor"];

const MIN_FORM_SIZE: u32 = 160;
const MAX_FORM_SIZE: u32 = 4096;
const MIN_CONTROL_SIZE: u32 = 1;
const MAX_CONTROL_SIZE: u32 = 2048;
const MIN_CONTROL_POS: i32 = -4096;
const MAX_CONTROL_POS: i32 = 4096;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum UiCodeTarget {
    Rust,
    Node,
}

impl DesignerControlKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Alert => "Alert",
            Self::AutoCompleteTextField => "AutoCompleteTextField",
            Self::Badge => "Badge",
            Self::Button => "Button",
            Self::Canvas => "Canvas",
            Self::Card => "Card",
            Self::Label => "Label",
            Self::LinkLabel => "LinkLabel",
            Self::PlainButton => "PlainButton",
            Self::TextField => "TextField",
            Self::TextArea => "TextArea",
            Self::TextEditor => "TextEditor",
            Self::SearchField => "SearchField",
            Self::CheckBox => "CheckBox",
            Self::RadioButton => "RadioButton",
            Self::RadioGroup => "RadioGroup",
            Self::ComboBox => "ComboBox",
            Self::DropDown => "DropDown",
            Self::ListBox => "ListBox",
            Self::TreeView => "TreeView",
            Self::DataGrid => "DataGrid",
            Self::TableView => "TableView",
            Self::ColorWell => "ColorWell",
            Self::DatePicker => "DatePicker",
            Self::DateTimePicker => "DateTimePicker",
            Self::TimePicker => "TimePicker",
            Self::Divider => "Divider",
            Self::Expander => "Expander",
            Self::FlowPanel => "FlowPanel",
            Self::GroupBox => "GroupBox",
            Self::IconButton => "IconButton",
            Self::ImageButton => "ImageButton",
            Self::ImageView => "ImageView",
            Self::NavigationBar => "NavigationBar",
            Self::Panel => "Panel",
            Self::ProgressBar => "ProgressBar",
            Self::ScrollView => "ScrollView",
            Self::SegmentedControl => "SegmentedControl",
            Self::Slider => "Slider",
            Self::Spinner => "Spinner",
            Self::SplitView => "SplitView",
            Self::StackPanel => "StackPanel",
            Self::StatusIndicator => "StatusIndicator",
            Self::Stepper => "Stepper",
            Self::TabBar => "TabBar",
            Self::TableLayout => "TableLayout",
            Self::Tag => "Tag",
            Self::Toggle => "Toggle",
            Self::Toolbar => "Toolbar",
            Self::Tooltip => "Tooltip",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "Alert" => Self::Alert,
            "AutoCompleteTextField" => Self::AutoCompleteTextField,
            "Badge" => Self::Badge,
            "Button" => Self::Button,
            "Canvas" => Self::Canvas,
            "Card" => Self::Card,
            "Label" => Self::Label,
            "LinkLabel" => Self::LinkLabel,
            "PlainButton" => Self::PlainButton,
            "TextField" => Self::TextField,
            "TextArea" => Self::TextArea,
            "TextEditor" => Self::TextEditor,
            "SearchField" => Self::SearchField,
            "CheckBox" => Self::CheckBox,
            "RadioButton" => Self::RadioButton,
            "RadioGroup" => Self::RadioGroup,
            "ComboBox" => Self::ComboBox,
            "DropDown" => Self::DropDown,
            "ListBox" => Self::ListBox,
            "TreeView" => Self::TreeView,
            "DataGrid" => Self::DataGrid,
            "TableView" => Self::TableView,
            "ColorWell" => Self::ColorWell,
            "DatePicker" => Self::DatePicker,
            "DateTimePicker" => Self::DateTimePicker,
            "TimePicker" => Self::TimePicker,
            "Divider" => Self::Divider,
            "Expander" => Self::Expander,
            "FlowPanel" => Self::FlowPanel,
            "GroupBox" => Self::GroupBox,
            "IconButton" => Self::IconButton,
            "ImageButton" => Self::ImageButton,
            "ImageView" => Self::ImageView,
            "NavigationBar" => Self::NavigationBar,
            "Panel" => Self::Panel,
            "ProgressBar" => Self::ProgressBar,
            "ScrollView" => Self::ScrollView,
            "SegmentedControl" => Self::SegmentedControl,
            "Slider" => Self::Slider,
            "Spinner" => Self::Spinner,
            "SplitView" => Self::SplitView,
            "StackPanel" => Self::StackPanel,
            "StatusIndicator" => Self::StatusIndicator,
            "Stepper" => Self::Stepper,
            "TabBar" => Self::TabBar,
            "TableLayout" => Self::TableLayout,
            "Tag" => Self::Tag,
            "Toggle" => Self::Toggle,
            "Toolbar" => Self::Toolbar,
            "Tooltip" => Self::Tooltip,
            _ => Self::Button,
        }
    }

    pub fn property_names(&self) -> &'static [&'static str] {
        match self {
            Self::Alert
            | Self::Badge
            | Self::Button
            | Self::IconButton
            | Self::LinkLabel
            | Self::NavigationBar
            | Self::PlainButton
            | Self::StatusIndicator
            | Self::Tag
            | Self::Tooltip => TEXTUAL_PROPERTIES,
            Self::Label => LABEL_PROPERTIES,
            Self::AutoCompleteTextField
            | Self::SearchField
            | Self::TextArea
            | Self::TextEditor
            | Self::TextField => TEXT_INPUT_PROPERTIES,
            Self::CheckBox | Self::RadioButton | Self::Toggle => &[
                "Text", "Checked", "X", "Y", "Width", "Height", "Enabled", "Visible", "Tooltip",
            ],
            Self::ComboBox
            | Self::DatePicker
            | Self::DateTimePicker
            | Self::DropDown
            | Self::ListBox
            | Self::RadioGroup
            | Self::TimePicker => CHOICE_PROPERTIES,
            Self::SegmentedControl | Self::TabBar => PAGED_PROPERTIES,
            Self::DataGrid | Self::TableView | Self::TreeView => DATA_PROPERTIES,
            Self::ColorWell | Self::ProgressBar | Self::Slider | Self::Stepper => VALUE_PROPERTIES,
            Self::Canvas | Self::ImageButton | Self::ImageView => MEDIA_PROPERTIES,
            Self::Card
            | Self::Divider
            | Self::Expander
            | Self::FlowPanel
            | Self::GroupBox
            | Self::Panel
            | Self::ScrollView
            | Self::Spinner
            | Self::SplitView
            | Self::StackPanel
            | Self::TableLayout
            | Self::Toolbar => CONTAINER_PROPERTIES,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DesignerProperty {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug)]
pub struct DesignerControl {
    pub name: String,
    pub kind: DesignerControlKind,
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub properties: Vec<DesignerProperty>,
}

impl DesignerControl {
    pub fn event_name(&self) -> String {
        format!("{}_click", self.name)
    }

    pub fn property_items(&self) -> String {
        let mut out = String::new();
        for name in self.kind.property_names() {
            if !out.is_empty() {
                out.push('|');
            }
            out.push_str(name);
        }
        for property in &self.properties {
            if !has_property_name(self.kind.property_names(), &property.name) {
                if !out.is_empty() {
                    out.push('|');
                }
                out.push_str(&property.name);
            }
        }
        for event in CONTROL_EVENTS {
            if !out.is_empty() {
                out.push('|');
            }
            out.push_str(event);
        }
        out
    }

    pub fn property_name_at(&self, index: u32) -> String {
        let names = self.kind.property_names();
        let idx = index as usize;
        let mut current = 0usize;
        for name in names {
            if current == idx {
                return String::from(*name);
            }
            current += 1;
        }
        for property in &self.properties {
            if has_property_name(names, &property.name) {
                continue;
            }
            if current == idx {
                return property.name.clone();
            }
            current += 1;
        }
        for event in CONTROL_EVENTS {
            if current == idx {
                return String::from(*event);
            }
            current += 1;
        }
        String::from("Text")
    }

    pub fn property_value(&self, property_name: &str) -> String {
        match normalized_property(property_name) {
            "text" => self.text.clone(),
            "x" => format!("{}", self.x),
            "y" => format!("{}", self.y),
            "width" => format!("{}", self.width),
            "height" => format!("{}", self.height),
            "items" => choice_items(self),
            "onclick" => self.custom_or_default_event(property_name, "Click"),
            "ondoubleclick" => self.custom_or_default_event(property_name, "DoubleClick"),
            "onchanged" => self.custom_or_default_event(property_name, "Changed"),
            "onsubmit" => self.custom_or_default_event(property_name, "Submit"),
            _ => self
                .properties
                .iter()
                .find(|property| same_property(&property.name, property_name))
                .map(|property| property.value.clone())
                .unwrap_or_else(|| default_property_value(&self.kind, property_name)),
        }
    }

    pub fn event_name_for(&self, event_name: &str) -> String {
        format!("{}_{}", self.name, event_name.to_ascii_lowercase())
    }

    fn custom_or_default_event(&self, property_name: &str, event_name: &str) -> String {
        self.properties
            .iter()
            .find(|property| same_property(&property.name, property_name))
            .map(|property| property.value.clone())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.event_name_for(event_name))
    }

    fn set_custom_property(&mut self, property_name: &str, value: &str) {
        if let Some(property) = self
            .properties
            .iter_mut()
            .find(|property| same_property(&property.name, property_name))
        {
            property.value = String::from(value);
            return;
        }
        self.properties.push(DesignerProperty {
            name: String::from(property_name),
            value: String::from(value),
        });
    }

    pub fn parent_name(&self) -> Option<&str> {
        self.properties
            .iter()
            .find(|property| same_property(&property.name, "Parent"))
            .map(|property| property.value.as_str())
            .filter(|value| !value.is_empty())
    }

    pub fn page_index(&self) -> u32 {
        self.properties
            .iter()
            .find(|property| same_property(&property.name, "PageIndex"))
            .and_then(|property| parse_u32(&property.value))
            .unwrap_or(0)
    }

    fn set_parent_name(&mut self, parent_name: Option<&str>) {
        if let Some(parent_name) = parent_name {
            self.set_custom_property("Parent", parent_name);
        }
    }

    fn set_page_index(&mut self, page_index: Option<u32>) {
        if let Some(page_index) = page_index {
            self.set_custom_property("PageIndex", &format!("{}", page_index));
        }
    }
}

#[derive(Clone, Debug)]
pub struct DesignerDocument {
    pub form_name: String,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub controls: Vec<DesignerControl>,
}

impl DesignerDocument {
    pub fn default_form(form_name: &str) -> Self {
        let mut controls = Vec::new();
        controls.push(DesignerControl {
            name: String::from("label_title"),
            kind: DesignerControlKind::Label,
            text: String::from("Title"),
            x: 24,
            y: 24,
            width: 220,
            height: 24,
            properties: Vec::new(),
        });
        controls.push(DesignerControl {
            name: String::from("button_ok"),
            kind: DesignerControlKind::Button,
            text: String::from("OK"),
            x: 24,
            y: 64,
            width: 96,
            height: 30,
            properties: Vec::new(),
        });
        Self {
            form_name: String::from(form_name),
            title: String::from(form_name),
            width: 640,
            height: 420,
            controls,
        }
    }

    pub fn parse(data: &str) -> Self {
        let mut doc = Self::default_form("Form");
        doc.controls.clear();
        for line in data.split('\n') {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "anycode-designer-v1" {
                continue;
            }
            if trimmed.starts_with("form ") {
                doc.form_name = attr(trimmed, "name").unwrap_or_else(|| doc.form_name.clone());
                doc.title = attr(trimmed, "title").unwrap_or_else(|| doc.title.clone());
                doc.width = attr_u32(trimmed, "width", doc.width);
                doc.height = attr_u32(trimmed, "height", doc.height);
            } else if trimmed.starts_with("control ") {
                doc.controls.push(DesignerControl {
                    name: attr(trimmed, "name").unwrap_or_else(|| String::from("control")),
                    kind: DesignerControlKind::from_str(
                        &attr(trimmed, "kind").unwrap_or_else(|| String::from("Button")),
                    ),
                    text: attr(trimmed, "text").unwrap_or_default(),
                    x: attr_i32(trimmed, "x", 0),
                    y: attr_i32(trimmed, "y", 0),
                    width: attr_u32(trimmed, "width", 100),
                    height: attr_u32(trimmed, "height", 28),
                    properties: Vec::new(),
                });
            } else if trimmed.starts_with("property ") {
                let control_name = attr(trimmed, "control").unwrap_or_default();
                let property_name = attr(trimmed, "name").unwrap_or_default();
                let value = attr(trimmed, "value").unwrap_or_default();
                if let Some(control) = doc
                    .controls
                    .iter_mut()
                    .find(|control| control.name == control_name)
                {
                    control.set_custom_property(&property_name, &value);
                }
            }
        }
        doc.normalize_layout();
        doc
    }

    pub fn normalize_layout(&mut self) {
        self.width = clamp_u32(self.width, MIN_FORM_SIZE, MAX_FORM_SIZE);
        self.height = clamp_u32(self.height, MIN_FORM_SIZE, MAX_FORM_SIZE);
        for control in &mut self.controls {
            control.x = clamp_i32(control.x, MIN_CONTROL_POS, MAX_CONTROL_POS);
            control.y = clamp_i32(control.y, MIN_CONTROL_POS, MAX_CONTROL_POS);
            control.width = clamp_u32(control.width, MIN_CONTROL_SIZE, MAX_CONTROL_SIZE);
            control.height = clamp_u32(control.height, MIN_CONTROL_SIZE, MAX_CONTROL_SIZE);
        }
    }

    pub fn form_property_items(&self) -> String {
        FORM_PROPERTIES.join("|")
    }

    pub fn form_property_name_at(&self, index: u32) -> String {
        FORM_PROPERTIES
            .get(index as usize)
            .map(|name| String::from(*name))
            .unwrap_or_else(|| String::from("Title"))
    }

    pub fn form_property_value(&self, property_name: &str) -> String {
        match normalized_property(property_name) {
            "name" => self.form_name.clone(),
            "title" => self.title.clone(),
            "width" => format!("{}", self.width),
            "height" => format!("{}", self.height),
            "background_color" => String::from("#FF1E1E1E"),
            _ => String::new(),
        }
    }

    pub fn form_event_items(&self) -> String {
        FORM_EVENTS.join("|")
    }

    pub fn form_event_name_at(&self, index: u32) -> String {
        FORM_EVENTS
            .get(index as usize)
            .map(|name| String::from(*name))
            .unwrap_or_else(|| String::from("OnLoad"))
    }

    pub fn form_event_value(&self, event_name: &str) -> String {
        match normalized_property(event_name) {
            "onshown" => self.form_event_handler("Shown"),
            "onclosing" => self.form_event_handler("Closing"),
            "onclosed" => self.form_event_handler("Closed"),
            _ => self.form_event_handler("Load"),
        }
    }

    pub fn form_event_handler(&self, event_name: &str) -> String {
        format!(
            "{}_{}",
            to_module_name(&self.form_name),
            event_name.to_ascii_lowercase()
        )
    }

    pub fn update_form_property(
        &mut self,
        property_name: &str,
        value: &str,
    ) -> Result<(), &'static str> {
        match normalized_property(property_name) {
            "name" => {
                if !is_valid_form_name(value) {
                    return Err("Form name must be a Rust type name");
                }
                self.form_name = String::from(value);
            }
            "title" => self.title = String::from(value),
            "width" => {
                let width = parse_u32(value).ok_or("Width must be a number")?;
                if !(MIN_FORM_SIZE..=MAX_FORM_SIZE).contains(&width) {
                    return Err("Width is outside the supported designer range");
                }
                self.width = width;
            }
            "height" => {
                let height = parse_u32(value).ok_or("Height must be a number")?;
                if !(MIN_FORM_SIZE..=MAX_FORM_SIZE).contains(&height) {
                    return Err("Height is outside the supported designer range");
                }
                self.height = height;
            }
            "background_color" => {
                validate_argb_color(value)?;
            }
            _ => return Err("Unknown form property"),
        }
        Ok(())
    }

    pub fn to_designer_metadata(&self) -> String {
        let mut out = String::from("anycode-designer-v1\n");
        out.push_str(&format!(
            "form name=\"{}\" title=\"{}\" width={} height={}\n",
            escape(&self.form_name),
            escape(&self.title),
            self.width,
            self.height
        ));
        for control in &self.controls {
            out.push_str(&format!(
                "control name=\"{}\" kind=\"{}\" text=\"{}\" x={} y={} width={} height={}\n",
                escape(&control.name),
                control.kind.as_str(),
                escape(&control.text),
                control.x,
                control.y,
                control.width,
                control.height
            ));
            for property in &control.properties {
                out.push_str(&format!(
                    "property control=\"{}\" name=\"{}\" value=\"{}\"\n",
                    escape(&control.name),
                    escape(&property.name),
                    escape(&property.value)
                ));
            }
        }
        out
    }

    pub fn designer_rs(&self) -> String {
        let struct_name = format!("{}Ui", self.form_name);
        let mut out = String::from("use libanyui_client as ui;\n\n");
        out.push_str(&format!("pub struct {} {{\n", struct_name));
        out.push_str("    pub root: ui::View,\n");
        for control in &self.controls {
            out.push_str(&format!(
                "    pub {}: ui::{},\n",
                control.name,
                rust_control_type(&control.kind)
            ));
        }
        out.push_str("}\n\n");
        out.push_str(&format!("impl {} {{\n", struct_name));
        out.push_str("    pub fn build() -> Self {\n");
        out.push_str("        let root = ui::View::new();\n");
        out.push_str(&format!(
            "        root.set_size({}, {});\n",
            self.width, self.height
        ));
        out.push_str("        let tc = ui::theme::colors();\n");
        out.push_str("        root.set_color(tc.editor_bg);\n");
        for control in &self.controls {
            out.push_str(&control_constructor(control));
            out.push_str(&format!(
                "        {}.set_position({}, {});\n",
                control.name, control.x, control.y
            ));
            out.push_str(&format!(
                "        {}.set_size({}, {});\n",
                control.name, control.width, control.height
            ));
            out.push_str(&control_layout_code(control));
        }
        for control in &self.controls {
            if control.parent_name().is_some_and(|parent_name| {
                self.controls
                    .iter()
                    .find(|candidate| candidate.name == parent_name)
                    .map(|candidate| is_paged_kind(&candidate.kind))
                    .unwrap_or(false)
            }) {
                continue;
            }
            if let Some(parent_name) = control.parent_name().filter(|parent_name| {
                self.controls
                    .iter()
                    .find(|candidate| candidate.name == *parent_name)
                    .map(|candidate| is_addable_container_kind(&candidate.kind))
                    .unwrap_or(false)
            }) {
                out.push_str(&format!(
                    "        {}.add(&{});\n",
                    parent_name, control.name
                ));
            } else {
                out.push_str(&format!("        root.add(&{});\n", control.name));
            }
            if is_paged_kind(&control.kind) && self.has_paged_children(&control.name) {
                out.push_str(&self.paged_panels_code(control));
            }
        }
        for control in &self.controls {
            let Some(parent_name) = control.parent_name() else {
                continue;
            };
            let Some(parent) = self
                .controls
                .iter()
                .find(|candidate| candidate.name == parent_name)
            else {
                continue;
            };
            if is_paged_kind(&parent.kind) {
                out.push_str(&format!(
                    "        {}.add(&{});\n",
                    paged_panel_name(parent_name, control.page_index()),
                    control.name
                ));
            }
        }
        for control in &self.controls {
            if is_paged_kind(&control.kind) && self.has_paged_children(&control.name) {
                out.push_str(&self.paged_connect_code(control));
            }
        }
        out.push_str("        Self {\n            root,\n");
        for control in &self.controls {
            out.push_str(&format!("            {},\n", control.name));
        }
        out.push_str("        }\n    }\n}\n");
        out
    }

    pub fn designer_js(&self) -> String {
        let mut out = String::from("// Generated by anyCode Designer. Do not edit by hand.\n");
        out.push_str("const ui = require('@anyos/anyui');\n\n");
        let ui_struct = format!("{}Ui", self.form_name);
        out.push_str(&format!("function {}() {{}}\n\n", ui_struct));
        out.push_str(&format!("{}.build = function() {{\n", ui_struct));
        out.push_str("    const root = new ui.View();\n");
        out.push_str(&format!(
            "    root.setSize({}, {});\n",
            self.width, self.height
        ));
        out.push_str("    root.setColor(ui.theme.colors().editorBg);\n");
        for control in &self.controls {
            out.push_str(&js_control_constructor(control));
            out.push_str(&format!(
                "    {}.setPosition({}, {});\n",
                control.name, control.x, control.y
            ));
            out.push_str(&format!(
                "    {}.setSize({}, {});\n",
                control.name, control.width, control.height
            ));
            out.push_str(&js_control_layout_code(control));
        }
        for control in &self.controls {
            if let Some(parent_name) = control.parent_name().filter(|parent_name| {
                self.controls
                    .iter()
                    .find(|candidate| candidate.name == *parent_name)
                    .map(|candidate| is_addable_container_kind(&candidate.kind))
                    .unwrap_or(false)
            }) {
                out.push_str(&format!("    {}.add({});\n", parent_name, control.name));
            } else {
                out.push_str(&format!("    root.add({});\n", control.name));
            }
        }
        out.push_str("    return {\n      root: root,\n");
        for control in &self.controls {
            out.push_str(&format!("      {}: {},\n", control.name, control.name));
        }
        out.push_str("    };\n};\n\n");
        out.push_str(&format!(
            "module.exports = {{ {}: {} }};\n",
            ui_struct, ui_struct
        ));
        out
    }

    pub fn codebehind_rs(&self) -> String {
        let struct_name = &self.form_name;
        let ui_struct = format!("{}Ui", self.form_name);
        let mut out = String::from("mod designer;\nmod events;\n\n");
        out.push_str(&format!("use self::designer::{};\n\n", ui_struct));
        out.push_str(&format!("pub struct {} {{\n", struct_name));
        out.push_str(&format!("    pub ui: {},\n", ui_struct));
        out.push_str("}\n\n");
        out.push_str(&format!("impl {} {{\n", struct_name));
        out.push_str("    pub fn new() -> Self {\n");
        out.push_str(&format!("        let ui = {}::build();\n", ui_struct));
        for control in &self.controls {
            for (event_name, handler) in event_handler_bindings(control) {
                if let Some(method) = event_hook_method(control.kind.as_str(), &event_name) {
                    out.push_str(&format!(
                        "        ui.{}.{}(|_| events::{}());\n",
                        control.name, method, handler
                    ));
                }
            }
        }
        out.push_str("        Self { ui }\n    }\n\n");
        out.push_str("    pub fn root(&self) -> &libanyui_client::View {\n");
        out.push_str("        &self.ui.root\n");
        out.push_str("    }\n");
        out.push_str("}\n");
        out
    }

    pub fn codebehind_js(&self) -> String {
        let ui_struct = format!("{}Ui", self.form_name);
        let mut out = String::from("const designer = require('./designer');\n");
        out.push_str("const events = require('./events');\n\n");
        out.push_str(&format!("function {}() {{\n", self.form_name));
        out.push_str(&format!("    this.ui = designer.{}.build();\n", ui_struct));
        for control in &self.controls {
            for (event_name, handler) in event_handler_bindings(control) {
                if let Some(method) = js_event_hook_method(control.kind.as_str(), &event_name) {
                    out.push_str(&format!(
                        "    this.ui.{}.{}(() => events.{}());\n",
                        control.name, method, handler
                    ));
                }
            }
        }
        out.push_str("}\n\n");
        out.push_str(&format!(
            "{}.prototype.root = function() {{\n    return this.ui.root;\n}};\n\n",
            self.form_name
        ));
        out.push_str(&format!("module.exports = {};\n", self.form_name));
        out.push_str(&format!(
            "module.exports.{} = {};\n",
            self.form_name, self.form_name
        ));
        out.push_str(&format!(
            "module.exports.{} = designer.{};\n",
            ui_struct, ui_struct
        ));
        out
    }

    pub fn events_rs(&self) -> String {
        let mut out = String::new();
        for control in &self.controls {
            for (_event_name, handler) in event_handler_bindings(control) {
                out.push_str(&format!(
                    "pub fn {}() {{\n    // TODO: handle event\n}}\n\n",
                    handler
                ));
            }
        }
        out
    }

    pub fn events_js(&self) -> String {
        let mut out = String::new();
        let mut handlers = Vec::new();
        for control in &self.controls {
            for (_event_name, handler) in event_handler_bindings(control) {
                handlers.push(handler.clone());
                out.push_str(&format!(
                    "function {}() {{\n  // TODO: handle event\n}}\n\n",
                    handler
                ));
            }
        }
        out.push_str("module.exports = {\n");
        for (idx, handler) in handlers.iter().enumerate() {
            out.push_str(&format!(
                "  {}: {}{}",
                handler,
                handler,
                if idx + 1 == handlers.len() {
                    "\n"
                } else {
                    ",\n"
                }
            ));
        }
        out.push_str("};\n");
        out
    }

    pub fn module_rs(&self) -> String {
        format!("mod view;\n\npub use view::{};\n", self.form_name)
    }

    pub fn module_js(&self) -> String {
        let ui_struct = format!("{}Ui", self.form_name);
        format!(
            "const view = require('./view');\nconst designer = require('./designer');\n\nmodule.exports = view;\nmodule.exports.{} = view.{} || view;\nmodule.exports.{} = designer.{};\n",
            self.form_name, self.form_name, ui_struct, ui_struct
        )
    }

    fn has_paged_children(&self, parent_name: &str) -> bool {
        self.controls
            .iter()
            .any(|control| control.parent_name() == Some(parent_name))
    }

    fn paged_panels_code(&self, control: &DesignerControl) -> String {
        let page_count = self.page_count_for_paged_control(control);
        let mut out = String::new();
        for page_index in 0..page_count {
            let panel_name = paged_panel_name(&control.name, page_index);
            out.push_str(&format!("        let {} = ui::View::new();\n", panel_name));
            out.push_str(&format!(
                "        {}.set_position({}, {});\n",
                panel_name,
                control.x,
                control.y + paged_content_offset_y(control)
            ));
            out.push_str(&format!(
                "        {}.set_size({}, {});\n",
                panel_name,
                control.width,
                paged_content_height(control)
            ));
            out.push_str(&format!(
                "        {}.set_color(tc.editor_bg);\n",
                panel_name
            ));
            if let Some(parent_name) = control.parent_name().filter(|parent_name| {
                self.controls
                    .iter()
                    .find(|candidate| candidate.name == *parent_name)
                    .map(|candidate| is_addable_container_kind(&candidate.kind))
                    .unwrap_or(false)
            }) {
                out.push_str(&format!("        {}.add(&{});\n", parent_name, panel_name));
            } else {
                out.push_str(&format!("        root.add(&{});\n", panel_name));
            }
        }
        out
    }

    fn paged_connect_code(&self, control: &DesignerControl) -> String {
        let page_count = self.page_count_for_paged_control(control);
        let mut refs = String::new();
        for page_index in 0..page_count {
            if page_index > 0 {
                refs.push_str(", ");
            }
            refs.push_str("&");
            refs.push_str(&paged_panel_name(&control.name, page_index));
        }
        format!("        {}.connect_panels(&[{}]);\n", control.name, refs)
    }

    fn page_count_for_paged_control(&self, control: &DesignerControl) -> u32 {
        let mut count = page_count_for_control(control);
        for child in &self.controls {
            if child.parent_name() == Some(control.name.as_str()) {
                count = count.max(child.page_index().saturating_add(1));
            }
        }
        count
    }

    pub fn update_control_property(
        &mut self,
        control_name: &str,
        property_name: &str,
        value: &str,
    ) -> Result<(), &'static str> {
        let Some(control) = self
            .controls
            .iter_mut()
            .find(|control| control.name == control_name)
        else {
            return Err("Designer control not found");
        };
        match normalized_property(property_name) {
            "text" => control.text = String::from(value),
            "x" => {
                let x = parse_i32(value).ok_or("X must be a number")?;
                if !(MIN_CONTROL_POS..=MAX_CONTROL_POS).contains(&x) {
                    return Err("X is outside the supported designer range");
                }
                control.x = x;
            }
            "y" => {
                let y = parse_i32(value).ok_or("Y must be a number")?;
                if !(MIN_CONTROL_POS..=MAX_CONTROL_POS).contains(&y) {
                    return Err("Y is outside the supported designer range");
                }
                control.y = y;
            }
            "width" => {
                let width = parse_u32(value).ok_or("Width must be a number")?;
                if !(MIN_CONTROL_SIZE..=MAX_CONTROL_SIZE).contains(&width) {
                    return Err("Width is outside the supported designer range");
                }
                control.width = width;
            }
            "height" => {
                let height = parse_u32(value).ok_or("Height must be a number")?;
                if !(MIN_CONTROL_SIZE..=MAX_CONTROL_SIZE).contains(&height) {
                    return Err("Height is outside the supported designer range");
                }
                control.height = height;
            }
            "text_color" | "background_color" | "border_color" => {
                validate_argb_color(value)?;
                control.set_custom_property(property_name, value);
            }
            "onclick" | "ondoubleclick" | "onchanged" | "onsubmit" => {
                if !is_valid_control_name(value) {
                    return Err("Event handler must be a valid Rust function name");
                }
                control.set_custom_property(property_name, value);
            }
            _ => control.set_custom_property(property_name, value),
        }
        Ok(())
    }

    pub fn add_control(
        &mut self,
        kind_name: &str,
        x: i32,
        y: i32,
        parent_name: Option<&str>,
        page_index: Option<u32>,
    ) -> Result<String, &'static str> {
        let kind = DesignerControlKind::from_str(kind_name);
        let base_name = default_control_base_name(kind.as_str());
        let name = self.next_control_name(base_name);
        let (width, height) = default_control_size(&kind);
        let text = default_control_text(&kind, &name);
        let (x, y, width, height) = self.fit_bounds_to_container(parent_name, x, y, width, height);
        let mut control = DesignerControl {
            name: name.clone(),
            kind,
            text,
            x,
            y,
            width,
            height,
            properties: Vec::new(),
        };
        control.set_parent_name(parent_name);
        control.set_page_index(page_index);
        self.controls.push(control);
        self.normalize_layout();
        Ok(name)
    }

    pub fn add_control_copy(
        &mut self,
        template: &DesignerControl,
        offset_x: i32,
        offset_y: i32,
    ) -> Result<String, &'static str> {
        let name = self.next_control_name(default_control_base_name(template.kind.as_str()));
        let parent_name = template
            .parent_name()
            .filter(|parent_name| {
                self.controls
                    .iter()
                    .any(|control| control.name == *parent_name)
            })
            .map(String::from);
        let (x, y, width, height) = self.fit_bounds_to_container(
            parent_name.as_deref(),
            template.x.saturating_add(offset_x),
            template.y.saturating_add(offset_y),
            template.width,
            template.height,
        );
        let mut control = template.clone();
        control.name = name.clone();
        control.x = x;
        control.y = y;
        control.width = width;
        control.height = height;
        control.set_parent_name(parent_name.as_deref());
        self.controls.push(control);
        self.normalize_layout();
        Ok(name)
    }

    pub fn set_control_bounds(
        &mut self,
        control_name: &str,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<(), &'static str> {
        let parent_name = self
            .controls
            .iter()
            .find(|control| control.name == control_name)
            .and_then(|control| control.parent_name())
            .map(String::from);
        let (x, y, width, height) =
            self.fit_bounds_to_container(parent_name.as_deref(), x, y, width, height);
        let Some(control) = self
            .controls
            .iter_mut()
            .find(|control| control.name == control_name)
        else {
            return Err("Designer control not found");
        };
        control.x = x;
        control.y = y;
        control.width = width;
        control.height = height;
        Ok(())
    }

    pub fn remove_control(&mut self, control_name: &str) -> Result<(), &'static str> {
        let Some(index) = self
            .controls
            .iter()
            .position(|control| control.name == control_name)
        else {
            return Err("Designer control not found");
        };
        self.controls.remove(index);
        Ok(())
    }

    pub fn control_property_name_at(&self, control_name: &str, index: u32) -> String {
        self.controls
            .iter()
            .find(|control| control.name == control_name)
            .map(|control| control.property_name_at(index))
            .unwrap_or_else(|| String::from("Text"))
    }

    pub fn control_property_value(&self, control_name: &str, property_name: &str) -> String {
        self.controls
            .iter()
            .find(|control| control.name == control_name)
            .map(|control| control.property_value(property_name))
            .unwrap_or_default()
    }

    pub fn control_absolute_bounds(&self, control_name: &str) -> Option<(i32, i32, u32, u32)> {
        self.control_absolute_bounds_inner(control_name, 0)
    }

    pub fn control_parent_page_index_at_form(
        &self,
        parent_name: &str,
        form_x: i32,
        form_y: i32,
    ) -> Option<u32> {
        let parent = self
            .controls
            .iter()
            .find(|control| control.name == parent_name)?;
        if !is_paged_kind(&parent.kind) {
            return None;
        }
        let (x, y, w, h) = self.control_absolute_bounds(parent_name)?;
        let local_x = form_x - x;
        let local_y = form_y - y;
        if local_x < 0 || local_y < 0 || local_x > w as i32 || local_y > h as i32 {
            return None;
        }
        Some(page_index_for_control(parent, local_x))
    }

    pub fn control_parent_client_origin(&self, parent_name: &str) -> Option<(i32, i32)> {
        let parent = self
            .controls
            .iter()
            .find(|control| control.name == parent_name)?;
        let (x, y, _, _) = self.control_absolute_bounds(parent_name)?;
        if is_paged_kind(&parent.kind) {
            Some((x, y.saturating_add(paged_content_offset_y(parent))))
        } else {
            Some((x, y))
        }
    }

    pub fn control_parent_client_size(&self, parent_name: &str) -> Option<(u32, u32)> {
        let parent = self
            .controls
            .iter()
            .find(|control| control.name == parent_name)?;
        if is_paged_kind(&parent.kind) {
            Some((parent.width, paged_content_height(parent)))
        } else {
            Some((parent.width, parent.height))
        }
    }

    fn control_absolute_bounds_inner(
        &self,
        control_name: &str,
        depth: u32,
    ) -> Option<(i32, i32, u32, u32)> {
        if depth > 16 {
            return None;
        }
        let control = self
            .controls
            .iter()
            .find(|control| control.name == control_name)?;
        let mut x = control.x;
        let mut y = control.y;
        if let Some(parent_name) = control.parent_name() {
            if let Some(parent) = self
                .controls
                .iter()
                .find(|candidate| candidate.name == parent_name)
            {
                if let Some((px, py, _, _)) =
                    self.control_absolute_bounds_inner(parent_name, depth.saturating_add(1))
                {
                    x = x.saturating_add(px);
                    y = y.saturating_add(py);
                    if is_paged_kind(&parent.kind) {
                        y = y.saturating_add(paged_content_offset_y(parent));
                    }
                }
            }
        }
        Some((x, y, control.width, control.height))
    }

    fn next_control_name(&self, base_name: &str) -> String {
        let mut index = 1u32;
        loop {
            let candidate = format!("{}{}", base_name, index);
            if !self
                .controls
                .iter()
                .any(|control| control.name == candidate)
            {
                return candidate;
            }
            index = index.saturating_add(1);
        }
    }

    fn fit_bounds_to_container(
        &self,
        parent_name: Option<&str>,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> (i32, i32, u32, u32) {
        let (container_w, container_h) = parent_name
            .and_then(|name| self.control_parent_client_size(name))
            .unwrap_or((self.width, self.height));
        let width = clamp_u32(width, MIN_CONTROL_SIZE, container_w.max(MIN_CONTROL_SIZE));
        let height = clamp_u32(height, MIN_CONTROL_SIZE, container_h.max(MIN_CONTROL_SIZE));
        let max_x = container_w.saturating_sub(width) as i32;
        let max_y = container_h.saturating_sub(height) as i32;
        let x = clamp_i32(x, 0, max_x.max(0));
        let y = clamp_i32(y, 0, max_y.max(0));
        (x, y, width, height)
    }
}

pub fn is_designer_file(file_path: &str) -> bool {
    file_path.ends_with(".Designer")
}

pub fn load_designer(file_path: &str) -> Option<DesignerDocument> {
    let data = anyos_std::fs::read_to_string(file_path).ok()?;
    Some(DesignerDocument::parse(&data))
}

pub fn try_create_designer_from_rust_ui(file_path: &str) -> Option<String> {
    if !file_path.ends_with(".rs") {
        return None;
    }
    let basename = crate::util::path::basename(file_path);
    if basename == "designer.rs" || basename.ends_with(".Designer.rs") {
        return None;
    }
    let data = anyos_std::fs::read_to_string(file_path).ok()?;
    if !data.contains("ui::") && !data.contains("libanyui_client") {
        return None;
    }

    let form_name =
        parse_rust_form_name(&data).unwrap_or_else(|| rust_form_name_from_path(file_path));
    let designer_path = sibling_designer_path(file_path, &form_name)?;
    if crate::util::path::exists(&designer_path) {
        return None;
    }

    let mut doc = DesignerDocument {
        form_name: form_name.clone(),
        title: form_name,
        width: 640,
        height: 420,
        controls: Vec::new(),
    };
    import_rust_controls(&data, &mut doc);
    if doc.controls.is_empty() {
        return None;
    }
    if save_designer(&designer_path, &doc).is_ok() {
        Some(designer_path)
    } else {
        None
    }
}

pub fn save_designer(file_path: &str, doc: &DesignerDocument) -> Result<(), &'static str> {
    let mut normalized = doc.clone();
    normalized.normalize_layout();
    anyos_std::fs::write_bytes(file_path, normalized.to_designer_metadata().as_bytes())
        .map_err(|_| "Could not write designer metadata")?;
    regenerate_generated_files(file_path, &normalized)
}

pub fn regenerate_generated_files(
    designer_file_path: &str,
    doc: &DesignerDocument,
) -> Result<(), &'static str> {
    regenerate_generated_files_for_target(designer_file_path, doc, infer_target(designer_file_path))
}

pub fn regenerate_generated_files_for_target(
    designer_file_path: &str,
    doc: &DesignerDocument,
    target: UiCodeTarget,
) -> Result<(), &'static str> {
    let form_dir = match designer_file_path.rfind('/') {
        Some(pos) => &designer_file_path[..pos],
        None => return Err("Invalid designer path"),
    };
    match target {
        UiCodeTarget::Rust => {
            anyos_std::fs::write_bytes(
                &format!("{}/designer.rs", form_dir),
                doc.designer_rs().as_bytes(),
            )
            .map_err(|_| "Could not update generated designer")?;
            anyos_std::fs::write_bytes(
                &format!("{}/view.rs", form_dir),
                doc.codebehind_rs().as_bytes(),
            )
            .map_err(|_| "Could not update codebehind")?;
            ensure_event_stubs(&format!("{}/events.rs", form_dir), doc)?;
            anyos_std::fs::write_bytes(&format!("{}/mod.rs", form_dir), doc.module_rs().as_bytes())
                .map_err(|_| "Could not update form module")?;
        }
        UiCodeTarget::Node => {
            anyos_std::fs::write_bytes(
                &format!("{}/designer.js", form_dir),
                doc.designer_js().as_bytes(),
            )
            .map_err(|_| "Could not update generated designer")?;
            anyos_std::fs::write_bytes(
                &format!("{}/view.js", form_dir),
                doc.codebehind_js().as_bytes(),
            )
            .map_err(|_| "Could not update codebehind")?;
            let events_path = format!("{}/events.js", form_dir);
            if !crate::util::path::exists(&events_path) {
                anyos_std::fs::write_bytes(&events_path, doc.events_js().as_bytes())
                    .map_err(|_| "Could not update event handlers")?;
            }
            anyos_std::fs::write_bytes(
                &format!("{}/index.js", form_dir),
                doc.module_js().as_bytes(),
            )
            .map_err(|_| "Could not update form module")?;
        }
    }
    Ok(())
}

pub fn create_form_files(project_root: &str, form_name: &str) -> Result<(), &'static str> {
    create_form_files_for_target(project_root, form_name, UiCodeTarget::Rust)
}

pub fn create_form_files_for_target(
    project_root: &str,
    form_name: &str,
    target: UiCodeTarget,
) -> Result<(), &'static str> {
    if !is_valid_form_name(form_name) {
        return Err("Use a valid type name, for example MainForm");
    }
    let ui_dir = format!("{}/src/ui", project_root);
    let _ = anyos_std::fs::mkdir(&format!("{}/src", project_root));
    let _ = anyos_std::fs::mkdir(&ui_dir);

    let module_name = to_module_name(form_name);
    let form_dir = format!("{}/{}", ui_dir, module_name);
    let _ = anyos_std::fs::mkdir(&form_dir);
    if target == UiCodeTarget::Node {
        crate::logic::node_project::ensure_support_files(project_root)?;
    }
    let doc = DesignerDocument::default_form(form_name);
    let designer_path = designer_file_path(project_root, form_name);
    write_new(&designer_path, &doc.to_designer_metadata())?;
    match target {
        UiCodeTarget::Rust => {
            write_new(&format!("{}/designer.rs", form_dir), &doc.designer_rs())?;
            write_new(&format!("{}/events.rs", form_dir), &doc.events_rs())?;
            write_new(&format!("{}/view.rs", form_dir), &doc.codebehind_rs())?;
            write_new(&format!("{}/mod.rs", form_dir), &doc.module_rs())?;
        }
        UiCodeTarget::Node => {
            write_new(&format!("{}/designer.js", form_dir), &doc.designer_js())?;
            write_new(&format!("{}/events.js", form_dir), &doc.events_js())?;
            write_new(&format!("{}/view.js", form_dir), &doc.codebehind_js())?;
            write_new(&format!("{}/index.js", form_dir), &doc.module_js())?;
        }
    }
    Ok(())
}

pub fn regenerate_node_forms_for_project(project_root: &str) -> usize {
    let ui_dir = format!("{}/src/ui", project_root);
    let mut changed = 0usize;
    regenerate_node_forms_in_dir(&ui_dir, &mut changed, 0);
    changed
}

fn regenerate_node_forms_in_dir(dir: &str, changed: &mut usize, depth: u32) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = anyos_std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let full = crate::util::path::join(dir, &entry.name);
        if entry.is_dir() {
            regenerate_node_forms_in_dir(&full, changed, depth + 1);
        } else if is_designer_file(&full) {
            if let Some(doc) = load_designer(&full) {
                if regenerate_generated_files_for_target(&full, &doc, UiCodeTarget::Node).is_ok() {
                    *changed += 1;
                }
            }
        }
    }
}

pub fn next_form_name(project_root: &str, base_name: &str) -> String {
    if !form_exists(project_root, base_name) {
        return String::from(base_name);
    }
    let mut index = 2u32;
    loop {
        let candidate = format!("{}{}", base_name, index);
        if !form_exists(project_root, &candidate) {
            return candidate;
        }
        index = index.saturating_add(1);
    }
}

pub fn is_valid_form_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return false;
    }
    for (i, b) in trimmed.bytes().enumerate() {
        let valid = match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => true,
            b'0'..=b'9' => i > 0,
            _ => false,
        };
        if !valid {
            return false;
        }
    }
    true
}

pub fn designer_file_path(project_root: &str, form_name: &str) -> String {
    format!(
        "{}/src/ui/{}/{}.Designer",
        project_root,
        to_module_name(form_name),
        form_name
    )
}

pub fn form_exists(project_root: &str, form_name: &str) -> bool {
    crate::util::path::exists(&designer_file_path(project_root, form_name))
}

pub fn events_file_for_designer(designer_file_path: &str) -> String {
    match designer_file_path.rfind('/') {
        Some(pos) => format!("{}/events.rs", &designer_file_path[..pos]),
        None => String::from("events.rs"),
    }
}

pub fn ensure_event_handler(
    designer_file_path: &str,
    control: &DesignerControl,
) -> Result<String, &'static str> {
    ensure_named_event_handler(designer_file_path, &control.event_name())
}

pub fn ensure_control_event_handler(
    designer_file_path: &str,
    control: &DesignerControl,
    event_property: &str,
) -> Result<(String, String), &'static str> {
    let handler = control.property_value(event_property);
    let events_path = ensure_named_event_handler(designer_file_path, &handler)?;
    Ok((events_path, handler))
}

pub fn ensure_named_event_handler(
    designer_file_path: &str,
    handler: &str,
) -> Result<String, &'static str> {
    if !is_valid_control_name(handler) {
        return Err("Event handler must be a valid Rust function name");
    }
    let events_path = events_file_for_designer(designer_file_path);
    let signature = format!("pub fn {}()", handler);
    let mut data = anyos_std::fs::read_to_string(&events_path).unwrap_or_default();
    if !data.contains(&signature) {
        if !data.ends_with('\n') {
            data.push('\n');
        }
        data.push_str(&format!(
            "\npub fn {}() {{\n    // TODO: handle event\n}}\n",
            handler
        ));
        anyos_std::fs::write_bytes(&events_path, data.as_bytes())
            .map_err(|_| "Could not update event handler")?;
    }
    Ok(events_path)
}

fn ensure_event_stubs(events_path: &str, doc: &DesignerDocument) -> Result<(), &'static str> {
    let mut data = anyos_std::fs::read_to_string(events_path).unwrap_or_default();
    let mut changed = false;
    for control in &doc.controls {
        for (_event_name, handler) in event_handler_bindings(control) {
            let signature = format!("pub fn {}()", handler);
            if data.contains(&signature) {
                continue;
            }
            if !data.ends_with('\n') {
                data.push('\n');
            }
            data.push_str(&format!(
                "\npub fn {}() {{\n    // TODO: handle event\n}}\n",
                handler
            ));
            changed = true;
        }
    }
    if changed {
        anyos_std::fs::write_bytes(events_path, data.as_bytes())
            .map_err(|_| "Could not update event handlers")?;
    }
    Ok(())
}

fn event_handler_bindings(control: &DesignerControl) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if event_hook_method(control.kind.as_str(), "OnClick").is_some() {
        out.push((String::from("OnClick"), control.property_value("OnClick")));
    }
    for property in &control.properties {
        if !is_control_event(&property.name) || property.value.is_empty() {
            continue;
        }
        if event_hook_method(control.kind.as_str(), &property.name).is_none() {
            continue;
        }
        if out.iter().any(|(_, handler)| handler == &property.value) {
            continue;
        }
        out.push((property.name.clone(), property.value.clone()));
    }
    out
}

fn is_control_event(name: &str) -> bool {
    CONTROL_EVENTS
        .iter()
        .any(|event| normalized_property(event) == normalized_property(name))
}

fn event_hook_method(kind: &str, event_name: &str) -> Option<&'static str> {
    match normalized_property(event_name) {
        "onclick" => match kind {
            "Button" | "IconButton" | "ImageButton" | "LinkLabel" | "PlainButton" | "Label"
            | "Tag" | "Canvas" => Some("on_click"),
            _ => None,
        },
        "ondoubleclick" => match kind {
            "TabBar" => Some("on_double_click"),
            _ => None,
        },
        "onchanged" => match kind {
            "TextField" | "TextArea" | "SearchField" | "AutoCompleteTextField" => {
                Some("on_text_changed")
            }
            "TextEditor" => Some("on_text_changed"),
            "ListBox" | "TreeView" | "DataGrid" | "TableView" | "ComboBox" | "DropDown"
            | "RadioGroup" => Some("on_selection_changed"),
            "SegmentedControl" | "TabBar" => Some("on_active_changed"),
            "CheckBox" | "RadioButton" | "Toggle" => Some("on_checked_changed"),
            "Slider" | "Stepper" => Some("on_value_changed"),
            "DatePicker" | "DateTimePicker" | "TimePicker" => Some("on_changed"),
            "ColorWell" => Some("on_color_selected"),
            _ => None,
        },
        "onsubmit" => match kind {
            "TextField" | "SearchField" | "AutoCompleteTextField" | "DataGrid" => Some("on_submit"),
            "TreeView" => Some("on_enter"),
            _ => None,
        },
        _ => None,
    }
}

fn default_control_base_name(kind_name: &str) -> &'static str {
    match kind_name {
        "Alert" => "alert",
        "AutoCompleteTextField" => "auto_complete",
        "Badge" => "badge",
        "Button" => "button",
        "Canvas" => "canvas",
        "Card" => "card",
        "CheckBox" => "check_box",
        "ColorWell" => "color_well",
        "ComboBox" => "combo_box",
        "DataGrid" => "data_grid",
        "DatePicker" => "date_picker",
        "DateTimePicker" => "date_time_picker",
        "Divider" => "divider",
        "DropDown" => "drop_down",
        "Expander" => "expander",
        "FlowPanel" => "flow_panel",
        "GroupBox" => "group_box",
        "IconButton" => "icon_button",
        "ImageButton" => "image_button",
        "ImageView" => "image_view",
        "Label" => "label",
        "LinkLabel" => "link_label",
        "ListBox" => "list_box",
        "NavigationBar" => "navigation_bar",
        "Panel" => "panel",
        "PlainButton" => "plain_button",
        "ProgressBar" => "progress_bar",
        "RadioButton" => "radio_button",
        "RadioGroup" => "radio_group",
        "ScrollView" => "scroll_view",
        "SearchField" => "search_field",
        "SegmentedControl" => "segmented",
        "Slider" => "slider",
        "Spinner" => "spinner",
        "SplitView" => "split_view",
        "StackPanel" => "stack_panel",
        "StatusIndicator" => "status",
        "Stepper" => "stepper",
        "TabBar" => "tab_bar",
        "TableLayout" => "table_layout",
        "TableView" => "table_view",
        "Tag" => "tag",
        "TextArea" => "text_area",
        "TextEditor" => "text_editor",
        "TextField" => "text_field",
        "TimePicker" => "time_picker",
        "Toggle" => "toggle",
        "Toolbar" => "toolbar",
        "Tooltip" => "tooltip",
        _ => "control",
    }
}

fn import_rust_controls(data: &str, doc: &mut DesignerDocument) {
    for line in data.split('\n') {
        let trimmed = line.trim();
        if let Some((w, h)) = parse_method_u32_2(trimmed, "root", "set_size") {
            doc.width = w;
            doc.height = h;
            continue;
        }
        if let Some((name, kind_name, text)) = parse_rust_control_new(trimmed) {
            if name == "root" || doc.controls.iter().any(|control| control.name == name) {
                continue;
            }
            let kind = DesignerControlKind::from_str(&kind_name);
            let (width, height) = default_control_size(&kind);
            doc.controls.push(DesignerControl {
                name,
                kind,
                text,
                x: 0,
                y: 0,
                width,
                height,
                properties: Vec::new(),
            });
            continue;
        }
        if let Some((control_name, x, y)) = parse_any_method_i32_2(trimmed, "set_position") {
            if let Some(control) = doc
                .controls
                .iter_mut()
                .find(|control| control.name == control_name)
            {
                control.x = x;
                control.y = y;
            }
            continue;
        }
        if let Some((control_name, w, h)) = parse_any_method_u32_2(trimmed, "set_size") {
            if control_name == "root" {
                doc.width = w;
                doc.height = h;
            } else if let Some(control) = doc
                .controls
                .iter_mut()
                .find(|control| control.name == control_name)
            {
                control.width = w;
                control.height = h;
            }
        }
    }
    doc.normalize_layout();
}

fn parse_rust_control_new(line: &str) -> Option<(String, String, String)> {
    let rest = line.strip_prefix("let ")?;
    let eq = rest.find('=')?;
    let name = rest[..eq].trim().trim_start_matches("mut ").trim();
    if !is_valid_control_name(name) {
        return None;
    }
    let rhs = rest[eq + 1..].trim();
    let ui_pos = rhs.find("ui::")? + 4;
    let after_ui = &rhs[ui_pos..];
    let kind_end = after_ui.find("::new")?;
    let kind = rust_type_to_designer_kind(after_ui[..kind_end].trim());
    let text = first_quoted_string(rhs).unwrap_or_default();
    Some((String::from(name), kind, text))
}

fn parse_any_method_i32_2(line: &str, method: &str) -> Option<(String, i32, i32)> {
    let dot = line.find(&format!(".{}(", method))?;
    let name = line[..dot].trim();
    if !is_valid_control_name(name) {
        return None;
    }
    let args = &line[dot + method.len() + 2..];
    let end = args.find(')')?;
    let mut parts = args[..end].split(',');
    let a = parse_i32(parts.next()?.trim())?;
    let b = parse_i32(parts.next()?.trim())?;
    Some((String::from(name), a, b))
}

fn parse_any_method_u32_2(line: &str, method: &str) -> Option<(String, u32, u32)> {
    let dot = line.find(&format!(".{}(", method))?;
    let name = line[..dot].trim();
    if !is_valid_control_name(name) {
        return None;
    }
    let args = &line[dot + method.len() + 2..];
    let end = args.find(')')?;
    let mut parts = args[..end].split(',');
    let a = parse_u32(parts.next()?.trim())?;
    let b = parse_u32(parts.next()?.trim())?;
    Some((String::from(name), a, b))
}

fn parse_method_u32_2(line: &str, receiver: &str, method: &str) -> Option<(u32, u32)> {
    let (name, a, b) = parse_any_method_u32_2(line, method)?;
    if name == receiver {
        Some((a, b))
    } else {
        None
    }
}

fn parse_rust_form_name(data: &str) -> Option<String> {
    for line in data.split('\n') {
        let trimmed = line.trim();
        let Some(rest) = trimmed
            .strip_prefix("pub struct ")
            .or_else(|| trimmed.strip_prefix("struct "))
        else {
            continue;
        };
        let name_end = rest
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        if is_valid_form_name(name) {
            return Some(String::from(name));
        }
    }
    None
}

fn rust_form_name_from_path(file_path: &str) -> String {
    let basename = match file_path.rfind('/') {
        Some(pos) => &file_path[pos + 1..],
        None => file_path,
    };
    let stem = basename.strip_suffix(".rs").unwrap_or(basename);
    let mut out = String::new();
    let mut upper_next = true;
    for b in stem.bytes() {
        match b {
            b'a'..=b'z' if upper_next => {
                out.push((b - 32) as char);
                upper_next = false;
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => {
                out.push(b as char);
                upper_next = false;
            }
            _ => upper_next = true,
        }
    }
    if out.is_empty() {
        String::from("ImportedForm")
    } else {
        out
    }
}

fn sibling_designer_path(file_path: &str, form_name: &str) -> Option<String> {
    let dir = match file_path.rfind('/') {
        Some(pos) => &file_path[..pos],
        None => return None,
    };
    Some(format!("{}/{}.Designer", dir, form_name))
}

fn rust_type_to_designer_kind(type_name: &str) -> String {
    match type_name {
        "Checkbox" => String::from("CheckBox"),
        "View" => String::from("Panel"),
        other => String::from(other),
    }
}

fn first_quoted_string(value: &str) -> Option<String> {
    let start = value.find('"')? + 1;
    let rest = &value[start..];
    let end = rest.find('"')?;
    Some(String::from(&rest[..end]))
}

fn is_valid_control_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return false;
    }
    for (i, b) in trimmed.bytes().enumerate() {
        let valid = match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => true,
            b'0'..=b'9' => i > 0,
            _ => false,
        };
        if !valid {
            return false;
        }
    }
    true
}

fn default_control_size(kind: &DesignerControlKind) -> (u32, u32) {
    match kind {
        DesignerControlKind::Badge
        | DesignerControlKind::Button
        | DesignerControlKind::IconButton
        | DesignerControlKind::ImageButton
        | DesignerControlKind::PlainButton
        | DesignerControlKind::Tag => (104, 30),
        DesignerControlKind::CheckBox
        | DesignerControlKind::RadioButton
        | DesignerControlKind::StatusIndicator
        | DesignerControlKind::Toggle => (132, 26),
        DesignerControlKind::Label
        | DesignerControlKind::LinkLabel
        | DesignerControlKind::Tooltip => (160, 24),
        DesignerControlKind::TextArea | DesignerControlKind::TextEditor => (240, 96),
        DesignerControlKind::DataGrid
        | DesignerControlKind::TableView
        | DesignerControlKind::TreeView => (260, 150),
        DesignerControlKind::Canvas
        | DesignerControlKind::Card
        | DesignerControlKind::FlowPanel
        | DesignerControlKind::GroupBox
        | DesignerControlKind::ImageView
        | DesignerControlKind::Panel
        | DesignerControlKind::ScrollView
        | DesignerControlKind::SplitView
        | DesignerControlKind::StackPanel
        | DesignerControlKind::TableLayout => (240, 140),
        DesignerControlKind::Divider => (180, 1),
        DesignerControlKind::NavigationBar | DesignerControlKind::Toolbar => (320, 36),
        DesignerControlKind::ProgressBar | DesignerControlKind::Slider => (180, 24),
        DesignerControlKind::Spinner => (32, 32),
        _ => (180, 30),
    }
}

fn default_control_text(kind: &DesignerControlKind, name: &str) -> String {
    match kind {
        DesignerControlKind::Button
        | DesignerControlKind::CheckBox
        | DesignerControlKind::Label
        | DesignerControlKind::LinkLabel
        | DesignerControlKind::PlainButton
        | DesignerControlKind::RadioButton
        | DesignerControlKind::Tag
        | DesignerControlKind::Toggle => String::from(kind.as_str()),
        DesignerControlKind::DropDown
        | DesignerControlKind::ListBox
        | DesignerControlKind::RadioGroup
        | DesignerControlKind::SegmentedControl
        | DesignerControlKind::TabBar => String::from("Item 1|Item 2"),
        DesignerControlKind::TextArea
        | DesignerControlKind::TextEditor
        | DesignerControlKind::TextField => String::new(),
        _ => String::from(name),
    }
}

fn rust_control_type(kind: &DesignerControlKind) -> &'static str {
    match kind {
        DesignerControlKind::Alert => "Alert",
        DesignerControlKind::AutoCompleteTextField => "AutoCompleteTextField",
        DesignerControlKind::Badge => "Badge",
        DesignerControlKind::Button => "Button",
        DesignerControlKind::Canvas => "Canvas",
        DesignerControlKind::Card => "Card",
        DesignerControlKind::Label => "Label",
        DesignerControlKind::LinkLabel => "LinkLabel",
        DesignerControlKind::PlainButton => "PlainButton",
        DesignerControlKind::TextField => "TextField",
        DesignerControlKind::TextArea => "TextArea",
        DesignerControlKind::TextEditor => "TextEditor",
        DesignerControlKind::SearchField => "SearchField",
        DesignerControlKind::CheckBox => "Checkbox",
        DesignerControlKind::RadioButton => "RadioButton",
        DesignerControlKind::RadioGroup => "RadioGroup",
        DesignerControlKind::ComboBox => "ComboBox",
        DesignerControlKind::DropDown => "DropDown",
        DesignerControlKind::ListBox => "ListBox",
        DesignerControlKind::TreeView => "TreeView",
        DesignerControlKind::DataGrid => "DataGrid",
        DesignerControlKind::TableView => "TableView",
        DesignerControlKind::ColorWell => "ColorWell",
        DesignerControlKind::DatePicker => "DatePicker",
        DesignerControlKind::DateTimePicker => "DateTimePicker",
        DesignerControlKind::TimePicker => "TimePicker",
        DesignerControlKind::Divider => "Divider",
        DesignerControlKind::Expander => "Expander",
        DesignerControlKind::FlowPanel => "FlowPanel",
        DesignerControlKind::GroupBox => "GroupBox",
        DesignerControlKind::IconButton => "IconButton",
        DesignerControlKind::ImageButton => "ImageButton",
        DesignerControlKind::ImageView => "ImageView",
        DesignerControlKind::NavigationBar => "NavigationBar",
        DesignerControlKind::Panel => "View",
        DesignerControlKind::ProgressBar => "ProgressBar",
        DesignerControlKind::ScrollView => "ScrollView",
        DesignerControlKind::SegmentedControl => "SegmentedControl",
        DesignerControlKind::Slider => "Slider",
        DesignerControlKind::Spinner => "Spinner",
        DesignerControlKind::SplitView => "SplitView",
        DesignerControlKind::StackPanel => "StackPanel",
        DesignerControlKind::StatusIndicator => "StatusIndicator",
        DesignerControlKind::Stepper => "Stepper",
        DesignerControlKind::TabBar => "TabBar",
        DesignerControlKind::TableLayout => "TableLayout",
        DesignerControlKind::Tag => "Tag",
        DesignerControlKind::Toggle => "Toggle",
        DesignerControlKind::Toolbar => "Toolbar",
        DesignerControlKind::Tooltip => "Tooltip",
    }
}

fn control_constructor(control: &DesignerControl) -> String {
    match &control.kind {
        DesignerControlKind::AutoCompleteTextField
        | DesignerControlKind::ComboBox
        | DesignerControlKind::DatePicker
        | DesignerControlKind::DateTimePicker
        | DesignerControlKind::FlowPanel
        | DesignerControlKind::RadioGroup
        | DesignerControlKind::ScrollView
        | DesignerControlKind::SearchField
        | DesignerControlKind::Spinner
        | DesignerControlKind::SplitView
        | DesignerControlKind::Stepper
        | DesignerControlKind::TableView
        | DesignerControlKind::TimePicker
        | DesignerControlKind::Toolbar => format!(
            "        let {} = ui::{}::new();\n",
            control.name,
            rust_control_type(&control.kind)
        ),
        DesignerControlKind::TextArea | DesignerControlKind::TextField => {
            let mut out = format!(
                "        let {} = ui::{}::new();\n",
                control.name,
                rust_control_type(&control.kind)
            );
            if !control.text.is_empty() {
                out.push_str(&format!(
                    "        {}.set_text(\"{}\");\n",
                    control.name,
                    escape(&control.text)
                ));
            }
            out
        }
        DesignerControlKind::Canvas
        | DesignerControlKind::ImageButton
        | DesignerControlKind::ImageView
        | DesignerControlKind::TextEditor
        | DesignerControlKind::TreeView => format!(
            "        let {} = ui::{}::new({}, {});\n",
            control.name,
            rust_control_type(&control.kind),
            control.width,
            control.height
        ),
        DesignerControlKind::ProgressBar | DesignerControlKind::Slider => {
            format!(
                "        let {} = ui::{}::new(0);\n",
                control.name,
                rust_control_type(&control.kind)
            )
        }
        DesignerControlKind::Toggle => {
            format!("        let {} = ui::Toggle::new(false);\n", control.name)
        }
        DesignerControlKind::StackPanel => {
            format!("        let {} = ui::StackPanel::new(0);\n", control.name)
        }
        DesignerControlKind::TableLayout => {
            format!("        let {} = ui::TableLayout::new(2);\n", control.name)
        }
        DesignerControlKind::Panel => format!("        let {} = ui::View::new();\n", control.name),
        DesignerControlKind::Card
        | DesignerControlKind::ColorWell
        | DesignerControlKind::Divider => format!(
            "        let {} = ui::{}::new();\n",
            control.name,
            rust_control_type(&control.kind)
        ),
        DesignerControlKind::DataGrid => format!(
            "        let {} = ui::DataGrid::new(\"Name|Value\");\n",
            control.name
        ),
        DesignerControlKind::DropDown
        | DesignerControlKind::ListBox
        | DesignerControlKind::SegmentedControl
        | DesignerControlKind::TabBar => {
            let items = choice_items(control);
            let items = if items.is_empty() {
                String::from("Item 1|Item 2")
            } else {
                escape(&items)
            };
            format!(
                "        let {} = ui::{}::new(\"{}\");\n",
                control.name,
                rust_control_type(&control.kind),
                items
            )
        }
        _ => format!(
            "        let {} = ui::{}::new(\"{}\");\n",
            control.name,
            rust_control_type(&control.kind),
            escape(&control.text)
        ),
    }
}

fn control_layout_code(control: &DesignerControl) -> String {
    let mut out = String::new();
    for property in &control.properties {
        match normalized_property(&property.name) {
            "dock" => {
                out.push_str(&format!(
                    "        {}.set_dock({});\n",
                    control.name,
                    dock_code(&property.value)
                ));
            }
            "margin" => {
                let (l, t, r, b) = parse_box_edges(&property.value);
                out.push_str(&format!(
                    "        {}.set_margin({}, {}, {}, {});\n",
                    control.name, l, t, r, b
                ));
            }
            "padding" => {
                let (l, t, r, b) = parse_box_edges(&property.value);
                out.push_str(&format!(
                    "        {}.set_padding({}, {}, {}, {});\n",
                    control.name, l, t, r, b
                ));
            }
            "orientation" if supports_orientation(&control.kind) => {
                out.push_str(&format!(
                    "        {}.set_orientation({});\n",
                    control.name,
                    orientation_code(&property.value)
                ));
            }
            "selected_index" | "active_page" => {
                let state = parse_u32(&property.value).unwrap_or(0);
                out.push_str(&format!("        {}.set_state({});\n", control.name, state));
            }
            "value" => {
                let state = parse_u32(&property.value).unwrap_or(0);
                out.push_str(&format!("        {}.set_state({});\n", control.name, state));
            }
            "checked" => {
                let state = if parse_boolish(&property.value).unwrap_or(false) {
                    1
                } else {
                    0
                };
                out.push_str(&format!("        {}.set_state({});\n", control.name, state));
            }
            "enabled" => out.push_str(&format!(
                "        {}.set_enabled({});\n",
                control.name,
                rust_bool_code(&property.value, true)
            )),
            "visible" => out.push_str(&format!(
                "        {}.set_visible({});\n",
                control.name,
                rust_bool_code(&property.value, true)
            )),
            "tooltip" if !property.value.is_empty() => out.push_str(&format!(
                "        {}.set_tooltip(\"{}\");\n",
                control.name,
                escape(&property.value)
            )),
            "font_size" => {
                let size = parse_u32(&property.value).unwrap_or(14);
                out.push_str(&format!(
                    "        {}.set_font_size({});\n",
                    control.name, size
                ));
            }
            "text_color" => out.push_str(&format!(
                "        {}.set_text_color({});\n",
                control.name,
                rust_color_code(&property.value, "0xFFFFFFFF")
            )),
            "background_color" => out.push_str(&format!(
                "        {}.set_color({});\n",
                control.name,
                rust_color_code(&property.value, "0x00000000")
            )),
            "placeholder" if !property.value.is_empty() => {
                if matches!(control.kind, DesignerControlKind::ComboBox) {
                    out.push_str(&format!(
                        "        {}.set_combobox_placeholder(\"{}\");\n",
                        control.name,
                        escape(&property.value)
                    ));
                } else {
                    out.push_str(&format!(
                        "        {}.set_textfield_placeholder(\"{}\");\n",
                        control.name,
                        escape(&property.value)
                    ));
                }
            }
            "read_only" => {
                let value = rust_bool_code(&property.value, false);
                if matches!(control.kind, DesignerControlKind::TextArea) {
                    out.push_str(&format!(
                        "        {}.set_textarea_read_only({});\n",
                        control.name, value
                    ));
                } else {
                    out.push_str(&format!(
                        "        {}.set_textfield_read_only({});\n",
                        control.name, value
                    ));
                }
            }
            "password" => out.push_str(&format!(
                "        {}.set_textfield_password_mode({});\n",
                control.name,
                rust_bool_code(&property.value, false)
            )),
            "max_length" => {
                let value = parse_u32(&property.value).unwrap_or(0);
                if matches!(control.kind, DesignerControlKind::TextArea) {
                    out.push_str(&format!(
                        "        {}.set_textarea_max_length({});\n",
                        control.name, value
                    ));
                } else {
                    out.push_str(&format!(
                        "        {}.set_textfield_max_length({});\n",
                        control.name, value
                    ));
                }
            }
            "items" if !property.value.is_empty() => {
                if matches!(control.kind, DesignerControlKind::ComboBox) {
                    out.push_str(&format!(
                        "        {}.set_combobox_items(\"{}\");\n",
                        control.name,
                        escape(&property.value)
                    ));
                } else {
                    out.push_str(&format!(
                        "        {}.set_text(\"{}\");\n",
                        control.name,
                        escape(&property.value)
                    ));
                }
            }
            _ => {}
        }
    }
    out
}

fn js_control_constructor(control: &DesignerControl) -> String {
    let ty = rust_control_type(&control.kind);
    match &control.kind {
        DesignerControlKind::TextArea | DesignerControlKind::TextField => {
            let mut out = format!("    const {} = new ui.{}();\n", control.name, ty);
            if !control.text.is_empty() {
                out.push_str(&format!(
                    "    {}.setText(\"{}\");\n",
                    control.name,
                    escape(&control.text)
                ));
            }
            out
        }
        DesignerControlKind::Canvas
        | DesignerControlKind::ImageButton
        | DesignerControlKind::ImageView
        | DesignerControlKind::TextEditor
        | DesignerControlKind::TreeView => {
            format!(
                "    const {} = new ui.{}({}, {});\n",
                control.name, ty, control.width, control.height
            )
        }
        DesignerControlKind::ProgressBar | DesignerControlKind::Slider => {
            format!("    const {} = new ui.{}(0);\n", control.name, ty)
        }
        DesignerControlKind::Toggle => {
            format!("    const {} = new ui.Toggle(false);\n", control.name)
        }
        DesignerControlKind::StackPanel => {
            format!("    const {} = new ui.StackPanel(0);\n", control.name)
        }
        DesignerControlKind::TableLayout => {
            format!("    const {} = new ui.TableLayout(2);\n", control.name)
        }
        DesignerControlKind::Panel => format!("    const {} = new ui.View();\n", control.name),
        DesignerControlKind::DropDown
        | DesignerControlKind::ListBox
        | DesignerControlKind::SegmentedControl
        | DesignerControlKind::TabBar => {
            let items = choice_items(control);
            let items = if items.is_empty() {
                String::from("Item 1|Item 2")
            } else {
                escape(&items)
            };
            format!(
                "    const {} = new ui.{}(\"{}\");\n",
                control.name, ty, items
            )
        }
        _ => format!(
            "    const {} = new ui.{}(\"{}\");\n",
            control.name,
            ty,
            escape(&control.text)
        ),
    }
}

fn js_control_layout_code(control: &DesignerControl) -> String {
    let mut out = String::new();
    for property in &control.properties {
        match normalized_property(&property.name) {
            "dock" => out.push_str(&format!(
                "    {}.setDock({});\n",
                control.name,
                js_dock_code(&property.value)
            )),
            "margin" => {
                let (l, t, r, b) = parse_box_edges(&property.value);
                out.push_str(&format!(
                    "    {}.setMargin({}, {}, {}, {});\n",
                    control.name, l, t, r, b
                ));
            }
            "padding" => {
                let (l, t, r, b) = parse_box_edges(&property.value);
                out.push_str(&format!(
                    "    {}.setPadding({}, {}, {}, {});\n",
                    control.name, l, t, r, b
                ));
            }
            "orientation" if supports_orientation(&control.kind) => out.push_str(&format!(
                "    {}.setOrientation({});\n",
                control.name,
                js_orientation_code(&property.value)
            )),
            "selected_index" | "active_page" => {
                let state = parse_u32(&property.value).unwrap_or(0);
                out.push_str(&format!(
                    "    {}.setSelectedIndex({});\n",
                    control.name, state
                ));
            }
            "value" => {
                let state = parse_u32(&property.value).unwrap_or(0);
                out.push_str(&format!("    {}.setState({});\n", control.name, state));
            }
            "checked" => {
                let state = if parse_boolish(&property.value).unwrap_or(false) {
                    1
                } else {
                    0
                };
                out.push_str(&format!("    {}.setState({});\n", control.name, state));
            }
            "enabled" => out.push_str(&format!(
                "    {}.setEnabled({});\n",
                control.name,
                js_bool_code(&property.value, true)
            )),
            "visible" => out.push_str(&format!(
                "    {}.setVisible({});\n",
                control.name,
                js_bool_code(&property.value, true)
            )),
            "tooltip" if !property.value.is_empty() => out.push_str(&format!(
                "    {}.setTooltip(\"{}\");\n",
                control.name,
                escape(&property.value)
            )),
            "font_size" => {
                let size = parse_u32(&property.value).unwrap_or(14);
                out.push_str(&format!("    {}.setFontSize({});\n", control.name, size));
            }
            "text_color" => out.push_str(&format!(
                "    {}.setTextColor(\"{}\");\n",
                control.name,
                js_color_value(&property.value, "#FFFFFFFF")
            )),
            "background_color" => out.push_str(&format!(
                "    {}.setColor(\"{}\");\n",
                control.name,
                js_color_value(&property.value, "#00000000")
            )),
            "placeholder" if !property.value.is_empty() => out.push_str(&format!(
                "    {}.setPlaceholder(\"{}\");\n",
                control.name,
                escape(&property.value)
            )),
            "read_only" => out.push_str(&format!(
                "    {}.setReadOnly({});\n",
                control.name,
                js_bool_code(&property.value, false)
            )),
            "password" => out.push_str(&format!(
                "    {}.setPasswordMode({});\n",
                control.name,
                js_bool_code(&property.value, false)
            )),
            "max_length" => out.push_str(&format!(
                "    {}.setMaxLength({});\n",
                control.name,
                parse_u32(&property.value).unwrap_or(0)
            )),
            "items" if !property.value.is_empty() => out.push_str(&format!(
                "    {}.setItems(\"{}\");\n",
                control.name,
                escape(&property.value)
            )),
            _ => {}
        }
    }
    out
}

fn js_event_hook_method(kind: &str, event_name: &str) -> Option<&'static str> {
    event_hook_method(kind, event_name).map(|method| match method {
        "on_click" => "onClick",
        "on_double_click" => "onDoubleClick",
        "on_text_changed" => "onTextChanged",
        "on_selection_changed" => "onSelectionChanged",
        "on_active_changed" => "onActiveChanged",
        "on_checked_changed" => "onCheckedChanged",
        "on_value_changed" => "onValueChanged",
        "on_changed" => "onChanged",
        "on_color_selected" => "onColorSelected",
        "on_submit" => "onSubmit",
        "on_enter" => "onEnter",
        _ => method,
    })
}

fn supports_orientation(kind: &DesignerControlKind) -> bool {
    matches!(
        kind,
        DesignerControlKind::SplitView | DesignerControlKind::StackPanel
    )
}

fn is_paged_kind(kind: &DesignerControlKind) -> bool {
    matches!(
        kind,
        DesignerControlKind::SegmentedControl | DesignerControlKind::TabBar
    )
}

fn is_addable_container_kind(kind: &DesignerControlKind) -> bool {
    matches!(
        kind,
        DesignerControlKind::Card
            | DesignerControlKind::Expander
            | DesignerControlKind::FlowPanel
            | DesignerControlKind::GroupBox
            | DesignerControlKind::Panel
            | DesignerControlKind::ScrollView
            | DesignerControlKind::SplitView
            | DesignerControlKind::StackPanel
            | DesignerControlKind::TableLayout
    )
}

fn page_index_for_control(control: &DesignerControl, local_x: i32) -> u32 {
    let count = page_count_for_control(control).max(1);
    let tab_width = (control.width as i32 / count as i32).max(1);
    ((local_x.max(0) / tab_width) as u32).min(count.saturating_sub(1))
}

fn page_count_for_control(control: &DesignerControl) -> u32 {
    let items = choice_items(control);
    let mut count = 0u32;
    for item in items.split('|') {
        if !item.trim().is_empty() {
            count = count.saturating_add(1);
        }
    }
    count.max(2)
}

fn choice_items(control: &DesignerControl) -> String {
    control
        .properties
        .iter()
        .find(|property| same_property(&property.name, "Items"))
        .map(|property| property.value.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| control.text.clone())
}

fn default_property_value(kind: &DesignerControlKind, property_name: &str) -> String {
    match normalized_property(property_name) {
        "enabled" | "visible" => String::from("true"),
        "dock" => String::from("None"),
        "padding" | "margin" => String::from("0"),
        "orientation" if supports_orientation(kind) => String::from("Vertical"),
        "selected_index" | "active_page" => String::from("0"),
        "page_height" if is_paged_kind(kind) => String::from("220"),
        "items"
            if matches!(
                kind,
                DesignerControlKind::DropDown
                    | DesignerControlKind::ListBox
                    | DesignerControlKind::RadioGroup
                    | DesignerControlKind::SegmentedControl
                    | DesignerControlKind::TabBar
            ) =>
        {
            String::from("Item 1|Item 2")
        }
        _ => String::new(),
    }
}

fn paged_content_offset_y(control: &DesignerControl) -> i32 {
    control.height as i32 + 8
}

fn paged_content_height(control: &DesignerControl) -> u32 {
    control
        .property_value("PageHeight")
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(220)
}

fn paged_panel_name(parent_name: &str, page_index: u32) -> String {
    format!("{}_page_{}", parent_name, page_index)
}

fn dock_code(value: &str) -> &'static str {
    match normalized_layout_value(value) {
        "top" => "ui::DOCK_TOP",
        "bottom" => "ui::DOCK_BOTTOM",
        "left" => "ui::DOCK_LEFT",
        "right" => "ui::DOCK_RIGHT",
        "fill" => "ui::DOCK_FILL",
        _ => "ui::DOCK_NONE",
    }
}

fn orientation_code(value: &str) -> &'static str {
    match normalized_layout_value(value) {
        "horizontal" => "ui::ORIENTATION_HORIZONTAL",
        _ => "ui::ORIENTATION_VERTICAL",
    }
}

fn js_dock_code(value: &str) -> &'static str {
    match normalized_layout_value(value) {
        "top" => "ui.DOCK_TOP",
        "bottom" => "ui.DOCK_BOTTOM",
        "left" => "ui.DOCK_LEFT",
        "right" => "ui.DOCK_RIGHT",
        "fill" => "ui.DOCK_FILL",
        _ => "ui.DOCK_NONE",
    }
}

fn js_orientation_code(value: &str) -> &'static str {
    match normalized_layout_value(value) {
        "horizontal" => "ui.ORIENTATION_HORIZONTAL",
        _ => "ui.ORIENTATION_VERTICAL",
    }
}

fn parse_box_edges(value: &str) -> (i32, i32, i32, i32) {
    let mut parts = value
        .split(',')
        .map(|part| parse_i32(part.trim()).unwrap_or(0));
    let first = parts.next().unwrap_or(0);
    let second = parts.next();
    let third = parts.next();
    let fourth = parts.next();
    match (second, third, fourth) {
        (Some(v), None, None) => (first, v, first, v),
        (Some(t), Some(r), None) => (first, t, r, t),
        (Some(t), Some(r), Some(b)) => (first, t, r, b),
        _ => (first, first, first, first),
    }
}

fn normalized_layout_value(value: &str) -> &'static str {
    let value = value.trim();
    if value.eq_ignore_ascii_case("top") || value == "1" {
        "top"
    } else if value.eq_ignore_ascii_case("bottom") || value == "2" {
        "bottom"
    } else if value.eq_ignore_ascii_case("left") || value == "3" {
        "left"
    } else if value.eq_ignore_ascii_case("right") || value == "4" {
        "right"
    } else if value.eq_ignore_ascii_case("fill") || value == "5" {
        "fill"
    } else if value.eq_ignore_ascii_case("horizontal") {
        "horizontal"
    } else {
        "none"
    }
}

fn has_property_name(names: &[&str], name: &str) -> bool {
    names.iter().any(|candidate| same_property(candidate, name))
}

fn same_property(a: &str, b: &str) -> bool {
    let na = normalized_property(a);
    let nb = normalized_property(b);
    if na == "custom" || nb == "custom" {
        a.eq_ignore_ascii_case(b)
    } else {
        na == nb
    }
}

fn normalized_property(value: &str) -> &'static str {
    match value {
        "Text" | "text" => "text",
        "X" | "x" => "x",
        "Y" | "y" => "y",
        "Width" | "width" => "width",
        "Height" | "height" => "height",
        "Enabled" | "enabled" => "enabled",
        "Visible" | "visible" => "visible",
        "Tooltip" | "tooltip" => "tooltip",
        "Accent" | "accent" => "accent",
        "Default" | "default" => "default",
        "TextAlign" | "text_align" | "textalign" => "text_align",
        "FontSize" | "font_size" | "fontsize" => "font_size",
        "FontWeight" | "font_weight" | "fontweight" => "font_weight",
        "TextColor" | "text_color" | "textcolor" => "text_color",
        "Placeholder" | "placeholder" => "placeholder",
        "ReadOnly" | "read_only" | "readonly" => "read_only",
        "Password" | "password" => "password",
        "MaxLength" | "max_length" | "maxlength" => "max_length",
        "Value" | "value" => "value",
        "Checked" | "checked" => "checked",
        "Items" | "items" => "items",
        "SelectedIndex" | "selected_index" | "selectedindex" => "selected_index",
        "Dock" | "dock" => "dock",
        "Padding" | "padding" => "padding",
        "Margin" | "margin" => "margin",
        "Orientation" | "orientation" => "orientation",
        "Spacing" | "spacing" => "spacing",
        "Parent" | "parent" => "parent",
        "PageIndex" | "page_index" | "pageindex" => "page_index",
        "ActivePage" | "active_page" | "activepage" => "active_page",
        "PageHeight" | "page_height" | "pageheight" => "page_height",
        "BackgroundColor" | "background_color" | "backgroundcolor" => "background_color",
        "BorderColor" | "border_color" | "bordercolor" => "border_color",
        _ => "custom",
    }
}

fn write_new(path: &str, data: &str) -> Result<(), &'static str> {
    if crate::util::path::exists(path) {
        return Err("Designer file already exists");
    }
    anyos_std::fs::write_bytes(path, data.as_bytes()).map_err(|_| "Could not write designer file")
}

fn infer_target(designer_file_path: &str) -> UiCodeTarget {
    let mut dir = designer_file_path;
    for _ in 0..4 {
        let Some(pos) = dir.rfind('/') else {
            break;
        };
        dir = &dir[..pos];
        if crate::util::path::exists(&format!("{}/package.json", dir)) {
            return UiCodeTarget::Node;
        }
        if crate::util::path::exists(&format!("{}/Cargo.toml", dir)) {
            return UiCodeTarget::Rust;
        }
    }
    UiCodeTarget::Rust
}

fn to_module_name(name: &str) -> String {
    let mut out = String::new();
    for (i, b) in name.bytes().enumerate() {
        let c = match b {
            b'a'..=b'z' | b'0'..=b'9' => b as char,
            b'A'..=b'Z' => {
                if i > 0 {
                    out.push('_');
                }
                (b + 32) as char
            }
            _ => '_',
        };
        if c == '_' && out.ends_with('_') {
            continue;
        }
        out.push(c);
    }
    out.trim_matches('_').into()
}

fn attr(line: &str, name: &str) -> Option<String> {
    let needle = format!("{}=", name);
    let pos = line.find(&needle)? + needle.len();
    let rest = &line[pos..];
    if let Some(rest) = rest.strip_prefix('"') {
        let end = rest.find('"')?;
        Some(String::from(&rest[..end]))
    } else {
        let end = rest.find(' ').unwrap_or(rest.len());
        Some(String::from(&rest[..end]))
    }
}

fn attr_u32(line: &str, name: &str, default: u32) -> u32 {
    attr(line, name)
        .and_then(|v| parse_u32(&v))
        .unwrap_or(default)
}

fn attr_i32(line: &str, name: &str, default: i32) -> i32 {
    attr(line, name)
        .and_then(|v| parse_i32(&v))
        .unwrap_or(default)
}

fn parse_u32(value: &str) -> Option<u32> {
    let mut out = 0u32;
    for b in value.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        out = out.saturating_mul(10).saturating_add((b - b'0') as u32);
    }
    Some(out)
}

fn parse_i32(value: &str) -> Option<i32> {
    if let Some(rest) = value.strip_prefix('-') {
        parse_u32(rest).and_then(|v| {
            if v <= i32::MAX as u32 {
                Some(-(v as i32))
            } else {
                None
            }
        })
    } else {
        parse_u32(value).and_then(|v| {
            if v <= i32::MAX as u32 {
                Some(v as i32)
            } else {
                None
            }
        })
    }
}

fn parse_boolish(value: &str) -> Option<bool> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("yes")
        || value.eq_ignore_ascii_case("on")
        || value == "1"
    {
        Some(true)
    } else if value.eq_ignore_ascii_case("false")
        || value.eq_ignore_ascii_case("no")
        || value.eq_ignore_ascii_case("off")
        || value == "0"
    {
        Some(false)
    } else {
        None
    }
}

fn rust_bool_code(value: &str, default: bool) -> &'static str {
    if parse_boolish(value).unwrap_or(default) {
        "true"
    } else {
        "false"
    }
}

fn js_bool_code(value: &str, default: bool) -> &'static str {
    rust_bool_code(value, default)
}

fn color_hex(value: &str) -> Option<String> {
    validate_argb_color(value).ok()?;
    let trimmed = value.trim();
    Some(String::from(&trimmed[1..]))
}

fn rust_color_code(value: &str, default: &str) -> String {
    color_hex(value)
        .map(|hex| format!("0x{}", hex))
        .unwrap_or_else(|| String::from(default))
}

fn js_color_value(value: &str, default: &str) -> String {
    validate_argb_color(value)
        .map(|_| String::from(value.trim()))
        .unwrap_or_else(|_| String::from(default))
}

fn validate_argb_color(value: &str) -> Result<(), &'static str> {
    let bytes = value.trim().as_bytes();
    if bytes.len() != 9 || bytes[0] != b'#' {
        return Err("Color must use #AARRGGBB");
    }
    for &byte in &bytes[1..] {
        if !byte.is_ascii_hexdigit() {
            return Err("Color must use #AARRGGBB");
        }
    }
    Ok(())
}

fn clamp_u32(value: u32, min: u32, max: u32) -> u32 {
    value.max(min).min(max)
}

fn clamp_i32(value: i32, min: i32, max: i32) -> i32 {
    value.max(min).min(max)
}

fn escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out
}

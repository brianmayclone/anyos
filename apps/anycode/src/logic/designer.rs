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

const MIN_FORM_SIZE: u32 = 160;
const MAX_FORM_SIZE: u32 = 4096;
const MIN_CONTROL_SIZE: u32 = 1;
const MAX_CONTROL_SIZE: u32 = 2048;
const MIN_CONTROL_POS: i32 = -4096;
const MAX_CONTROL_POS: i32 = 4096;

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
            | Self::SegmentedControl
            | Self::TabBar
            | Self::TimePicker => CHOICE_PROPERTIES,
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
        out
    }

    pub fn property_name_at(&self, index: u32) -> String {
        let names = self.kind.property_names();
        let idx = index as usize;
        if idx < names.len() {
            return String::from(names[idx]);
        }
        let custom_idx = idx.saturating_sub(names.len());
        self.properties
            .get(custom_idx)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| String::from("Text"))
    }

    pub fn property_value(&self, property_name: &str) -> String {
        match normalized_property(property_name) {
            "text" => self.text.clone(),
            "x" => format!("{}", self.x),
            "y" => format!("{}", self.y),
            "width" => format!("{}", self.width),
            "height" => format!("{}", self.height),
            _ => self
                .properties
                .iter()
                .find(|property| same_property(&property.name, property_name))
                .map(|property| property.value.clone())
                .unwrap_or_default(),
        }
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
            out.push_str(&format!("        root.add(&{});\n", control.name));
        }
        out.push_str("        Self {\n            root,\n");
        for control in &self.controls {
            out.push_str(&format!("            {},\n", control.name));
        }
        out.push_str("        }\n    }\n}\n");
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
            if matches!(
                control.kind.as_str(),
                "Button" | "IconButton" | "ImageButton" | "LinkLabel" | "PlainButton"
            ) {
                out.push_str(&format!(
                    "        ui.{}.on_click(|_| events::{}());\n",
                    control.name,
                    control.event_name()
                ));
            }
        }
        out.push_str("        Self { ui }\n    }\n\n");
        out.push_str("    pub fn root(&self) -> &libanyui_client::View {\n");
        out.push_str("        &self.ui.root\n");
        out.push_str("    }\n");
        out.push_str("}\n");
        out
    }

    pub fn events_rs(&self) -> String {
        let mut out = String::new();
        for control in &self.controls {
            if matches!(
                control.kind.as_str(),
                "Button" | "IconButton" | "ImageButton" | "LinkLabel" | "PlainButton"
            ) {
                out.push_str(&format!("pub fn {}() {{\n", control.event_name()));
                out.push_str("    // TODO: handle event\n");
                out.push_str("}\n\n");
            }
        }
        out
    }

    pub fn module_rs(&self) -> String {
        format!("mod view;\n\npub use view::{};\n", self.form_name)
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
            _ => control.set_custom_property(property_name, value),
        }
        Ok(())
    }

    pub fn add_control(&mut self, kind_name: &str, x: i32, y: i32) -> Result<String, &'static str> {
        let kind = DesignerControlKind::from_str(kind_name);
        let base_name = default_control_base_name(kind.as_str());
        let name = self.next_control_name(base_name);
        let (width, height) = default_control_size(&kind);
        let text = default_control_text(&kind, &name);
        self.controls.push(DesignerControl {
            name: name.clone(),
            kind,
            text,
            x: x.max(0),
            y: y.max(0),
            width,
            height,
            properties: Vec::new(),
        });
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
        let Some(control) = self
            .controls
            .iter_mut()
            .find(|control| control.name == control_name)
        else {
            return Err("Designer control not found");
        };
        control.x = clamp_i32(x, MIN_CONTROL_POS, MAX_CONTROL_POS);
        control.y = clamp_i32(y, MIN_CONTROL_POS, MAX_CONTROL_POS);
        control.width = clamp_u32(width, MIN_CONTROL_SIZE, MAX_CONTROL_SIZE);
        control.height = clamp_u32(height, MIN_CONTROL_SIZE, MAX_CONTROL_SIZE);
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
}

pub fn is_designer_file(file_path: &str) -> bool {
    file_path.ends_with(".Designer")
}

pub fn load_designer(file_path: &str) -> Option<DesignerDocument> {
    let data = anyos_std::fs::read_to_string(file_path).ok()?;
    Some(DesignerDocument::parse(&data))
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
    let form_dir = match designer_file_path.rfind('/') {
        Some(pos) => &designer_file_path[..pos],
        None => return Err("Invalid designer path"),
    };
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
    Ok(())
}

pub fn create_form_files(project_root: &str, form_name: &str) -> Result<(), &'static str> {
    if !is_valid_form_name(form_name) {
        return Err("Use a valid Rust type name, for example MainForm");
    }
    let ui_dir = format!("{}/src/ui", project_root);
    let _ = anyos_std::fs::mkdir(&format!("{}/src", project_root));
    let _ = anyos_std::fs::mkdir(&ui_dir);

    let module_name = to_module_name(form_name);
    let form_dir = format!("{}/{}", ui_dir, module_name);
    let _ = anyos_std::fs::mkdir(&form_dir);
    let doc = DesignerDocument::default_form(form_name);
    let designer_path = designer_file_path(project_root, form_name);
    let generated_path = format!("{}/designer.rs", form_dir);
    let events_path = format!("{}/events.rs", form_dir);
    let codebehind_path = format!("{}/view.rs", form_dir);
    let module_path = format!("{}/mod.rs", form_dir);

    write_new(&designer_path, &doc.to_designer_metadata())?;
    write_new(&generated_path, &doc.designer_rs())?;
    write_new(&events_path, &doc.events_rs())?;
    write_new(&codebehind_path, &doc.codebehind_rs())?;
    write_new(&module_path, &doc.module_rs())?;
    Ok(())
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
    let events_path = events_file_for_designer(designer_file_path);
    let handler = control.event_name();
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
        if !matches!(
            control.kind.as_str(),
            "Button" | "IconButton" | "ImageButton" | "LinkLabel" | "PlainButton"
        ) {
            continue;
        }
        let handler = control.event_name();
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
    if changed {
        anyos_std::fs::write_bytes(events_path, data.as_bytes())
            .map_err(|_| "Could not update event handlers")?;
    }
    Ok(())
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
            let items = if control.text.is_empty() {
                String::from("Item 1|Item 2")
            } else {
                escape(&control.text)
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
        "Checked" | "checked" => "checked",
        "Dock" | "dock" => "dock",
        "Padding" | "padding" => "padding",
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

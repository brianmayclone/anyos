use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
pub enum DesignerControlKind {
    Button,
    Label,
    TextField,
    CheckBox,
    Panel,
}

impl DesignerControlKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Button => "Button",
            Self::Label => "Label",
            Self::TextField => "TextField",
            Self::CheckBox => "CheckBox",
            Self::Panel => "Panel",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "Label" => Self::Label,
            "TextField" => Self::TextField,
            "CheckBox" => Self::CheckBox,
            "Panel" => Self::Panel,
            _ => Self::Button,
        }
    }
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
}

impl DesignerControl {
    pub fn event_name(&self) -> String {
        format!("{}_click", self.name)
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
        });
        controls.push(DesignerControl {
            name: String::from("button_ok"),
            kind: DesignerControlKind::Button,
            text: String::from("OK"),
            x: 24,
            y: 64,
            width: 96,
            height: 30,
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
                });
            }
        }
        doc
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
            if matches!(control.kind, DesignerControlKind::Button) {
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
            if matches!(control.kind, DesignerControlKind::Button) {
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
}

pub fn is_designer_file(file_path: &str) -> bool {
    file_path.ends_with(".Designer")
}

pub fn load_designer(file_path: &str) -> Option<DesignerDocument> {
    let data = anyos_std::fs::read_to_string(file_path).ok()?;
    Some(DesignerDocument::parse(&data))
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

fn rust_control_type(kind: &DesignerControlKind) -> &'static str {
    match kind {
        DesignerControlKind::Button => "Button",
        DesignerControlKind::Label => "Label",
        DesignerControlKind::TextField => "TextField",
        DesignerControlKind::CheckBox => "Checkbox",
        DesignerControlKind::Panel => "View",
    }
}

fn control_constructor(control: &DesignerControl) -> String {
    match control.kind {
        DesignerControlKind::TextField => {
            let mut out = format!("        let {} = ui::TextField::new();\n", control.name);
            if !control.text.is_empty() {
                out.push_str(&format!(
                    "        {}.set_text(\"{}\");\n",
                    control.name,
                    escape(&control.text)
                ));
            }
            out
        }
        DesignerControlKind::Panel => format!("        let {} = ui::View::new();\n", control.name),
        _ => format!(
            "        let {} = ui::{}::new(\"{}\");\n",
            control.name,
            rust_control_type(&control.kind),
            escape(&control.text)
        ),
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
        parse_u32(rest).map(|v| -(v as i32))
    } else {
        parse_u32(value).map(|v| v as i32)
    }
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

use alloc::format;
use libanyui_client as ui;

use crate::logic::designer::{DesignerControl, DesignerDocument};
use crate::ui::designer_toolbox;

const STYLE_BOLD: u32 = 1;

pub struct InspectorPanel {
    pub panel: ui::View,
    pub property_dropdown: ui::DropDown,
    pub property_value: ui::TextField,
    pub btn_apply_property: ui::Button,
    title: ui::Label,
    subtitle: ui::Label,
    tree: ui::TreeView,
    property_editor: ui::View,
}

impl InspectorPanel {
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.sidebar_bg);

        let header = ui::View::new();
        header.set_dock(ui::DOCK_TOP);
        header.set_size(240, 58);
        header.set_color(tc.sidebar_bg);
        panel.add(&header);

        let title = ui::Label::new("Properties");
        title.set_position(12, 10);
        title.set_size(210, 20);
        title.set_font_size(13);
        title.set_text_color(tc.text);
        header.add(&title);

        let subtitle = ui::Label::new("No selection");
        subtitle.set_position(12, 32);
        subtitle.set_size(210, 16);
        subtitle.set_font_size(10);
        subtitle.set_text_color(tc.text_secondary);
        header.add(&subtitle);

        let tree = ui::TreeView::new(240, 420);
        tree.set_dock(ui::DOCK_FILL);
        tree.set_indent_width(14);
        tree.set_row_height(20);
        panel.add(&tree);

        let property_editor = ui::View::new();
        property_editor.set_dock(ui::DOCK_BOTTOM);
        property_editor.set_size(240, 118);
        property_editor.set_color(tc.sidebar_bg);
        property_editor.set_visible(false);
        panel.add(&property_editor);

        let property_title = ui::Label::new("Edit Property");
        property_title.set_position(12, 8);
        property_title.set_size(210, 18);
        property_title.set_font_size(11);
        property_title.set_text_color(tc.text_secondary);
        property_editor.add(&property_title);

        let property_dropdown = ui::DropDown::new("Text|X|Y|Width|Height");
        property_dropdown.set_position(12, 30);
        property_dropdown.set_size(216, 26);
        property_editor.add(&property_dropdown);

        let property_value = ui::TextField::new();
        property_value.set_position(12, 62);
        property_value.set_size(128, 28);
        property_value.set_color(tc.control_bg);
        property_value.set_text_color(tc.text);
        property_editor.add(&property_value);

        let btn_apply_property = ui::Button::new("Apply");
        btn_apply_property.set_position(148, 62);
        btn_apply_property.set_size(80, 28);
        btn_apply_property.set_color(tc.accent);
        property_editor.add(&btn_apply_property);

        let this = Self {
            panel,
            property_dropdown,
            property_value,
            btn_apply_property,
            title,
            subtitle,
            tree,
            property_editor,
        };
        this.show_empty();
        this
    }

    pub fn show_empty(&self) {
        let tc = ui::theme::colors();
        self.title.set_text("Properties");
        self.subtitle.set_text("No selection");
        self.property_editor.set_visible(false);
        self.tree.clear();
        let root = self.tree.add_root("Inspector");
        self.tree.set_node_style(root, STYLE_BOLD);
        self.tree.set_node_text_color(root, tc.text_secondary);
        self.tree.add_child(root, "Open a UI .Designer file");
        self.tree
            .add_child(root, "or select a designer surface item");
        self.tree.set_expanded(root, true);
    }

    pub fn show_file(&self, file_path: &str) {
        let tc = ui::theme::colors();
        self.title.set_text("Properties");
        self.subtitle.set_text(file_path);
        self.property_editor.set_visible(false);
        self.tree.clear();
        let root = self.tree.add_root("File");
        self.tree.set_node_style(root, STYLE_BOLD);
        self.tree.set_node_text_color(root, tc.text);
        self.tree.add_child(root, &format!("Path: {}", file_path));
        self.tree.set_expanded(root, true);
    }

    pub fn show_designer(&self, doc: &DesignerDocument) {
        let tc = ui::theme::colors();
        self.title.set_text("Designer Properties");
        self.subtitle.set_text(&doc.form_name);
        self.property_editor.set_visible(false);
        self.tree.clear();

        let form = self.tree.add_root(&format!("Form: {}", doc.form_name));
        self.tree.set_node_style(form, STYLE_BOLD);
        self.tree.set_node_text_color(form, tc.text);
        self.tree.add_child(form, &format!("Title: {}", doc.title));
        self.tree
            .add_child(form, &format!("Size: {} x {}", doc.width, doc.height));
        self.tree.set_expanded(form, true);

        let controls = self.tree.add_root("Controls");
        self.tree.set_node_style(controls, STYLE_BOLD);
        self.tree.set_node_text_color(controls, tc.text);
        for control in &doc.controls {
            let node = self.tree.add_child(
                controls,
                &format!("{}: {}", control.name, control.kind.as_str()),
            );
            self.tree
                .add_child(node, &format!("Text: {}", control.text));
            self.tree.add_child(
                node,
                &format!(
                    "Bounds: {}, {}, {}, {}",
                    control.x, control.y, control.width, control.height
                ),
            );
            self.tree
                .add_child(node, &format!("Event: {}", control.event_name()));
        }
        self.tree.set_expanded(controls, true);

        let toolbox = self.tree.add_root("Toolbox");
        self.tree.set_node_style(toolbox, STYLE_BOLD);
        self.tree.set_node_text_color(toolbox, tc.accent);
        designer_toolbox::populate_toolbox_tree(&self.tree, toolbox);
        self.tree.set_expanded(toolbox, true);
    }

    pub fn show_designer_control(&self, doc: &DesignerDocument, control_name: &str) {
        if let Some(control) = doc.controls.iter().find(|c| c.name == control_name) {
            self.show_control_properties(doc, control);
        } else {
            self.show_designer(doc);
        }
    }

    fn show_control_properties(&self, doc: &DesignerDocument, control: &DesignerControl) {
        let tc = ui::theme::colors();
        self.title.set_text("Properties");
        self.subtitle
            .set_text(&format!("{} / {}", doc.form_name, control.name));
        self.property_editor.set_visible(true);
        self.property_dropdown.set_state(0);
        self.property_value.set_text(&control.text);
        self.tree.clear();

        let root = self.tree.add_root(&control.name);
        self.tree.set_node_style(root, STYLE_BOLD);
        self.tree.set_node_text_color(root, tc.accent);
        self.tree
            .add_child(root, &format!("Type: {}", control.kind.as_str()));
        self.tree
            .add_child(root, &format!("Name: {}", control.name));
        self.tree
            .add_child(root, &format!("Text: {}", control.text));
        self.tree.add_child(root, &format!("X: {}", control.x));
        self.tree.add_child(root, &format!("Y: {}", control.y));
        self.tree
            .add_child(root, &format!("Width: {}", control.width));
        self.tree
            .add_child(root, &format!("Height: {}", control.height));
        self.tree.set_expanded(root, true);

        let events = self.tree.add_root("Events");
        self.tree.set_node_style(events, STYLE_BOLD);
        self.tree.set_node_text_color(events, tc.text);
        self.tree
            .add_child(events, &format!("Default: {}", control.event_name()));
        self.tree.set_expanded(events, true);

        let toolbox = self.tree.add_root("Toolbox");
        self.tree.set_node_style(toolbox, STYLE_BOLD);
        self.tree.set_node_text_color(toolbox, tc.text_secondary);
        designer_toolbox::populate_toolbox_tree(&self.tree, toolbox);
    }

    pub fn update_property_value_from_selection(&self, doc: &DesignerDocument, control_name: &str) {
        let Some(control) = doc.controls.iter().find(|c| c.name == control_name) else {
            return;
        };
        let value = match self.property_dropdown.get_state() {
            1 => format!("{}", control.x),
            2 => format!("{}", control.y),
            3 => format!("{}", control.width),
            4 => format!("{}", control.height),
            _ => control.text.clone(),
        };
        self.property_value.set_text(&value);
    }

    pub fn selected_property_name(&self) -> &'static str {
        match self.property_dropdown.get_state() {
            1 => "x",
            2 => "y",
            3 => "width",
            4 => "height",
            _ => "text",
        }
    }

    pub fn property_value_text(&self) -> alloc::string::String {
        let mut buf = [0u8; 512];
        let len = self.property_value.get_text(&mut buf);
        core::str::from_utf8(&buf[..len as usize])
            .unwrap_or("")
            .trim()
            .into()
    }
}

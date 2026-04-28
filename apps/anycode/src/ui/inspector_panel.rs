use alloc::format;
use alloc::string::String;
use libanyui_client as ui;

use crate::logic::designer::{DesignerControl, DesignerDocument};
use crate::logic::storyboard::StoryboardSegue;

const STYLE_BOLD: u32 = 1;

pub struct InspectorPanel {
    pub panel: ui::View,
    pub property_dropdown: ui::DropDown,
    pub property_grid: ui::DataGrid,
    pub event_grid: ui::DataGrid,
    pub property_value: ui::TextField,
    pub btn_apply_property: ui::Button,
    pub btn_pick_color: ui::PlainButton,
    pub btn_delete_control: ui::Button,
    title: ui::Label,
    subtitle: ui::Label,
    tabs: ui::TabBar,
    property_page: ui::View,
    event_page: ui::View,
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

        let tabs = ui::TabBar::new("Properties|Events");
        tabs.set_dock(ui::DOCK_TOP);
        tabs.set_size(240, 30);
        tabs.set_color(tc.toolbar_bg);
        tabs.set_style(ui::STYLE_ACTIVE_BG, tc.sidebar_bg);
        tabs.set_style(ui::STYLE_ACTIVE_TEXT, tc.text);
        tabs.set_style(ui::STYLE_INACTIVE_BG, tc.toolbar_bg);
        tabs.set_style(ui::STYLE_INACTIVE_TEXT, tc.text_secondary);
        tabs.set_style(ui::STYLE_HOVER_BG, tc.control_hover);
        tabs.set_style(ui::STYLE_ACCENT, tc.accent);
        tabs.set_style(ui::STYLE_RADIUS, 3);
        tabs.set_visible(false);
        panel.add(&tabs);

        let property_page = ui::View::new();
        property_page.set_dock(ui::DOCK_FILL);
        property_page.set_color(tc.sidebar_bg);
        property_page.set_visible(false);
        panel.add(&property_page);

        let event_page = ui::View::new();
        event_page.set_dock(ui::DOCK_FILL);
        event_page.set_color(tc.sidebar_bg);
        event_page.set_visible(false);
        panel.add(&event_page);

        let tree = ui::TreeView::new(240, 420);
        tree.set_dock(ui::DOCK_FILL);
        tree.set_indent_width(14);
        tree.set_row_height(20);
        property_page.add(&tree);

        let property_grid = ui::DataGrid::new(240, 320);
        property_grid.set_dock(ui::DOCK_FILL);
        property_grid.set_columns(&[
            ui::ColumnDef::new("Property").width(94),
            ui::ColumnDef::new("Value").width(146),
        ]);
        property_grid.set_row_height(22);
        property_grid.set_header_height(24);
        property_grid.set_selection_mode(ui::SELECTION_SINGLE);
        property_grid.set_editable_columns(1 << 1);
        property_grid.set_visible(false);
        property_page.add(&property_grid);

        let event_grid = ui::DataGrid::new(240, 320);
        event_grid.set_dock(ui::DOCK_FILL);
        event_grid.set_columns(&[
            ui::ColumnDef::new("Event").width(94),
            ui::ColumnDef::new("Handler").width(142),
        ]);
        event_grid.set_row_height(22);
        event_grid.set_header_height(24);
        event_grid.set_selection_mode(ui::SELECTION_SINGLE);
        event_grid.set_editable_columns(1 << 1);
        event_grid.set_visible(true);
        event_page.add(&event_grid);

        let property_editor = ui::View::new();
        property_editor.set_dock(ui::DOCK_BOTTOM);
        property_editor.set_size(240, 42);
        property_editor.set_color(tc.sidebar_bg);
        property_editor.set_visible(false);
        property_page.add(&property_editor);

        let property_title = ui::Label::new("Value");
        property_title.set_position(12, 8);
        property_title.set_size(210, 18);
        property_title.set_font_size(11);
        property_title.set_text_color(tc.text_secondary);
        property_title.set_visible(false);
        property_editor.add(&property_title);

        let property_dropdown = ui::DropDown::new("Text|X|Y|Width|Height");
        property_dropdown.set_position(12, 2);
        property_dropdown.set_size(1, 1);
        property_dropdown.set_visible(false);
        property_editor.add(&property_dropdown);

        let property_value = ui::TextField::new();
        property_value.set_position(12, 30);
        property_value.set_size(216, 28);
        property_value.set_color(tc.control_bg);
        property_value.set_text_color(tc.text);
        property_value.set_visible(false);
        property_editor.add(&property_value);

        let btn_apply_property = ui::Button::new("Apply");
        btn_apply_property.set_position(12, 66);
        btn_apply_property.set_size(104, 28);
        btn_apply_property.set_color(tc.accent);
        btn_apply_property.set_visible(false);
        property_editor.add(&btn_apply_property);

        let btn_pick_color = ui::PlainButton::new("");
        btn_pick_color.set_position(12, 7);
        btn_pick_color.set_size(28, 28);
        btn_pick_color.set_system_icon("palette", ui::IconType::Outline, tc.text, 18);
        btn_pick_color.set_tooltip("Pick color");
        btn_pick_color.set_visible(false);
        property_editor.add(&btn_pick_color);

        let btn_delete_control = ui::Button::new("Delete Control");
        btn_delete_control.set_position(46, 7);
        btn_delete_control.set_size(182, 28);
        btn_delete_control.set_color(0xff7f1d1d);
        property_editor.add(&btn_delete_control);

        let this = Self {
            panel,
            property_dropdown,
            property_grid,
            event_grid,
            property_value,
            btn_apply_property,
            btn_pick_color,
            btn_delete_control,
            title,
            subtitle,
            tabs,
            property_page,
            event_page,
            tree,
            property_editor,
        };
        this.tabs
            .connect_panels(&[&this.property_page, &this.event_page]);
        this.show_empty();
        this
    }

    pub fn show_empty(&self) {
        let tc = ui::theme::colors();
        self.title.set_text("Properties");
        self.subtitle.set_text("No selection");
        self.tabs.set_visible(false);
        self.property_page.set_visible(true);
        self.event_page.set_visible(false);
        self.property_grid.set_visible(false);
        self.property_editor.set_visible(false);
        self.btn_pick_color.set_visible(false);
        self.btn_delete_control.set_text("Delete Control");
        self.btn_delete_control.set_visible(false);
        self.tree.set_dock(ui::DOCK_FILL);
        self.tree.set_visible(true);
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
        self.tabs.set_visible(false);
        self.property_page.set_visible(true);
        self.event_page.set_visible(false);
        self.property_grid.set_visible(false);
        self.property_editor.set_visible(false);
        self.btn_pick_color.set_visible(false);
        self.btn_delete_control.set_text("Delete Control");
        self.btn_delete_control.set_visible(false);
        self.tree.set_dock(ui::DOCK_FILL);
        self.tree.set_visible(true);
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
        self.tabs.set_visible(true);
        self.property_page.set_visible(self.tabs.get_state() == 0);
        self.event_page.set_visible(self.tabs.get_state() == 1);
        self.property_editor.set_visible(false);
        self.property_grid.set_visible(true);
        self.btn_delete_control.set_visible(false);
        self.btn_pick_color.set_visible(false);
        self.btn_delete_control.set_text("Delete Control");
        self.property_dropdown.set_items(&doc.form_property_items());
        self.property_dropdown.set_state(0);
        self.property_value
            .set_text(&doc.form_property_value("Title"));
        self.populate_form_property_grid(doc);
        self.populate_form_event_grid(doc);
        self.tree.set_dock(ui::DOCK_BOTTOM);
        self.tree.set_size(240, 150);
        self.tree.set_visible(true);
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
    }

    pub fn show_designer_control(&self, doc: &DesignerDocument, control_name: &str) {
        if let Some(control) = doc.controls.iter().find(|c| c.name == control_name) {
            self.show_control_properties(doc, control);
        } else {
            self.show_designer(doc);
        }
    }

    pub fn show_storyboard_segue(&self, storyboard_name: &str, segue: &StoryboardSegue) {
        let tc = ui::theme::colors();
        self.title.set_text("Segue Properties");
        self.subtitle
            .set_text(&format!("{} -> {}", segue.from_control, segue.to_form));
        self.tabs.set_visible(false);
        self.property_page.set_visible(true);
        self.event_page.set_visible(false);
        self.property_editor.set_visible(false);
        self.property_grid.set_visible(true);
        self.btn_delete_control.set_text("Delete Segue");
        self.btn_delete_control.set_position(12, 7);
        self.btn_delete_control.set_size(216, 28);
        self.btn_delete_control.set_visible(true);
        self.btn_pick_color.set_visible(false);
        self.tree.set_dock(ui::DOCK_BOTTOM);
        self.tree.set_size(240, 150);
        self.tree.set_visible(true);
        self.populate_storyboard_segue_grid(segue);
        self.tree.clear();

        let root = self.tree.add_root(&format!("Segue: {}", segue.id));
        self.tree.set_node_style(root, STYLE_BOLD);
        self.tree.set_node_text_color(root, tc.accent);
        self.tree
            .add_child(root, &format!("Storyboard: {}", storyboard_name));
        self.tree.add_child(
            root,
            &format!("From: {}.{}", segue.from_form, segue.from_control),
        );
        self.tree
            .add_child(root, &format!("Trigger: {}", segue.trigger_event));
        self.tree.add_child(root, &format!("To: {}", segue.to_form));
        self.tree
            .add_child(root, &format!("Mode: {}", segue.navigation_mode));
        self.tree.set_expanded(root, true);
    }

    fn show_control_properties(&self, doc: &DesignerDocument, control: &DesignerControl) {
        let tc = ui::theme::colors();
        self.title.set_text("Properties");
        self.subtitle
            .set_text(&format!("{} / {}", doc.form_name, control.name));
        self.tabs.set_visible(true);
        self.property_page.set_visible(self.tabs.get_state() == 0);
        self.event_page.set_visible(self.tabs.get_state() == 1);
        self.property_editor.set_visible(true);
        self.property_grid.set_visible(true);
        self.btn_delete_control.set_text("Delete Control");
        self.btn_delete_control.set_visible(true);
        self.tree.set_dock(ui::DOCK_BOTTOM);
        self.tree.set_size(240, 150);
        self.tree.set_visible(true);
        self.property_dropdown.set_items(&control.property_items());
        self.property_dropdown.set_state(0);
        self.property_value
            .set_text(&control.property_value("Text"));
        self.populate_property_grid(control);
        self.populate_control_event_grid(control);
        self.update_property_actions("Text", true);
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
        for property in &control.properties {
            self.tree
                .add_child(root, &format!("{}: {}", property.name, property.value));
        }
        self.tree.set_expanded(root, true);

        let events = self.tree.add_root("Events");
        self.tree.set_node_style(events, STYLE_BOLD);
        self.tree.set_node_text_color(events, tc.text);
        self.tree
            .add_child(events, &format!("Default: {}", control.event_name()));
        self.tree.set_expanded(events, true);
    }

    pub fn update_property_value_from_selection(&self, doc: &DesignerDocument, control_name: &str) {
        let row = self.property_grid.selected_row();
        if row == u32::MAX {
            return;
        }
        self.property_dropdown.set_state(row);
        let (property_name, value) = if control_name.is_empty() {
            let property_name = doc.form_property_name_at(row);
            let value = doc.form_property_value(&property_name);
            (property_name, value)
        } else {
            let property_name = doc.control_property_name_at(control_name, row);
            let value = doc.control_property_value(control_name, &property_name);
            (property_name, value)
        };
        self.property_value.set_text(&value);
        self.update_property_actions(&property_name, !control_name.is_empty());
    }

    pub fn selected_property_index(&self) -> u32 {
        let row = self.property_grid.selected_row();
        if row == u32::MAX {
            self.property_dropdown.get_state()
        } else {
            row
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

    pub fn property_grid_value_text(&self) -> alloc::string::String {
        let row = self.property_grid.selected_row();
        if row == u32::MAX {
            return self.property_value_text();
        }
        let mut buf = [0u8; 512];
        let len = self.property_grid.get_cell(row, 1, &mut buf);
        core::str::from_utf8(&buf[..len as usize])
            .unwrap_or("")
            .trim()
            .into()
    }

    pub fn selected_event_index(&self) -> u32 {
        self.event_grid.selected_row()
    }

    pub fn event_grid_handler_text(&self) -> alloc::string::String {
        let row = self.event_grid.selected_row();
        if row == u32::MAX {
            return String::new();
        }
        let mut buf = [0u8; 512];
        let len = self.event_grid.get_cell(row, 1, &mut buf);
        core::str::from_utf8(&buf[..len as usize])
            .unwrap_or("")
            .trim()
            .into()
    }

    pub fn update_property_actions(&self, property_name: &str, has_control: bool) {
        let is_color = matches!(
            property_name.to_ascii_lowercase().as_str(),
            "textcolor" | "backgroundcolor" | "bordercolor"
        );
        self.property_editor.set_visible(has_control || is_color);
        self.btn_pick_color.set_visible(is_color);
        self.btn_delete_control.set_visible(has_control);
        if has_control {
            self.btn_delete_control
                .set_position(if is_color { 46 } else { 12 }, 7);
            self.btn_delete_control
                .set_size(if is_color { 182 } else { 216 }, 28);
        }
    }

    fn populate_property_grid(&self, control: &DesignerControl) {
        let mut raw = String::new();
        let mut kinds = String::new();
        let mut options = String::new();
        let mut row = 0u32;
        for name in control.property_items().split('|') {
            if row > 0 {
                raw.push('\x1e');
                kinds.push('\x1e');
                options.push('\x1e');
            }
            append_grid_cell(&mut raw, name);
            raw.push('\x1f');
            append_grid_cell(&mut raw, &control.property_value(name));
            kinds.push(editor_kind_code(name));
            options.push_str(editor_options(name));
            row += 1;
        }
        self.property_grid.set_data_raw(raw.as_bytes());
        self.property_grid.set_row_editor_kinds(&kinds);
        self.property_grid.set_row_editor_options(&options);
        if row > 0 {
            self.property_grid.set_selected_row(0);
        }
    }

    fn populate_form_property_grid(&self, doc: &DesignerDocument) {
        let mut raw = String::new();
        let mut kinds = String::new();
        let mut options = String::new();
        let mut row = 0u32;
        for name in doc.form_property_items().split('|') {
            if row > 0 {
                raw.push('\x1e');
                kinds.push('\x1e');
                options.push('\x1e');
            }
            append_grid_cell(&mut raw, name);
            raw.push('\x1f');
            append_grid_cell(&mut raw, &doc.form_property_value(name));
            kinds.push(editor_kind_code(name));
            options.push_str(editor_options(name));
            row += 1;
        }
        self.property_grid.set_data_raw(raw.as_bytes());
        self.property_grid.set_row_editor_kinds(&kinds);
        self.property_grid.set_row_editor_options(&options);
        if row > 0 {
            self.property_grid.set_selected_row(0);
        }
    }

    fn populate_storyboard_segue_grid(&self, segue: &StoryboardSegue) {
        let rows = [
            ("Id", segue.id.as_str(), "0", ""),
            ("FromForm", segue.from_form.as_str(), "0", ""),
            ("FromControl", segue.from_control.as_str(), "0", ""),
            ("TriggerEvent", segue.trigger_event.as_str(), "0", ""),
            ("ToForm", segue.to_form.as_str(), "0", ""),
            ("Condition", segue.condition.as_str(), "0", ""),
            (
                "NavigationMode",
                segue.navigation_mode.as_str(),
                "4",
                "SameWindow|NewWindow|Dialog",
            ),
            ("Handler", segue.handler.as_str(), "0", ""),
        ];
        let mut raw = String::new();
        let mut kinds = String::new();
        let mut options = String::new();
        for (row, (name, value, kind, option)) in rows.iter().enumerate() {
            if row > 0 {
                raw.push('\x1e');
                kinds.push('\x1e');
                options.push('\x1e');
            }
            append_grid_cell(&mut raw, name);
            raw.push('\x1f');
            append_grid_cell(&mut raw, value);
            kinds.push_str(kind);
            options.push_str(option);
        }
        self.property_grid.set_data_raw(raw.as_bytes());
        self.property_grid.set_row_editor_kinds(&kinds);
        self.property_grid.set_row_editor_options(&options);
        self.property_grid.set_selected_row(0);
    }

    fn populate_control_event_grid(&self, control: &DesignerControl) {
        let mut raw = String::new();
        for (row, name) in ["OnClick", "OnDoubleClick", "OnChanged", "OnSubmit"]
            .iter()
            .enumerate()
        {
            if row > 0 {
                raw.push('\x1e');
            }
            append_grid_cell(&mut raw, name);
            raw.push('\x1f');
            append_grid_cell(&mut raw, &control.property_value(name));
        }
        self.event_grid.set_data_raw(raw.as_bytes());
        self.event_grid.set_selected_row(0);
    }

    fn populate_form_event_grid(&self, doc: &DesignerDocument) {
        let mut raw = String::new();
        for (row, name) in doc.form_event_items().split('|').enumerate() {
            if row > 0 {
                raw.push('\x1e');
            }
            append_grid_cell(&mut raw, name);
            raw.push('\x1f');
            append_grid_cell(&mut raw, &doc.form_event_value(name));
        }
        self.event_grid.set_data_raw(raw.as_bytes());
        self.event_grid.set_selected_row(0);
    }
}

fn append_grid_cell(out: &mut String, value: &str) {
    for ch in value.chars() {
        if ch != '\x1e' && ch != '\x1f' {
            out.push(ch);
        }
    }
}

fn editor_kind_code(name: &str) -> char {
    match name.to_ascii_lowercase().as_str() {
        "x" | "y" | "width" | "height" | "fontsize" | "maxlength" | "selectedindex"
        | "activepage" | "pageheight" | "rowheight" | "headerheight" | "indentwidth" | "value"
        | "min" | "max" | "step" => '1',
        "enabled" | "visible" | "readonly" | "password" | "checked" | "interactive" => '2',
        "textcolor" | "backgroundcolor" | "bordercolor" => '3',
        "dock" | "orientation" | "selectionmode" | "scalemode" | "textalign" | "fontweight" => '4',
        _ => '0',
    }
}

fn editor_options(name: &str) -> &'static str {
    match name.to_ascii_lowercase().as_str() {
        "dock" => "None|Top|Bottom|Left|Right|Fill",
        "orientation" => "Vertical|Horizontal",
        "selectionmode" => "Single|Multi",
        "scalemode" => "Fit|Fill|Stretch|Center",
        "textalign" => "Left|Center|Right",
        "fontweight" => "Normal|Bold",
        _ => "",
    }
}

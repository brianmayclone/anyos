use alloc::format;
use libanyui_client as ui;

use crate::logic::designer::DesignerDocument;
use crate::ui::designer_toolbox;

const STYLE_BOLD: u32 = 1;

pub struct InspectorPanel {
    pub panel: ui::View,
    title: ui::Label,
    subtitle: ui::Label,
    tree: ui::TreeView,
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

        let this = Self {
            panel,
            title,
            subtitle,
            tree,
        };
        this.show_empty();
        this
    }

    pub fn show_empty(&self) {
        let tc = ui::theme::colors();
        self.title.set_text("Properties");
        self.subtitle.set_text("No selection");
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
}

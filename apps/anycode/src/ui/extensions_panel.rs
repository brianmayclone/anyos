use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libanyui_client as ui;

use crate::logic::plugin::{PluginCapability, PluginManager, PluginState};

const STYLE_BOLD: u32 = 1;

// ════════════════════════════════════════════════════════════════
//  Extensions panel — browse and manage plugins
// ════════════════════════════════════════════════════════════════

pub struct ExtensionsPanel {
    pub panel: ui::View,
    pub search_field: ui::TextField,
    pub tree: ui::TreeView,
    pub btn_refresh: ui::Button,
    pub detail_label: ui::Label,
    plugin_names: Vec<String>,
}

impl ExtensionsPanel {
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.sidebar_bg);

        let t = anyos_std::i18n::t;

        // Header
        let header = ui::Label::new(t("EXTENSIONS"));
        header.set_dock(ui::DOCK_TOP);
        header.set_size(200, 20);
        header.set_font_size(11);
        header.set_text_color(tc.text_secondary);
        header.set_margin(8, 6, 0, 2);
        panel.add(&header);

        // Search field
        let search_field = ui::TextField::new();
        search_field.set_dock(ui::DOCK_TOP);
        search_field.set_size(200, 28);
        search_field.set_margin(4, 2, 4, 2);
        search_field.set_placeholder(t("Search extensions..."));
        search_field.set_font_size(12);
        search_field.set_color(tc.control_bg);
        search_field.set_text_color(tc.text);
        panel.add(&search_field);

        // Refresh button
        let btn_bar = ui::FlowPanel::new();
        btn_bar.set_dock(ui::DOCK_TOP);
        btn_bar.set_size(200, 30);
        btn_bar.set_color(tc.sidebar_bg);
        panel.add(&btn_bar);

        let btn_refresh = ui::Button::new(t("Refresh"));
        btn_refresh.set_size(65, 24);
        btn_bar.add(&btn_refresh);

        // Tree view for installed extensions
        let tree = ui::TreeView::new(200, 250);
        tree.set_dock(ui::DOCK_FILL);
        tree.set_indent_width(16);
        tree.set_row_height(24);
        panel.add(&tree);

        // Detail label at bottom
        let detail_label = ui::Label::new("");
        detail_label.set_dock(ui::DOCK_BOTTOM);
        detail_label.set_size(200, 40);
        detail_label.set_font_size(11);
        detail_label.set_text_color(tc.text_secondary);
        detail_label.set_margin(8, 4, 0, 4);
        panel.add(&detail_label);

        Self {
            panel,
            search_field,
            tree,
            btn_refresh,
            detail_label,
            plugin_names: Vec::new(),
        }
    }

    /// Update the extension list from the plugin manager.
    pub fn update(&mut self, plugin_mgr: &PluginManager) {
        let tc = ui::theme::colors();
        self.tree.clear();
        self.plugin_names.clear();

        if plugin_mgr.plugins.is_empty() {
            let t = anyos_std::i18n::t;
            let node = self.tree.add_root(t("No extensions installed"));
            self.tree.set_node_text_color(node, tc.text_secondary);
            self.plugin_names.push(String::new());

            let hint = self.tree.add_root(t("Place plugins in:"));
            self.tree.set_node_text_color(hint, tc.text_secondary);
            self.plugin_names.push(String::new());

            let path_node = self.tree.add_root(&plugin_mgr.plugin_dir);
            self.tree.set_node_text_color(path_node, tc.text_secondary);
            self.plugin_names.push(String::new());
            return;
        }

        // Installed section
        let t = anyos_std::i18n::t;
        let installed_label = format!("{} ({})", t("Installed"), plugin_mgr.count());
        let installed_node = self.tree.add_root(&installed_label);
        self.tree.set_node_style(installed_node, STYLE_BOLD);
        self.tree.set_node_text_color(installed_node, tc.text);
        self.plugin_names.push(String::new());
        self.tree.set_expanded(installed_node, true);

        for plugin in &plugin_mgr.plugins {
            let state_badge = match plugin.state {
                PluginState::Active => "\u{2713}",    // ✓
                PluginState::Installed => "\u{2713}", // ✓
                PluginState::Disabled => "\u{2717}",  // ✗
                PluginState::Error => "\u{2716}",     // ✖
            };

            let caps = format_capabilities(&plugin.capabilities);
            let label = format!(
                "{} {} v{}",
                state_badge, plugin.display_name, plugin.version
            );
            let node = self.tree.add_child(installed_node, &label);

            let color = match plugin.state {
                PluginState::Active | PluginState::Installed => tc.success,
                PluginState::Disabled => tc.text_secondary,
                PluginState::Error => tc.destructive,
            };
            self.tree.set_node_text_color(node, color);
            self.plugin_names.push(plugin.name.clone());

            // Add capability sub-items
            if !caps.is_empty() {
                let cap_node = self.tree.add_child(node, &format!("  {}", caps));
                self.tree.set_node_text_color(cap_node, tc.text_secondary);
                self.plugin_names.push(String::new());
            }

            // Show file extensions
            if !plugin.extensions.is_empty() {
                let exts = plugin
                    .extensions
                    .iter()
                    .map(|e| format!(".{}", e))
                    .collect::<Vec<_>>();
                let mut ext_str = String::from("  Files: ");
                for (i, e) in exts.iter().enumerate() {
                    if i > 0 {
                        ext_str.push_str(", ");
                    }
                    ext_str.push_str(e);
                }
                let ext_node = self.tree.add_child(node, &ext_str);
                self.tree.set_node_text_color(ext_node, tc.text_secondary);
                self.plugin_names.push(String::new());
            }
        }
    }

    /// Get the plugin name for a tree node selection.
    pub fn plugin_name_for_node(&self, index: u32) -> Option<&str> {
        self.plugin_names
            .get(index as usize)
            .filter(|s| !s.is_empty())
            .map(|s| s.as_str())
    }

    /// Show detail for the selected extension.
    pub fn show_detail(&self, plugin_mgr: &PluginManager, name: &str) {
        if let Some(plugin) = plugin_mgr.plugins.iter().find(|p| p.name == name) {
            let detail = format!(
                "{}\n{}\nby {}",
                plugin.display_name,
                plugin.description,
                if plugin.author.is_empty() {
                    "Unknown"
                } else {
                    &plugin.author
                },
            );
            self.detail_label.set_text(&detail);
        }
    }
}

fn format_capabilities(caps: &[PluginCapability]) -> String {
    let mut parts = Vec::new();
    for cap in caps {
        let label = match cap {
            PluginCapability::Syntax => "Syntax",
            PluginCapability::Language => "Language",
            PluginCapability::Build => "Build",
            PluginCapability::Theme => "Theme",
            PluginCapability::Formatter => "Formatter",
            PluginCapability::Linter => "Linter",
            PluginCapability::Snippets => "Snippets",
            PluginCapability::ProjectType => "Project",
            PluginCapability::Command => "Command",
            PluginCapability::Panel => "Panel",
            PluginCapability::CodeAction => "Code Action",
            PluginCapability::Test => "Test",
            PluginCapability::AiTool => "AI Tool",
        };
        parts.push(label);
    }
    let mut result = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            result.push_str(", ");
        }
        result.push_str(p);
    }
    result
}

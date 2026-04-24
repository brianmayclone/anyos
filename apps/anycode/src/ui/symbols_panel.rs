use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
#[allow(unused_imports)]
use libanyui_client as ui;

use crate::logic::symbols::{Symbol, SymbolKind};

const STYLE_BOLD: u32 = 1;

// ════════════════════════════════════════════════════════════════
//  Symbols panel — code outline view in the sidebar
// ════════════════════════════════════════════════════════════════

pub struct SymbolsPanel {
    pub panel: ui::View,
    pub tree: ui::TreeView,
    pub filter_field: ui::TextField,
    symbol_lines: Vec<u32>, // line number per tree node
    pub current_file: String,
}

impl SymbolsPanel {
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.sidebar_bg);

        let t = anyos_std::i18n::t;

        // Header
        let header = ui::Label::new(t("OUTLINE"));
        header.set_dock(ui::DOCK_TOP);
        header.set_size(200, 20);
        header.set_font_size(11);
        header.set_text_color(tc.text_secondary);
        header.set_margin(8, 6, 0, 2);
        panel.add(&header);

        // Filter field
        let filter_field = ui::TextField::new();
        filter_field.set_dock(ui::DOCK_TOP);
        filter_field.set_size(200, 26);
        filter_field.set_margin(4, 2, 4, 2);
        filter_field.set_placeholder(t("Filter symbols..."));
        filter_field.set_font_size(11);
        filter_field.set_color(tc.control_bg);
        filter_field.set_text_color(tc.text);
        panel.add(&filter_field);

        // Tree view
        let tree = ui::TreeView::new(200, 400);
        tree.set_dock(ui::DOCK_FILL);
        tree.set_indent_width(12);
        tree.set_row_height(20);
        panel.add(&tree);

        Self {
            panel,
            tree,
            filter_field,
            symbol_lines: Vec::new(),
            current_file: String::new(),
        }
    }

    /// Update the outline from extracted symbols.
    pub fn update(&mut self, symbols: &[Symbol], filename: &str) {
        let tc = ui::theme::colors();
        self.tree.clear();
        self.symbol_lines.clear();
        self.current_file = String::from(filename);

        if symbols.is_empty() {
            let t = anyos_std::i18n::t;
            let node = self.tree.add_root(t("No symbols found"));
            self.tree.set_node_text_color(node, tc.text_secondary);
            self.symbol_lines.push(0);
            return;
        }

        // Group symbols by kind for cleaner display
        let kind_groups: &[(SymbolKind, &str)] = &[
            (SymbolKind::Struct, "Structs"),
            (SymbolKind::Class, "Classes"),
            (SymbolKind::Enum, "Enums"),
            (SymbolKind::Trait, "Traits"),
            (SymbolKind::Interface, "Interfaces"),
            (SymbolKind::Impl, "Implementations"),
        ];

        // Add type definitions in groups
        for (kind, group_name) in kind_groups {
            let group_symbols: Vec<&Symbol> = symbols.iter().filter(|s| s.kind == *kind).collect();

            if group_symbols.is_empty() {
                continue;
            }

            let label = format!("{} ({})", group_name, group_symbols.len());
            let group_node = self.tree.add_root(&label);
            self.tree.set_node_style(group_node, STYLE_BOLD);
            self.tree.set_node_text_color(group_node, kind.icon_color());
            self.symbol_lines.push(0);
            self.tree.set_expanded(group_node, true);

            for sym in &group_symbols {
                let label = format!("{} {}", kind_prefix(sym.kind), sym.name);
                let node = self.tree.add_child(group_node, &label);
                self.tree.set_node_text_color(node, sym.kind.icon_color());
                self.symbol_lines.push(sym.line);

                // Add methods that belong to this type
                if sym.kind == SymbolKind::Impl
                    || sym.kind == SymbolKind::Struct
                    || sym.kind == SymbolKind::Class
                {
                    let methods: Vec<&Symbol> = symbols
                        .iter()
                        .filter(|s| {
                            s.kind == SymbolKind::Method && s.parent.as_deref() == Some(&sym.name)
                        })
                        .collect();
                    for method in &methods {
                        let m_label = format!("  {} {}", kind_prefix(method.kind), method.name);
                        let m_node = self.tree.add_child(node, &m_label);
                        self.tree
                            .set_node_text_color(m_node, method.kind.icon_color());
                        self.symbol_lines.push(method.line);
                    }
                }
            }
        }

        // Add standalone functions (not methods)
        let functions: Vec<&Symbol> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();

        if !functions.is_empty() {
            let label = format!("Functions ({})", functions.len());
            let fn_node = self.tree.add_root(&label);
            self.tree.set_node_style(fn_node, STYLE_BOLD);
            self.tree
                .set_node_text_color(fn_node, SymbolKind::Function.icon_color());
            self.symbol_lines.push(0);
            self.tree.set_expanded(fn_node, true);

            for sym in &functions {
                let label = format!("{} {}", kind_prefix(sym.kind), sym.name);
                let node = self.tree.add_child(fn_node, &label);
                self.tree.set_node_text_color(node, sym.kind.icon_color());
                self.symbol_lines.push(sym.line);
            }
        }

        // Add constants
        let constants: Vec<&Symbol> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .collect();

        if !constants.is_empty() {
            let label = format!("Constants ({})", constants.len());
            let const_node = self.tree.add_root(&label);
            self.tree.set_node_style(const_node, STYLE_BOLD);
            self.tree
                .set_node_text_color(const_node, SymbolKind::Constant.icon_color());
            self.symbol_lines.push(0);

            for sym in &constants {
                let label = format!("{} {}", kind_prefix(sym.kind), sym.name);
                let node = self.tree.add_child(const_node, &label);
                self.tree.set_node_text_color(node, sym.kind.icon_color());
                self.symbol_lines.push(sym.line);
            }
        }

        // Add modules
        let modules: Vec<&Symbol> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Module)
            .collect();

        if !modules.is_empty() {
            let label = format!("Modules ({})", modules.len());
            let mod_node = self.tree.add_root(&label);
            self.tree.set_node_style(mod_node, STYLE_BOLD);
            self.tree
                .set_node_text_color(mod_node, SymbolKind::Module.icon_color());
            self.symbol_lines.push(0);

            for sym in &modules {
                let label = format!("{} {}", kind_prefix(sym.kind), sym.name);
                let node = self.tree.add_child(mod_node, &label);
                self.tree.set_node_text_color(node, sym.kind.icon_color());
                self.symbol_lines.push(sym.line);
            }
        }

        // Add macros
        let macros: Vec<&Symbol> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Macro)
            .collect();

        if !macros.is_empty() {
            let label = format!("Macros ({})", macros.len());
            let macro_node = self.tree.add_root(&label);
            self.tree.set_node_style(macro_node, STYLE_BOLD);
            self.tree
                .set_node_text_color(macro_node, SymbolKind::Macro.icon_color());
            self.symbol_lines.push(0);

            for sym in &macros {
                let label = format!("{} {}", kind_prefix(sym.kind), sym.name);
                let node = self.tree.add_child(macro_node, &label);
                self.tree.set_node_text_color(node, sym.kind.icon_color());
                self.symbol_lines.push(sym.line);
            }
        }
    }

    /// Get the line number for a tree node selection.
    pub fn line_for_node(&self, index: u32) -> Option<u32> {
        self.symbol_lines
            .get(index as usize)
            .copied()
            .filter(|l| *l > 0)
    }

    /// Clear the outline.
    pub fn clear(&mut self) {
        self.tree.clear();
        self.symbol_lines.clear();
        self.current_file.clear();
    }
}

/// Small prefix icon per symbol kind.
fn kind_prefix(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "\u{0192}", // ƒ
        SymbolKind::Method => "\u{0192}",   // ƒ
        SymbolKind::Struct => "S",
        SymbolKind::Enum => "E",
        SymbolKind::Trait => "T",
        SymbolKind::Impl => "I",
        SymbolKind::Class => "C",
        SymbolKind::Interface => "I",
        SymbolKind::Constant => "#",
        SymbolKind::Variable => "$",
        SymbolKind::Module => "M",
        SymbolKind::Import => "\u{2192}", // →
        SymbolKind::Macro => "!",
        SymbolKind::TypeAlias => "T",
        SymbolKind::Field => "\u{2022}", // •
    }
}

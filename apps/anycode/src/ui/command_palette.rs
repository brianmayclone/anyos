use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use libanyui_client as ui;
use ui::Widget;

// ════════════════════════════════════════════════════════════════
//  Command Palette — VS Code-style Ctrl+P quick command launcher
// ════════════════════════════════════════════════════════════════

/// A command in the palette.
#[derive(Clone)]
pub struct PaletteCommand {
    pub id: u32,
    pub label: String,
    pub category: String,
    pub shortcut: String,
}

pub struct CommandPalette {
    pub overlay: ui::View,
    pub input_field: ui::TextField,
    pub list: ui::TreeView,
    commands: Vec<PaletteCommand>,
    filtered: Vec<usize>,
    pub visible: bool,
}

impl CommandPalette {
    pub fn new(parent: &ui::Window) -> Self {
        let tc = ui::theme::colors();

        // Full-window overlay (semi-transparent)
        let overlay = ui::View::new();
        overlay.set_dock(ui::DOCK_FILL);
        overlay.set_color(0x80000000); // semi-transparent black
        overlay.set_visible(false);
        parent.add(&overlay);

        // Palette container (centered box)
        let palette = ui::View::new();
        palette.set_position(200, 40);
        palette.set_size(600, 400);
        palette.set_color(tc.sidebar_bg);
        overlay.add(&palette);

        // Search input
        let input_field = ui::TextField::new();
        input_field.set_dock(ui::DOCK_TOP);
        input_field.set_size(600, 36);
        input_field.set_font(4);
        input_field.set_font_size(14);
        input_field.set_color(tc.control_bg);
        input_field.set_text_color(tc.text);
        input_field.set_placeholder("> Type a command...");
        input_field.set_margin(0, 0, 0, 0);
        palette.add(&input_field);

        // Separator
        let sep = ui::View::new();
        sep.set_dock(ui::DOCK_TOP);
        sep.set_size(600, 1);
        sep.set_color(tc.tab_border_active);
        palette.add(&sep);

        // Command list
        let list = ui::TreeView::new(600, 360);
        list.set_dock(ui::DOCK_FILL);
        list.set_indent_width(0);
        list.set_row_height(28);
        palette.add(&list);

        let mut cp = Self {
            overlay,
            input_field,
            list,
            commands: Vec::new(),
            filtered: Vec::new(),
            visible: false,
        };

        // Register built-in commands
        cp.register_commands();
        cp
    }

    fn register_commands(&mut self) {
        let cmds = [
            (100, "New File", "File", ""),
            (101, "Open Folder", "File", ""),
            (102, "Save", "File", "Ctrl+S"),
            (103, "Save All", "File", "Ctrl+Shift+S"),
            (104, "Close Tab", "File", "Ctrl+W"),
            (110, "Build", "Build", "F7"),
            (111, "Run", "Build", "F5"),
            (112, "Test", "Build", ""),
            (113, "Check", "Build", ""),
            (114, "Clean", "Build", ""),
            (115, "Stop", "Build", "Shift+F5"),
            (120, "Show Explorer", "View", ""),
            (121, "Show Source Control", "View", ""),
            (122, "Show Search", "View", "Ctrl+Shift+F"),
            (123, "Show Run and Debug", "View", ""),
            (124, "Show Outline", "View", ""),
            (125, "Show Extensions", "View", ""),
            (126, "Show AI Assistant", "View", "Ctrl+Shift+A"),
            (127, "Show Output", "View", ""),
            (128, "Show Problems", "View", ""),
            (129, "Show Terminal", "View", "Ctrl+`"),
            (130, "AI: Explain Code", "AI", ""),
            (131, "AI: Refactor Code", "AI", ""),
            (132, "AI: Fix Code", "AI", ""),
            (133, "AI: Generate Code", "AI", ""),
            (134, "AI: Generate Tests", "AI", ""),
            (135, "AI: Review Code", "AI", ""),
            (140, "Toggle Line Numbers", "Editor", ""),
            (141, "Change Language Mode", "Editor", ""),
            (150, "Git: Commit", "Git", ""),
            (151, "Git: Push", "Git", ""),
            (152, "Git: Pull", "Git", ""),
            (153, "Git: Stage All", "Git", ""),
            (160, "Preferences: Open Settings", "Settings", "Ctrl+,"),
            (161, "Preferences: AI Settings", "Settings", ""),
            (199, "About anyOS Code", "Help", ""),
        ];

        for (id, label, category, shortcut) in cmds {
            self.commands.push(PaletteCommand {
                id,
                label: String::from(label),
                category: String::from(category),
                shortcut: String::from(shortcut),
            });
        }
    }

    /// Show the command palette.
    pub fn show(&mut self) {
        self.visible = true;
        self.overlay.set_visible(true);
        self.input_field.set_text("");
        self.input_field.focus();
        self.update_list("");
    }

    /// Hide the command palette.
    pub fn hide(&mut self) {
        self.visible = false;
        self.overlay.set_visible(false);
    }

    /// Toggle visibility.
    pub fn toggle(&mut self) {
        if self.visible {
            self.hide();
        } else {
            self.show();
        }
    }

    /// Update the command list based on filter text.
    pub fn update_list(&mut self, filter: &str) {
        let tc = ui::theme::colors();
        self.list.clear();
        self.filtered.clear();

        let filter_lower = ascii_lower(filter);

        for (i, cmd) in self.commands.iter().enumerate() {
            if !filter.is_empty() {
                let label_lower = ascii_lower(&cmd.label);
                let cat_lower = ascii_lower(&cmd.category);
                if !label_lower.contains(&filter_lower) && !cat_lower.contains(&filter_lower) {
                    continue;
                }
            }

            let display = if cmd.shortcut.is_empty() {
                format!("{}:  {}", cmd.category, cmd.label)
            } else {
                format!("{}:  {}    ({})", cmd.category, cmd.label, cmd.shortcut)
            };

            let node = self.list.add_root(&display);
            self.list.set_node_text_color(node, tc.text);
            self.filtered.push(i);
        }
    }

    /// Get the command ID for the selected list item.
    pub fn selected_command_id(&self) -> Option<u32> {
        let sel = self.list.selected();
        if sel == u32::MAX {
            return None;
        }
        let idx = *self.filtered.get(sel as usize)?;
        Some(self.commands[idx].id)
    }

    /// Get the filter text.
    pub fn get_filter(&self) -> String {
        let mut buf = [0u8; 256];
        let len = self.input_field.get_text(&mut buf);
        match core::str::from_utf8(&buf[..len as usize]) {
            Ok(s) => String::from(s),
            Err(_) => String::new(),
        }
    }
}

fn ascii_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c >= 'A' && c <= 'Z' {
            out.push((c as u8 + 32) as char);
        } else {
            out.push(c);
        }
    }
    out
}

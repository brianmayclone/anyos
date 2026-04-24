use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libanyui_client as ui;

/// A command in the palette.
#[derive(Clone)]
pub struct PaletteCommand {
    pub id: u32,
    pub label: String,
    pub category: String,
    pub shortcut: String,
}

#[derive(Clone)]
struct PaletteFile {
    display: String,
    path: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum PaletteMode {
    Commands,
    Files,
    Projects,
}

pub struct CommandPalette {
    pub overlay: ui::View,
    pub input_field: ui::TextField,
    pub list: ui::TreeView,
    title_label: ui::Label,
    hint_label: ui::Label,
    commands: Vec<PaletteCommand>,
    files: Vec<PaletteFile>,
    projects: Vec<String>,
    filtered: Vec<usize>,
    mode: PaletteMode,
    pub visible: bool,
}

impl CommandPalette {
    pub fn new(parent: &ui::Window) -> Self {
        let tc = ui::theme::colors();

        let overlay = ui::View::new();
        overlay.set_dock(ui::DOCK_FILL);
        overlay.set_color(0x80000000);
        overlay.set_visible(false);
        parent.add(&overlay);

        let palette = ui::View::new();
        palette.set_position(176, 34);
        palette.set_size(672, 430);
        palette.set_color(tc.sidebar_bg);
        overlay.add(&palette);

        let title_bar = ui::View::new();
        title_bar.set_dock(ui::DOCK_TOP);
        title_bar.set_size(672, 30);
        title_bar.set_color(tc.editor_bg);
        palette.add(&title_bar);

        let title_label = ui::Label::new("Command Palette");
        title_label.set_position(12, 8);
        title_label.set_font_size(12);
        title_label.set_text_color(tc.text);
        title_bar.add(&title_label);

        let input_field = ui::TextField::new();
        input_field.set_dock(ui::DOCK_TOP);
        input_field.set_size(672, 38);
        input_field.set_font(4);
        input_field.set_font_size(14);
        input_field.set_color(tc.control_bg);
        input_field.set_text_color(tc.text);
        input_field.set_placeholder("> Type a command...");
        palette.add(&input_field);

        let hint_bar = ui::View::new();
        hint_bar.set_dock(ui::DOCK_TOP);
        hint_bar.set_size(672, 24);
        hint_bar.set_color(tc.sidebar_bg);
        palette.add(&hint_bar);

        let hint_label = ui::Label::new("Ctrl+Shift+P for commands");
        hint_label.set_position(12, 5);
        hint_label.set_font_size(10);
        hint_label.set_text_color(tc.text_secondary);
        hint_bar.add(&hint_label);

        let sep = ui::View::new();
        sep.set_dock(ui::DOCK_TOP);
        sep.set_size(672, 1);
        sep.set_color(tc.tab_border_active);
        palette.add(&sep);

        let list = ui::TreeView::new(672, 336);
        list.set_dock(ui::DOCK_FILL);
        list.set_indent_width(0);
        list.set_row_height(28);
        palette.add(&list);

        let mut cp = Self {
            overlay,
            input_field,
            list,
            title_label,
            hint_label,
            commands: Vec::new(),
            files: Vec::new(),
            projects: Vec::new(),
            filtered: Vec::new(),
            mode: PaletteMode::Commands,
            visible: false,
        };
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
            (105, "Split Editor Right", "View", "Ctrl+\\"),
            (106, "Open Active File to Side", "View", ""),
            (107, "Open Recent Project", "File", ""),
            (110, "Build", "Build", "F7"),
            (111, "Run", "Build", "F5"),
            (112, "Test", "Build", ""),
            (113, "Check", "Build", ""),
            (114, "Clean", "Build", ""),
            (115, "Stop", "Build", "Shift+F5"),
            (116, "Analyze Active File", "Analyze", ""),
            (117, "Restart Live Analysis", "Analyze", ""),
            (118, "Clear Problems", "Analyze", ""),
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

    pub fn show_commands(&mut self) {
        self.mode = PaletteMode::Commands;
        self.visible = true;
        self.overlay.set_visible(true);
        self.title_label.set_text("Command Palette");
        self.hint_label.set_text("Run editor, workspace and AI commands");
        self.input_field.set_placeholder("> Type a command...");
        self.input_field.set_text("");
        self.input_field.focus();
        self.update_list("");
    }

    pub fn show_files(&mut self, root: Option<&str>) {
        self.mode = PaletteMode::Files;
        self.visible = true;
        self.overlay.set_visible(true);
        self.title_label.set_text("Quick Open");
        self.hint_label.set_text("Jump to files in the current workspace");
        self.input_field.set_placeholder("Type a file name or path...");
        self.input_field.set_text("");
        self.files.clear();
        if let Some(root) = root {
            collect_project_files(root, root, &mut self.files, 0, 4000);
        }
        self.input_field.focus();
        self.update_list("");
    }

    pub fn show_recent_projects(&mut self, projects: &[String]) {
        self.mode = PaletteMode::Projects;
        self.visible = true;
        self.overlay.set_visible(true);
        self.title_label.set_text("Open Recent");
        self.hint_label.set_text("Switch between recently used workspaces");
        self.input_field.set_placeholder("Type a workspace path...");
        self.input_field.set_text("");
        self.projects.clear();
        for project in projects {
            self.projects.push(project.clone());
        }
        self.input_field.focus();
        self.update_list("");
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.overlay.set_visible(false);
    }

    pub fn update_list(&mut self, filter: &str) {
        let tc = ui::theme::colors();
        self.list.clear();
        self.filtered.clear();

        let filter_lower = ascii_lower(filter);
        match self.mode {
            PaletteMode::Commands => {
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
            PaletteMode::Files => {
                for (i, file) in self.files.iter().enumerate() {
                    if !filter.is_empty() {
                        let display_lower = ascii_lower(&file.display);
                        let path_lower = ascii_lower(&file.path);
                        if !display_lower.contains(&filter_lower) && !path_lower.contains(&filter_lower) {
                            continue;
                        }
                    }

                    let node = self.list.add_root(&file.display);
                    self.list.set_node_text_color(node, tc.text);
                    self.filtered.push(i);
                }
            }
            PaletteMode::Projects => {
                for (i, project) in self.projects.iter().enumerate() {
                    if !filter.is_empty() {
                        let path_lower = ascii_lower(project);
                        if !path_lower.contains(&filter_lower) {
                            continue;
                        }
                    }
                    let node = self.list.add_root(project);
                    self.list.set_node_text_color(node, tc.text);
                    self.filtered.push(i);
                }
            }
        }
    }

    pub fn selected_command_id(&self) -> Option<u32> {
        if self.mode != PaletteMode::Commands {
            return None;
        }
        let sel = self.list.selected();
        if sel == u32::MAX {
            return None;
        }
        let idx = *self.filtered.get(sel as usize)?;
        Some(self.commands[idx].id)
    }

    pub fn selected_file_path(&self) -> Option<&str> {
        if self.mode != PaletteMode::Files {
            return None;
        }
        let sel = self.list.selected();
        if sel == u32::MAX {
            return None;
        }
        let idx = *self.filtered.get(sel as usize)?;
        Some(self.files.get(idx)?.path.as_str())
    }

    pub fn is_file_mode(&self) -> bool {
        self.mode == PaletteMode::Files
    }

    pub fn is_project_mode(&self) -> bool {
        self.mode == PaletteMode::Projects
    }

    pub fn selected_project_path(&self) -> Option<&str> {
        if self.mode != PaletteMode::Projects {
            return None;
        }
        let sel = self.list.selected();
        if sel == u32::MAX {
            return None;
        }
        let idx = *self.filtered.get(sel as usize)?;
        Some(self.projects.get(idx)?.as_str())
    }

    pub fn get_filter(&self) -> String {
        let mut buf = [0u8; 256];
        let len = self.input_field.get_text(&mut buf);
        match core::str::from_utf8(&buf[..len as usize]) {
            Ok(s) => String::from(s),
            Err(_) => String::new(),
        }
    }
}

fn collect_project_files(
    root: &str,
    dir: &str,
    out: &mut Vec<PaletteFile>,
    depth: u32,
    limit: usize,
) {
    if depth > 10 || out.len() >= limit {
        return;
    }
    let entries = match anyos_std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };

    for entry in entries {
        if entry.name == "." || entry.name == ".." {
            continue;
        }
        let full = if dir.ends_with('/') {
            format!("{}{}", dir, entry.name)
        } else {
            format!("{}/{}", dir, entry.name)
        };
        if entry.is_dir() {
            if entry.name == ".git" || entry.name == "target" || entry.name == "build" {
                continue;
            }
            collect_project_files(root, &full, out, depth + 1, limit);
            if out.len() >= limit {
                return;
            }
        } else {
            let display = relativize(root, &full);
            out.push(PaletteFile { display, path: full });
            if out.len() >= limit {
                return;
            }
        }
    }
}

fn relativize(root: &str, path: &str) -> String {
    if path.starts_with(root) {
        let rel = &path[root.len()..];
        if let Some(stripped) = rel.strip_prefix('/') {
            return String::from(stripped);
        }
    }
    String::from(path)
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

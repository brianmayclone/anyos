use std::path::{Path, PathBuf};
use eframe::egui;
use crate::theme;

pub struct FileBrowserDialog {
    current_dir: PathBuf,
    entries: Vec<DirEntry>,
    selected: Option<usize>,
    filename: String,
    title: String,
    filters: Vec<String>,  // extensions like "iso", "img"
    save_mode: bool,
    pub open: bool,
    pub picked: Option<String>,
    error: Option<String>,
    focus_requested: bool,
}

struct DirEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

impl FileBrowserDialog {
    pub fn new_open(title: &str, filters: &[&str]) -> Self {
        let start = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"));
        let mut dlg = Self {
            current_dir: start,
            entries: Vec::new(),
            selected: None,
            filename: String::new(),
            title: title.to_string(),
            filters: filters.iter().map(|s| s.to_string()).collect(),
            save_mode: false,
            open: true,
            picked: None,
            error: None,
            focus_requested: false,
        };
        dlg.refresh();
        dlg
    }

    pub fn new_save(title: &str, filters: &[&str]) -> Self {
        let mut dlg = Self::new_open(title, filters);
        dlg.save_mode = true;
        dlg
    }

    pub fn new_save_with_name(title: &str, filters: &[&str], default_name: &str) -> Self {
        let mut dlg = Self::new_save(title, filters);
        dlg.filename = default_name.to_string();
        dlg
    }

    fn refresh(&mut self) {
        self.entries.clear();
        self.selected = None;

        // Parent dir entry
        if let Some(parent) = self.current_dir.parent() {
            self.entries.push(DirEntry {
                name: "..".into(),
                path: parent.to_path_buf(),
                is_dir: true,
            });
        }

        if let Ok(read_dir) = std::fs::read_dir(&self.current_dir) {
            let mut dirs = Vec::new();
            let mut files = Vec::new();

            for entry in read_dir.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();

                // Skip hidden files
                if name.starts_with('.') { continue; }

                if path.is_dir() {
                    dirs.push(DirEntry { name, path, is_dir: true });
                } else {
                    // Filter by extension if filters set
                    if !self.filters.is_empty() {
                        let ext = path.extension()
                            .map(|e| e.to_string_lossy().to_lowercase())
                            .unwrap_or_default();
                        if !self.filters.iter().any(|f| f == &ext) {
                            continue;
                        }
                    }
                    files.push(DirEntry { name, path, is_dir: false });
                }
            }

            dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

            self.entries.extend(dirs);
            self.entries.extend(files);
        }
    }

    fn navigate_to(&mut self, path: &Path) {
        self.current_dir = path.to_path_buf();
        self.refresh();
    }

    /// Returns true while open
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if !self.open { return false; }

        let mut still_open = self.open;
        let mut button_close = false;

        let max_h = (ctx.screen_rect().height() - 40.0).max(200.0);

        egui::Window::new(&self.title)
            .open(&mut still_open)
            .collapsible(false)
            .resizable(true)
            .min_width(500.0)
            .max_height(max_h)
            .default_size([550.0, max_h.min(400.0)])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                // Path bar
                ui.horizontal(|ui| {
                    ui.label("Path:");
                    let dir_str = self.current_dir.to_string_lossy().to_string();
                    ui.label(egui::RichText::new(&dir_str).monospace().color(egui::Color32::from_rgb(180, 180, 180)));
                });
                ui.separator();

                // File list
                let avail = ui.available_height() - 70.0; // room for filename + buttons
                egui::ScrollArea::vertical()
                    .max_height(avail.max(100.0))
                    .show(ui, |ui| {
                        let mut navigate_to: Option<PathBuf> = None;

                        for (i, entry) in self.entries.iter().enumerate() {
                            let is_selected = self.selected == Some(i);
                            let icon = if entry.is_dir { "📁" } else { "📄" };
                            let label = format!("{} {}", icon, entry.name);

                            let resp = ui.selectable_label(is_selected, &label);

                            if resp.clicked() {
                                self.selected = Some(i);
                                if !entry.is_dir {
                                    self.filename = entry.name.clone();
                                }
                            }

                            if resp.double_clicked() {
                                if entry.is_dir {
                                    navigate_to = Some(entry.path.clone());
                                } else {
                                    // Double-click file = pick it
                                    self.picked = Some(entry.path.to_string_lossy().to_string());
                                    button_close = true;
                                }
                            }
                        }

                        if let Some(path) = navigate_to {
                            self.navigate_to(&path);
                        }
                    });

                ui.separator();

                // Filename row (always visible in save mode, only when file selected in open mode)
                ui.horizontal(|ui| {
                    ui.label("Filename:");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.filename)
                            .desired_width(ui.available_width() - 10.0)
                    );
                    // Keep focus on the filename field in save mode until user interacts
                    if self.save_mode && !self.focus_requested {
                        // Request focus for the first few frames to ensure it sticks
                        response.request_focus();
                        if response.has_focus() {
                            self.focus_requested = true;
                        }
                    }
                });

                if let Some(err) = &self.error {
                    ui.colored_label(theme::ERROR_RED, err);
                }

                // Buttons
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("Cancel").min_size(egui::vec2(80.0, 28.0))).clicked() {
                            button_close = true;
                        }
                        let ok_label = if self.save_mode { "Save" } else { "Open" };
                        if ui.add(egui::Button::new(ok_label).fill(theme::ACCENT_BLUE).min_size(egui::vec2(80.0, 28.0))).clicked() {
                            if self.filename.is_empty() {
                                self.error = Some("Please enter a filename.".into());
                            } else {
                                let full_path = self.current_dir.join(&self.filename);
                                self.picked = Some(full_path.to_string_lossy().to_string());
                                button_close = true;
                            }
                        }
                    });
                });
            });

        if button_close {
            self.open = false;
        } else {
            self.open = still_open;
        }
        self.open
    }
}

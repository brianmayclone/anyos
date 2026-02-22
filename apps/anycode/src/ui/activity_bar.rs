use libanyui_client as ui;

/// VS Code-style vertical activity bar on the left edge.
pub struct ActivityBar {
    pub panel: ui::View,
    pub btn_files: ui::IconButton,
    pub btn_git: ui::IconButton,
    pub btn_search: ui::IconButton,
    active_index: u32,
}

const BAR_WIDTH: u32 = 40;
const ICON_SIZE: u32 = 32;
const ACTIVE_COLOR: u32 = 0xFFE6E6E6; // bright white for active icon
const INACTIVE_COLOR: u32 = 0xFF8E8E93; // gray for inactive

impl ActivityBar {
    pub fn new() -> Self {
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_LEFT);
        panel.set_size(BAR_WIDTH, 600);
        panel.set_color(0xFF1E1E1E);

        let btn_files = ui::IconButton::new("");
        btn_files.set_size(BAR_WIDTH, ICON_SIZE);
        btn_files.set_dock(ui::DOCK_TOP);
        btn_files.set_icon(ui::ICON_FILES);
        btn_files.set_text_color(ACTIVE_COLOR);
        panel.add(&btn_files);

        let btn_git = ui::IconButton::new("");
        btn_git.set_size(BAR_WIDTH, ICON_SIZE);
        btn_git.set_dock(ui::DOCK_TOP);
        btn_git.set_icon(ui::ICON_GIT_BRANCH);
        btn_git.set_text_color(INACTIVE_COLOR);
        panel.add(&btn_git);

        let btn_search = ui::IconButton::new("");
        btn_search.set_size(BAR_WIDTH, ICON_SIZE);
        btn_search.set_dock(ui::DOCK_TOP);
        btn_search.set_icon(ui::ICON_SEARCH);
        btn_search.set_text_color(INACTIVE_COLOR);
        panel.add(&btn_search);

        Self {
            panel,
            btn_files,
            btn_git,
            btn_search,
            active_index: 0,
        }
    }

    /// Update visual state: highlight active, dim inactive.
    pub fn set_active(&mut self, index: u32) {
        self.active_index = index;
        self.btn_files.set_text_color(if index == 0 { ACTIVE_COLOR } else { INACTIVE_COLOR });
        self.btn_git.set_text_color(if index == 1 { ACTIVE_COLOR } else { INACTIVE_COLOR });
        self.btn_search.set_text_color(if index == 2 { ACTIVE_COLOR } else { INACTIVE_COLOR });
    }
}

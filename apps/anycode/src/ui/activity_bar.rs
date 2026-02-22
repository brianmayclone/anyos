use libanyui_client as ui;

/// VS Code-style vertical activity bar on the left edge.
pub struct ActivityBar {
    pub panel: ui::View,
    pub btn_files: ui::IconButton,
    pub btn_git: ui::IconButton,
    pub btn_search: ui::IconButton,
    /// Thin indicator views (2px blue bar) for each button position.
    indicators: [ui::View; 3],
    active_index: u32,
}

const BAR_WIDTH: u32 = 40;
const ICON_SIZE: u32 = 32;
const ACTIVE_COLOR: u32 = 0xFFE6E6E6; // bright white for active icon
const INACTIVE_COLOR: u32 = 0xFF8E8E93; // gray for inactive
const INDICATOR_COLOR: u32 = 0xFFFFFFFF; // white indicator bar

impl ActivityBar {
    pub fn new() -> Self {
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_LEFT);
        panel.set_size(BAR_WIDTH, 600);
        panel.set_color(0xFF1E1E1E);

        // Files button row
        let row0 = ui::View::new();
        row0.set_dock(ui::DOCK_TOP);
        row0.set_size(BAR_WIDTH, ICON_SIZE);
        let ind0 = ui::View::new();
        ind0.set_dock(ui::DOCK_LEFT);
        ind0.set_size(2, ICON_SIZE);
        ind0.set_color(INDICATOR_COLOR);
        row0.add(&ind0);
        let btn_files = ui::IconButton::new("");
        btn_files.set_size(BAR_WIDTH - 2, ICON_SIZE);
        btn_files.set_dock(ui::DOCK_FILL);
        btn_files.set_icon(ui::ICON_FILES);
        btn_files.set_text_color(ACTIVE_COLOR);
        row0.add(&btn_files);
        panel.add(&row0);

        // Git button row
        let row1 = ui::View::new();
        row1.set_dock(ui::DOCK_TOP);
        row1.set_size(BAR_WIDTH, ICON_SIZE);
        let ind1 = ui::View::new();
        ind1.set_dock(ui::DOCK_LEFT);
        ind1.set_size(2, ICON_SIZE);
        ind1.set_color(0x00000000); // transparent initially
        row1.add(&ind1);
        let btn_git = ui::IconButton::new("");
        btn_git.set_size(BAR_WIDTH - 2, ICON_SIZE);
        btn_git.set_dock(ui::DOCK_FILL);
        btn_git.set_icon(ui::ICON_GIT_BRANCH);
        btn_git.set_text_color(INACTIVE_COLOR);
        row1.add(&btn_git);
        panel.add(&row1);

        // Search button row
        let row2 = ui::View::new();
        row2.set_dock(ui::DOCK_TOP);
        row2.set_size(BAR_WIDTH, ICON_SIZE);
        let ind2 = ui::View::new();
        ind2.set_dock(ui::DOCK_LEFT);
        ind2.set_size(2, ICON_SIZE);
        ind2.set_color(0x00000000); // transparent initially
        row2.add(&ind2);
        let btn_search = ui::IconButton::new("");
        btn_search.set_size(BAR_WIDTH - 2, ICON_SIZE);
        btn_search.set_dock(ui::DOCK_FILL);
        btn_search.set_icon(ui::ICON_SEARCH);
        btn_search.set_text_color(INACTIVE_COLOR);
        row2.add(&btn_search);
        panel.add(&row2);

        Self {
            panel,
            btn_files,
            btn_git,
            btn_search,
            indicators: [ind0, ind1, ind2],
            active_index: 0,
        }
    }

    /// Update visual state: highlight active, dim inactive.
    pub fn set_active(&mut self, index: u32) {
        self.active_index = index;
        // Update icon colors
        self.btn_files.set_text_color(if index == 0 { ACTIVE_COLOR } else { INACTIVE_COLOR });
        self.btn_git.set_text_color(if index == 1 { ACTIVE_COLOR } else { INACTIVE_COLOR });
        self.btn_search.set_text_color(if index == 2 { ACTIVE_COLOR } else { INACTIVE_COLOR });
        // Update indicator bars
        for (i, ind) in self.indicators.iter().enumerate() {
            ind.set_color(if i as u32 == index { INDICATOR_COLOR } else { 0x00000000 });
        }
    }
}

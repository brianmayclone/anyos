use libanyui_client as ui;
use ui::IconType;

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

const BAR_WIDTH: u32 = 48;
const BTN_SIZE: u32 = 40;
const ICON_SZ: u32 = 24;
const ACTIVE_COLOR: u32 = 0xFFE6E6E6; // bright white for active icon
const INACTIVE_COLOR: u32 = 0xFF8E8E93; // gray for inactive
const INDICATOR_COLOR: u32 = 0xFFFFFFFF; // white indicator bar

/// Icon names for the activity bar buttons.
const ICON_NAMES: [&str; 3] = ["files", "git-branch", "search"];

impl ActivityBar {
    pub fn new() -> Self {
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_LEFT);
        panel.set_size(BAR_WIDTH, 600);
        panel.set_color(0xFF1E1E1E);

        // Files button row
        let row0 = ui::View::new();
        row0.set_dock(ui::DOCK_TOP);
        row0.set_size(BAR_WIDTH, BTN_SIZE);
        let ind0 = ui::View::new();
        ind0.set_dock(ui::DOCK_LEFT);
        ind0.set_size(2, BTN_SIZE);
        ind0.set_color(INDICATOR_COLOR);
        row0.add(&ind0);
        let btn_files = ui::IconButton::new("");
        btn_files.set_size(BAR_WIDTH - 2, BTN_SIZE);
        btn_files.set_dock(ui::DOCK_FILL);
        btn_files.set_system_icon(ICON_NAMES[0], IconType::Outline, ACTIVE_COLOR, ICON_SZ);
        row0.add(&btn_files);
        panel.add(&row0);

        // Git button row
        let row1 = ui::View::new();
        row1.set_dock(ui::DOCK_TOP);
        row1.set_size(BAR_WIDTH, BTN_SIZE);
        let ind1 = ui::View::new();
        ind1.set_dock(ui::DOCK_LEFT);
        ind1.set_size(2, BTN_SIZE);
        ind1.set_color(0x00000000);
        row1.add(&ind1);
        let btn_git = ui::IconButton::new("");
        btn_git.set_size(BAR_WIDTH - 2, BTN_SIZE);
        btn_git.set_dock(ui::DOCK_FILL);
        btn_git.set_system_icon(ICON_NAMES[1], IconType::Outline, INACTIVE_COLOR, ICON_SZ);
        row1.add(&btn_git);
        panel.add(&row1);

        // Search button row
        let row2 = ui::View::new();
        row2.set_dock(ui::DOCK_TOP);
        row2.set_size(BAR_WIDTH, BTN_SIZE);
        let ind2 = ui::View::new();
        ind2.set_dock(ui::DOCK_LEFT);
        ind2.set_size(2, BTN_SIZE);
        ind2.set_color(0x00000000);
        row2.add(&ind2);
        let btn_search = ui::IconButton::new("");
        btn_search.set_size(BAR_WIDTH - 2, BTN_SIZE);
        btn_search.set_dock(ui::DOCK_FILL);
        btn_search.set_system_icon(ICON_NAMES[2], IconType::Outline, INACTIVE_COLOR, ICON_SZ);
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
        let btns = [&self.btn_files, &self.btn_git, &self.btn_search];
        for (i, btn) in btns.iter().enumerate() {
            let color = if i as u32 == index { ACTIVE_COLOR } else { INACTIVE_COLOR };
            btn.set_system_icon(ICON_NAMES[i], IconType::Outline, color, ICON_SZ);
        }
        for (i, ind) in self.indicators.iter().enumerate() {
            ind.set_color(if i as u32 == index { INDICATOR_COLOR } else { 0x00000000 });
        }
    }
}

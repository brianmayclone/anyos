use libanyui_client as ui;
use ui::IconType;
use ui::Widget;

/// VS Code-style vertical activity bar on the left edge.
/// Buttons: Files, Git, Search, Run, Outline, Extensions
pub struct ActivityBar {
    pub panel: ui::View,
    pub btn_files: ui::IconButton,
    pub btn_git: ui::IconButton,
    pub btn_search: ui::IconButton,
    pub btn_run: ui::IconButton,
    pub btn_outline: ui::IconButton,
    pub btn_extensions: ui::IconButton,
    /// Thin indicator views (2px bar) for each button position.
    indicators: [ui::View; 6],
    active_index: u32,
}

const BAR_WIDTH: u32 = 48;
const BTN_SIZE: u32 = 40;
const ICON_SZ: u32 = 24;

/// Icon names for the activity bar buttons.
const ICON_NAMES: [&str; 6] = [
    "files",        // 0: Explorer
    "git-branch",   // 1: Source Control
    "search",       // 2: Search
    "player-play",  // 3: Run & Debug
    "list-tree",    // 4: Outline
    "puzzle",       // 5: Extensions
];

/// Tooltip keys for each button.
const TOOLTIP_KEYS: [&str; 6] = [
    "Explorer",
    "Source Control",
    "Search",
    "Run and Debug",
    "Outline",
    "Extensions",
];

impl ActivityBar {
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_LEFT);
        panel.set_size(BAR_WIDTH, 600);
        panel.set_color(tc.window_bg);

        let t = anyos_std::i18n::t;

        let mut buttons: [Option<ui::IconButton>; 6] = [None, None, None, None, None, None];
        let mut indicators: [Option<ui::View>; 6] = [None, None, None, None, None, None];

        for i in 0..6 {
            let row = ui::View::new();
            row.set_dock(ui::DOCK_TOP);
            row.set_size(BAR_WIDTH, BTN_SIZE);

            let ind = ui::View::new();
            ind.set_dock(ui::DOCK_LEFT);
            ind.set_size(2, BTN_SIZE);
            ind.set_color(if i == 0 { tc.check_mark } else { 0x00000000 });
            row.add(&ind);

            let btn = ui::IconButton::new("");
            btn.set_size(BAR_WIDTH - 2, BTN_SIZE);
            btn.set_dock(ui::DOCK_FILL);
            let icon_color = if i == 0 { tc.text } else { tc.text_secondary };
            btn.set_system_icon(ICON_NAMES[i], IconType::Outline, icon_color, ICON_SZ);
            btn.set_tooltip(t(TOOLTIP_KEYS[i]));
            row.add(&btn);

            panel.add(&row);

            buttons[i] = Some(btn);
            indicators[i] = Some(ind);
        }

        // Add separator before extensions (bottom group)
        let spacer = ui::View::new();
        spacer.set_dock(ui::DOCK_FILL);
        spacer.set_color(tc.window_bg);
        // Note: spacer is not added — extensions stays in top section for simplicity

        Self {
            panel,
            btn_files: buttons[0].take().unwrap(),
            btn_git: buttons[1].take().unwrap(),
            btn_search: buttons[2].take().unwrap(),
            btn_run: buttons[3].take().unwrap(),
            btn_outline: buttons[4].take().unwrap(),
            btn_extensions: buttons[5].take().unwrap(),
            indicators: [
                indicators[0].take().unwrap(),
                indicators[1].take().unwrap(),
                indicators[2].take().unwrap(),
                indicators[3].take().unwrap(),
                indicators[4].take().unwrap(),
                indicators[5].take().unwrap(),
            ],
            active_index: 0,
        }
    }

    /// Update visual state: highlight active, dim inactive.
    pub fn set_active(&mut self, index: u32) {
        self.active_index = index;
        let tc = ui::theme::colors();
        let btns = [
            &self.btn_files, &self.btn_git, &self.btn_search,
            &self.btn_run, &self.btn_outline, &self.btn_extensions,
        ];
        for (i, btn) in btns.iter().enumerate() {
            let color = if i as u32 == index { tc.text } else { tc.text_secondary };
            btn.set_system_icon(ICON_NAMES[i], IconType::Outline, color, ICON_SZ);
        }
        for (i, ind) in self.indicators.iter().enumerate() {
            ind.set_color(if i as u32 == index { tc.check_mark } else { 0x00000000 });
        }
    }

    /// Get the number of buttons.
    pub fn button_count(&self) -> u32 {
        6
    }
}

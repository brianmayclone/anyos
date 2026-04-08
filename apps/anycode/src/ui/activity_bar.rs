use libanyui_client as ui;
use ui::IconType;
use ui::Widget;

/// VS Code-style vertical activity bar on the left edge.
/// Uses PlainButton — no border, no fill, only hover highlight.
pub struct ActivityBar {
    pub panel: ui::View,
    pub btn_files: ui::PlainButton,
    pub btn_git: ui::PlainButton,
    pub btn_search: ui::PlainButton,
    pub btn_run: ui::PlainButton,
    pub btn_outline: ui::PlainButton,
    pub btn_ai: ui::PlainButton,
    pub btn_extensions: ui::PlainButton,
    indicators: [ui::View; BUTTON_COUNT],
    active_index: u32,
}

const BAR_WIDTH: u32 = 48;
const BTN_SIZE: u32 = 40;
const ICON_SZ: u32 = 24;
const BUTTON_COUNT: usize = 7;

const ICON_NAMES: [&str; BUTTON_COUNT] = [
    "files",        // 0: Explorer
    "git-branch",   // 1: Source Control
    "search",       // 2: Search
    "player-play",  // 3: Run & Debug
    "list-tree",    // 4: Outline
    "sparkles",     // 5: AI Assistant
    "puzzle",       // 6: Extensions
];

const TOOLTIP_KEYS: [&str; BUTTON_COUNT] = [
    "Explorer",
    "Source Control",
    "Search",
    "Run and Debug",
    "Outline",
    "AI Assistant",
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

        let mut buttons: [Option<ui::PlainButton>; BUTTON_COUNT] = Default::default();
        let mut indicators: [Option<ui::View>; BUTTON_COUNT] = Default::default();

        for i in 0..BUTTON_COUNT {
            let row = ui::View::new();
            row.set_dock(ui::DOCK_TOP);
            row.set_size(BAR_WIDTH, BTN_SIZE);

            let ind = ui::View::new();
            ind.set_dock(ui::DOCK_LEFT);
            ind.set_size(2, BTN_SIZE);
            ind.set_color(if i == 0 { tc.check_mark } else { 0x00000000 });
            row.add(&ind);

            let btn = ui::PlainButton::new("");
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

        Self {
            panel,
            btn_files: buttons[0].take().unwrap(),
            btn_git: buttons[1].take().unwrap(),
            btn_search: buttons[2].take().unwrap(),
            btn_run: buttons[3].take().unwrap(),
            btn_outline: buttons[4].take().unwrap(),
            btn_ai: buttons[5].take().unwrap(),
            btn_extensions: buttons[6].take().unwrap(),
            indicators: [
                indicators[0].take().unwrap(),
                indicators[1].take().unwrap(),
                indicators[2].take().unwrap(),
                indicators[3].take().unwrap(),
                indicators[4].take().unwrap(),
                indicators[5].take().unwrap(),
                indicators[6].take().unwrap(),
            ],
            active_index: 0,
        }
    }

    pub fn set_active(&mut self, index: u32) {
        self.active_index = index;
        let tc = ui::theme::colors();
        let btns: [&ui::PlainButton; BUTTON_COUNT] = [
            &self.btn_files, &self.btn_git, &self.btn_search,
            &self.btn_run, &self.btn_outline, &self.btn_ai, &self.btn_extensions,
        ];
        for (i, btn) in btns.iter().enumerate() {
            let color = if i as u32 == index { tc.text } else { tc.text_secondary };
            btn.set_system_icon(ICON_NAMES[i], IconType::Outline, color, ICON_SZ);
        }
        for (i, ind) in self.indicators.iter().enumerate() {
            ind.set_color(if i as u32 == index { tc.check_mark } else { 0x00000000 });
        }
    }
}

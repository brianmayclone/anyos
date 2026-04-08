use libanyui_client as ui;

use crate::logic::config::Config;

// ════════════════════════════════════════════════════════════════
//  Welcome Tab — shown when no file is open, inside editor panel
// ════════════════════════════════════════════════════════════════

pub struct WelcomeTab {
    pub panel: ui::View,
    pub btn_open_folder: ui::Button,
    pub btn_new_file: ui::Button,
    pub btn_open_recent: ui::Button,
    pub btn_ai_setup: ui::Button,
}

impl WelcomeTab {
    pub fn new(config: &Config) -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.editor_bg);

        let t = anyos_std::i18n::t;

        // ── Title ──
        let hero = ui::View::new();
        hero.set_position(44, 28);
        hero.set_size(820, 82);
        hero.set_color(tc.sidebar_bg);
        panel.add(&hero);

        let title = ui::Label::new("anyOS Code");
        title.set_position(60, 42);
        title.set_font_size((config.font_size + 16).max(24));
        title.set_text_color(tc.text);
        panel.add(&title);

        let subtitle = ui::Label::new("Workspace-first editor for anyOS projects");
        subtitle.set_position(62, 78);
        subtitle.set_font_size(13);
        subtitle.set_text_color(tc.text_secondary);
        panel.add(&subtitle);

        let left_card = ui::View::new();
        left_card.set_position(44, 132);
        left_card.set_size(250, 260);
        left_card.set_color(tc.sidebar_bg);
        panel.add(&left_card);

        let right_card = ui::View::new();
        right_card.set_position(320, 132);
        right_card.set_size(430, 260);
        right_card.set_color(tc.sidebar_bg);
        panel.add(&right_card);

        // ── Start section ──
        let start_label = ui::Label::new(t("Start"));
        start_label.set_position(60, 152);
        start_label.set_font_size(13);
        start_label.set_text_color(tc.text);
        panel.add(&start_label);

        let btn_new_file = ui::Button::new(t("New File"));
        btn_new_file.set_position(60, 178);
        btn_new_file.set_size(190, 30);
        btn_new_file.set_color(tc.control_bg);
        panel.add(&btn_new_file);

        let btn_open_folder = ui::Button::new(t("Open Folder..."));
        btn_open_folder.set_position(60, 216);
        btn_open_folder.set_size(190, 30);
        btn_open_folder.set_color(tc.control_bg);
        panel.add(&btn_open_folder);

        let btn_open_recent = ui::Button::new(t("Open Recent..."));
        btn_open_recent.set_position(60, 254);
        btn_open_recent.set_size(190, 30);
        btn_open_recent.set_color(tc.control_bg);
        panel.add(&btn_open_recent);

        // ── AI section ──
        let ai_label = ui::Label::new(t("AI Assistant"));
        ai_label.set_position(60, 306);
        ai_label.set_font_size(13);
        ai_label.set_text_color(tc.text);
        panel.add(&ai_label);

        let btn_ai_setup = ui::Button::new(t("Configure AI..."));
        btn_ai_setup.set_position(60, 332);
        btn_ai_setup.set_size(190, 30);
        btn_ai_setup.set_color(tc.accent);
        panel.add(&btn_ai_setup);

        // ── Keyboard shortcuts ──
        let kb_label = ui::Label::new(t("Keyboard Shortcuts"));
        kb_label.set_position(60, 418);
        kb_label.set_font_size(13);
        kb_label.set_text_color(tc.text);
        panel.add(&kb_label);

        let shortcuts = ui::Label::new(
            "Ctrl+P       Command Palette\n\
             Ctrl+S       Save\n\
             Ctrl+Shift+F Search in Files\n\
             F5           Run\n\
             F7           Build"
        );
        shortcuts.set_position(60, 442);
        shortcuts.set_font_size(11);
        shortcuts.set_text_color(tc.text_secondary);
        panel.add(&shortcuts);

        // ── Right column: features ──
        let feat_label = ui::Label::new(t("Features"));
        feat_label.set_position(340, 152);
        feat_label.set_font_size(13);
        feat_label.set_text_color(tc.text);
        panel.add(&feat_label);

        let features = ui::Label::new(
            "\u{2713} Multi-language (Rust, C, C++, Python, JS, TS)\n\
             \u{2713} Project detection (Cargo, CMake, Make, npm)\n\
             \u{2713} Automatic task/target discovery\n\
             \u{2713} Symbol outline & navigation\n\
             \u{2713} Compiler diagnostics\n\
             \u{2713} Git integration\n\
             \u{2713} Plugin system\n\
             \u{2713} Project-wide search & replace\n\
             \u{2713} AI coding assistant (OpenAI / Claude)\n\
             \u{2713} Command palette\n\
             \u{2713} Integrated terminal"
        );
        features.set_position(340, 178);
        features.set_font_size(11);
        features.set_text_color(tc.success);
        panel.add(&features);

        Self {
            panel,
            btn_open_folder,
            btn_new_file,
            btn_open_recent,
            btn_ai_setup,
        }
    }

    pub fn show(&self) {
        self.panel.set_visible(true);
    }

    pub fn hide(&self) {
        self.panel.set_visible(false);
    }
}

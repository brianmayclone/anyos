use alloc::format;
use alloc::string::String;

use libanyui_client as ui;

use crate::logic::config::Config;
use crate::util::path;

// Welcome tab shown when no file is open, inside the editor panel.

pub struct WelcomeTab {
    pub panel: ui::View,
    pub btn_open_folder: ui::LinkLabel,
    pub btn_new_file: ui::LinkLabel,
    pub btn_open_recent: ui::LinkLabel,
    pub btn_ai_setup: ui::LinkLabel,
}

impl WelcomeTab {
    pub fn new(config: &Config) -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.editor_bg);

        let t = anyos_std::i18n::t;

        let bg_path = crate::logic::config::bundle_path("assets/bezier_neon_start.jpg");
        let background = ui::ImageView::from_file(&bg_path, 1200, 720);
        background.set_dock(ui::DOCK_FILL);
        background.set_scale_mode(ui::SCALE_FILL);
        panel.add(&background);

        let title = ui::Label::new("anyOS Code");
        title.set_position(56, 46);
        title.set_font_size((config.font_size + 12).max(22));
        title.set_text_color(tc.text);
        panel.add(&title);

        let subtitle = ui::Label::new(t("Open a workspace to start building anyOS software."));
        subtitle.set_position(58, 78);
        subtitle.set_font_size(13);
        subtitle.set_text_color(tc.text_secondary);
        panel.add(&subtitle);

        let divider = ui::View::new();
        divider.set_position(56, 114);
        divider.set_size(760, 1);
        divider.set_color(tc.tab_border_active);
        panel.add(&divider);

        let start_label = ui::Label::new(t("Start"));
        start_label.set_position(58, 146);
        start_label.set_font_size(13);
        start_label.set_text_color(tc.text);
        panel.add(&start_label);

        let btn_open_folder = make_start_link(t("Open a folder or workspace..."));
        btn_open_folder.set_position(58, 174);
        panel.add(&btn_open_folder);

        let btn_new_file = make_start_link(t("Create a new file..."));
        btn_new_file.set_position(58, 202);
        panel.add(&btn_new_file);

        let btn_open_recent = make_start_link(t("Open a recent workspace..."));
        btn_open_recent.set_position(58, 230);
        panel.add(&btn_open_recent);

        let tool_label = ui::Label::new(t("Tools"));
        tool_label.set_position(58, 316);
        tool_label.set_font_size(13);
        tool_label.set_text_color(tc.text);
        panel.add(&tool_label);

        let btn_ai_setup = make_start_link(t("Configure AI assistant..."));
        btn_ai_setup.set_position(58, 344);
        panel.add(&btn_ai_setup);

        let recent_label = ui::Label::new(t("Recent Workspaces"));
        recent_label.set_position(332, 146);
        recent_label.set_font_size(13);
        recent_label.set_text_color(tc.text);
        panel.add(&recent_label);

        let recent = ui::Label::new(&recent_workspace_text(config));
        recent.set_position(332, 176);
        recent.set_font_size(12);
        recent.set_text_color(tc.text_secondary);
        panel.add(&recent);

        let workspace_label = ui::Label::new(t("Workspace"));
        workspace_label.set_position(332, 316);
        workspace_label.set_font_size(13);
        workspace_label.set_text_color(tc.text);
        panel.add(&workspace_label);

        let workspace_hint = ui::Label::new(t("No workspace loaded."));
        workspace_hint.set_position(332, 346);
        workspace_hint.set_font_size(12);
        workspace_hint.set_text_color(tc.text_secondary);
        panel.add(&workspace_hint);

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

fn make_start_link(text: &str) -> ui::LinkLabel {
    let link = ui::LinkLabel::new(text);
    link.set_size(260, 24);
    link.set_font_size(13);
    link
}

fn recent_workspace_text(config: &Config) -> String {
    if config.recent_projects.is_empty() {
        return String::from(anyos_std::i18n::t("No recent workspaces yet."));
    }

    let mut text = String::new();
    for (idx, project) in config.recent_projects.iter().take(5).enumerate() {
        if idx > 0 {
            text.push('\n');
        }
        text.push_str(&format!("{}  {}", path::basename(project), project));
    }
    text
}

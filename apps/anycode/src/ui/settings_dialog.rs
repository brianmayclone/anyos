use alloc::format;
use alloc::string::String;
use libanyui_client as ui;
use ui::Widget;

use crate::logic::ai::{AiConfig, AiProvider};
use crate::logic::config::Config;

const DLG_W: u32 = 740;
const DLG_H: u32 = 540;
const NAV_W: u32 = 176;
const LABEL_X: i32 = 28;
const FIELD_X: i32 = 190;
const FIELD_W: u32 = 340;

pub fn show() {
    let config = Config::load();
    let ai_config = AiConfig::load();
    let tc = ui::theme::colors();
    let t = anyos_std::i18n::t;

    let win = ui::Window::new(t("Settings"), -1, -1, DLG_W, DLG_H);
    let win_id = win.id();

    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(DLG_W, 56);
    header.set_color(tc.sidebar_bg);
    win.add(&header);

    let stripe = ui::View::new();
    stripe.set_dock(ui::DOCK_TOP);
    stripe.set_size(DLG_W, 3);
    stripe.set_color(tc.accent);
    header.add(&stripe);

    let title = ui::Label::new(t("Settings"));
    title.set_position(22, 15);
    title.set_font_size(18);
    title.set_text_color(tc.text);
    header.add(&title);

    let subtitle = ui::Label::new(t(
        "Configure editor, toolchains, AI and workspace behaviour.",
    ));
    subtitle.set_position(22, 37);
    subtitle.set_font_size(11);
    subtitle.set_text_color(tc.text_secondary);
    header.add(&subtitle);

    let btn_bar = ui::View::new();
    btn_bar.set_dock(ui::DOCK_BOTTOM);
    btn_bar.set_size(DLG_W, 56);
    btn_bar.set_color(tc.sidebar_bg);
    win.add(&btn_bar);

    let btn_save = ui::Button::new(t("Save"));
    btn_save.set_size(92, 32);
    btn_save.set_position((DLG_W as i32) - 210, 12);
    btn_save.set_color(tc.accent);
    btn_bar.add(&btn_save);

    let btn_cancel = ui::Button::new(t("Cancel"));
    btn_cancel.set_size(92, 32);
    btn_cancel.set_position((DLG_W as i32) - 106, 12);
    btn_cancel.set_color(tc.control_bg);
    btn_bar.add(&btn_cancel);

    let btn_reset = ui::Button::new(t("Reset"));
    btn_reset.set_size(92, 32);
    btn_reset.set_position(16, 12);
    btn_reset.set_color(tc.control_bg);
    btn_bar.add(&btn_reset);

    let tab_bar = ui::View::new();
    tab_bar.set_dock(ui::DOCK_LEFT);
    tab_bar.set_size(NAV_W, 400);
    tab_bar.set_color(tc.sidebar_bg);
    win.add(&tab_bar);

    let settings_search = ui::TextField::new();
    settings_search.set_dock(ui::DOCK_TOP);
    settings_search.set_size(NAV_W, 34);
    settings_search.set_margin(10, 10, 10, 8);
    settings_search.set_color(tc.control_bg);
    settings_search.set_text_color(tc.text);
    settings_search.set_placeholder(t("Search settings..."));
    tab_bar.add(&settings_search);

    let nav_title = ui::Label::new(t("Categories"));
    nav_title.set_dock(ui::DOCK_TOP);
    nav_title.set_size(NAV_W, 22);
    nav_title.set_font_size(10);
    nav_title.set_text_color(tc.text_secondary);
    nav_title.set_margin(12, 0, 0, 2);
    tab_bar.add(&nav_title);

    let tab_names = [
        t("Text Editor"),
        t("AI Assistant"),
        t("Toolchains"),
        t("Workbench"),
    ];
    let mut tab_hit_btns: [Option<ui::PlainButton>; 4] = [None, None, None, None];
    let mut tab_row_ids: [u32; 4] = [0, 0, 0, 0];
    let mut tab_marker_ids: [u32; 4] = [0, 0, 0, 0];
    let mut tab_label_ids: [u32; 4] = [0, 0, 0, 0];

    for (i, name) in tab_names.iter().enumerate() {
        let selected = i == 0;

        let row = ui::View::new();
        row.set_dock(ui::DOCK_TOP);
        row.set_size(NAV_W, 38);
        row.set_color(if selected {
            tc.editor_bg
        } else {
            tc.sidebar_bg
        });
        row.set_margin(8, 2, 8, 0);
        tab_bar.add(&row);

        let marker = ui::View::new();
        marker.set_position(0, 6);
        marker.set_size(3, 24);
        marker.set_color(if selected { tc.accent } else { tc.sidebar_bg });
        row.add(&marker);

        let label = ui::Label::new(name);
        label.set_position(16, 10);
        label.set_size(NAV_W - 32, 18);
        label.set_font_size(12);
        label.set_text_color(if selected { tc.text } else { tc.text_secondary });
        row.add(&label);

        let hit_btn = ui::PlainButton::new("");
        hit_btn.set_position(0, 0);
        hit_btn.set_size(NAV_W - 16, 38);
        hit_btn.set_tooltip(name);
        row.add(&hit_btn);

        tab_row_ids[i] = row.id();
        tab_marker_ids[i] = marker.id();
        tab_label_ids[i] = label.id();
        tab_hit_btns[i] = Some(hit_btn);
    }

    let content_area = ui::View::new();
    content_area.set_dock(ui::DOCK_FILL);
    content_area.set_color(tc.editor_bg);
    win.add(&content_area);

    // ── Page 0: Editor ──
    let page_editor = ui::View::new();
    page_editor.set_dock(ui::DOCK_FILL);
    page_editor.set_color(tc.editor_bg);
    content_area.add(&page_editor);

    let mut y: i32 = 20;

    let editor_title = ui::Label::new(t("Text Editor"));
    editor_title.set_position(LABEL_X, y);
    editor_title.set_font_size(16);
    editor_title.set_text_color(tc.text);
    page_editor.add(&editor_title);
    y += 24;

    let editor_desc = ui::Label::new(t("Typography, spacing and save behaviour."));
    editor_desc.set_position(LABEL_X, y);
    editor_desc.set_font_size(11);
    editor_desc.set_text_color(tc.text_secondary);
    page_editor.add(&editor_desc);
    y += 34;

    let fs_label = ui::Label::new(t("Font Size"));
    fs_label.set_position(LABEL_X, y + 4);
    fs_label.set_font_size(13);
    fs_label.set_text_color(tc.text);
    page_editor.add(&fs_label);

    let font_size_field = ui::TextField::new();
    font_size_field.set_position(FIELD_X, y);
    font_size_field.set_size(80, 28);
    font_size_field.set_color(tc.control_bg);
    font_size_field.set_text_color(tc.text);
    font_size_field.set_text(&format!("{}", config.font_size));
    page_editor.add(&font_size_field);

    y += 42;

    let tw_label = ui::Label::new(t("Tab Width"));
    tw_label.set_position(LABEL_X, y + 4);
    tw_label.set_font_size(13);
    tw_label.set_text_color(tc.text);
    page_editor.add(&tw_label);

    let tab_width_field = ui::TextField::new();
    tab_width_field.set_position(FIELD_X, y);
    tab_width_field.set_size(80, 28);
    tab_width_field.set_color(tc.control_bg);
    tab_width_field.set_text_color(tc.text);
    tab_width_field.set_text(&format!("{}", config.tab_width));
    page_editor.add(&tab_width_field);

    y += 42;

    let lh_label = ui::Label::new(t("Line Height"));
    lh_label.set_position(LABEL_X, y + 4);
    lh_label.set_font_size(13);
    lh_label.set_text_color(tc.text);
    page_editor.add(&lh_label);

    let line_height_field = ui::TextField::new();
    line_height_field.set_position(FIELD_X, y);
    line_height_field.set_size(80, 28);
    line_height_field.set_color(tc.control_bg);
    line_height_field.set_text_color(tc.text);
    line_height_field.set_text(&format!("{}", config.line_height));
    page_editor.add(&line_height_field);

    y += 42;

    let ln_label = ui::Label::new(t("Line Numbers"));
    ln_label.set_position(LABEL_X, y + 4);
    ln_label.set_font_size(13);
    ln_label.set_text_color(tc.text);
    page_editor.add(&ln_label);

    let line_numbers_toggle = ui::Toggle::new(config.show_line_numbers);
    line_numbers_toggle.set_position(FIELD_X, y);
    line_numbers_toggle.set_size(50, 26);
    page_editor.add(&line_numbers_toggle);

    y += 42;

    let auto_save_label = ui::Label::new(t("Auto Save"));
    auto_save_label.set_position(LABEL_X, y + 4);
    auto_save_label.set_font_size(13);
    auto_save_label.set_text_color(tc.text);
    page_editor.add(&auto_save_label);

    let auto_save_toggle = ui::Toggle::new(config.auto_save);
    auto_save_toggle.set_position(FIELD_X, y);
    auto_save_toggle.set_size(50, 26);
    page_editor.add(&auto_save_toggle);

    // ── Page 1: AI Assistant ──
    let page_ai = ui::View::new();
    page_ai.set_dock(ui::DOCK_FILL);
    page_ai.set_color(tc.editor_bg);
    page_ai.set_visible(false);
    content_area.add(&page_ai);

    y = 20;

    let ai_title = ui::Label::new(t("AI Assistant"));
    ai_title.set_position(LABEL_X, y);
    ai_title.set_font_size(16);
    ai_title.set_text_color(tc.text);
    page_ai.add(&ai_title);
    y += 24;

    let ai_desc = ui::Label::new(t("Provider, model and endpoint used by AI features."));
    ai_desc.set_position(LABEL_X, y);
    ai_desc.set_font_size(11);
    ai_desc.set_text_color(tc.text_secondary);
    page_ai.add(&ai_desc);
    y += 34;

    let prov_label = ui::Label::new(t("Provider"));
    prov_label.set_position(LABEL_X, y + 4);
    prov_label.set_font_size(13);
    prov_label.set_text_color(tc.text);
    page_ai.add(&prov_label);

    let ai_provider = ui::DropDown::new("OpenAI|Anthropic (Claude)");
    ai_provider.set_position(FIELD_X, y);
    ai_provider.set_size(FIELD_W, 28);
    ai_provider.set_state(match ai_config.provider {
        AiProvider::OpenAI => 0,
        AiProvider::Anthropic => 1,
    });
    page_ai.add(&ai_provider);

    y += 42;

    let key_label = ui::Label::new(t("API Key"));
    key_label.set_position(LABEL_X, y + 4);
    key_label.set_font_size(13);
    key_label.set_text_color(tc.text);
    page_ai.add(&key_label);

    let ai_key = ui::TextField::new();
    ai_key.set_position(FIELD_X, y);
    ai_key.set_size(FIELD_W, 28);
    ai_key.set_font(4);
    ai_key.set_font_size(12);
    ai_key.set_color(tc.control_bg);
    ai_key.set_text_color(tc.text);
    ai_key.set_placeholder("sk-... or anthropic key");
    if !ai_config.api_key.is_empty() {
        ai_key.set_text(&ai_config.api_key);
    }
    page_ai.add(&ai_key);

    y += 24;
    let key_hint = ui::Label::new(t("Stored in confd on user level"));
    key_hint.set_position(FIELD_X, y + 4);
    key_hint.set_font_size(10);
    key_hint.set_text_color(tc.text_secondary);
    page_ai.add(&key_hint);

    y += 30;

    let model_label = ui::Label::new(t("Model"));
    model_label.set_position(LABEL_X, y + 4);
    model_label.set_font_size(13);
    model_label.set_text_color(tc.text);
    page_ai.add(&model_label);

    let model_items = "gpt-5.3-codex|gpt-5.4|gpt-5.4-mini|gpt-4o|claude-sonnet-4-20250514|claude-haiku-4-5-20251001|claude-opus-4-20250514";
    let ai_model = ui::DropDown::new(model_items);
    ai_model.set_position(FIELD_X, y);
    ai_model.set_size(FIELD_W, 28);
    ai_model.set_state(find_model_index(model_items, &ai_config.model));
    page_ai.add(&ai_model);

    y += 42;

    let tok_label = ui::Label::new(t("Max Tokens"));
    tok_label.set_position(LABEL_X, y + 4);
    tok_label.set_font_size(13);
    tok_label.set_text_color(tc.text);
    page_ai.add(&tok_label);

    let ai_tokens = ui::TextField::new();
    ai_tokens.set_position(FIELD_X, y);
    ai_tokens.set_size(120, 28);
    ai_tokens.set_color(tc.control_bg);
    ai_tokens.set_text_color(tc.text);
    ai_tokens.set_text(&format!("{}", ai_config.max_tokens));
    page_ai.add(&ai_tokens);

    y += 42;

    let ep_label = ui::Label::new(t("Custom Endpoint"));
    ep_label.set_position(LABEL_X, y + 4);
    ep_label.set_font_size(13);
    ep_label.set_text_color(tc.text);
    page_ai.add(&ep_label);

    let ai_endpoint = ui::TextField::new();
    ai_endpoint.set_position(FIELD_X, y);
    ai_endpoint.set_size(FIELD_W, 28);
    ai_endpoint.set_font(4);
    ai_endpoint.set_font_size(12);
    ai_endpoint.set_color(tc.control_bg);
    ai_endpoint.set_text_color(tc.text);
    ai_endpoint.set_placeholder(t("Leave empty for default"));
    if !ai_config.custom_endpoint.is_empty() {
        ai_endpoint.set_text(&ai_config.custom_endpoint);
    }
    page_ai.add(&ai_endpoint);

    // ── Page 2: Build Tools ──
    let page_build = ui::View::new();
    page_build.set_dock(ui::DOCK_FILL);
    page_build.set_color(tc.editor_bg);
    page_build.set_visible(false);
    content_area.add(&page_build);

    y = 20;

    let build_title = ui::Label::new(t("Toolchains"));
    build_title.set_position(LABEL_X, y);
    build_title.set_font_size(16);
    build_title.set_text_color(tc.text);
    page_build.add(&build_title);
    y += 24;

    let build_desc = ui::Label::new(t(
        "Compilers and build tools used by Build, Run and diagnostics.",
    ));
    build_desc.set_position(LABEL_X, y);
    build_desc.set_font_size(11);
    build_desc.set_text_color(tc.text_secondary);
    page_build.add(&build_desc);
    y += 34;

    let tool_names = [
        (t("Rust Compiler"), config.crust_path.as_str()),
        (t("Cargo Build"), config.ccargo_path.as_str()),
        (t("Make"), config.make_path.as_str()),
        (t("C Compiler"), config.cc_path.as_str()),
        (t("C++ Compiler"), config.cxx_path.as_str()),
        (t("Git"), "/System/bin/cgit"),
    ];
    let mut tool_fields: [Option<ui::TextField>; 6] = [None, None, None, None, None, None];

    for (idx, (name, path)) in tool_names.iter().enumerate() {
        let lbl = ui::Label::new(name);
        lbl.set_position(LABEL_X, y + 4);
        lbl.set_font_size(13);
        lbl.set_text_color(tc.text);
        page_build.add(&lbl);

        let field = ui::TextField::new();
        field.set_position(FIELD_X, y);
        field.set_size(FIELD_W, 28);
        field.set_font(4);
        field.set_font_size(11);
        field.set_color(tc.control_bg);
        field.set_text_color(if path.is_empty() {
            tc.destructive
        } else {
            tc.text
        });
        field.set_placeholder(t("auto-detect"));
        if !path.is_empty() {
            field.set_text(path);
        }
        if idx == 5 {
            field.set_enabled(false);
        }
        page_build.add(&field);
        tool_fields[idx] = Some(field);

        y += 34;
    }

    y += 10;
    let tools_hint = ui::Label::new(t(
        "Empty fields are auto-discovered from PATH and /System/bin/. Git uses /System/bin/cgit.",
    ));
    tools_hint.set_position(LABEL_X, y);
    tools_hint.set_font_size(11);
    tools_hint.set_text_color(tc.text_secondary);
    page_build.add(&tools_hint);

    // ── Page 3: Appearance ──
    let page_appearance = ui::View::new();
    page_appearance.set_dock(ui::DOCK_FILL);
    page_appearance.set_color(tc.editor_bg);
    page_appearance.set_visible(false);
    content_area.add(&page_appearance);

    y = 20;

    let wb_title = ui::Label::new(t("Workbench"));
    wb_title.set_position(LABEL_X, y);
    wb_title.set_font_size(16);
    wb_title.set_text_color(tc.text);
    page_appearance.add(&wb_title);
    y += 24;

    let wb_desc = ui::Label::new(t(
        "Window layout, terminal readability and startup behaviour.",
    ));
    wb_desc.set_position(LABEL_X, y);
    wb_desc.set_font_size(11);
    wb_desc.set_text_color(tc.text_secondary);
    page_appearance.add(&wb_desc);
    y += 34;

    let sw_label = ui::Label::new(t("Sidebar Width %"));
    sw_label.set_position(LABEL_X, y + 4);
    sw_label.set_font_size(13);
    sw_label.set_text_color(tc.text);
    page_appearance.add(&sw_label);

    let sidebar_width_field = ui::TextField::new();
    sidebar_width_field.set_position(FIELD_X, y);
    sidebar_width_field.set_size(80, 28);
    sidebar_width_field.set_color(tc.control_bg);
    sidebar_width_field.set_text_color(tc.text);
    sidebar_width_field.set_text(&format!("{}", config.sidebar_width));
    page_appearance.add(&sidebar_width_field);

    y += 42;

    let oh_label = ui::Label::new(t("Output Height %"));
    oh_label.set_position(LABEL_X, y + 4);
    oh_label.set_font_size(13);
    oh_label.set_text_color(tc.text);
    page_appearance.add(&oh_label);

    let output_height_field = ui::TextField::new();
    output_height_field.set_position(FIELD_X, y);
    output_height_field.set_size(80, 28);
    output_height_field.set_color(tc.control_bg);
    output_height_field.set_text_color(tc.text);
    output_height_field.set_text(&format!("{}", config.output_height));
    page_appearance.add(&output_height_field);

    y += 42;

    let tf_label = ui::Label::new(t("Terminal Font Size"));
    tf_label.set_position(LABEL_X, y + 4);
    tf_label.set_font_size(13);
    tf_label.set_text_color(tc.text);
    page_appearance.add(&tf_label);

    let terminal_font_size_field = ui::TextField::new();
    terminal_font_size_field.set_position(FIELD_X, y);
    terminal_font_size_field.set_size(80, 28);
    terminal_font_size_field.set_color(tc.control_bg);
    terminal_font_size_field.set_text_color(tc.text);
    terminal_font_size_field.set_text(&format!("{}", config.terminal_font_size));
    page_appearance.add(&terminal_font_size_field);

    y += 42;

    let reopen_label = ui::Label::new(t("Reopen Last Project"));
    reopen_label.set_position(LABEL_X, y + 4);
    reopen_label.set_font_size(13);
    reopen_label.set_text_color(tc.text);
    page_appearance.add(&reopen_label);

    let reopen_toggle = ui::Toggle::new(config.reopen_last_project);
    reopen_toggle.set_position(FIELD_X, y);
    reopen_toggle.set_size(50, 26);
    page_appearance.add(&reopen_toggle);

    // ── Page IDs for switching ──
    let page_ids = [
        page_editor.id(),
        page_ai.id(),
        page_build.id(),
        page_appearance.id(),
    ];

    // ── Tab switching ──
    for i in 0..4 {
        let pages = page_ids;
        let rows = tab_row_ids;
        let markers = tab_marker_ids;
        let labels = tab_label_ids;
        let idx = i;
        tab_hit_btns[i].as_ref().unwrap().on_click(move |_| {
            switch_settings_page(pages, rows, markers, labels, idx);
        });
    }

    let pages = page_ids;
    let rows = tab_row_ids;
    let markers = tab_marker_ids;
    let labels = tab_label_ids;
    settings_search.on_text_changed(move |_| {
        let query = read_string(ui::Control::from_id(settings_search.id()).id());
        let q = ascii_lower(&query);
        let idx = if q.contains("ai")
            || q.contains("model")
            || q.contains("token")
            || q.contains("endpoint")
        {
            1
        } else if q.contains("build")
            || q.contains("git")
            || q.contains("tool")
            || q.contains("compiler")
            || q.contains("toolchain")
            || q.contains("rust")
            || q.contains("cargo")
            || q.contains("ccargo")
            || q.contains("c++")
            || q.contains("cpp")
        {
            2
        } else if q.contains("terminal")
            || q.contains("sidebar")
            || q.contains("project")
            || q.contains("startup")
            || q.contains("workbench")
            || q.contains("layout")
        {
            3
        } else {
            0
        };
        switch_settings_page(pages, rows, markers, labels, idx);
    });

    // ── Save ──
    let fs_id = font_size_field.id();
    let tw_id = tab_width_field.id();
    let lh_id = line_height_field.id();
    let ln_id = line_numbers_toggle.id();
    let auto_save_id = auto_save_toggle.id();
    let sw_id = sidebar_width_field.id();
    let oh_id = output_height_field.id();
    let term_fs_id = terminal_font_size_field.id();
    let reopen_id = reopen_toggle.id();
    let ai_prov_id = ai_provider.id();
    let ai_key_id = ai_key.id();
    let ai_model_id = ai_model.id();
    let ai_tok_id = ai_tokens.id();
    let ai_ep_id = ai_endpoint.id();
    let crust_id = tool_fields[0].as_ref().unwrap().id();
    let ccargo_id = tool_fields[1].as_ref().unwrap().id();
    let make_id = tool_fields[2].as_ref().unwrap().id();
    let cc_id = tool_fields[3].as_ref().unwrap().id();
    let cxx_id = tool_fields[4].as_ref().unwrap().id();
    let git_id = tool_fields[5].as_ref().unwrap().id();

    let fs_reset_id = font_size_field.id();
    let tw_reset_id = tab_width_field.id();
    let lh_reset_id = line_height_field.id();
    let ln_reset_id = line_numbers_toggle.id();
    let auto_save_reset_id = auto_save_toggle.id();
    let sw_reset_id = sidebar_width_field.id();
    let oh_reset_id = output_height_field.id();
    let term_fs_reset_id = terminal_font_size_field.id();
    let reopen_reset_id = reopen_toggle.id();
    let ai_prov_reset_id = ai_provider.id();
    let ai_model_reset_id = ai_model.id();
    let ai_tok_reset_id = ai_tokens.id();
    let ai_ep_reset_id = ai_endpoint.id();
    let ai_key_reset_id = ai_key.id();
    let crust_reset_id = crust_id;
    let ccargo_reset_id = ccargo_id;
    let make_reset_id = make_id;
    let cc_reset_id = cc_id;
    let cxx_reset_id = cxx_id;
    let git_reset_id = git_id;

    btn_reset.on_click(move |_| {
        let defaults = Config::defaults();
        ui::Control::from_id(fs_reset_id).set_text(&format!("{}", defaults.font_size));
        ui::Control::from_id(tw_reset_id).set_text(&format!("{}", defaults.tab_width));
        ui::Control::from_id(lh_reset_id).set_text(&format!("{}", defaults.line_height));
        ui::Control::from_id(ln_reset_id).set_state(defaults.show_line_numbers as u32);
        ui::Control::from_id(auto_save_reset_id).set_state(defaults.auto_save as u32);
        ui::Control::from_id(sw_reset_id).set_text(&format!("{}", defaults.sidebar_width));
        ui::Control::from_id(oh_reset_id).set_text(&format!("{}", defaults.output_height));
        ui::Control::from_id(term_fs_reset_id)
            .set_text(&format!("{}", defaults.terminal_font_size));
        ui::Control::from_id(reopen_reset_id).set_state(defaults.reopen_last_project as u32);
        ui::Control::from_id(ai_prov_reset_id).set_state(0);
        ui::Control::from_id(ai_model_reset_id).set_state(0);
        ui::Control::from_id(ai_tok_reset_id).set_text("4096");
        ui::Control::from_id(ai_ep_reset_id).set_text("");
        ui::Control::from_id(ai_key_reset_id).set_text("");
        ui::Control::from_id(crust_reset_id).set_text("");
        ui::Control::from_id(ccargo_reset_id).set_text("");
        ui::Control::from_id(make_reset_id).set_text("");
        ui::Control::from_id(cc_reset_id).set_text("");
        ui::Control::from_id(cxx_reset_id).set_text("");
        ui::Control::from_id(git_reset_id).set_text("");
    });

    btn_save.on_click(move |_| {
        // Save editor config
        let mut cfg = Config::load();
        cfg.font_size = read_u32(fs_id, cfg.font_size);
        cfg.tab_width = read_u32(tw_id, cfg.tab_width);
        cfg.line_height = read_u32(lh_id, cfg.line_height);
        cfg.show_line_numbers = ui::Control::from_id(ln_id).get_state() != 0;
        cfg.auto_save = ui::Control::from_id(auto_save_id).get_state() != 0;
        cfg.sidebar_width = read_u32(sw_id, cfg.sidebar_width);
        cfg.output_height = read_u32(oh_id, cfg.output_height);
        cfg.terminal_font_size = read_u32(term_fs_id, cfg.terminal_font_size);
        cfg.reopen_last_project = ui::Control::from_id(reopen_id).get_state() != 0;
        cfg.crust_path = read_string(crust_id);
        cfg.ccargo_path = read_string(ccargo_id);
        cfg.make_path = read_string(make_id);
        cfg.cc_path = read_string(cc_id);
        cfg.cxx_path = read_string(cxx_id);
        cfg.git_path = String::from("/System/bin/cgit");
        cfg.auto_discover();
        cfg.save();

        // Save AI config
        let prov_state = ui::Control::from_id(ai_prov_id).get_state();
        let provider = if prov_state == 1 {
            AiProvider::Anthropic
        } else {
            AiProvider::OpenAI
        };

        let model_state = ui::Control::from_id(ai_model_id).get_state();
        let all_models = [
            "gpt-5.3-codex",
            "gpt-5.4",
            "gpt-5.4-mini",
            "gpt-4o",
            "claude-sonnet-4-20250514",
            "claude-haiku-4-5-20251001",
            "claude-opus-4-20250514",
        ];
        let model = all_models
            .get(model_state as usize)
            .unwrap_or(&"gpt-5.3-codex");

        let ai_cfg = AiConfig {
            provider,
            api_key: read_string(ai_key_id),
            model: String::from(*model),
            max_tokens: read_u32(ai_tok_id, 4096),
            temperature: 0.2,
            reasoning_effort: String::from("medium"),
            privacy_mode: true,
            require_patch_review: true,
            custom_endpoint: read_string(ai_ep_id),
        };
        ai_cfg.save();

        // Update running app
        let s = crate::app();
        s.config = Config::load();
        s.editor_view.apply_config(&s.config);
        s.output.apply_config(&s.config);
        s.ai_client.config = AiConfig::load();
        s.ai_panel.set_provider(s.ai_client.config.provider);
        if let Some(ref proj) = s.current_project {
            s.task_mgr.detect_from_project(proj, &s.config);
            s.run_panel.update(&s.task_mgr);
            s.sidebar.populate_project(proj, &s.task_mgr);
            crate::logic::commands::refresh_run_config_selector();
        }

        // Close
        ui::Window::from_id(win_id).destroy();
    });

    btn_cancel.on_click(move |_| {
        ui::Window::from_id(win_id).destroy();
    });
}

fn read_u32(id: u32, default: u32) -> u32 {
    let mut buf = [0u8; 32];
    let len = ui::Control::from_id(id).get_text(&mut buf);
    let s = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
    parse_u32(s).unwrap_or(default)
}

fn read_string(id: u32) -> String {
    let mut buf = [0u8; 512];
    let len = ui::Control::from_id(id).get_text(&mut buf);
    String::from(
        core::str::from_utf8(&buf[..len as usize])
            .unwrap_or("")
            .trim(),
    )
}

fn find_model_index(items: &str, model: &str) -> u32 {
    for (i, item) in items.split('|').enumerate() {
        if item == model {
            return i as u32;
        }
    }
    0
}

fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut r: u32 = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        r = r.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(r)
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

fn switch_settings_page(
    pages: [u32; 4],
    rows: [u32; 4],
    markers: [u32; 4],
    labels: [u32; 4],
    idx: usize,
) {
    let tc = ui::theme::colors();
    for j in 0..4 {
        let selected = j == idx;
        ui::Control::from_id(pages[j]).set_visible(selected);
        ui::Control::from_id(rows[j]).set_color(if selected {
            tc.editor_bg
        } else {
            tc.sidebar_bg
        });
        ui::Control::from_id(markers[j]).set_color(if selected {
            tc.accent
        } else {
            tc.sidebar_bg
        });
        ui::Control::from_id(labels[j]).set_text_color(if selected {
            tc.text
        } else {
            tc.text_secondary
        });
    }
}

use alloc::string::String;

use libanyui_client as ui;

use crate::save::WorldSummary;
use crate::settings::GameSettings;
use crate::state::{AppMode, STATE};

#[derive(Clone, Copy)]
pub struct MenuUi {
    pub overlay: ui::View,
    pub title: ui::Label,
    pub subtitle: ui::Label,
    pub hero_panel: ui::View,
    pub play_btn: ui::Button,
    pub settings_btn: ui::Button,
    pub back_btn: ui::Button,
    pub world_panel: ui::View,
    pub world_title: ui::Label,
    pub world_hint: ui::Label,
    pub worlds_scroll: ui::ScrollView,
    pub worlds_list: ui::StackPanel,
    pub new_world_name: ui::TextField,
    pub create_world_btn: ui::Button,
    pub settings_panel: ui::View,
    pub settings_title: ui::Label,
    pub graphics_label: ui::Label,
    pub graphics_drop: ui::DropDown,
    pub shadows_label: ui::Label,
    pub shadows_toggle: ui::Toggle,
    pub shadow_quality_label: ui::Label,
    pub shadow_quality_drop: ui::DropDown,
    pub apply_btn: ui::Button,
    pub status: ui::Label,
}

pub fn build(window: &ui::Window, canvas_w: u32, canvas_h: u32, settings: &GameSettings) -> MenuUi {
    let overlay = ui::View::new();
    overlay.set_color(0x12000000);
    window.add(&overlay);

    let title = ui::Label::new("FORGER");
    title.set_font_size(32);
    title.set_text_color(0xFFF6F2D7);
    title.set_text_align(ui::TEXT_ALIGN_CENTER);
    title.set_color(0x00000000);
    overlay.add(&title);

    let subtitle = ui::Label::new("Blockwelt Edition");
    subtitle.set_font_size(14);
    subtitle.set_text_color(0xFFE0C97F);
    subtitle.set_text_align(ui::TEXT_ALIGN_CENTER);
    subtitle.set_color(0x00000000);
    overlay.add(&subtitle);

    let hero_panel = ui::View::new();
    hero_panel.set_color(0xAA1B120B);
    overlay.add(&hero_panel);

    let play_btn = ui::Button::new("Spielen");
    play_btn.set_color(0xFF6A8B2F);
    hero_panel.add(&play_btn);

    let settings_btn = ui::Button::new("Einstellungen");
    settings_btn.set_color(0xFF6C4B2C);
    hero_panel.add(&settings_btn);

    let back_btn = ui::Button::new("Zurueck");
    back_btn.set_color(0xFF43362B);
    overlay.add(&back_btn);

    let world_panel = ui::View::new();
    world_panel.set_color(0xC419120B);
    overlay.add(&world_panel);

    let world_title = ui::Label::new("Welten");
    world_title.set_font_size(24);
    world_title.set_text_color(0xFFF5E8C8);
    world_panel.add(&world_title);

    let world_hint = ui::Label::new("Erstelle eine Welt oder waehle eine bestehende aus.");
    world_hint.set_text_color(0xFFD9C8AE);
    world_panel.add(&world_hint);

    let worlds_scroll = ui::ScrollView::new();
    world_panel.add(&worlds_scroll);

    let worlds_list = ui::StackPanel::vertical();
    worlds_scroll.add(&worlds_list);

    let new_world_name = ui::TextField::new();
    new_world_name.set_placeholder("Name der neuen Welt");
    world_panel.add(&new_world_name);

    let create_world_btn = ui::Button::new("Erstellen");
    create_world_btn.set_color(0xFF7D5A32);
    world_panel.add(&create_world_btn);

    let settings_panel = ui::View::new();
    settings_panel.set_color(0xC419120B);
    overlay.add(&settings_panel);

    let settings_title = ui::Label::new("Einstellungen");
    settings_title.set_font_size(24);
    settings_title.set_text_color(0xFFF5E8C8);
    settings_panel.add(&settings_title);

    let graphics_label = ui::Label::new("Grafikqualitaet");
    graphics_label.set_text_color(0xFFE4D8C0);
    settings_panel.add(&graphics_label);

    let graphics_drop = ui::DropDown::new("Schnell|Normal|Hoch");
    graphics_drop.set_selected_index(settings.graphics_quality_index());
    settings_panel.add(&graphics_drop);

    let shadows_label = ui::Label::new("Schatten");
    shadows_label.set_text_color(0xFFE4D8C0);
    settings_panel.add(&shadows_label);

    let shadows_toggle = ui::Toggle::new(settings.shadows_enabled);
    settings_panel.add(&shadows_toggle);

    let shadow_quality_label = ui::Label::new("Schattenqualitaet");
    shadow_quality_label.set_text_color(0xFFE4D8C0);
    settings_panel.add(&shadow_quality_label);

    let shadow_quality_drop = ui::DropDown::new("Weich|Normal|Scharf");
    shadow_quality_drop.set_selected_index(settings.shadow_quality_index());
    settings_panel.add(&shadow_quality_drop);

    let apply_btn = ui::Button::new("Anwenden");
    apply_btn.set_color(0xFF6A8B2F);
    settings_panel.add(&apply_btn);

    let status = ui::Label::new("");
    status.set_text_color(0xFFE7D7C0);
    overlay.add(&status);

    let ui = MenuUi {
        overlay,
        title,
        subtitle,
        hero_panel,
        play_btn,
        settings_btn,
        back_btn,
        world_panel,
        world_title,
        world_hint,
        worlds_scroll,
        worlds_list,
        new_world_name,
        create_world_btn,
        settings_panel,
        settings_title,
        graphics_label,
        graphics_drop,
        shadows_label,
        shadows_toggle,
        shadow_quality_label,
        shadow_quality_drop,
        apply_btn,
        status,
    };

    layout(&ui, canvas_w, canvas_h);
    wire_callbacks(ui);
    ui
}

pub fn layout(ui: &MenuUi, canvas_w: u32, canvas_h: u32) {
    let cw = canvas_w as i32;
    let ch = canvas_h as i32;
    let world_panel_w = 500i32;
    let world_panel_h = (ch - 220).clamp(300, 430);
    let world_panel_x = (cw / 2) - (world_panel_w / 2);
    let world_panel_y = 136i32;
    let create_row_y = 84i32;
    let world_scroll_y = 132i32;
    let world_scroll_h = (world_panel_h - world_scroll_y - 24).max(120);

    ui.overlay.set_position(0, 0);
    ui.overlay.set_size(canvas_w, canvas_h);

    ui.title.set_position((cw / 2) - 170, 52);
    ui.title.set_size(340, 40);
    ui.subtitle.set_position((cw / 2) - 170, 90);
    ui.subtitle.set_size(340, 24);

    ui.hero_panel.set_position((cw / 2) - 170, 136);
    ui.hero_panel.set_size(340, 150);
    ui.play_btn.set_position(30, 34);
    ui.play_btn.set_size(280, 40);
    ui.settings_btn.set_position(30, 88);
    ui.settings_btn.set_size(280, 34);

    ui.back_btn.set_position(28, ch - 54);
    ui.back_btn.set_size(110, 34);

    ui.world_panel.set_position(world_panel_x, world_panel_y);
    ui.world_panel.set_size(world_panel_w as u32, world_panel_h as u32);
    ui.world_title.set_position(20, 18);
    ui.world_title.set_size(200, 28);
    ui.world_hint.set_position(20, 48);
    ui.world_hint.set_size(420, 20);
    ui.new_world_name.set_position(20, create_row_y);
    ui.new_world_name.set_size(324, 34);
    ui.create_world_btn.set_position(356, create_row_y);
    ui.create_world_btn.set_size(124, 34);
    ui.worlds_scroll.set_position(20, world_scroll_y);
    ui.worlds_scroll.set_size(460, world_scroll_h as u32);
    ui.worlds_list.set_position(0, 0);
    ui.worlds_list.set_size(448, world_scroll_h as u32);
    ui.worlds_list.set_padding(0, 0, 0, 0);

    ui.settings_panel.set_position((cw / 2) - 220, 146);
    ui.settings_panel.set_size(440, 248);
    ui.settings_title.set_position(20, 18);
    ui.settings_title.set_size(200, 28);
    ui.graphics_label.set_position(20, 66);
    ui.graphics_label.set_size(180, 20);
    ui.graphics_drop.set_position(220, 62);
    ui.graphics_drop.set_size(180, 28);
    ui.shadows_label.set_position(20, 110);
    ui.shadows_label.set_size(180, 20);
    ui.shadows_toggle.set_position(220, 106);
    ui.shadow_quality_label.set_position(20, 154);
    ui.shadow_quality_label.set_size(180, 20);
    ui.shadow_quality_drop.set_position(220, 150);
    ui.shadow_quality_drop.set_size(180, 28);
    ui.apply_btn.set_position(260, 198);
    ui.apply_btn.set_size(140, 34);

    ui.status.set_position((cw / 2) - 210, ch - 54);
    ui.status.set_size(420, 22);
}

pub fn show_mode(ui: &MenuUi, mode: AppMode) {
    let in_menu = mode != AppMode::InGame;
    ui.overlay.set_visible(in_menu);
    ui.back_btn.set_visible(matches!(mode, AppMode::WorldSelect | AppMode::Settings));
    ui.hero_panel.set_visible(mode == AppMode::MainMenu);
    ui.world_panel.set_visible(mode == AppMode::WorldSelect);
    ui.settings_panel.set_visible(mode == AppMode::Settings);
}

pub fn sync_settings(ui: &MenuUi, settings: &GameSettings) {
    ui.graphics_drop.set_selected_index(settings.graphics_quality_index());
    ui.shadows_toggle.set_state(if settings.shadows_enabled { 1 } else { 0 });
    ui.shadow_quality_drop
        .set_selected_index(settings.shadow_quality_index());
}

pub fn set_status(ui: &MenuUi, text: &str) {
    ui.status.set_text(text);
}

pub fn refresh_world_list(ui: &MenuUi, worlds: &[WorldSummary]) {
    ui.worlds_list.clear();
    let mut content_h = 0u32;

    if worlds.is_empty() {
        let empty = ui::Label::new("Noch keine Welten vorhanden.");
        empty.set_size(420, 24);
        empty.set_margin(0, 8, 0, 0);
        empty.set_text_color(0xFFE8D4B6);
        ui.worlds_list.add(&empty);
        content_h = 32;
    } else {
        for summary in worlds {
            let row = ui::View::new();
            row.set_size(448, 56);
            row.set_margin(0, 0, 0, 10);
            row.set_color(0x7A2A2016);
            ui.worlds_list.add(&row);

            let name = ui::Label::new(&summary.name);
            name.set_position(14, 8);
            name.set_size(286, 18);
            name.set_text_color(0xFFF7E8C8);
            row.add(&name);

            let sub = ui::Label::new(&alloc::format!("Seed {}", summary.seed));
            sub.set_position(14, 28);
            sub.set_size(220, 16);
            sub.set_text_color(0xFFC7B79C);
            row.add(&sub);

            let open_btn = ui::Button::new("Spielen");
            open_btn.set_position(332, 10);
            open_btn.set_size(100, 34);
            open_btn.set_color(0xFF6A8B2F);
            let world_id = summary.id.clone();
            open_btn.on_click(move |_| {
                let s = unsafe { STATE.as_mut().unwrap() };
                crate::menu::open_world(s, &world_id);
            });
            row.add(&open_btn);
            content_h += 62;
        }
    }

    ui.worlds_list.set_size(448, content_h.max(80));
}

pub fn read_text(field: &ui::TextField) -> String {
    let mut buf = [0u8; 128];
    let len = field.get_text(&mut buf) as usize;
    let len = len.min(buf.len());
    core::str::from_utf8(&buf[..len])
        .ok()
        .map(|s| String::from(s.trim_end_matches('\0')))
        .unwrap_or_default()
}

pub fn wire_callbacks(ui: MenuUi) {
    ui.play_btn.on_click(|_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.app_mode = AppMode::WorldSelect;
        crate::menu::refresh_menu_state(s);
    });

    ui.settings_btn.on_click(|_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.app_mode = AppMode::Settings;
        crate::menu::refresh_menu_state(s);
    });

    ui.back_btn.on_click(|_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.app_mode = AppMode::MainMenu;
        crate::menu::refresh_menu_state(s);
    });

    ui.create_world_btn.on_click(|_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        crate::menu::create_world_from_field(s);
    });

    ui.new_world_name.on_submit(|_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        crate::menu::create_world_from_field(s);
    });

    ui.apply_btn.on_click(|_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        crate::menu::apply_settings_from_controls(s);
    });
}

pub fn refresh_menu_state(s: &mut crate::state::GameState) {
    sync_settings(&s.menu_ui, &s.settings);
    refresh_world_list(&s.menu_ui, &s.world_summaries);
    show_mode(&s.menu_ui, s.app_mode);
    apply_ingame_visibility(s);
    if s.app_mode == AppMode::WorldSelect {
        set_status(&s.menu_ui, "Gib einen Namen ein und druecke Enter.");
        s.menu_ui.new_world_name.focus();
    }
}

pub fn create_world_from_field(s: &mut crate::state::GameState) {
    let name = read_text(&s.menu_ui.new_world_name);
    let seed = anyos_std::sys::uptime_ms().wrapping_mul(1664525).wrapping_add(1013904223);
    if let Some(summary) = crate::save::create_world(&name, seed) {
        s.world_summaries = crate::save::load_world_summaries();
        s.menu_ui.new_world_name.set_text("");
        set_status(&s.menu_ui, "Welt erstellt.");
        open_world(s, &summary.id);
    } else {
        set_status(&s.menu_ui, "Welt konnte nicht erstellt werden.");
        s.menu_ui.new_world_name.focus();
    }
}

pub fn apply_settings_from_controls(s: &mut crate::state::GameState) {
    s.settings
        .set_graphics_quality_from_index(s.menu_ui.graphics_drop.selected_index());
    s.settings.shadows_enabled = s.menu_ui.shadows_toggle.get_state() != 0;
    s.settings
        .set_shadow_quality_from_index(s.menu_ui.shadow_quality_drop.selected_index());
    s.settings.save();
    crate::state::apply_settings(s);
    set_status(&s.menu_ui, "Einstellungen gespeichert.");
    refresh_menu_state(s);
}

pub fn open_world(s: &mut crate::state::GameState, world_id: &str) {
    let Some(snapshot) = crate::save::load_world(world_id) else {
        set_status(&s.menu_ui, "Welt konnte nicht geladen werden.");
        return;
    };

    crate::state::load_runtime_world(s, snapshot);
    crate::state::apply_settings(s);
    s.app_mode = AppMode::InGame;
    set_status(&s.menu_ui, "");
    refresh_menu_state(s);
}

fn apply_ingame_visibility(s: &mut crate::state::GameState) {
    let in_game = s.app_mode == AppMode::InGame;
    s.mode_toggle.set_visible(in_game);
    s.shadow_toggle.set_visible(in_game);
    s.fps_label.set_visible(in_game);
    s.sun_debug_label.set_visible(in_game);
}

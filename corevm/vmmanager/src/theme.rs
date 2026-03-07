use eframe::egui::{self, Color32, CornerRadius, Stroke, Style, Visuals};

pub fn apply_theme(ctx: &egui::Context) {
    let mut style = Style::default();
    let mut visuals = Visuals::dark();

    // Background colors — warm dark tones
    visuals.window_fill = Color32::from_rgb(28, 28, 30);
    visuals.panel_fill = Color32::from_rgb(36, 36, 38);
    visuals.faint_bg_color = Color32::from_rgb(44, 44, 46);
    visuals.extreme_bg_color = Color32::from_rgb(22, 22, 24);

    // Widget styling — softer, rounder
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(44, 44, 46);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(180, 180, 185));
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);

    visuals.widgets.inactive.bg_fill = Color32::from_rgb(54, 54, 56);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(180, 180, 185));
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);

    visuals.widgets.hovered.bg_fill = Color32::from_rgb(64, 64, 68);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::from_rgb(230, 230, 235));
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);

    visuals.widgets.active.bg_fill = ACCENT_BLUE;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);

    // Selection — blue background, white text
    visuals.selection.bg_fill = Color32::from_rgb(10, 132, 255);
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);

    // Separator — very subtle
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(0.5, Color32::from_rgb(50, 50, 52));

    style.visuals = visuals;

    // Spacing — a bit more breathing room
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.window_margin = egui::Margin::same(16);

    ctx.set_style(style);
}

// Apple-style accent colors
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(10, 132, 255);
pub const SUCCESS_GREEN: Color32 = Color32::from_rgb(48, 209, 88);
pub const WARNING_ORANGE: Color32 = Color32::from_rgb(255, 159, 10);
pub const ERROR_RED: Color32 = Color32::from_rgb(255, 69, 58);
pub const SIDEBAR_BG: Color32 = Color32::from_rgb(28, 28, 30);
pub const TOOLBAR_BG: Color32 = Color32::from_rgb(36, 36, 38);
pub const STATUSBAR_BG: Color32 = Color32::from_rgb(36, 36, 38);
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(230, 230, 235);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(142, 142, 147);
pub const TEXT_TERTIARY: Color32 = Color32::from_rgb(99, 99, 102);

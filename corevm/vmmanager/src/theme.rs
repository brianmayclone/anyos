use eframe::egui::{self, Color32, CornerRadius, Stroke, Style, Visuals};

pub fn apply_theme(ctx: &egui::Context) {
    let mut style = Style::default();
    let mut visuals = Visuals::dark();

    // Background colors
    visuals.window_fill = Color32::from_rgb(30, 30, 30);
    visuals.panel_fill = Color32::from_rgb(37, 37, 38);
    visuals.faint_bg_color = Color32::from_rgb(45, 45, 48);
    visuals.extreme_bg_color = Color32::from_rgb(25, 25, 25);

    // Widget styling
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(45, 45, 48);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(204, 204, 204));
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(4);

    visuals.widgets.inactive.bg_fill = Color32::from_rgb(55, 55, 58);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(204, 204, 204));

    visuals.widgets.hovered.bg_fill = Color32::from_rgb(62, 62, 66);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    visuals.widgets.active.bg_fill = Color32::from_rgb(0, 122, 204);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    // Selection
    visuals.selection.bg_fill = Color32::from_rgb(0, 122, 204);
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);

    // Separator
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(60, 60, 60));

    style.visuals = visuals;

    // Spacing
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(12);

    ctx.set_style(style);
}

/// Accent color for active/running indicators
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(0, 122, 204);
pub const SUCCESS_GREEN: Color32 = Color32::from_rgb(76, 175, 80);
pub const WARNING_ORANGE: Color32 = Color32::from_rgb(255, 152, 0);
pub const ERROR_RED: Color32 = Color32::from_rgb(244, 67, 54);
pub const SIDEBAR_BG: Color32 = Color32::from_rgb(30, 30, 30);
pub const TOOLBAR_BG: Color32 = Color32::from_rgb(45, 45, 48);
pub const STATUSBAR_BG: Color32 = Color32::from_rgb(0, 122, 204);

use eframe::egui;
use crate::theme;

/// Actions the toolbar can trigger
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToolbarAction {
    Start,
    Pause,
    Stop,
    Settings,
    Snapshot,
    Screenshot,
    ClipboardToGuest,
    ClipboardFromGuest,
}

fn icon_button(ui: &mut egui::Ui, icon: &str, tooltip: &str, enabled: bool, accent: bool) -> bool {
    let color = if !enabled {
        theme::TEXT_TERTIARY
    } else if accent {
        egui::Color32::WHITE
    } else {
        theme::TEXT_PRIMARY
    };

    let fill = if accent && enabled {
        theme::ACCENT_BLUE
    } else {
        egui::Color32::TRANSPARENT
    };

    let resp = ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(icon).size(15.0).color(color))
            .fill(fill)
            .corner_radius(egui::CornerRadius::same(4))
            .min_size(egui::vec2(24.0, 20.0)),
    );
    let clicked = resp.clicked();
    resp.on_hover_text(tooltip);
    clicked
}

/// Render the toolbar inline (meant to be called inside the menu bar).
pub fn render_toolbar(
    ui: &mut egui::Ui,
    vm_selected: bool,
    vm_running: bool,
    vm_paused: bool,
) -> Option<ToolbarAction> {
    let mut action = None;

    ui.style_mut().spacing.item_spacing = egui::vec2(2.0, 0.0);
    ui.style_mut().spacing.button_padding = egui::vec2(4.0, 2.0);

    // Start / Resume
    if !vm_running || vm_paused {
        let (icon, tip) = if vm_paused { ("\u{25B6}", "Resume") } else { ("\u{25B6}", "Start") };
        let enabled = vm_selected && (!vm_running || vm_paused);
        if icon_button(ui, icon, tip, enabled, true) {
            action = Some(if vm_paused { ToolbarAction::Pause } else { ToolbarAction::Start });
        }
    }

    // Pause
    if vm_running && !vm_paused {
        if icon_button(ui, "\u{23F8}", "Pause", true, false) {
            action = Some(ToolbarAction::Pause);
        }
    }

    // Stop
    let stop_enabled = vm_selected && (vm_running || vm_paused);
    if icon_button(ui, "\u{23F9}", "Stop", stop_enabled, false) {
        action = Some(ToolbarAction::Stop);
    }

    ui.add_space(2.0);
    ui.colored_label(egui::Color32::from_rgb(50, 50, 52), "|");
    ui.add_space(2.0);

    // Settings
    let settings_enabled = vm_selected && !vm_running;
    if icon_button(ui, "\u{2699}", "Settings", settings_enabled, false) {
        action = Some(ToolbarAction::Settings);
    }

    // Snapshot
    if icon_button(ui, "\u{1F4F7}", "Snapshots", vm_selected, false) {
        action = Some(ToolbarAction::Snapshot);
    }

    // Screenshot — only enabled when VM is running
    let screenshot_enabled = vm_selected && vm_running;
    if icon_button(ui, "\u{1F5BC}", "Screenshot to Clipboard", screenshot_enabled, false) {
        action = Some(ToolbarAction::Screenshot);
    }

    ui.add_space(2.0);
    ui.colored_label(egui::Color32::from_rgb(50, 50, 52), "|");
    ui.add_space(2.0);

    // Clipboard Host → Guest (paste host clipboard as keystrokes)
    let clip_enabled = vm_selected && vm_running;
    if icon_button(ui, "\u{2398}", "Paste Host Clipboard to Guest", clip_enabled, false) {
        action = Some(ToolbarAction::ClipboardToGuest);
    }

    // Clipboard Guest → Host (copy VGA text to host clipboard)
    if icon_button(ui, "\u{2397}", "Copy Guest Text to Host Clipboard", clip_enabled, false) {
        action = Some(ToolbarAction::ClipboardFromGuest);
    }

    action
}

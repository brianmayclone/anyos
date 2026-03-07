use eframe::egui;
use egui::Color32;
use crate::theme;

/// Runtime metrics for the status bar
pub struct VmMetrics {
    pub state_label: &'static str,
    pub mips: f64,
    pub ipc: f64,
    pub cpu_mode: &'static str,
    pub jit_blocks: u64,
    pub jit_hit_rate: f64,
}

impl Default for VmMetrics {
    fn default() -> Self {
        Self {
            state_label: "Stopped",
            mips: 0.0,
            ipc: 0.0,
            cpu_mode: "N/A",
            jit_blocks: 0,
            jit_hit_rate: 0.0,
        }
    }
}

/// Render the status bar at the bottom.
pub fn render_statusbar(
    ctx: &egui::Context,
    metrics: Option<&VmMetrics>,
    vm_selected: bool,
    last_key: Option<&str>,
) {
    egui::TopBottomPanel::bottom("statusbar")
        .exact_height(22.0)
        .frame(
            egui::Frame::new()
                .fill(theme::STATUSBAR_BG)
                .inner_margin(egui::Margin::symmetric(12, 0)),
        )
        .show(ctx, |ui| {
            // Subtle top border
            let rect = ui.max_rect();
            ui.painter().line_segment(
                [rect.left_top(), rect.right_top()],
                egui::Stroke::new(0.5, Color32::from_rgb(55, 55, 58)),
            );

            ui.horizontal_centered(|ui| {
                let label_color = theme::TEXT_SECONDARY;
                let value_color = theme::TEXT_PRIMARY;
                ui.style_mut().spacing.item_spacing = egui::vec2(4.0, 0.0);

                match metrics {
                    Some(m) => {
                        // State dot
                        let dot_color = match m.state_label {
                            "Running" => theme::SUCCESS_GREEN,
                            "Paused" => theme::WARNING_ORANGE,
                            _ => theme::TEXT_TERTIARY,
                        };
                        ui.colored_label(dot_color, "\u{25CF}");
                        ui.colored_label(value_color, egui::RichText::new(m.state_label).size(11.0));
                        ui.add_space(12.0);

                        ui.colored_label(label_color, egui::RichText::new(format!("{:.0} MIPS", m.mips)).size(11.0));
                        ui.add_space(8.0);
                        ui.colored_label(label_color, egui::RichText::new(format!("IPC {:.1}", m.ipc)).size(11.0));
                        ui.add_space(8.0);
                        ui.colored_label(label_color, egui::RichText::new(m.cpu_mode).size(11.0));

                        if m.jit_blocks > 0 {
                            ui.add_space(8.0);
                            ui.colored_label(
                                label_color,
                                egui::RichText::new(format!(
                                    "JIT {} blk ({:.0}%)",
                                    m.jit_blocks,
                                    m.jit_hit_rate * 100.0
                                )).size(11.0),
                            );
                        }
                    }
                    None => {
                        let msg = if vm_selected { "Ready" } else { "No VM selected" };
                        ui.colored_label(label_color, egui::RichText::new(msg).size(11.0));
                    }
                }

                if let Some(key_str) = last_key {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.colored_label(
                            theme::TEXT_TERTIARY,
                            egui::RichText::new(format!("Key: {}", key_str)).size(11.0),
                        );
                    });
                }
            });
        });
}

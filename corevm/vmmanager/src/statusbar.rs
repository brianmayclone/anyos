use eframe::egui;
use egui::Color32;

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
/// If metrics is None, show "Ready" or "No VM selected".
pub fn render_statusbar(
    ctx: &egui::Context,
    metrics: Option<&VmMetrics>,
    vm_selected: bool,
    last_key: Option<&str>,
) {
    let statusbar_bg = Color32::from_rgb(0, 122, 204);

    egui::TopBottomPanel::bottom("statusbar")
        .exact_height(24.0)
        .show(ctx, |ui| {
            ui.painter().rect_filled(ui.max_rect(), 0.0, statusbar_bg);

            ui.horizontal_centered(|ui| {
                ui.style_mut().visuals.override_text_color = Some(Color32::WHITE);

                match metrics {
                    Some(m) => {
                        ui.label(m.state_label);
                        ui.separator();
                        ui.label(format!("{:.0} MIPS", m.mips));
                        ui.separator();
                        ui.label(format!("IPC: {:.1}", m.ipc));
                        ui.separator();
                        ui.label(m.cpu_mode);
                        if m.jit_blocks > 0 {
                            ui.separator();
                            ui.label(format!(
                                "JIT: {} blocks ({:.0}%)",
                                m.jit_blocks,
                                m.jit_hit_rate * 100.0
                            ));
                        }
                    }
                    None => {
                        if vm_selected {
                            ui.label("Ready");
                        } else {
                            ui.label("No VM selected");
                        }
                    }
                }

                if let Some(key_str) = last_key {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("Key: {}", key_str))
                                .color(Color32::from_rgb(200, 230, 255)),
                        );
                    });
                }
            });
        });
}

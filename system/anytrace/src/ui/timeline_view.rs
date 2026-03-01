//! CPU profiling timeline view using Canvas.
//!
//! Shows a scrolling timeline of RIP samples collected during debugging.
//! Each vertical bar represents a sample; color indicates the address range.

use libanyui_client as ui;
use ui::Widget;
use crate::logic::sampler::Sample;

/// Timeline view panel.
pub struct TimelineView {
    pub canvas: ui::Canvas,
}

impl TimelineView {
    /// Create the timeline view.
    pub fn new(_parent: &impl Widget) -> Self {
        let canvas = ui::Canvas::new(800, 200);
        canvas.set_dock(ui::DOCK_FILL);
        Self { canvas }
    }

    /// Render the timeline from collected samples.
    ///
    /// Shows the most recent samples as vertical bars. Each bar's height
    /// is proportional to the RIP address within the observed range,
    /// giving a visual profile of execution location over time.
    pub fn update_timeline(&self, samples: &[Sample]) {
        let w = self.canvas.get_stride();
        let h = self.canvas.get_height();
        if w == 0 || h == 0 {
            return;
        }

        self.canvas.clear(0xFF1E1E1E);

        if samples.is_empty() {
            // Draw placeholder text area
            draw_label(&self.canvas, w, h);
            return;
        }

        // Show the most recent N samples that fit the width
        let bar_w = 4u32;
        let max_bars = (w / bar_w) as usize;
        let start_idx = if samples.len() > max_bars { samples.len() - max_bars } else { 0 };
        let visible = &samples[start_idx..];

        // Find RIP range for normalization
        let rip_min = visible.iter().map(|s| s.rip).min().unwrap_or(0);
        let rip_max = visible.iter().map(|s| s.rip).max().unwrap_or(1);
        let rip_range = (rip_max - rip_min).max(1) as f64;

        // Draw a bar for each sample
        let usable_h = (h - 20) as f64; // Leave 20px for axis label area
        for (i, sample) in visible.iter().enumerate() {
            let x = (i as u32 * bar_w) as i32;
            let norm = (sample.rip - rip_min) as f64 / rip_range;
            let bar_h = ((norm * usable_h * 0.8) as u32).max(2);
            let y = (h - 10 - bar_h) as i32;

            // Color based on position in address range (blue→green gradient)
            let color = rip_color(norm);
            self.canvas.fill_rect(x, y, bar_w - 1, bar_h, color);
        }

        // Draw baseline
        let baseline_y = (h - 10) as i32;
        self.canvas.draw_line(0, baseline_y, w as i32, baseline_y, 0xFF555555);

        // Draw sample count label
        let count_x = (w as i32) - 120;
        self.canvas.fill_rect(count_x, 2, 118, 14, 0xFF2A2A2A);
    }
}

/// Draw a placeholder label on the empty canvas.
fn draw_label(canvas: &ui::Canvas, w: u32, h: u32) {
    // Draw a centered horizontal line to indicate the timeline axis
    let y = (h / 2) as i32;
    canvas.draw_line(10, y, (w - 10) as i32, y, 0xFF333333);
    // Small tick marks
    for i in 0..10 {
        let x = (10 + i * ((w - 20) / 10)) as i32;
        canvas.draw_line(x, y - 3, x, y + 3, 0xFF444444);
    }
}

/// Map a normalized value (0.0–1.0) to a blue→green color gradient.
fn rip_color(t: f64) -> u32 {
    let r = (30.0 + t * 40.0) as u8;
    let g = (100.0 + t * 155.0) as u8;
    let b = (200.0 - t * 150.0) as u8;
    0xFF000000 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

//! CPU profiling timeline view using Canvas.

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
    pub fn update(&self, samples: &[Sample], _width: u32, height: u32) {
        self.canvas.clear(0xFF1E1E1E);

        if samples.is_empty() {
            return;
        }

        let t_start = samples[0].timestamp;
        let t_end = samples.last().map(|s| s.timestamp).unwrap_or(t_start + 1);
        let t_range = (t_end - t_start).max(1) as f64;
        // Use stride as a proxy for canvas width
        let canvas_w = self.canvas.get_stride() as f64;
        if canvas_w <= 0.0 {
            return;
        }

        // Draw sample marks as vertical lines
        for sample in samples {
            let t = (sample.timestamp - t_start) as f64;
            let x = ((t / t_range) * canvas_w) as i32;
            let color = 0xFF4CAF50; // Green
            self.canvas.draw_line(x, 0, x, height as i32, color);
        }
    }
}

use crate::control::{prepare_render, Control, ControlBase, ControlKind};
use crate::draw;

/// Number of dots in the spinner ring.
const DOT_COUNT: u32 = 12;

pub struct Spinner {
    pub(crate) base: ControlBase,
}

impl Spinner {
    pub fn new(base: ControlBase) -> Self {
        Self { base }
    }
}

impl Control for Spinner {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
    fn kind(&self) -> ControlKind {
        ControlKind::Spinner
    }

    fn render(&self, surface: &draw::Surface, ax: i32, ay: i32) {
        let b = self.base();
        let ctx = prepare_render(b, ax, ay);
        let (x, y, w, h) = (ctx.x, ctx.y, ctx.w, ctx.h);

        // state = animation frame (0..DOT_COUNT*4, driven by client timer)
        let frame = b.state;

        // color = accent color (overridable via set_color)
        let tc = crate::theme::colors();
        let base_color = if b.color != 0 { b.color } else { 0xFFFFFFFF };

        // Geometry: spinner fits inside the smaller of w, h
        let size = w.min(h);
        if size < 8 {
            return;
        }

        let cx = x + (w as i32) / 2;
        let cy = y + (h as i32) / 2;
        let radius = (size as i32) / 2 - 2;
        let dot_r = (size / 8).max(2);

        // Sine/cosine lookup (fixed-point, 12 positions around circle)
        // Pre-computed: cos(i * 2π/12), sin(i * 2π/12) * 1024
        const COS12: [i32; 12] = [
            1024, 887, 512, 0, -512, -887, -1024, -887, -512, 0, 512, 887,
        ];
        const SIN12: [i32; 12] = [
            0, -512, -887, -1024, -887, -512, 0, 512, 887, 1024, 887, 512,
        ];

        let active_dot = (frame / 4) % DOT_COUNT;

        for i in 0..DOT_COUNT {
            let idx = i as usize;
            let dx = cx + (radius * COS12[idx]) / 1024 - dot_r as i32 / 2;
            let dy = cy + (radius * SIN12[idx]) / 1024 - dot_r as i32 / 2;

            // Dots fade based on distance from the active dot (trailing tail)
            let dist = ((DOT_COUNT + active_dot - i) % DOT_COUNT) as u32;
            let alpha = if dist == 0 {
                255u32
            } else if dist <= 4 {
                255 - (dist * 45)
            } else {
                60
            };

            let r = (base_color >> 16) & 0xFF;
            let g = (base_color >> 8) & 0xFF;
            let b_ch = base_color & 0xFF;
            let color = (alpha << 24) | (r << 16) | (g << 8) | b_ch;

            draw::fill_rounded_rect(surface, dx, dy, dot_r, dot_r, dot_r / 2, color);
        }
    }
}

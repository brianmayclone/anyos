use alloc::vec::Vec;
use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct IconButton {
    pub(crate) text_base: TextControlBase,
    pressed: bool,
    /// Pre-rendered ARGB icon pixels (from SVG rasterizer).
    pub(crate) icon_pixels: Vec<u32>,
    pub(crate) icon_w: u32,
    pub(crate) icon_h: u32,
}

impl IconButton {
    pub fn new(text_base: TextControlBase) -> Self {
        Self { text_base, pressed: false, icon_pixels: Vec::new(), icon_w: 0, icon_h: 0 }
    }

    /// Horizontal padding inside the button (left and right).
    const H_PAD: i32 = 6;
    /// Gap between icon and text when both are present.
    const ICON_TEXT_GAP: i32 = 4;

    /// Set pre-rendered icon pixel data.
    ///
    /// Auto-resizes the button width when text is present to fit
    /// icon + gap + text horizontally.
    pub fn set_icon_pixels(&mut self, data: &[u32], w: u32, h: u32) {
        let expected = (w as usize) * (h as usize);
        if data.len() < expected { return; }
        self.icon_pixels.clear();
        self.icon_pixels.extend_from_slice(&data[..expected]);
        self.icon_w = w;
        self.icon_h = h;
        self.auto_size();
        self.text_base.base.mark_dirty();
    }

    /// Recompute button size to fit its content.
    fn auto_size(&mut self) {
        let has_icon = !self.icon_pixels.is_empty() || self.text_base.base.state > 0;
        let has_text = !self.text_base.text.is_empty();

        if has_icon && has_text {
            let iw = if !self.icon_pixels.is_empty() { self.icon_w as i32 } else { 16 };
            let ih = if !self.icon_pixels.is_empty() { self.icon_h as i32 } else { 16 };
            let font_size = self.text_base.text_style.font_size;
            let (tw, _) = crate::draw::text_size_at(&self.text_base.text, font_size);
            let needed_w = Self::H_PAD + iw + Self::ICON_TEXT_GAP + tw as i32 + Self::H_PAD;
            let needed_h = (ih + 8).max(self.text_base.base.h as i32);
            self.text_base.base.w = needed_w as u32;
            self.text_base.base.h = needed_h as u32;
        }
    }
}

impl Control for IconButton {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::IconButton }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let h = self.text_base.base.h;
        let tc = crate::theme::colors();
        let icon_id = self.text_base.base.state;
        let custom = self.text_base.base.color;
        let disabled = self.text_base.base.disabled;
        let hovered = self.text_base.base.hovered;
        let focused = self.text_base.base.focused;
        let corner = crate::theme::BUTTON_CORNER;
        let has_icon = !self.icon_pixels.is_empty() || icon_id > 0;

        // Background: hover highlight, pressed darken, custom color
        if self.pressed && !disabled {
            let bg = if custom != 0 { crate::theme::darken(custom, 30) } else { tc.control_pressed };
            crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, bg);
        } else if hovered && !disabled {
            let bg = if custom != 0 { crate::theme::lighten(custom, 12) } else { tc.control_hover };
            crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, bg);
        } else if custom != 0 {
            crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, custom);
        }

        let has_text = !self.text_base.text.is_empty();
        let text_color = if disabled {
            tc.text_disabled
        } else if self.text_base.text_style.text_color != 0 {
            self.text_base.text_style.text_color
        } else {
            tc.text
        };
        let icon_color = if disabled {
            tc.text_disabled
        } else if self.text_base.text_style.text_color != 0 {
            self.text_base.text_style.text_color
        } else {
            tc.text_secondary
        };

        if has_icon && has_text {
            // ── Horizontal layout: [pad | icon | gap | text | pad] ──
            let iw = if !self.icon_pixels.is_empty() { self.icon_w as i32 } else { 16 };
            let ih = if !self.icon_pixels.is_empty() { self.icon_h as i32 } else { 16 };
            let ix = x + Self::H_PAD;
            let iy = y + (h as i32 - ih) / 2;

            if !self.icon_pixels.is_empty() {
                blit_alpha(surface, ix, iy, self.icon_w, self.icon_h, &self.icon_pixels);
            } else {
                crate::icons::draw_icon(surface, ix, iy, icon_id, icon_color);
            }

            let font_size = self.text_base.text_style.font_size;
            let tx = ix + iw + Self::ICON_TEXT_GAP;
            let ty = y + (h as i32 - font_size as i32) / 2;
            crate::draw::draw_text_sized(surface, tx, ty, text_color, &self.text_base.text, font_size);
        } else if has_icon {
            // ── Icon only: centered ──
            if !self.icon_pixels.is_empty() {
                let ix = x + (w as i32 - self.icon_w as i32) / 2;
                let iy = y + (h as i32 - self.icon_h as i32) / 2;
                blit_alpha(surface, ix, iy, self.icon_w, self.icon_h, &self.icon_pixels);
            } else {
                let ix = x + (w as i32 - 16) / 2;
                let iy = y + (h as i32 - 16) / 2;
                crate::icons::draw_icon(surface, ix, iy, icon_id, icon_color);
            }
        } else if has_text {
            // ── Text only: centered ──
            let font_size = self.text_base.text_style.font_size;
            let (tw, _) = crate::draw::text_size_at(&self.text_base.text, font_size);
            let tx = x + (w as i32 - tw as i32) / 2;
            let ty = y + (h as i32 - font_size as i32) / 2;
            crate::draw::draw_text_sized(surface, tx, ty, text_color, &self.text_base.text, font_size);
        }

        // Focus ring
        if focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, w, h, corner, tc.accent);
        }
    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }

    fn handle_mouse_down(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.pressed = true;
        EventResponse::CONSUMED
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.pressed = false;
        EventResponse::CONSUMED
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CLICK
    }
}

/// Blit ARGB pixels with alpha blending (for SVG icon rendering).
fn blit_alpha(s: &crate::draw::Surface, x: i32, y: i32, w: u32, h: u32, src: &[u32]) {
    if w == 0 || h == 0 || src.is_empty() { return; }
    let sw = s.width as i32;
    let clip_x0 = s.clip_x.max(0);
    let clip_y0 = s.clip_y.max(0);
    let clip_x1 = (s.clip_x + s.clip_w as i32).min(sw);
    let clip_y1 = (s.clip_y + s.clip_h as i32).min(s.height as i32);

    for row in 0..h as i32 {
        let py = y + row;
        if py < clip_y0 || py >= clip_y1 { continue; }
        let src_off = row as usize * w as usize;
        let x0 = x.max(clip_x0);
        let x1 = (x + w as i32).min(clip_x1);
        if x0 >= x1 { continue; }
        let dst_row = py as usize * s.width as usize;

        for px in x0..x1 {
            let si = src_off + (px - x) as usize;
            if si >= src.len() { break; }
            let pixel = src[si];
            let a = pixel >> 24;
            if a == 0 { continue; }
            let di = dst_row + px as usize;
            if a >= 255 {
                unsafe { *s.pixels.add(di) = pixel; }
            } else {
                let dst = unsafe { *s.pixels.add(di) };
                let inv = 255 - a;
                let sr = (pixel >> 16) & 0xFF;
                let sg = (pixel >> 8) & 0xFF;
                let sb = pixel & 0xFF;
                let dr = (dst >> 16) & 0xFF;
                let dg = (dst >> 8) & 0xFF;
                let db = dst & 0xFF;
                let r = (sr * a + dr * inv) / 255;
                let g = (sg * a + dg * inv) / 255;
                let b = (sb * a + db * inv) / 255;
                unsafe { *s.pixels.add(di) = 0xFF000000 | (r << 16) | (g << 8) | b; }
            }
        }
    }
}

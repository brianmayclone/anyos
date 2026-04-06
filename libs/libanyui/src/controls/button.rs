use crate::control::{Control, ControlBase, ControlKind, EventResponse, TextControlBase};

pub struct Button {
    pub(crate) text_base: TextControlBase,
    pressed: bool,
}

impl Button {
    pub fn new(text_base: TextControlBase) -> Self {
        Self {
            text_base,
            pressed: false,
        }
    }

    fn current_size(&self) -> (u32, u32) {
        let b = &self.text_base.base;

        let w = if b.auto_size {
            let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
            let (tw, _th) = crate::draw::text_size_at(&self.text_base.text, font_size);
            (tw + 34).min(0xFFFF) as u32
        } else {
            b.w
        };

        (w, b.h)
    }

    fn corner_radius(h: u32) -> u32 {
        ((h as i32) / 2).max(10) as u32
    }

    fn palette_normal(&self) -> Palette {
        let custom = self.text_base.base.color;

        if custom != 0 {
            Palette {
                border: crate::theme::darken(custom, 24),
                fill: custom,
                text: 0xFFFFFFFF,
                top_gloss: 0x18FFFFFF,
                inner_light: 0x14FFFFFF,
                bottom_shadow: 0x22000000,
                focus_ring: 0xFF2D8CFF,
                focus_glow: 0x66308FFF,
            }
        } else {
            Palette {
                border: 0xFF2A2A2A,
                fill: 0xFF3B3B3B,
                text: if self.text_base.text_style.text_color != 0 {
                    self.text_base.text_style.text_color
                } else {
                    0xFFEAEAEA
                },
                top_gloss: 0x18FFFFFF,
                inner_light: 0x12FFFFFF,
                bottom_shadow: 0x22000000,
                focus_ring: 0xFF2D8CFF,
                focus_glow: 0x66308FFF,
            }
        }
    }

    fn palette_hover(&self) -> Palette {
        let custom = self.text_base.base.color;

        if custom != 0 {
            let fill = crate::theme::lighten(custom, 6);
            Palette {
                border: crate::theme::darken(fill, 24),
                fill,
                text: 0xFFFFFFFF,
                top_gloss: 0x20FFFFFF,
                inner_light: 0x18FFFFFF,
                bottom_shadow: 0x24000000,
                focus_ring: 0xFF2D8CFF,
                focus_glow: 0x70308FFF,
            }
        } else {
            Palette {
                border: 0xFF313131,
                fill: 0xFF454545,
                text: 0xFFFFFFFF,
                top_gloss: 0x20FFFFFF,
                inner_light: 0x16FFFFFF,
                bottom_shadow: 0x24000000,
                focus_ring: 0xFF2D8CFF,
                focus_glow: 0x70308FFF,
            }
        }
    }

    fn palette_pressed(&self) -> Palette {
        let custom = self.text_base.base.color;

        if custom != 0 {
            let fill = crate::theme::darken(custom, 10);
            Palette {
                border: crate::theme::darken(fill, 18),
                fill,
                text: 0xFFFFFFFF,
                top_gloss: 0x0EFFFFFF,
                inner_light: 0x0AFFFFFF,
                bottom_shadow: 0x18000000,
                focus_ring: 0xFF2D8CFF,
                focus_glow: 0x55308FFF,
            }
        } else {
            Palette {
                border: 0xFF252525,
                fill: 0xFF343434,
                text: 0xFFE8E8E8,
                top_gloss: 0x0EFFFFFF,
                inner_light: 0x0AFFFFFF,
                bottom_shadow: 0x18000000,
                focus_ring: 0xFF2D8CFF,
                focus_glow: 0x55308FFF,
            }
        }
    }

    fn palette_disabled(&self) -> Palette {
        let custom = self.text_base.base.color;

        if custom != 0 {
            let fill = crate::theme::darken(custom, 30);
            Palette {
                border: crate::theme::darken(fill, 16),
                fill,
                text: 0xFF8C8C8C,
                top_gloss: 0x08FFFFFF,
                inner_light: 0x06FFFFFF,
                bottom_shadow: 0x14000000,
                focus_ring: 0xFF2D8CFF,
                focus_glow: 0x00000000,
            }
        } else {
            Palette {
                border: 0xFF252525,
                fill: 0xFF303030,
                text: 0xFF8C8C8C,
                top_gloss: 0x08FFFFFF,
                inner_light: 0x06FFFFFF,
                bottom_shadow: 0x14000000,
                focus_ring: 0xFF2D8CFF,
                focus_glow: 0x00000000,
            }
        }
    }

    fn resolve_palette(&self) -> Palette {
        let b = &self.text_base.base;
        if b.disabled {
            self.palette_disabled()
        } else if self.pressed {
            self.palette_pressed()
        } else if b.hovered {
            self.palette_hover()
        } else {
            self.palette_normal()
        }
    }

    fn draw_focus_glow(
        &self,
        surface: &crate::draw::Surface,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        corner: u32,
        color: u32,
    ) {
        if color >> 24 == 0 || w <= 4 || h <= 4 {
            return;
        }

        crate::draw::fill_rounded_rect(
            surface,
            x + 1,
            y + 1,
            w.saturating_sub(2),
            h.saturating_sub(2),
            corner.saturating_sub(1),
            color,
        );
    }

    fn draw_top_gloss(
        &self,
        surface: &crate::draw::Surface,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        corner: u32,
        color: u32,
    ) {
        let gloss_h = ((h as f32) * 0.42) as u32;
        if gloss_h < 2 || w < 4 {
            return;
        }

        crate::draw::fill_rounded_rect(
            surface,
            x + 1,
            y + 1,
            w.saturating_sub(2),
            gloss_h,
            corner.saturating_sub(1),
            color,
        );
    }

    fn draw_inner_light_edge(
        &self,
        surface: &crate::draw::Surface,
        x: i32,
        y: i32,
        w: u32,
        _h: u32,
        _corner: u32,
        color: u32,
    ) {
        if w < 8 {
            return;
        }

        crate::draw::fill_rounded_rect(surface, x + 3, y + 1, w.saturating_sub(6), 1, 0, color);
    }

    fn draw_bottom_inner_shadow(
        &self,
        surface: &crate::draw::Surface,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: u32,
    ) {
        if w < 8 || h < 6 {
            return;
        }

        crate::draw::fill_rounded_rect(
            surface,
            x + 3,
            y + h as i32 - 2,
            w.saturating_sub(6),
            1,
            0,
            color,
        );
    }
}

#[derive(Clone, Copy)]
struct Palette {
    border: u32,
    fill: u32,
    text: u32,
    top_gloss: u32,
    inner_light: u32,
    bottom_shadow: u32,
    focus_ring: u32,
    focus_glow: u32,
}

impl Control for Button {
    fn base(&self) -> &ControlBase {
        &self.text_base.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.text_base.base
    }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> {
        Some(&self.text_base)
    }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> {
        Some(&mut self.text_base)
    }
    fn kind(&self) -> ControlKind {
        ControlKind::Button
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let (button_w, button_h) = self.current_size();

        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, button_w, button_h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);

        let corner = Self::corner_radius(h);
        let pal = self.resolve_palette();

        if b.focused && !b.disabled {
            self.draw_focus_glow(surface, x, y, w, h, corner, pal.focus_glow);
            crate::draw::draw_rounded_border(surface, x, y, w, h, corner, pal.focus_ring);
            if w > 4 && h > 4 {
                crate::draw::draw_rounded_border(
                    surface,
                    x + 1,
                    y + 1,
                    w.saturating_sub(2),
                    h.saturating_sub(2),
                    corner.saturating_sub(1),
                    crate::theme::with_alpha(pal.focus_ring, 120),
                );
            }
        }

        crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, pal.border);

        let ix = x + 1;
        let iy = y + 1;
        let iw = w.saturating_sub(2);
        let ih = h.saturating_sub(2);
        let ic = corner.saturating_sub(1);

        crate::draw::fill_rounded_rect(surface, ix, iy, iw, ih, ic, pal.fill);

        self.draw_top_gloss(surface, ix, iy, iw, ih, ic, pal.top_gloss);
        self.draw_inner_light_edge(surface, ix, iy, iw, ih, ic, pal.inner_light);
        self.draw_bottom_inner_shadow(surface, ix, iy, iw, ih, pal.bottom_shadow);

        let content_offset_y = if self.pressed && !b.disabled { 1 } else { 0 };

        let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
        let (tw, th) = crate::draw::text_size_at(&self.text_base.text, font_size);

        let tx = x + ((w as i32 - tw as i32) / 2);
        let ty = y + ((h as i32 - th as i32) / 2) + content_offset_y - 1;

        crate::draw::draw_text_sized(surface, tx, ty, pal.text, &self.text_base.text, font_size);
    }

    fn is_interactive(&self) -> bool {
        !self.text_base.base.disabled
    }

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

    fn handle_key_down(&mut self, keycode: u32, _char_code: u32, _modifiers: u32) -> EventResponse {
        if keycode == crate::control::KEY_SPACE || keycode == crate::control::KEY_ENTER {
            EventResponse::CLICK
        } else {
            EventResponse::IGNORED
        }
    }
}

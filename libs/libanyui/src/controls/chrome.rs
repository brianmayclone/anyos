// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
use crate::draw::Surface;

#[derive(Clone, Copy)]
pub struct Palette {
    pub border: u32,
    pub fill: u32,
    pub top_gloss: u32,
    pub inner_light: u32,
    pub bottom_shadow: u32,
    pub focus_ring: u32,
    pub focus_glow: u32,
}

pub fn blend(a: u32, b: u32, t: u32) -> u32 {
    let inv = 255 - t.min(255);
    let aa = (a >> 24) & 0xFF;
    let ar = (a >> 16) & 0xFF;
    let ag = (a >> 8) & 0xFF;
    let ab = a & 0xFF;
    let ba = (b >> 24) & 0xFF;
    let br = (b >> 16) & 0xFF;
    let bg = (b >> 8) & 0xFF;
    let bb = b & 0xFF;
    let oa = (aa * inv + ba * t) / 255;
    let or = (ar * inv + br * t) / 255;
    let og = (ag * inv + bg * t) / 255;
    let ob = (ab * inv + bb * t) / 255;
    (oa << 24) | (or << 16) | (og << 8) | ob
}

pub fn neutral_palette(hovered: bool, pressed: bool, disabled: bool) -> Palette {
    let tc = crate::theme::colors();
    let fill = if disabled {
        crate::theme::darken(tc.control_bg, 6)
    } else if pressed {
        crate::theme::darken(tc.control_bg, 10)
    } else if hovered {
        crate::theme::lighten(tc.control_bg, 10)
    } else {
        tc.control_bg
    };
    Palette {
        border: crate::theme::darken(fill, if disabled { 8 } else { 18 }),
        fill,
        top_gloss: if disabled { 0x06FFFFFF } else { 0x16FFFFFF },
        inner_light: if disabled { 0x04FFFFFF } else { 0x12FFFFFF },
        bottom_shadow: if disabled { 0x10000000 } else { 0x22000000 },
        focus_ring: tc.accent,
        focus_glow: if disabled {
            0
        } else {
            crate::theme::with_alpha(tc.accent_hover, 96)
        },
    }
}

pub fn accent_palette(base: u32, hovered: bool, pressed: bool, disabled: bool) -> Palette {
    let fill = if disabled {
        crate::theme::darken(base, 24)
    } else if pressed {
        crate::theme::darken(base, 10)
    } else if hovered {
        crate::theme::lighten(base, 10)
    } else {
        base
    };
    let tc = crate::theme::colors();
    Palette {
        border: crate::theme::darken(fill, if disabled { 10 } else { 22 }),
        fill,
        top_gloss: if disabled { 0x06FFFFFF } else { 0x1EFFFFFF },
        inner_light: if disabled { 0x04FFFFFF } else { 0x14FFFFFF },
        bottom_shadow: if disabled { 0x10000000 } else { 0x24000000 },
        focus_ring: tc.accent,
        focus_glow: if disabled {
            0
        } else {
            crate::theme::with_alpha(tc.accent_hover, 112)
        },
    }
}

pub fn field_palette(custom: u32, hovered: bool, focused: bool, disabled: bool) -> Palette {
    let tc = crate::theme::colors();
    let base = if custom != 0 { custom } else { tc.input_bg };
    let fill = if disabled {
        crate::theme::darken(base, 8)
    } else if hovered {
        crate::theme::lighten(base, 4)
    } else {
        base
    };
    Palette {
        border: if focused {
            tc.input_focus
        } else if hovered && !disabled {
            blend(tc.input_border, tc.accent, 110)
        } else {
            tc.input_border
        },
        fill,
        top_gloss: if disabled { 0x04FFFFFF } else { 0x10FFFFFF },
        inner_light: if disabled { 0x02FFFFFF } else { 0x0AFFFFFF },
        bottom_shadow: if disabled { 0x0C000000 } else { 0x1A000000 },
        focus_ring: tc.accent,
        focus_glow: if focused && !disabled {
            crate::theme::with_alpha(tc.accent_hover, 88)
        } else {
            0
        },
    }
}

pub fn flat_field_palette(custom: u32, hovered: bool, focused: bool, disabled: bool) -> Palette {
    let tc = crate::theme::colors();
    let base = if custom != 0 { custom } else { tc.input_bg };
    let fill = if disabled {
        crate::theme::darken(base, 6)
    } else if hovered {
        blend(base, tc.control_hover, 28)
    } else {
        base
    };
    Palette {
        border: if focused {
            tc.input_focus
        } else if hovered && !disabled {
            blend(tc.input_border, tc.accent, 72)
        } else {
            tc.input_border
        },
        fill,
        top_gloss: 0,
        inner_light: 0,
        bottom_shadow: 0,
        focus_ring: tc.accent,
        focus_glow: if focused && !disabled {
            crate::theme::with_alpha(tc.accent_hover, 88)
        } else {
            0
        },
    }
}

pub fn card_palette(hovered: bool) -> Palette {
    let tc = crate::theme::colors();
    let fill = if hovered {
        blend(tc.card_bg, tc.control_hover, 48)
    } else {
        tc.card_bg
    };
    Palette {
        border: blend(
            tc.card_border,
            crate::theme::lighten(tc.card_border, 10),
            48,
        ),
        fill,
        top_gloss: 0x16FFFFFF,
        inner_light: 0x0CFFFFFF,
        bottom_shadow: 0x24000000,
        focus_ring: tc.accent,
        focus_glow: crate::theme::with_alpha(tc.accent_hover, 90),
    }
}

pub fn draw_focus(
    surface: &Surface,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    corner: u32,
    palette: Palette,
) {
    if w <= 4 || h <= 4 {
        return;
    }

    if palette.focus_glow >> 24 != 0 {
        crate::draw::fill_rounded_rect(
            surface,
            x + 1,
            y + 1,
            w.saturating_sub(2),
            h.saturating_sub(2),
            corner.saturating_sub(1),
            palette.focus_glow,
        );
    }

    crate::draw::draw_rounded_border(surface, x, y, w, h, corner, palette.focus_ring);
    if w > 4 && h > 4 {
        crate::draw::draw_rounded_border(
            surface,
            x + 1,
            y + 1,
            w.saturating_sub(2),
            h.saturating_sub(2),
            corner.saturating_sub(1),
            crate::theme::with_alpha(palette.focus_ring, 120),
        );
    }
}

pub fn draw_surface(
    surface: &Surface,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    corner: u32,
    palette: Palette,
) {
    crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, palette.border);
    if w < 2 || h < 2 {
        return;
    }

    let ix = x + 1;
    let iy = y + 1;
    let iw = w.saturating_sub(2);
    let ih = h.saturating_sub(2);
    let ic = corner.saturating_sub(1);

    crate::draw::fill_rounded_rect(surface, ix, iy, iw, ih, ic, palette.fill);

    let gloss_h = ((ih as f32) * 0.52) as u32;
    if gloss_h >= 4 && iw >= 8 {
        let gloss_bands = [
            (0u32, palette.top_gloss),
            (1u32, blend(palette.top_gloss, 0, 56)),
            (2u32, blend(palette.top_gloss, 0, 112)),
            (3u32, blend(palette.top_gloss, 0, 168)),
            (4u32, blend(palette.top_gloss, 0, 216)),
        ];
        for (band, color) in gloss_bands {
            let top = iy + 1 + ((gloss_h * band) / 5) as i32;
            let next = iy + 1 + ((gloss_h * (band + 1)) / 5) as i32;
            let band_h = next - top;
            if band_h <= 0 {
                continue;
            }
            let inset = 1 + band as i32;
            let band_w = iw.saturating_sub((inset as u32) * 2);
            if band_w < 4 {
                continue;
            }
            crate::draw::fill_rounded_rect(
                surface,
                ix + inset,
                top,
                band_w,
                band_h as u32,
                ic.saturating_sub(1 + band),
                color,
            );
        }
    }

    if iw >= 8 {
        crate::draw::fill_rounded_rect(
            surface,
            ix + 3,
            iy + 1,
            iw.saturating_sub(6),
            1,
            0,
            palette.inner_light,
        );
    }

    if iw >= 8 && ih >= 6 {
        crate::draw::fill_rounded_rect(
            surface,
            ix + 3,
            iy + ih as i32 - 2,
            iw.saturating_sub(6),
            1,
            0,
            palette.bottom_shadow,
        );
    }
}

pub fn draw_card(surface: &Surface, x: i32, y: i32, w: u32, h: u32, corner: u32, hovered: bool) {
    let palette = card_palette(hovered);
    crate::draw::draw_shadow_rounded_rect(
        surface,
        x,
        y,
        w,
        h,
        corner as i32,
        0,
        crate::theme::scale_i32(2),
        crate::theme::scale_i32(6),
        36,
    );
    draw_surface(surface, x, y, w, h, corner, palette);
    if w > 6 && h > 6 {
        crate::draw::draw_rounded_border(
            surface,
            x + 1,
            y + 1,
            w.saturating_sub(2),
            h.saturating_sub(2),
            corner.saturating_sub(1),
            crate::theme::with_alpha(0xFFFFFFFF, if crate::theme::is_light() { 36 } else { 22 }),
        );
    }
}

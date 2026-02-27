//! Menu bar and dropdown rendering.

use crate::compositor::{alpha_blend, Compositor, Rect};
use crate::desktop::drawing::{fill_rect, fill_rounded_rect};

use super::MenuBar;
use super::types::*;

impl MenuBar {
    /// Render menu titles into the menubar pixel buffer.
    pub fn render_titles(&self, pixels: &mut [u32], stride: u32, height: u32) {
        let def = match self.active_def() {
            Some(d) => d,
            None => return,
        };

        let open_idx = self.open_dropdown.as_ref().map(|d| d.menu_idx);

        for layout in &self.title_layouts {
            let menu = &def.menus[layout.menu_idx];

            // Highlight background if this menu is open
            if open_idx == Some(layout.menu_idx) {
                for y in 0..MENUBAR_HEIGHT {
                    for x in layout.x.max(0)..(layout.x + layout.width as i32).min(stride as i32) {
                        let idx = (y * stride + x as u32) as usize;
                        if idx < pixels.len() {
                            pixels[idx] = alpha_blend(color_menubar_highlight(), pixels[idx]);
                        }
                    }
                }
            }

            // App name (first menu) is bold, rest are regular
            let font = if layout.menu_idx == 0 { FONT_ID_BOLD } else { FONT_ID };

            // Render title text centered in its region
            let (tw, th) =
                anyos_std::ui::window::font_measure(font, FONT_SIZE, &menu.title);
            let tx = layout.x + (layout.width as i32 - tw as i32) / 2;
            let ty = ((MENUBAR_HEIGHT as i32 - th as i32) / 2).max(0);
            anyos_std::ui::window::font_render_buf(
                font,
                FONT_SIZE,
                pixels,
                stride,
                height,
                tx,
                ty,
                color_menubar_text(),
                &menu.title,
            );
        }
    }

    /// Render status icons into the menubar pixel buffer.
    pub fn render_status_icons(&self, pixels: &mut [u32], stride: u32) {
        for (i, icon) in self.status_icons.iter().enumerate() {
            let ix = match self.status_icon_x.get(i) {
                Some(&x) => x,
                None => continue,
            };
            let iy = ((MENUBAR_HEIGHT as i32 - 16) / 2).max(0);
            for row in 0..16i32 {
                let py = iy + row;
                if py < 0 || py >= MENUBAR_HEIGHT as i32 {
                    continue;
                }
                for col in 0..16i32 {
                    let px = ix + col;
                    if px < 0 || px >= stride as i32 {
                        continue;
                    }
                    let src = icon.pixels[(row * 16 + col) as usize];
                    let a = (src >> 24) & 0xFF;
                    if a == 0 {
                        continue;
                    }
                    let didx = (py as u32 * stride + px as u32) as usize;
                    if didx < pixels.len() {
                        if a >= 255 {
                            pixels[didx] = src;
                        } else {
                            pixels[didx] = alpha_blend(src, pixels[didx]);
                        }
                    }
                }
            }
        }
    }

    /// Render the dropdown layer content.
    pub fn render_dropdown(&self, compositor: &mut Compositor) {
        let dd = match &self.open_dropdown {
            Some(d) => d,
            None => return,
        };
        let def = match self.active_def() {
            Some(d) => d,
            None => return,
        };
        let menu = match def.menus.get(dd.menu_idx) {
            Some(m) => m,
            None => return,
        };

        if let Some(pixels) = compositor.layer_pixels(dd.layer_id) {
            let w = dd.width;
            let h = dd.height;

            // Clear to transparent
            for p in pixels.iter_mut() {
                *p = 0x00000000;
            }

            // Fill background with rounded corners
            fill_rounded_rect(pixels, w, h, 0, 0, w, h, 6, color_dropdown_bg());

            // 1px border
            draw_rect_outline(pixels, w, 0, 0, w, h, color_dropdown_border());

            // Render each item
            for (i, item) in menu.items.iter().enumerate() {
                let iy = dd.items_y[i];

                if item.is_separator() {
                    let line_y = iy + SEPARATOR_HEIGHT as i32 / 2;
                    if line_y >= 0 && (line_y as u32) < h {
                        for x in 8i32..(w as i32 - 8) {
                            if x >= 0 && (x as u32) < w {
                                let idx = (line_y as u32 * w + x as u32) as usize;
                                if idx < pixels.len() {
                                    pixels[idx] = color_separator();
                                }
                            }
                        }
                    }
                    continue;
                }

                // Highlight hovered item
                if dd.hover_idx == Some(i) && !item.is_disabled() {
                    fill_rect(pixels, w, h, 4, iy, w - 8, ITEM_HEIGHT, COLOR_HOVER_BG);
                }

                // Checkmark if checked
                if item.is_checked() {
                    let check_x = 8i32;
                    let (_, ch) = anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, "\u{2713}");
                    let check_y = iy + ((ITEM_HEIGHT as i32 - ch as i32) / 2).max(0);
                    anyos_std::ui::window::font_render_buf(
                        FONT_ID,
                        FONT_SIZE,
                        pixels,
                        w,
                        h,
                        check_x,
                        check_y,
                        if item.is_disabled() {
                            color_disabled_text()
                        } else {
                            COLOR_CHECK
                        },
                        "\u{2713}",
                    );
                }

                // Item label
                let text_x = 28i32;
                let (_, th) =
                    anyos_std::ui::window::font_measure(FONT_ID, FONT_SIZE, &item.label);
                let text_y = iy + ((ITEM_HEIGHT as i32 - th as i32) / 2).max(0);
                let text_color = if item.is_disabled() {
                    color_disabled_text()
                } else if dd.hover_idx == Some(i) {
                    0xFFFFFFFF
                } else {
                    color_menubar_text()
                };
                anyos_std::ui::window::font_render_buf(
                    FONT_ID,
                    FONT_SIZE,
                    pixels,
                    w,
                    h,
                    text_x,
                    text_y,
                    text_color,
                    &item.label,
                );
            }
        }

        compositor.mark_layer_dirty(dd.layer_id);
        let bounds = Rect::new(dd.x, dd.y, dd.width, dd.height);
        compositor.add_damage(bounds);
    }
}

// ── Drawing Helpers ──────────────────────────────────────────────────────────

/// Draw a 1px rectangle outline (no rounded corners).
pub(crate) fn draw_rect_outline(
    pixels: &mut [u32],
    stride: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: u32,
) {
    // Top
    for col in 0..w as i32 {
        let px = x + col;
        if px >= 0 && (px as u32) < stride && y >= 0 {
            let idx = (y as u32 * stride + px as u32) as usize;
            if idx < pixels.len() {
                pixels[idx] = color;
            }
        }
    }
    // Bottom
    let by = y + h as i32 - 1;
    for col in 0..w as i32 {
        let px = x + col;
        if px >= 0 && (px as u32) < stride && by >= 0 {
            let idx = (by as u32 * stride + px as u32) as usize;
            if idx < pixels.len() {
                pixels[idx] = color;
            }
        }
    }
    // Left
    for row in 0..h as i32 {
        let py = y + row;
        if x >= 0 && py >= 0 {
            let idx = (py as u32 * stride + x as u32) as usize;
            if idx < pixels.len() {
                pixels[idx] = color;
            }
        }
    }
    // Right
    let rx = x + w as i32 - 1;
    for row in 0..h as i32 {
        let py = y + row;
        if rx >= 0 && (rx as u32) < stride && py >= 0 {
            let idx = (py as u32 * stride + rx as u32) as usize;
            if idx < pixels.len() {
                pixels[idx] = color;
            }
        }
    }
}

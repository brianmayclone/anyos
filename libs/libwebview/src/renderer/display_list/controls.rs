impl DisplayList {
    fn emit_submit(&mut self, x: i32, y: i32, bx: &LayoutBox) {
        let label_text = if let Some(ref t) = bx.text {
            t.clone()
        } else {
            String::from("Submit")
        };

        // Default web button bg + border if no CSS styling.
        if bx.bg_color == 0 && bx.border_width == 0 {
            self.push(
                x,
                y,
                bx.width,
                bx.height,
                DrawKind::Rect { color: 0xFFE0E0E0 },
            );
            self.push(x, y, bx.width, 1, DrawKind::Rect { color: 0xFF808080 });
            self.push(
                x,
                y + bx.height - 1,
                bx.width,
                1,
                DrawKind::Rect { color: 0xFF808080 },
            );
            self.push(
                x,
                y + 1,
                1,
                (bx.height - 2).max(0),
                DrawKind::Rect { color: 0xFF808080 },
            );
            self.push(
                x + bx.width - 1,
                y + 1,
                1,
                (bx.height - 2).max(0),
                DrawKind::Rect { color: 0xFF808080 },
            );
        }

        // Center text in button.
        let font_size = bx.font_size.max(1) as u16;
        let text_color = self.default_control_fg(bx);
        let font_id = crate::layout::resolve_font_id(bx.custom_font_id, bx.bold, bx.italic);
        let (tw, _) = libfont_client::measure(font_id, font_size, &label_text);
        let tx = x + (bx.width - tw as i32) / 2;
        let ty = y + (bx.height - font_size as i32) / 2;
        self.push(
            tx,
            ty,
            tw as i32,
            font_size as i32,
            DrawKind::Text {
                color: text_color,
                font_id,
                font_size,
                scale_x_percent: 100,
                synthetic_bold: bx.bold && bx.custom_font_id != 0,
                text: label_text,
            },
        );
    }

    /// Draw a text input / search / password field.
    fn emit_text_input(&mut self, x: i32, y: i32, bx: &LayoutBox) {
        let bg = self.default_control_bg(bx);
        let border_color = self.default_control_border(bx);
        let radii = [
            bx.border_top_left_radius,
            bx.border_top_right_radius,
            bx.border_bottom_right_radius,
            bx.border_bottom_left_radius,
        ];
        let has_radius = radii.iter().any(|&r| r > 0);
        let has_css_visuals = bx.bg_color != 0
            || bx.border_top_width > 0
            || bx.border_right_width > 0
            || bx.border_bottom_width > 0
            || bx.border_left_width > 0
            || has_radius
            || !bx.box_shadows.is_empty()
            || !matches!(bx.background_image, crate::style::BackgroundImageVal::None);
        let paint_default_chrome = !bx.appearance_none && !has_css_visuals;
        if !paint_default_chrome {
            // Author CSS already painted the control box during normal box
            // rendering.  Keep the form text/cursor layer transparent so
            // modern rounded search bars and button containers remain intact.
        } else if has_radius {
            self.push(
                x,
                y,
                bx.width,
                bx.height,
                DrawKind::RoundedRect { color: bg, radii },
            );
            self.push(
                x,
                y,
                bx.width,
                bx.height,
                DrawKind::RoundedBorder {
                    color: border_color,
                    radii,
                    widths: [
                        bx.border_top_width.max(1),
                        bx.border_right_width.max(1),
                        bx.border_bottom_width.max(1),
                        bx.border_left_width.max(1),
                    ],
                },
            );
        } else {
            self.push(x, y, bx.width, bx.height, DrawKind::Rect { color: bg });
            self.push(
                x,
                y,
                bx.width,
                1,
                DrawKind::Rect {
                    color: border_color,
                },
            );
            self.push(
                x,
                y + bx.height - 1,
                bx.width,
                1,
                DrawKind::Rect {
                    color: border_color,
                },
            );
            self.push(
                x,
                y,
                1,
                bx.height,
                DrawKind::Rect {
                    color: border_color,
                },
            );
            self.push(
                x + bx.width - 1,
                y,
                1,
                bx.height,
                DrawKind::Rect {
                    color: border_color,
                },
            );
        }
        // Show placeholder or value text.
        let text = if let Some(ref v) = bx.form_value {
            if !v.is_empty() {
                Some((v.clone(), if bx.color != 0 { bx.color } else { 0xFF000000 }))
            } else if let Some(ref ph) = bx.form_placeholder {
                Some((ph.clone(), 0xFF999999))
            } else {
                None
            }
        } else if let Some(ref ph) = bx.form_placeholder {
            Some((ph.clone(), 0xFF999999))
        } else {
            None
        };
        if let Some((txt, color)) = text {
            let font_size = bx.font_size.max(1) as u16;
            let font_id = crate::layout::resolve_font_id(bx.custom_font_id, bx.bold, bx.italic);
            let tx = x + bx.padding.left.max(4);
            let ty = y + (bx.height - font_size as i32) / 2;
            self.push(
                tx,
                ty,
                (bx.width - bx.padding.left.max(4) - bx.padding.right.max(4)).max(0),
                font_size as i32,
                DrawKind::Text {
                    color,
                    font_id,
                    font_size,
                    scale_x_percent: 100,
                    synthetic_bold: bx.bold && bx.custom_font_id != 0,
                    text: txt,
                },
            );
        }
    }

    /// Draw a checkbox as a small square box.
    fn emit_checkbox(&mut self, x: i32, y: i32, bx: &LayoutBox) {
        let sz = bx.height.min(bx.width).min(16);
        let cx = x + (bx.width - sz) / 2;
        let cy = y + (bx.height - sz) / 2;
        let bg = if bx.form_checked {
            self.effective_accent_color(bx)
        } else {
            self.default_control_bg(bx)
        };
        let border = if bx.form_checked {
            self.effective_accent_color(bx)
        } else {
            self.default_control_border(bx)
        };
        self.push(cx, cy, sz, sz, DrawKind::Rect { color: bg });
        // Border.
        self.push(cx, cy, sz, 1, DrawKind::Rect { color: border });
        self.push(cx, cy + sz - 1, sz, 1, DrawKind::Rect { color: border });
        self.push(cx, cy, 1, sz, DrawKind::Rect { color: border });
        self.push(cx + sz - 1, cy, 1, sz, DrawKind::Rect { color: border });
        if bx.form_checked && sz >= 10 {
            let check = 0xFFFFFFFF;
            self.push(cx + 3, cy + sz / 2, 2, 1, DrawKind::Rect { color: check });
            self.push(
                cx + 4,
                cy + sz / 2 + 1,
                2,
                1,
                DrawKind::Rect { color: check },
            );
            self.push(
                cx + 5,
                cy + sz / 2 + 2,
                2,
                1,
                DrawKind::Rect { color: check },
            );
            self.push(
                cx + 6,
                cy + sz / 2 + 1,
                2,
                1,
                DrawKind::Rect { color: check },
            );
            self.push(cx + 7, cy + sz / 2, 2, 1, DrawKind::Rect { color: check });
            self.push(
                cx + 8,
                cy + sz / 2 - 1,
                1,
                1,
                DrawKind::Rect { color: check },
            );
        }
    }

    /// Draw a radio button as a rounded rectangle (circle approximation).
    fn emit_radio(&mut self, x: i32, y: i32, bx: &LayoutBox) {
        let sz = bx.height.min(bx.width).min(16);
        let cx = x + (bx.width - sz) / 2;
        let cy = y + (bx.height - sz) / 2;
        let r = sz / 2;
        // Simple circle: rounded rect with radius = half size.
        self.push(
            cx,
            cy,
            sz,
            sz,
            DrawKind::RoundedRect {
                color: self.default_control_bg(bx),
                radii: [r, r, r, r],
            },
        );
        // Border ring.
        let border = self.default_control_border(bx);
        self.push(cx + 1, cy, sz - 2, 1, DrawKind::Rect { color: border });
        self.push(
            cx + 1,
            cy + sz - 1,
            sz - 2,
            1,
            DrawKind::Rect { color: border },
        );
        self.push(cx, cy + 1, 1, sz - 2, DrawKind::Rect { color: border });
        self.push(
            cx + sz - 1,
            cy + 1,
            1,
            sz - 2,
            DrawKind::Rect { color: border },
        );
        if bx.form_checked && sz >= 8 {
            let inner = (sz / 2).max(4);
            let inset = (sz - inner) / 2;
            let ir = inner / 2;
            self.push(
                cx + inset,
                cy + inset,
                inner,
                inner,
                DrawKind::RoundedRect {
                    color: self.effective_accent_color(bx),
                    radii: [ir, ir, ir, ir],
                },
            );
        }
    }

    /// Draw a simple single-select dropdown for `appearance: none`.
    fn emit_select_canvas(&mut self, x: i32, y: i32, bx: &LayoutBox) {
        let bg = self.default_control_bg(bx);
        let border = self.default_control_border(bx);
        let fg = self.default_control_fg(bx);
        self.push(x, y, bx.width, bx.height, DrawKind::Rect { color: bg });
        self.push(x, y, bx.width, 1, DrawKind::Rect { color: border });
        self.push(
            x,
            y + bx.height - 1,
            bx.width,
            1,
            DrawKind::Rect { color: border },
        );
        self.push(x, y, 1, bx.height, DrawKind::Rect { color: border });
        self.push(
            x + bx.width - 1,
            y,
            1,
            bx.height,
            DrawKind::Rect { color: border },
        );

        if let Some(text) = bx.text.as_deref() {
            if !text.is_empty() {
                let font_size = bx.font_size.max(1) as u16;
                let ty = y + (bx.height - font_size as i32) / 2;
                self.push(
                    x + 6,
                    ty,
                    (bx.width - 22).max(0),
                    font_size as i32,
                    DrawKind::Text {
                        color: fg,
                        font_id: 0,
                        font_size,
                        scale_x_percent: 100,
                        synthetic_bold: false,
                        text: String::from(text),
                    },
                );
            }
        }

        let arrow_x = x + bx.width - 12;
        let arrow_y = y + bx.height / 2 - 1;
        self.push(arrow_x, arrow_y, 5, 1, DrawKind::Rect { color: fg });
        self.push(arrow_x + 1, arrow_y + 1, 3, 1, DrawKind::Rect { color: fg });
        self.push(arrow_x + 2, arrow_y + 2, 1, 1, DrawKind::Rect { color: fg });
    }

    /// Draw an `<input type="range">` as a track with a thumb indicator.
    fn emit_range(&mut self, x: i32, y: i32, bx: &LayoutBox) {
        // Decode percentage from form_value (encoded as 0..1000 or "X" for 100%).
        let pct = if let Some(ref v) = bx.form_value {
            if v == "X" {
                1.0f32
            } else {
                let bytes = v.as_bytes();
                let n = bytes
                    .iter()
                    .fold(0i32, |acc, &b| acc * 10 + (b - b'0') as i32);
                (n as f32) / 1000.0
            }
        } else {
            0.5
        };

        let w = bx.width;
        let h = bx.height;
        let track_h = 6;
        let track_y = y + (h - track_h) / 2;
        let r = track_h / 2;

        // Track background (light gray, rounded).
        let accent = self.effective_accent_color(bx);
        self.push(
            x,
            track_y,
            w,
            track_h,
            DrawKind::RoundedRect {
                color: if bx.uses_dark_color_scheme {
                    0xFF3A3A3A
                } else {
                    0xFFE0E0E0
                },
                radii: [r, r, r, r],
            },
        );

        // Filled portion (blue).
        let fill_w = ((w as f32) * pct) as i32;
        if fill_w > 0 {
            self.push(
                x,
                track_y,
                fill_w,
                track_h,
                DrawKind::RoundedRect {
                    color: accent,
                    radii: [r, r, r, r],
                },
            );
        }

        // Thumb circle.
        let thumb_sz = 16;
        let thumb_r = thumb_sz / 2;
        let thumb_x = x + fill_w - thumb_r;
        let thumb_y = y + (h - thumb_sz) / 2;
        self.push(
            thumb_x,
            thumb_y,
            thumb_sz,
            thumb_sz,
            DrawKind::RoundedRect {
                color: accent,
                radii: [thumb_r, thumb_r, thumb_r, thumb_r],
            },
        );
        // Thumb border.
        self.push(
            thumb_x + 1,
            thumb_y,
            thumb_sz - 2,
            1,
            DrawKind::Rect { color: accent },
        );
        self.push(
            thumb_x + 1,
            thumb_y + thumb_sz - 1,
            thumb_sz - 2,
            1,
            DrawKind::Rect { color: accent },
        );
        self.push(
            thumb_x,
            thumb_y + 1,
            1,
            thumb_sz - 2,
            DrawKind::Rect { color: accent },
        );
        self.push(
            thumb_x + thumb_sz - 1,
            thumb_y + 1,
            1,
            thumb_sz - 2,
            DrawKind::Rect { color: accent },
        );
    }

    /// Draw a `<progress>` element as a track with a colored fill bar.
    fn emit_progress(&mut self, x: i32, y: i32, bx: &LayoutBox) {
        // Decode percentage from form_value (encoded as 0..1000 or "X" for 100%).
        let pct = if let Some(ref v) = bx.form_value {
            if v == "X" {
                1.0f32
            } else {
                let bytes = v.as_bytes();
                let n = bytes
                    .iter()
                    .fold(0i32, |acc, &b| acc * 10 + (b - b'0') as i32);
                (n as f32) / 1000.0
            }
        } else {
            0.0
        };

        let w = bx.width;
        let h = bx.height;
        let r = 4;

        // Track background (light gray, rounded).
        let accent = self.effective_accent_color(bx);
        self.push(
            x,
            y,
            w,
            h,
            DrawKind::RoundedRect {
                color: if bx.uses_dark_color_scheme {
                    0xFF3A3A3A
                } else {
                    0xFFE0E0E0
                },
                radii: [r, r, r, r],
            },
        );

        // Fill bar (blue).
        let fill_w = ((w as f32) * pct) as i32;
        if fill_w > 0 {
            let fr = if pct >= 0.99 { r } else { 0 };
            self.push(
                x,
                y,
                fill_w,
                h,
                DrawKind::RoundedRect {
                    color: accent,
                    radii: [r, fr, fr, r],
                },
            );
        }
    }

    /// Draw a color swatch for `<input type="color">`.
    fn emit_color_swatch(&mut self, x: i32, y: i32, bx: &LayoutBox) {
        let val = bx.form_value.as_deref().unwrap_or("#000000");
        let color = parse_color_value(val);
        let w = bx.width;
        let h = bx.height;
        // Border.
        self.push(x, y, w, h, DrawKind::Rect { color: 0xFF767676 });
        // Inner swatch (2px border).
        if w > 4 && h > 4 {
            self.push(x + 2, y + 2, w - 4, h - 4, DrawKind::Rect { color });
        }
    }

    /// Draw a `<meter>` element — like progress but with green/yellow/red coloring.
    fn emit_meter(&mut self, x: i32, y: i32, bx: &LayoutBox) {
        let pct = if let Some(ref v) = bx.form_value {
            if v == "X" {
                1.0f32
            } else {
                let bytes = v.as_bytes();
                let n = bytes
                    .iter()
                    .fold(0i32, |acc, &b| acc * 10 + (b - b'0') as i32);
                (n as f32) / 1000.0
            }
        } else {
            0.0
        };

        let w = bx.width;
        let h = bx.height;
        let r = 4;

        // Track background.
        self.push(
            x,
            y,
            w,
            h,
            DrawKind::RoundedRect {
                color: if bx.uses_dark_color_scheme {
                    0xFF3A3A3A
                } else {
                    0xFFE0E0E0
                },
                radii: [r, r, r, r],
            },
        );

        // Fill bar — color depends on value (green=ok, yellow=suboptimal, red=danger).
        let fill_color = if pct < 0.25 {
            0xFFE74C3C // red
        } else if pct < 0.75 {
            0xFFF1C40F // yellow
        } else {
            0xFF2ECC71 // green
        };
        let fill_w = ((w as f32) * pct) as i32;
        if fill_w > 0 {
            let fr = if pct >= 0.99 { r } else { 0 };
            self.push(
                x,
                y,
                fill_w,
                h,
                DrawKind::RoundedRect {
                    color: fill_color,
                    radii: [r, fr, fr, r],
                },
            );
        }
    }

    /// Draw a `<select>` dropdown as a text field with a dropdown arrow.
    fn emit_select(&mut self, x: i32, y: i32, bx: &LayoutBox) {
        let bg = self.default_control_bg(bx);
        let border_color = self.default_control_border(bx);

        // Background fill.
        self.push(x, y, bx.width, bx.height, DrawKind::Rect { color: bg });
        // 1px border.
        self.push(
            x,
            y,
            bx.width,
            1,
            DrawKind::Rect {
                color: border_color,
            },
        );
        self.push(
            x,
            y + bx.height - 1,
            bx.width,
            1,
            DrawKind::Rect {
                color: border_color,
            },
        );
        self.push(
            x,
            y,
            1,
            bx.height,
            DrawKind::Rect {
                color: border_color,
            },
        );
        self.push(
            x + bx.width - 1,
            y,
            1,
            bx.height,
            DrawKind::Rect {
                color: border_color,
            },
        );

        // Draw the selected option text.
        if let Some(ref txt) = bx.text {
            let font_size = bx.font_size.max(1) as u16;
            let text_color = if bx.color != 0 { bx.color } else { 0xFF000000 };
            let tx = x + 6;
            let ty = y + (bx.height - font_size as i32) / 2;
            self.push(
                tx,
                ty,
                bx.width - 30,
                font_size as i32,
                DrawKind::Text {
                    color: text_color,
                    font_id: 0,
                    font_size,
                    scale_x_percent: 100,
                    synthetic_bold: false,
                    text: txt.clone(),
                },
            );
        }

        // Dropdown arrow indicator (small downward-pointing triangle).
        let arrow_x = x + bx.width - 20;
        let arrow_y = y + bx.height / 2 - 2;
        // Draw a simple 8x5 downward triangle using horizontal lines.
        for i in 0..5i32 {
            let lx = arrow_x + i;
            let lw = (8 - i * 2).max(0);
            if lw > 0 {
                self.push(lx, arrow_y + i, lw, 1, DrawKind::Rect { color: 0xFF555555 });
            }
        }
    }

    fn push(&mut self, x: i32, y: i32, w: i32, h: i32, kind: DrawKind) {
        let (draw_x, draw_y, draw_w, draw_h) = transformed_bounds(x, y, w, h, &self.rotation_stack);
        if !self.rotation_stack.is_empty() {
            crate::debug_surf!(
                "[renderer] push rotated src=({},{} {}x{}) dst=({},{} {}x{}) rotations={}",
                x,
                y,
                w,
                h,
                draw_x,
                draw_y,
                draw_w,
                draw_h,
                self.rotation_stack.len()
            );
        }
        if let Some((y_start, y_end)) = self.cull_y_range {
            if draw_y + draw_h <= y_start || draw_y >= y_end {
                return;
            }
        }
        if draw_h > self.max_h {
            self.max_h = draw_h;
        }
        let clip = self.clip_stack.last().copied();
        let opacity = self.opacity_stack.last().copied().unwrap_or(255);
        self.cmds.push(DrawCmd {
            x: draw_x,
            y: draw_y,
            w: draw_w,
            h: draw_h,
            src_x: x,
            src_y: y,
            src_w: w,
            src_h: h,
            kind,
            clip,
            masks: self.mask_stack.clone(),
            rounded_clips: self.rounded_clip_stack.clone(),
            opacity,
            rotations: self.rotation_stack.clone(),
        });
    }
}

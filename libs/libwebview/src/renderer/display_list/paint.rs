impl DisplayList {
    fn emit_border_edge(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
        style: crate::style::BorderStyleVal,
        vertical: bool,
    ) {
        use crate::style::BorderStyleVal;
        match style {
            BorderStyleVal::Dashed => {
                // Dashed: dash_len = 3 * border_width, gap = same
                let bw = if vertical { w } else { h };
                let dash = (bw * 3).max(3);
                self.push(
                    x,
                    y,
                    w,
                    h,
                    DrawKind::DashedLine {
                        color,
                        dash_len: dash,
                        gap_len: dash,
                        vertical,
                    },
                );
            }
            BorderStyleVal::Dotted => {
                // Dotted: dash = border_width (square dots), gap = border_width
                let bw = if vertical { w } else { h };
                let dot = bw.max(1);
                self.push(
                    x,
                    y,
                    w,
                    h,
                    DrawKind::DashedLine {
                        color,
                        dash_len: dot,
                        gap_len: dot,
                        vertical,
                    },
                );
            }
            BorderStyleVal::Double => {
                // Double: two lines with a gap in between.
                let bw = if vertical { w } else { h };
                let line_w = (bw / 3).max(1);
                if vertical {
                    // Two vertical lines, left and right of the border area.
                    self.push(x, y, line_w, h, DrawKind::Rect { color });
                    self.push(x + w - line_w, y, line_w, h, DrawKind::Rect { color });
                } else {
                    // Two horizontal lines, top and bottom of the border area.
                    self.push(x, y, w, line_w, DrawKind::Rect { color });
                    self.push(x, y + h - line_w, w, line_w, DrawKind::Rect { color });
                }
            }
            BorderStyleVal::Groove => {
                // Groove: 3D inset effect — dark top-left, light bottom-right.
                let half = if vertical { w / 2 } else { h / 2 };
                let half = half.max(1);
                let dark = darken_color(color, 60);
                let light = lighten_color(color, 60);
                if vertical {
                    self.push(x, y, half, h, DrawKind::Rect { color: dark });
                    self.push(x + half, y, w - half, h, DrawKind::Rect { color: light });
                } else {
                    self.push(x, y, w, half, DrawKind::Rect { color: dark });
                    self.push(x, y + half, w, h - half, DrawKind::Rect { color: light });
                }
            }
            BorderStyleVal::Ridge => {
                // Ridge: opposite of groove.
                let half = if vertical { w / 2 } else { h / 2 };
                let half = half.max(1);
                let dark = darken_color(color, 60);
                let light = lighten_color(color, 60);
                if vertical {
                    self.push(x, y, half, h, DrawKind::Rect { color: light });
                    self.push(x + half, y, w - half, h, DrawKind::Rect { color: dark });
                } else {
                    self.push(x, y, w, half, DrawKind::Rect { color: light });
                    self.push(x, y + half, w, h - half, DrawKind::Rect { color: dark });
                }
            }
            BorderStyleVal::Inset => {
                let dark = darken_color(color, 80);
                self.push(x, y, w, h, DrawKind::Rect { color: dark });
            }
            BorderStyleVal::Outset => {
                let light = lighten_color(color, 80);
                self.push(x, y, w, h, DrawKind::Rect { color: light });
            }
            BorderStyleVal::None | BorderStyleVal::Hidden => {}
            _ => {
                // Solid (default)
                self.push(x, y, w, h, DrawKind::Rect { color });
            }
        }
    }

    fn emit_collapsed_border_triangles(
        &mut self,
        abs_x: i32,
        abs_y: i32,
        bx: &LayoutBox,
        ts: crate::style::BorderStyleVal,
        rs: crate::style::BorderStyleVal,
        bs: crate::style::BorderStyleVal,
        ls: crate::style::BorderStyleVal,
    ) {
        let w = bx.border_left_width + bx.border_right_width;
        let h = bx.border_top_width + bx.border_bottom_width;
        let apex_x = bx.border_left_width;
        let apex_y = bx.border_top_width;
        crate::debug_surf!(
            "[renderer] collapsed-border-triangles x={} y={} w={} h={} borders=({}, {}, {}, {}) colors=({:08X}, {:08X}, {:08X}, {:08X})",
            abs_x,
            abs_y,
            w,
            h,
            bx.border_top_width,
            bx.border_right_width,
            bx.border_bottom_width,
            bx.border_left_width,
            bx.border_top_color,
            bx.border_right_color,
            bx.border_bottom_color,
            bx.border_left_color
        );

        if bx.border_top_width > 0 && bx.border_top_color != 0 {
            self.emit_border_triangle(
                abs_x,
                abs_y,
                w,
                h,
                bx.border_top_color,
                ts,
                (0, bx.border_top_width),
                (w, bx.border_top_width),
                (apex_x, 0),
            );
        }
        if bx.border_bottom_width > 0 && bx.border_bottom_color != 0 {
            self.emit_border_triangle(
                abs_x,
                abs_y,
                w,
                h,
                bx.border_bottom_color,
                bs,
                (0, h - bx.border_bottom_width),
                (w, h - bx.border_bottom_width),
                (apex_x, h),
            );
        }
        if bx.border_left_width > 0 && bx.border_left_color != 0 {
            self.emit_border_triangle(
                abs_x,
                abs_y,
                w,
                h,
                bx.border_left_color,
                ls,
                (bx.border_left_width, 0),
                (bx.border_left_width, h),
                (0, apex_y),
            );
        }
        if bx.border_right_width > 0 && bx.border_right_color != 0 {
            self.emit_border_triangle(
                abs_x,
                abs_y,
                w,
                h,
                bx.border_right_color,
                rs,
                (w - bx.border_right_width, 0),
                (w - bx.border_right_width, h),
                (w, apex_y),
            );
        }
    }

    fn emit_border_triangle(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
        style: crate::style::BorderStyleVal,
        p0: (i32, i32),
        p1: (i32, i32),
        p2: (i32, i32),
    ) {
        use crate::style::BorderStyleVal;
        match style {
            BorderStyleVal::None | BorderStyleVal::Hidden => {}
            _ => self.push(x, y, w, h, DrawKind::Triangle { color, p0, p1, p2 }),
        }
    }

    /// Get per-side border styles from the LayoutBox.
    fn border_styles_for(
        &self,
        bx: &LayoutBox,
    ) -> (
        crate::style::BorderStyleVal,
        crate::style::BorderStyleVal,
        crate::style::BorderStyleVal,
        crate::style::BorderStyleVal,
    ) {
        use crate::style::BorderStyleVal;
        let fallback = BorderStyleVal::Solid;
        let ts = if bx.border_top_style != BorderStyleVal::None {
            bx.border_top_style
        } else {
            fallback
        };
        let rs = if bx.border_right_style != BorderStyleVal::None {
            bx.border_right_style
        } else {
            fallback
        };
        let bs = if bx.border_bottom_style != BorderStyleVal::None {
            bx.border_bottom_style
        } else {
            fallback
        };
        let ls = if bx.border_left_style != BorderStyleVal::None {
            bx.border_left_style
        } else {
            fallback
        };
        (ts, rs, bs, ls)
    }

    /// Emit draw commands for a linear gradient background.
    /// Emit a text decoration line (underline/overline/line-through) with style support.
    fn emit_text_deco_line(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        thickness: i32,
        color: u32,
        style: crate::style::TextDecorationStyle,
    ) {
        use crate::style::TextDecorationStyle;
        match style {
            TextDecorationStyle::Solid => {
                self.push(x, y, w, thickness, DrawKind::Rect { color });
            }
            TextDecorationStyle::Double => {
                let t = thickness.max(1);
                self.push(x, y, w, t, DrawKind::Rect { color });
                self.push(x, y + t * 2, w, t, DrawKind::Rect { color });
            }
            TextDecorationStyle::Dotted => {
                self.push(
                    x,
                    y,
                    w,
                    thickness,
                    DrawKind::DashedLine {
                        color,
                        dash_len: thickness,
                        gap_len: thickness,
                        vertical: false,
                    },
                );
            }
            TextDecorationStyle::Dashed => {
                let dash = (thickness * 3).max(3);
                self.push(
                    x,
                    y,
                    w,
                    thickness,
                    DrawKind::DashedLine {
                        color,
                        dash_len: dash,
                        gap_len: dash,
                        vertical: false,
                    },
                );
            }
            TextDecorationStyle::Wavy => {
                // Approximate wavy as alternating up/down segments.
                let wave_len = (thickness * 4).max(4);
                let half = wave_len / 2;
                let mut pos = 0;
                while pos < w {
                    let seg = half.min(w - pos);
                    // Up segment
                    self.push(
                        x + pos,
                        y - thickness,
                        seg,
                        thickness,
                        DrawKind::Rect { color },
                    );
                    pos += half;
                    if pos >= w {
                        break;
                    }
                    let seg = half.min(w - pos);
                    // Down segment
                    self.push(
                        x + pos,
                        y + thickness,
                        seg,
                        thickness,
                        DrawKind::Rect { color },
                    );
                    pos += half;
                }
            }
        }
    }

    fn emit_background_image(&mut self, abs_x: i32, abs_y: i32, bx: &LayoutBox) {
        use crate::style::BackgroundImageVal;
        if matches!(bx.background_clip, BackgroundClipVal::Text) {
            return;
        }
        let (bg_x, bg_y, bg_w, bg_h) = self.background_paint_rect(abs_x, abs_y, bx);
        if bg_w <= 0 || bg_h <= 0 {
            return;
        }
        match &bx.background_image {
            BackgroundImageVal::LinearGradient { angle_deg, stops } => {
                if stops.len() < 2 {
                    return;
                }
                let angle = *angle_deg;
                let is_horizontal = angle == 90 || angle == 270;
                let is_vertical = angle == 0 || angle == 180;

                if is_horizontal || is_vertical {
                    // Axis-aligned gradients can be sampled exactly along one pixel axis.
                    // Coalescing adjacent pixels with the same color keeps hard stops exact
                    // without exploding the display list for common solid bands.
                    let dimension = if is_horizontal { bg_w } else { bg_h };
                    if dimension <= 0 {
                        return;
                    }

                    let reversed = angle == 270 || angle == 0;
                    let mut run_start = 0;
                    let mut run_color: Option<u32> = None;

                    for pos in 0..dimension {
                        let t_raw = (((pos as i64 * 10000) + (dimension as i64 / 2))
                            / dimension as i64)
                            .min(10000) as i32;
                        let t = if reversed { 10000 - t_raw } else { t_raw };
                        let color = interpolate_gradient_color(stops, t);

                        if let Some(previous) = run_color {
                            if previous != color {
                                if is_horizontal {
                                    self.push(
                                        bg_x + run_start,
                                        bg_y,
                                        pos - run_start,
                                        bg_h,
                                        DrawKind::Rect { color: previous },
                                    );
                                } else {
                                    self.push(
                                        bg_x,
                                        bg_y + run_start,
                                        bg_w,
                                        pos - run_start,
                                        DrawKind::Rect { color: previous },
                                    );
                                }
                                run_start = pos;
                                run_color = Some(color);
                            }
                        } else {
                            run_color = Some(color);
                        }
                    }

                    if let Some(color) = run_color {
                        if is_horizontal {
                            self.push(
                                bg_x + run_start,
                                bg_y,
                                dimension - run_start,
                                bg_h,
                                DrawKind::Rect { color },
                            );
                        } else {
                            self.push(
                                bg_x,
                                bg_y + run_start,
                                bg_w,
                                dimension - run_start,
                                DrawKind::Rect { color },
                            );
                        }
                    }
                } else {
                    // Diagonal gradient: decompose into scanline stripes.
                    // Project each scanline position onto the gradient axis.
                    // Gradient direction vector from angle (CSS angles: 0=up, 90=right, 180=down).
                    let rad = (angle as f32 - 90.0) * core::f32::consts::PI / 180.0;
                    let dx = cos_approx(rad);
                    let dy = sin_approx(rad);
                    // Gradient length = projection of the rect diagonal onto the direction.
                    let w_f = bg_w as f32;
                    let h_f = bg_h as f32;
                    let half_w = w_f / 2.0;
                    let half_h = h_f / 2.0;
                    let grad_len = (dx.abs() * w_f + dy.abs() * h_f).max(1.0);

                    // Render as horizontal scan-line stripes, max 64 for perf.
                    let stripe_count = bg_h.min(64).max(2);
                    let stripe_h = bg_h / stripe_count;
                    if stripe_h <= 0 {
                        return;
                    }

                    for i in 0..stripe_count {
                        let cy =
                            (i * bg_h / stripe_count) as f32 + stripe_h as f32 / 2.0 - half_h;
                        let cx = 0.0_f32; // center of scanline
                        let proj = (cx * dx + cy * dy) / grad_len + 0.5;
                        let t = (proj * 10000.0).max(0.0).min(10000.0) as i32;
                        let color = interpolate_gradient_color(stops, t);
                        let sy = bg_y + i * stripe_h;
                        let sh = if i == stripe_count - 1 {
                            bg_h - i * stripe_h
                        } else {
                            stripe_h
                        };
                        self.push(bg_x, sy, bg_w, sh, DrawKind::Rect { color });
                    }
                }
            }
            BackgroundImageVal::Url(ref src) => {
                if !src.is_empty() {
                    let (img_w, img_h, object_fit) = match bx.background_size {
                        crate::style::BackgroundSizeVal::Explicit(w, h) => (
                            if w > 0 { w } else { bg_w },
                            if h > 0 { h } else { bg_h },
                            crate::style::ObjectFit::Fill,
                        ),
                        crate::style::BackgroundSizeVal::Cover => {
                            (bg_w, bg_h, crate::style::ObjectFit::Cover)
                        }
                        crate::style::BackgroundSizeVal::Contain => {
                            (bg_w, bg_h, crate::style::ObjectFit::Contain)
                        }
                        crate::style::BackgroundSizeVal::Auto => {
                            (bg_w, bg_h, crate::style::ObjectFit::None)
                        }
                    };
                    let img_x = match bx.background_position_x {
                        5000 | 10000 => bg_x,
                        px => bg_x + px,
                    };
                    let img_y = match bx.background_position_y {
                        5000 | 10000 => bg_y,
                        px => bg_y + px,
                    };
                    let pushed_clip = self.push_clip_rect((bg_x, bg_y, bg_w, bg_h));
                    self.push(
                        img_x,
                        img_y,
                        img_w,
                        img_h,
                        DrawKind::Image {
                            src: src.clone(),
                            object_fit,
                            object_position_x: 0,
                            object_position_x_is_percent: false,
                            object_position_y: 0,
                            object_position_y_is_percent: false,
                        },
                    );
                    if pushed_clip {
                        self.clip_stack.pop();
                    }
                }
            }
            _ => {}
        }
    }

    fn default_accent_color(&self) -> u32 { 0xFF0A84FF }

    fn effective_accent_color(&self, bx: &LayoutBox) -> u32 {
        if bx.accent_color != 0 {
            bx.accent_color
        } else {
            self.default_accent_color()
        }
    }

    fn default_control_bg(&self, bx: &LayoutBox) -> u32 {
        if bx.bg_color != 0 {
            bx.bg_color
        } else if bx.uses_dark_color_scheme {
            0xFF1E1E1E
        } else {
            0xFFFFFFFF
        }
    }

    fn default_control_fg(&self, bx: &LayoutBox) -> u32 {
        if bx.color != 0 {
            bx.color
        } else if bx.uses_dark_color_scheme {
            0xFFF5F5F5
        } else {
            0xFF000000
        }
    }

    fn default_control_border(&self, bx: &LayoutBox) -> u32 {
        if bx.uses_dark_color_scheme {
            0xFF8A8A8A
        } else {
            0xFF767676
        }
    }

    fn background_paint_rect(&self, abs_x: i32, abs_y: i32, bx: &LayoutBox) -> (i32, i32, i32, i32) {
        self.box_area_rect(abs_x, abs_y, bx, bx.background_clip)
    }

    fn box_area_rect(
        &self,
        abs_x: i32,
        abs_y: i32,
        bx: &LayoutBox,
        area: BackgroundClipVal,
    ) -> (i32, i32, i32, i32) {
        let left_inset = match area {
            BackgroundClipVal::BorderBox => 0,
            BackgroundClipVal::PaddingBox => bx.border_left_width.max(0),
            BackgroundClipVal::ContentBox => (bx.border_left_width + bx.padding.left).max(0),
            BackgroundClipVal::Text => (bx.border_left_width + bx.padding.left).max(0),
        };
        let right_inset = match area {
            BackgroundClipVal::BorderBox => 0,
            BackgroundClipVal::PaddingBox => bx.border_right_width.max(0),
            BackgroundClipVal::ContentBox => (bx.border_right_width + bx.padding.right).max(0),
            BackgroundClipVal::Text => (bx.border_right_width + bx.padding.right).max(0),
        };
        let top_inset = match area {
            BackgroundClipVal::BorderBox => 0,
            BackgroundClipVal::PaddingBox => bx.border_top_width.max(0),
            BackgroundClipVal::ContentBox => (bx.border_top_width + bx.padding.top).max(0),
            BackgroundClipVal::Text => (bx.border_top_width + bx.padding.top).max(0),
        };
        let bottom_inset = match area {
            BackgroundClipVal::BorderBox => 0,
            BackgroundClipVal::PaddingBox => bx.border_bottom_width.max(0),
            BackgroundClipVal::ContentBox => (bx.border_bottom_width + bx.padding.bottom).max(0),
            BackgroundClipVal::Text => (bx.border_bottom_width + bx.padding.bottom).max(0),
        };
        let w = (bx.width - left_inset - right_inset).max(0);
        let h = (bx.height - top_inset - bottom_inset).max(0);
        (abs_x + left_inset, abs_y + top_inset, w, h)
    }

    fn background_clip_radii(&self, bx: &LayoutBox, bg_w: i32, bg_h: i32) -> [i32; 4] {
        let radii = self.border_radii_for_rect(bx, bg_w, bg_h);
        let inset_x = match bx.background_clip {
            BackgroundClipVal::BorderBox => 0,
            BackgroundClipVal::PaddingBox => bx.border_left_width.max(bx.border_right_width).max(0),
            BackgroundClipVal::ContentBox => (bx.border_left_width + bx.padding.left)
                .max(bx.border_right_width + bx.padding.right)
                .max(0),
            BackgroundClipVal::Text => (bx.border_left_width + bx.padding.left)
                .max(bx.border_right_width + bx.padding.right)
                .max(0),
        };
        let inset_y = match bx.background_clip {
            BackgroundClipVal::BorderBox => 0,
            BackgroundClipVal::PaddingBox => bx.border_top_width.max(bx.border_bottom_width).max(0),
            BackgroundClipVal::ContentBox => (bx.border_top_width + bx.padding.top)
                .max(bx.border_bottom_width + bx.padding.bottom)
                .max(0),
            BackgroundClipVal::Text => (bx.border_top_width + bx.padding.top)
                .max(bx.border_bottom_width + bx.padding.bottom)
                .max(0),
        };
        [
            (radii[0] - inset_x.max(inset_y)).max(0),
            (radii[1] - inset_x.max(inset_y)).max(0),
            (radii[2] - inset_x.max(inset_y)).max(0),
            (radii[3] - inset_x.max(inset_y)).max(0),
        ]
    }

    fn should_clip_background(&self, bx: &LayoutBox) -> bool {
        matches!(
            bx.background_clip,
            BackgroundClipVal::PaddingBox | BackgroundClipVal::ContentBox
        )
    }

    fn push_clip_rect(&mut self, new_clip: (i32, i32, i32, i32)) -> bool {
        let clip = if let Some(&(cx, cy, cw, ch)) = self.clip_stack.last() {
            let x0 = new_clip.0.max(cx);
            let y0 = new_clip.1.max(cy);
            let x1 = (new_clip.0 + new_clip.2).min(cx + cw);
            let y1 = (new_clip.1 + new_clip.3).min(cy + ch);
            if x1 > x0 && y1 > y0 {
                (x0, y0, x1 - x0, y1 - y0)
            } else {
                (0, 0, 0, 0)
            }
        } else {
            new_clip
        };
        self.clip_stack.push(clip);
        true
    }
}

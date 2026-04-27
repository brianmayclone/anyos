impl DisplayList {
    pub fn new() -> Self {
        Self {
            cmds: Vec::new(),
            max_h: 0,
            clip_stack: Vec::new(),
            mask_stack: Vec::new(),
            rotation_stack: Vec::new(),
            cull_y_range: None,
        }
    }

    /// Build the display list from a layout tree.  Walks the tree once
    /// in CSS2 Appendix E stacking order, emitting DrawCmds back-to-front.
    /// The root element always forms the initial stacking context.
    pub fn build(root: &LayoutBox) -> Self {
        let mut dl = DisplayList {
            cmds: Vec::new(),
            max_h: 0,
            clip_stack: Vec::new(),
            mask_stack: Vec::new(),
            rotation_stack: Vec::new(),
            cull_y_range: None,
        };
        dl.flatten(root, 0, 0, None);
        dl
    }

    /// Build only commands overlapping the given document-space Y band.
    ///
    /// This is used for the first visible paint so we can show a usable,
    /// styled viewport before building the full display list on demand.
    pub fn build_visible(root: &LayoutBox, y_start: i32, y_end: i32) -> Self {
        let mut dl = DisplayList {
            cmds: Vec::new(),
            max_h: 0,
            clip_stack: Vec::new(),
            mask_stack: Vec::new(),
            rotation_stack: Vec::new(),
            cull_y_range: Some((y_start, y_end)),
        };
        dl.flatten(root, 0, 0, None);
        dl
    }

    /// Clear the display list (called on relayout / navigation).
    pub fn clear(&mut self) {
        self.cmds.clear();
        self.max_h = 0;
        self.cull_y_range = None;
    }

    fn resolve_radius_for_rect(value: i32, w: i32, h: i32) -> i32 {
        if value < 0 {
            let pct = (-value) as i64;
            ((w.min(h).max(0) as i64 * pct) / 10000) as i32
        } else {
            value
        }
    }

    fn border_radii_for_rect(&self, bx: &LayoutBox, w: i32, h: i32) -> [i32; 4] {
        [
            Self::resolve_radius_for_rect(bx.border_top_left_radius, w, h),
            Self::resolve_radius_for_rect(bx.border_top_right_radius, w, h),
            Self::resolve_radius_for_rect(bx.border_bottom_right_radius, w, h),
            Self::resolve_radius_for_rect(bx.border_bottom_left_radius, w, h),
        ]
    }

    /// Rasterize all commands overlapping `[tile_y_start, tile_y_end)` into `buf`.
    pub fn rasterize_tile(
        &self,
        images: &ImageCache,
        buf: *mut u32,
        stride: u32,
        buf_h: u32,
        tile_y_start: i32,
        tile_y_end: i32,
    ) {
        // Commands are in correct paint order (back-to-front) from the
        // stacking-context-aware tree walk.  Scan linearly.
        for i in 0..self.cmds.len() {
            let cmd = &self.cmds[i];
            // Skip commands that don't overlap the tile vertically.
            if cmd.y >= tile_y_end || cmd.y + cmd.h <= tile_y_start {
                continue;
            }

            let draw_y = cmd.y - tile_y_start;

            // Apply clip rect if present (from overflow:hidden parents).
            let (cx, cy, cw, ch) = if let Some((clip_x, clip_y, clip_w, clip_h)) = cmd.clip {
                // Clip rect is in document coordinates — adjust for tile offset.
                let clip_draw_y = clip_y - tile_y_start;
                // Intersect command rect with clip rect.
                let x0 = cmd.x.max(clip_x);
                let y0 = draw_y.max(clip_draw_y);
                let x1 = (cmd.x + cmd.w).min(clip_x + clip_w);
                let y1 = (draw_y + cmd.h).min(clip_draw_y + clip_h);
                if x1 <= x0 || y1 <= y0 {
                    continue;
                } // fully clipped
                (x0, y0, x1 - x0, y1 - y0)
            } else {
                (cmd.x, draw_y, cmd.w, cmd.h)
            };

            if cmd.masks.is_empty() {
                rasterize_draw_cmd(
                    images,
                    cmd,
                    buf,
                    stride,
                    buf_h,
                    tile_y_start,
                    draw_y,
                    (cx, cy, cw, ch),
                );
            } else {
                rasterize_masked_cmd(
                    images,
                    cmd,
                    buf,
                    stride,
                    buf_h,
                    draw_y,
                    tile_y_start,
                    (cx, cy, cw, ch),
                );
            }
        }
    }

    /// Recursively flatten the layout tree into draw commands.
    /// Children are processed in CSS2 Appendix E stacking order when the
    /// parent creates a stacking context.
    fn flatten(
        &mut self,
        bx: &LayoutBox,
        offset_x: i32,
        offset_y: i32,
        sticky_ctx: Option<StickyContext>,
    ) {
        if bx.visibility_hidden {
            return;
        }

        let orig_abs_x = if bx.is_fixed { bx.x } else { offset_x + bx.x };
        let orig_abs_y = if bx.is_fixed { bx.y } else { offset_y + bx.y };

        if let Some((y_start, y_end)) = self.cull_y_range {
            if !bx.subtree_has_viewport_positioned {
                let subtree_abs_top = orig_abs_y + bx.subtree_top;
                let subtree_abs_bottom = orig_abs_y + bx.subtree_bottom;
                if subtree_abs_bottom <= y_start || subtree_abs_top >= y_end {
                    return;
                }
            }
        }

        let sticky_abs_y = if bx.is_sticky {
            if let Some(ctx) = sticky_ctx {
                let min_y = ctx.top + bx.sticky_top;
                let max_y = (ctx.top + ctx.height - bx.height).max(min_y);
                orig_abs_y.max(min_y).min(max_y)
            } else {
                orig_abs_y
            }
        } else {
            orig_abs_y
        };

        // Apply CSS transform: scale around the resolved transform origin.
        let (abs_x, abs_y, draw_w, draw_h) = if bx.transform_sx != 1000 || bx.transform_sy != 1000 {
            let sx = bx.transform_sx as f32 / 1000.0;
            let sy = bx.transform_sy as f32 / 1000.0;
            let cx = resolve_axis_origin(
                orig_abs_x,
                bx.width,
                bx.transform_origin_x,
                bx.transform_origin_x_is_percent,
            );
            let cy = resolve_axis_origin(
                sticky_abs_y,
                bx.height,
                bx.transform_origin_y,
                bx.transform_origin_y_is_percent,
            );
            let new_w = (bx.width as f32 * sx) as i32;
            let new_h = (bx.height as f32 * sy) as i32;
            let new_x = cx + ((orig_abs_x - cx) as f32 * sx) as i32;
            let new_y = cy + ((sticky_abs_y - cy) as f32 * sy) as i32;
            (new_x, new_y, new_w, new_h)
        } else {
            (orig_abs_x, sticky_abs_y, bx.width, bx.height)
        };

        let pushed_rotation = if bx.transform_rotate != 0 {
            let origin_x = resolve_axis_origin(
                abs_x,
                draw_w,
                bx.transform_origin_x,
                bx.transform_origin_x_is_percent,
            );
            let origin_y = resolve_axis_origin(
                abs_y,
                draw_h,
                bx.transform_origin_y,
                bx.transform_origin_y_is_percent,
            );
            self.rotation_stack.push(DrawRotation {
                origin_x,
                origin_y,
                angle_deg100: bx.transform_rotate,
            });
            true
        } else {
            false
        };

        // Check if we have border-radius.
        let radii = self.border_radii_for_rect(bx, draw_w, draw_h);
        let has_radius = radii.iter().any(|&r| r > 0);

        // Box shadows (behind the background, outer shadows only).
        for shadow in &bx.box_shadows {
            if !shadow.inset {
                let sx = abs_x + shadow.offset_x - shadow.spread;
                let sy = abs_y + shadow.offset_y - shadow.spread;
                let sw = draw_w + shadow.spread * 2;
                let sh = draw_h + shadow.spread * 2;
                // Multi-pass blur approximation: draw progressively larger/fainter rects.
                if shadow.blur > 0 {
                    let steps = (shadow.blur / 2).max(1).min(6);
                    for s in 0..steps {
                        let ext = (s + 1) * shadow.blur / steps;
                        let alpha_frac = 255 / (steps + 1) / (s + 1);
                        let c = alpha_blend(shadow.color, alpha_frac as u32);
                        self.push(
                            sx - ext,
                            sy - ext,
                            sw + ext * 2,
                            sh + ext * 2,
                            DrawKind::Rect { color: c },
                        );
                    }
                }
                if has_radius {
                    self.push(
                        sx,
                        sy,
                        sw,
                        sh,
                        DrawKind::RoundedRect {
                            color: shadow.color,
                            radii,
                        },
                    );
                } else {
                    self.push(
                        sx,
                        sy,
                        sw,
                        sh,
                        DrawKind::Rect {
                            color: shadow.color,
                        },
                    );
                }
            }
        }

        // Backdrop-filter approximation: frosted glass overlay behind the element.
        if bx.backdrop_filter_blur > 0 {
            let blur_strength = (bx.backdrop_filter_blur as u32).min(20);
            let overlay_alpha = (blur_strength * 8).min(160) as u32;
            let overlay_color = 0x00FFFFFF | (overlay_alpha << 24);
            if has_radius {
                self.push(
                    abs_x,
                    abs_y,
                    draw_w,
                    draw_h,
                    DrawKind::RoundedRect {
                        color: overlay_color,
                        radii,
                    },
                );
            } else {
                self.push(
                    abs_x,
                    abs_y,
                    draw_w,
                    draw_h,
                    DrawKind::Rect {
                        color: overlay_color,
                    },
                );
            }
        }

        let (bg_x, bg_y, bg_w, bg_h) = self.background_paint_rect(abs_x, abs_y, bx);
        let bg_radii = self.background_clip_radii(bx, bg_w, bg_h);
        let has_bg_radius = bg_radii.iter().any(|&r| r > 0);

        let pushed_bg_clip = if self.should_clip_background(bx) && bg_w > 0 && bg_h > 0 {
            self.push_clip_rect((bg_x, bg_y, bg_w, bg_h))
        } else {
            false
        };

        // Background.
        if bx.bg_color != 0 && bx.bg_color != 0x00000000 && bg_w > 0 && bg_h > 0 {
            if has_bg_radius {
                self.push(
                    bg_x,
                    bg_y,
                    bg_w,
                    bg_h,
                    DrawKind::RoundedRect {
                        color: bx.bg_color,
                        radii: bg_radii,
                    },
                );
            } else {
                self.push(
                    bg_x,
                    bg_y,
                    bg_w,
                    bg_h,
                    DrawKind::Rect { color: bx.bg_color },
                );
            }
        }

        // Background image / gradient.
        self.emit_background_image(abs_x, abs_y, bx);

        if pushed_bg_clip {
            self.clip_stack.pop();
        }

        let mut pushed_mask = false;
        if !matches!(bx.mask_image, BackgroundImageVal::None) {
            let clip_rect = self.box_area_rect(abs_x, abs_y, bx, bx.mask_clip);
            if clip_rect.2 > 0 && clip_rect.3 > 0 {
                let origin_rect = self.box_area_rect(abs_x, abs_y, bx, bx.mask_origin);
                self.mask_stack.push(MaskLayer {
                    clip_rect,
                    origin_rect,
                    image: bx.mask_image.clone(),
                    size: bx.mask_size,
                    repeat: bx.mask_repeat,
                    position_x: bx.mask_position_x,
                    position_x_is_percent: bx.mask_position_x_is_percent,
                    position_y: bx.mask_position_y,
                    position_y_is_percent: bx.mask_position_y_is_percent,
                });
                pushed_mask = true;
            }
        }

        // Inset box shadows (inside the background).
        for shadow in &bx.box_shadows {
            if shadow.inset {
                let s = shadow.spread.max(1);
                let c = shadow.color;
                self.push(abs_x, abs_y, draw_w, s, DrawKind::Rect { color: c });
                self.push(
                    abs_x,
                    abs_y + draw_h - s,
                    draw_w,
                    s,
                    DrawKind::Rect { color: c },
                );
                self.push(
                    abs_x,
                    abs_y + s,
                    s,
                    (draw_h - s * 2).max(0),
                    DrawKind::Rect { color: c },
                );
                self.push(
                    abs_x + draw_w - s,
                    abs_y + s,
                    s,
                    (draw_h - s * 2).max(0),
                    DrawKind::Rect { color: c },
                );
            }
        }

        // Per-side borders (litehtml-style: each side can have different width/color/style).
        let has_per_side = bx.border_top_width > 0
            || bx.border_right_width > 0
            || bx.border_bottom_width > 0
            || bx.border_left_width > 0;
        if has_per_side {
            let w = draw_w;
            let h = draw_h;
            // Determine border styles from the node style (fallback: Solid).
            let (ts, rs, bs, ls) = self.border_styles_for(bx);
            let content_w = (w - bx.border_left_width - bx.border_right_width).max(0);
            let content_h = (h - bx.border_top_width - bx.border_bottom_width).max(0);

            if content_w == 0 && content_h == 0 {
                self.emit_collapsed_border_triangles(abs_x, abs_y, bx, ts, rs, bs, ls);
            } else {
            // Top border
                if bx.border_top_width > 0 && bx.border_top_color != 0 {
                    self.emit_border_edge(
                        abs_x,
                        abs_y,
                        w,
                        bx.border_top_width,
                        bx.border_top_color,
                        ts,
                        false,
                    );
                }
                // Bottom border
                if bx.border_bottom_width > 0 && bx.border_bottom_color != 0 {
                    self.emit_border_edge(
                        abs_x,
                        abs_y + h - bx.border_bottom_width,
                        w,
                        bx.border_bottom_width,
                        bx.border_bottom_color,
                        bs,
                        false,
                    );
                }
                // Left border
                if bx.border_left_width > 0 && bx.border_left_color != 0 {
                    let inner_h = (h - bx.border_top_width - bx.border_bottom_width).max(0);
                    self.emit_border_edge(
                        abs_x,
                        abs_y + bx.border_top_width,
                        bx.border_left_width,
                        inner_h,
                        bx.border_left_color,
                        ls,
                        true,
                    );
                }
                // Right border
                if bx.border_right_width > 0 && bx.border_right_color != 0 {
                    let inner_h = (h - bx.border_top_width - bx.border_bottom_width).max(0);
                    self.emit_border_edge(
                        abs_x + w - bx.border_right_width,
                        abs_y + bx.border_top_width,
                        bx.border_right_width,
                        inner_h,
                        bx.border_right_color,
                        rs,
                        true,
                    );
                }
            }
        } else if bx.border_width > 0 && bx.border_color != 0 && bx.border_color != 0x00000000 {
            // Fallback: unified border (legacy path)
            let bw = bx.border_width;
            let w = draw_w;
            let h = draw_h;
            self.push(
                abs_x,
                abs_y,
                w,
                bw,
                DrawKind::Rect {
                    color: bx.border_color,
                },
            );
            self.push(
                abs_x,
                abs_y + h - bw,
                w,
                bw,
                DrawKind::Rect {
                    color: bx.border_color,
                },
            );
            let inner_h = (h - bw * 2).max(0);
            self.push(
                abs_x,
                abs_y + bw,
                bw,
                inner_h,
                DrawKind::Rect {
                    color: bx.border_color,
                },
            );
            self.push(
                abs_x + w - bw,
                abs_y + bw,
                bw,
                inner_h,
                DrawKind::Rect {
                    color: bx.border_color,
                },
            );
        }

        // Horizontal rule.
        if bx.is_hr {
            self.push(
                abs_x,
                abs_y,
                draw_w,
                1,
                DrawKind::Rect { color: 0xFF999999 },
            );
        }

        // List marker.
        if let Some(ref marker) = bx.list_marker {
            let font_size = bx.font_size.max(1) as u16;
            let color = if bx.color != 0 { bx.color } else { 0xFF000000 };
            if bx.list_marker_inside {
                // inside (CSS list-style-position: inside):
                // block.rs reserved 20px inside the content area by adding 20 to
                // padding.left, so inline content starts 20px further right.
                // Draw the marker at the start of the ORIGINAL content area:
                //   abs_x + border + padding.left - 20
                let border = bx.border_width;
                let marker_x = abs_x + border + bx.padding.left - 20;
                self.push(
                    marker_x,
                    abs_y,
                    20,
                    draw_h,
                    DrawKind::Text {
                        color,
                        font_id: 0,
                        font_size,
                        text: marker.clone(),
                    },
                );
            } else {
                // outside (default): marker hangs 20px to the left of abs_x.
                self.push(
                    abs_x - 20,
                    abs_y,
                    20,
                    draw_h,
                    DrawKind::Text {
                        color,
                        font_id: 0,
                        font_size,
                        text: marker.clone(),
                    },
                );
            }
        }

        // Text fragment.
        if let Some(ref text) = bx.text {
            if !text.is_empty() && bx.form_field.is_none() {
                let font_id = crate::layout::resolve_font_id(bx.custom_font_id, bx.bold, bx.italic);
                let font_size = bx.font_size.max(1) as u16;
                let color = if bx.color != 0 { bx.color } else { 0xFF000000 };

                // Text shadows (behind the text).
                for ts in &bx.text_shadows {
                    self.push(
                        abs_x + ts.offset_x,
                        abs_y + ts.offset_y,
                        draw_w,
                        draw_h,
                        DrawKind::Text {
                            color: ts.color,
                            font_id,
                            font_size,
                            text: text.clone(),
                        },
                    );
                }

                self.push(
                    abs_x,
                    abs_y,
                    draw_w,
                    draw_h,
                    DrawKind::Text {
                        color,
                        font_id,
                        font_size,
                        text: text.clone(),
                    },
                );

                // Text decorations with sub-property support.
                let deco_color = if bx.text_decoration_color != 0 {
                    bx.text_decoration_color
                } else {
                    color
                };
                let deco_thick = if bx.text_decoration_thickness > 0 {
                    bx.text_decoration_thickness
                } else {
                    1
                };
                let deco_offset = bx.text_underline_offset;

                // Overline.
                if bx.text_decoration == TextDeco::Overline {
                    self.emit_text_deco_line(
                        abs_x,
                        abs_y,
                        draw_w,
                        deco_thick,
                        deco_color,
                        bx.text_decoration_style,
                    );
                }

                // Underline — only if text-decoration says so (not just because it's a link).
                // Per CSS spec, `text-decoration: none` on a link suppresses the underline.
                if bx.text_decoration == TextDeco::Underline {
                    let y_pos = abs_y + draw_h - deco_thick + deco_offset;
                    self.emit_text_deco_line(
                        abs_x,
                        y_pos,
                        draw_w,
                        deco_thick,
                        deco_color,
                        bx.text_decoration_style,
                    );
                }

                // Line-through.
                if bx.text_decoration == TextDeco::LineThrough {
                    self.emit_text_deco_line(
                        abs_x,
                        abs_y + draw_h / 2,
                        draw_w,
                        deco_thick,
                        deco_color,
                        bx.text_decoration_style,
                    );
                }
            }
        }

        // Image.
        if let Some(ref src) = bx.image_src {
            let dw = bx.image_width.unwrap_or(draw_w);
            let dh = bx.image_height.unwrap_or(draw_h);
            let image_x = abs_x + bx.border_left_width + bx.padding.left;
            let image_y = abs_y + bx.border_top_width + bx.padding.top;
            self.push(
                image_x,
                image_y,
                dw,
                dh,
                DrawKind::Image {
                    src: src.clone(),
                    object_fit: bx.object_fit,
                    object_position_x: bx.object_position_x,
                    object_position_x_is_percent: bx.object_position_x_is_percent,
                    object_position_y: bx.object_position_y,
                    object_position_y_is_percent: bx.object_position_y_is_percent,
                },
            );
        }

        // Form control pixel drawing.
        // Native-widget controls are generally rendered by the anyui toolkit.
        // Controls switched to canvas mode via `appearance: none` are painted here.
        if let Some(kind) = bx.form_field {
            match kind {
                FormFieldKind::Submit | FormFieldKind::ButtonEl | FormFieldKind::Reset => {
                    self.emit_submit(abs_x, abs_y, bx);
                }
                FormFieldKind::TextInput | FormFieldKind::Password => {
                    self.emit_text_input(abs_x, abs_y, bx);
                }
                FormFieldKind::Checkbox => {
                    self.emit_checkbox(abs_x, abs_y, bx);
                }
                FormFieldKind::Radio => {
                    self.emit_radio(abs_x, abs_y, bx);
                }
                FormFieldKind::Progress => {
                    self.emit_progress(abs_x, abs_y, bx);
                }
                FormFieldKind::Meter => {
                    self.emit_meter(abs_x, abs_y, bx);
                }
                FormFieldKind::File => {
                    self.emit_submit(abs_x, abs_y, bx);
                }
                FormFieldKind::Color => {
                    self.emit_color_swatch(abs_x, abs_y, bx);
                }
                FormFieldKind::Select => {
                    if bx.appearance_none && !bx.form_multiple && bx.form_size <= 1 {
                        self.emit_select_canvas(abs_x, abs_y, bx);
                    }
                }
                FormFieldKind::Range => {
                    if bx.appearance_none {
                        self.emit_range(abs_x, abs_y, bx);
                    }
                }
                _ => {}
            }
        }

        // Outline (drawn outside the border box).
        if bx.outline_width > 0 && bx.outline_color != 0 {
            let ow = bx.outline_width;
            let off = bx.outline_offset;
            let ox = abs_x - ow - off;
            let oy = abs_y - ow - off;
            let ow_total = draw_w + (ow + off) * 2;
            let oh_total = draw_h + (ow + off) * 2;
            // Top
            self.push(
                ox,
                oy,
                ow_total,
                ow,
                DrawKind::Rect {
                    color: bx.outline_color,
                },
            );
            // Bottom
            self.push(
                ox,
                oy + oh_total - ow,
                ow_total,
                ow,
                DrawKind::Rect {
                    color: bx.outline_color,
                },
            );
            // Left
            let inner_h = (oh_total - ow * 2).max(0);
            self.push(
                ox,
                oy + ow,
                ow,
                inner_h,
                DrawKind::Rect {
                    color: bx.outline_color,
                },
            );
            // Right
            self.push(
                ox + ow_total - ow,
                oy + ow,
                ow,
                inner_h,
                DrawKind::Rect {
                    color: bx.outline_color,
                },
            );
        }

        // Recurse into children, with optional clip rect for overflow:hidden.
        // Note: draw_h == 0 is intentionally allowed here (height: 0; overflow: hidden
        // must clip ALL child content — a zero-height clip rect achieves this).
        let pushed_clip = if bx.overflow_hidden && draw_w > 0 {
            // Intersect with any existing clip rect.
            let new_clip = (abs_x, abs_y, draw_w, draw_h);
            let clip = if let Some(&(cx, cy, cw, ch)) = self.clip_stack.last() {
                let x0 = abs_x.max(cx);
                let y0 = abs_y.max(cy);
                let x1 = (abs_x + draw_w).min(cx + cw);
                let y1 = (abs_y + draw_h).min(cy + ch);
                if x1 > x0 && y1 > y0 {
                    (x0, y0, x1 - x0, y1 - y0)
                } else {
                    (0, 0, 0, 0) // fully clipped
                }
            } else {
                new_clip
            };
            self.clip_stack.push(clip);
            true
        } else {
            false
        };

        let next_sticky_ctx = if bx.overflow_hidden {
            let content_top = abs_y + bx.border_width + bx.padding.top;
            let content_h =
                (draw_h - bx.padding.top - bx.padding.bottom - bx.border_width * 2).max(0);
            Some(StickyContext {
                top: content_top,
                height: content_h,
            })
        } else {
            sticky_ctx
        };

        // Process children in stacking order per CSS2 Appendix E.
        // We always partition children that create stacking contexts, even if
        // the current element does not itself create one.  Per spec, positioned
        // children with explicit z-index participate in the nearest ancestor
        // stacking context.  Always-partitioning propagates that through
        // intermediate non-SC boxes (e.g. <body>), so top-level positioned
        // elements are correctly sorted by z-index even when the root <html>
        // element has no explicit stacking context properties.
        // Apply scroll offsets for overflow:auto/scroll containers.
        // Children are shifted by -scroll_top/-scroll_left so content scrolls.
        let (cx, cy) = if bx.is_fixed {
            (bx.x, bx.y)
        } else {
            (abs_x - bx.scroll_left, abs_y - bx.scroll_top)
        };

        // Check if any children create stacking contexts at all.
        let has_sc_children = bx.children.iter().any(|c| c.creates_stacking_context);

        if has_sc_children {
            // Partition children into three groups:
            // 1. Child stacking contexts with negative z-index (sorted ascending)
            // 2. Non-stacking-context in-flow children in document order
            // 3. Non-stacking-context positioned out-of-flow children in document order
            // 4. Child stacking contexts with z-index >= 0 (sorted ascending)
            let mut neg: Vec<(i32, usize)> = Vec::new();
            let mut pos: Vec<(i32, usize)> = Vec::new();
            let mut normal: Vec<usize> = Vec::new();
            let mut positioned_auto: Vec<usize> = Vec::new();

            for (i, child) in bx.children.iter().enumerate() {
                if child.creates_stacking_context {
                    if child.z_index < 0 {
                        neg.push((child.z_index, i));
                    } else {
                        pos.push((child.z_index, i));
                    }
                } else if child.is_out_of_flow || child.is_fixed {
                    positioned_auto.push(i);
                } else {
                    normal.push(i);
                }
            }

            // Negative z-index stacking contexts (most negative first)
            neg.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            for &(_, idx) in &neg {
                self.flatten(&bx.children[idx], cx, cy, next_sticky_ctx);
            }

            // Non-stacking-context in-flow children in document order
            for idx in normal {
                self.flatten(&bx.children[idx], cx, cy, next_sticky_ctx);
            }

            // Positioned out-of-flow children with auto z-index paint after
            // normal in-flow content/floats but before positive stacking contexts.
            for idx in positioned_auto {
                self.flatten(&bx.children[idx], cx, cy, next_sticky_ctx);
            }

            // Non-negative z-index stacking contexts (ascending)
            pos.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            for &(_, idx) in &pos {
                self.flatten(&bx.children[idx], cx, cy, next_sticky_ctx);
            }
        } else {
            // No children create stacking contexts — document order.
            for child in &bx.children {
                self.flatten(child, cx, cy, next_sticky_ctx);
            }
        }

        if pushed_clip {
            self.clip_stack.pop();
        }
        if pushed_mask {
            self.mask_stack.pop();
        }
        if pushed_rotation {
            self.rotation_stack.pop();
        }
    }

}

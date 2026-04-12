use super::*;

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
        let has_radius = bx.border_top_left_radius > 0
            || bx.border_top_right_radius > 0
            || bx.border_bottom_right_radius > 0
            || bx.border_bottom_left_radius > 0;
        let radii = [
            bx.border_top_left_radius,
            bx.border_top_right_radius,
            bx.border_bottom_right_radius,
            bx.border_bottom_left_radius,
        ];

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
        let bg_radii = self.background_clip_radii(bx);
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
            self.push(
                abs_x,
                abs_y,
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
            // 2. Non-stacking-context children in document order
            // 3. Child stacking contexts with z-index >= 0 (sorted ascending)
            let mut neg: Vec<(i32, usize)> = Vec::new();
            let mut pos: Vec<(i32, usize)> = Vec::new();

            for (i, child) in bx.children.iter().enumerate() {
                if child.creates_stacking_context {
                    if child.z_index < 0 {
                        neg.push((child.z_index, i));
                    } else {
                        pos.push((child.z_index, i));
                    }
                }
            }

            // Negative z-index stacking contexts (most negative first)
            neg.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            for &(_, idx) in &neg {
                self.flatten(&bx.children[idx], cx, cy, next_sticky_ctx);
            }

            // Non-stacking-context children in document order
            for child in &bx.children {
                if !child.creates_stacking_context {
                    self.flatten(child, cx, cy, next_sticky_ctx);
                }
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

    /// Emit a border edge with the given style (solid/dashed/dotted).

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
        let (bg_x, bg_y, bg_w, bg_h) = self.background_paint_rect(abs_x, abs_y, bx);
        match &bx.background_image {
            BackgroundImageVal::LinearGradient { angle_deg, stops } => {
                if stops.len() < 2 || bg_w <= 0 || bg_h <= 0 {
                    return;
                }
                let angle = *angle_deg;
                let is_horizontal = angle == 90 || angle == 270;
                let is_vertical = angle == 0 || angle == 180;

                if is_horizontal || is_vertical {
                    // Fast path: axis-aligned gradients rendered as stripe rects.
                    let dimension = if is_horizontal { bg_w } else { bg_h };
                    let stripe_count = dimension.min(64).max(2);
                    let stripe_size = dimension / stripe_count;
                    if stripe_size <= 0 {
                        return;
                    }

                    let reversed = angle == 270 || angle == 0;
                    for i in 0..stripe_count {
                        let t_raw = i * 10000 / stripe_count;
                        let t = if reversed { 10000 - t_raw } else { t_raw };
                        let color = interpolate_gradient_color(stops, t);

                        if is_horizontal {
                            let sx = bg_x + i * stripe_size;
                            let sw = if i == stripe_count - 1 {
                                bg_w - i * stripe_size
                            } else {
                                stripe_size
                            };
                            self.push(sx, bg_y, sw, bg_h, DrawKind::Rect { color });
                        } else {
                            let sy = bg_y + i * stripe_size;
                            let sh = if i == stripe_count - 1 {
                                bg_h - i * stripe_size
                            } else {
                                stripe_size
                            };
                            self.push(bg_x, sy, bg_w, sh, DrawKind::Rect { color });
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
                if !src.is_empty() && bg_w > 0 && bg_h > 0 {
                    self.push(
                        bg_x,
                        bg_y,
                        bg_w,
                        bg_h,
                        DrawKind::Image {
                            src: src.clone(),
                            object_fit: bx.object_fit,
                            object_position_x: 5000,
                            object_position_x_is_percent: true,
                            object_position_y: 5000,
                            object_position_y_is_percent: true,
                        },
                    );
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
        };
        let right_inset = match area {
            BackgroundClipVal::BorderBox => 0,
            BackgroundClipVal::PaddingBox => bx.border_right_width.max(0),
            BackgroundClipVal::ContentBox => (bx.border_right_width + bx.padding.right).max(0),
        };
        let top_inset = match area {
            BackgroundClipVal::BorderBox => 0,
            BackgroundClipVal::PaddingBox => bx.border_top_width.max(0),
            BackgroundClipVal::ContentBox => (bx.border_top_width + bx.padding.top).max(0),
        };
        let bottom_inset = match area {
            BackgroundClipVal::BorderBox => 0,
            BackgroundClipVal::PaddingBox => bx.border_bottom_width.max(0),
            BackgroundClipVal::ContentBox => (bx.border_bottom_width + bx.padding.bottom).max(0),
        };
        let w = (bx.width - left_inset - right_inset).max(0);
        let h = (bx.height - top_inset - bottom_inset).max(0);
        (abs_x + left_inset, abs_y + top_inset, w, h)
    }

    fn background_clip_radii(&self, bx: &LayoutBox) -> [i32; 4] {
        let inset_x = match bx.background_clip {
            BackgroundClipVal::BorderBox => 0,
            BackgroundClipVal::PaddingBox => bx.border_left_width.max(bx.border_right_width).max(0),
            BackgroundClipVal::ContentBox => (bx.border_left_width + bx.padding.left)
                .max(bx.border_right_width + bx.padding.right)
                .max(0),
        };
        let inset_y = match bx.background_clip {
            BackgroundClipVal::BorderBox => 0,
            BackgroundClipVal::PaddingBox => bx.border_top_width.max(bx.border_bottom_width).max(0),
            BackgroundClipVal::ContentBox => (bx.border_top_width + bx.padding.top)
                .max(bx.border_bottom_width + bx.padding.bottom)
                .max(0),
        };
        [
            (bx.border_top_left_radius - inset_x.max(inset_y)).max(0),
            (bx.border_top_right_radius - inset_x.max(inset_y)).max(0),
            (bx.border_bottom_right_radius - inset_x.max(inset_y)).max(0),
            (bx.border_bottom_left_radius - inset_x.max(inset_y)).max(0),
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
                text: label_text,
            },
        );
    }

    /// Draw a text input / search / password field as a simple rectangle with border.
    fn emit_text_input(&mut self, x: i32, y: i32, bx: &LayoutBox) {
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
            let tx = x + 4;
            let ty = y + (bx.height - font_size as i32) / 2;
            self.push(
                tx,
                ty,
                bx.width - 8,
                font_size as i32,
                DrawKind::Text {
                    color,
                    font_id: 0,
                    font_size,
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
            self.push(cx + 4, cy + sz / 2 + 1, 2, 1, DrawKind::Rect { color: check });
            self.push(cx + 5, cy + sz / 2 + 2, 2, 1, DrawKind::Rect { color: check });
            self.push(cx + 6, cy + sz / 2 + 1, 2, 1, DrawKind::Rect { color: check });
            self.push(cx + 7, cy + sz / 2, 2, 1, DrawKind::Rect { color: check });
            self.push(cx + 8, cy + sz / 2 - 1, 1, 1, DrawKind::Rect { color: check });
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
                color: if bx.uses_dark_color_scheme { 0xFF3A3A3A } else { 0xFFE0E0E0 },
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
                color: if bx.uses_dark_color_scheme { 0xFF3A3A3A } else { 0xFFE0E0E0 },
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
                color: if bx.uses_dark_color_scheme { 0xFF3A3A3A } else { 0xFFE0E0E0 },
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
            self.push(x, y, fill_w, h, DrawKind::RoundedRect { color: fill_color, radii: [r, fr, fr, r] });
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
            rotations: self.rotation_stack.clone(),
        });
    }
}

fn transformed_bounds(x: i32, y: i32, w: i32, h: i32, rotations: &[DrawRotation]) -> (i32, i32, i32, i32) {
    if w <= 0 || h <= 0 || rotations.is_empty() {
        return (x, y, w, h);
    }

    let mut points = [
        (x as f32, y as f32),
        ((x + w) as f32, y as f32),
        ((x + w) as f32, (y + h) as f32),
        (x as f32, (y + h) as f32),
    ];
    for rot in rotations {
        let rad = rot.angle_deg100 as f32 / 100.0 * core::f32::consts::PI / 180.0;
        let sin = sin_approx(rad);
        let cos = cos_approx(rad);
        for pt in &mut points {
            let dx = pt.0 - rot.origin_x as f32;
            let dy = pt.1 - rot.origin_y as f32;
            pt.0 = rot.origin_x as f32 + dx * cos - dy * sin;
            pt.1 = rot.origin_y as f32 + dx * sin + dy * cos;
        }
    }

    let mut min_x = points[0].0;
    let mut max_x = points[0].0;
    let mut min_y = points[0].1;
    let mut max_y = points[0].1;
    for (px, py) in points.iter().skip(1) {
        min_x = min_x.min(*px);
        max_x = max_x.max(*px);
        min_y = min_y.min(*py);
        max_y = max_y.max(*py);
    }

    let bx = floor_f32(min_x);
    let by = floor_f32(min_y);
    let bw = (ceil_f32(max_x) - bx).max(0);
    let bh = (ceil_f32(max_y) - by).max(0);
    (bx, by, bw, bh)
}

#[inline]
fn floor_f32(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 { i - 1 } else { i }
}

#[inline]
fn ceil_f32(v: f32) -> i32 {
    let i = v as i32;
    if v > i as f32 { i + 1 } else { i }
}


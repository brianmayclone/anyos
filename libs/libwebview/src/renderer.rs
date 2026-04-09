//! Display-list renderer with compositor-driven smooth scrolling.
//!
//! After layout, the tree is flattened into a sorted `Vec<DrawCmd>` (the
//! display list).  Each tile is rasterized by binary-searching for the
//! first command that overlaps the tile Y range, then linearly executing
//! commands until they fall below the tile.  This is O(k) per tile where
//! k = commands visible in the tile, compared to O(n) for a full tree walk.

use alloc::string::String;
use alloc::vec::Vec;

use libanyui_client::{self as ui, Widget};

use crate::layout::{FormFieldKind, LayoutBox};
use crate::style::TextDeco;

// ═══════════════════════════════════════════════════════════════════════════
// Image cache
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum total decoded image bytes in the cache.
///
/// The old 512 MiB cap protected against aggressive evictions, but it also let
/// Surf accumulate far too much decoded image memory on pages like Heise. That
/// pushed allocator pressure and made failures around `sbrk` much more likely.
/// Keep the cache large enough for smooth scrolling, but substantially smaller.
const IMAGE_CACHE_MAX_BYTES: usize = 192 * 1024 * 1024;
/// When we cross the hard cap, trim to a softer target so we do not thrash
/// around the limit by evicting only one entry per insertion.
const IMAGE_CACHE_TRIM_TARGET_BYTES: usize = 144 * 1024 * 1024;
const PROGRESSIVE_BAND_VIEWPORTS_BEFORE: i32 = 1;
const PROGRESSIVE_BAND_VIEWPORTS_AFTER: i32 = 3;

/// Image cache entry (decoded pixel data).
pub struct ImageEntry {
    pub src: String,
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
    /// LRU generation (higher = more recently used).
    generation: u64,
}

impl ImageEntry {
    /// Size in bytes of the decoded pixel data.
    fn byte_size(&self) -> usize {
        self.pixels.len() * 4
    }

    fn has_pixels(&self) -> bool {
        !self.pixels.is_empty() && self.width > 0 && self.height > 0
    }
}

/// LRU cache of decoded images with a total byte-size cap.
pub struct ImageCache {
    pub entries: Vec<ImageEntry>,
    generation: u64,
    total_bytes: usize,
}

impl ImageCache {
    pub fn new() -> Self {
        ImageCache {
            entries: Vec::new(),
            generation: 0,
            total_bytes: 0,
        }
    }

    /// Look up a cached image by URL.  Bumps the LRU generation on hit.
    pub fn get(&mut self, src: &str) -> Option<&ImageEntry> {
        self.generation += 1;
        let gen = self.generation;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.src == src) {
            entry.generation = gen;
            return Some(entry);
        }
        None
    }

    /// Read-only lookup (no LRU bump).
    pub fn get_ref(&self, src: &str) -> Option<&ImageEntry> {
        self.entries.iter().find(|e| e.src == src)
    }

    pub fn has_pixels_for(&self, src: &str) -> bool {
        self.get_ref(src).is_some_and(|e| e.has_pixels())
    }

    /// Add a decoded image.  Evicts LRU entries if the cache exceeds the byte cap.
    pub fn add(&mut self, src: String, pixels: Vec<u32>, width: u32, height: u32) {
        let new_bytes = pixels.len() * 4;

        if let Some(entry) = self.entries.iter_mut().find(|e| e.src == src) {
            self.total_bytes -= entry.byte_size();
            entry.pixels = pixels;
            entry.width = width;
            entry.height = height;
            self.generation += 1;
            entry.generation = self.generation;
            self.total_bytes += new_bytes;
            self.evict_to_budget();
            return;
        }

        self.generation += 1;
        let gen = self.generation;
        self.entries.push(ImageEntry {
            src,
            pixels,
            width,
            height,
            generation: gen,
        });
        self.total_bytes += new_bytes;
        self.evict_to_budget();
    }

    /// Drop all cached images (called on page navigation).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.total_bytes = 0;
    }

    fn evict_to_budget(&mut self) {
        let target = if self.total_bytes > IMAGE_CACHE_MAX_BYTES {
            IMAGE_CACHE_TRIM_TARGET_BYTES
        } else {
            IMAGE_CACHE_MAX_BYTES
        };
        while self.total_bytes > target {
            let Some(min_idx) = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.pixels.is_empty())
                .min_by_key(|(_, e)| e.generation)
                .map(|(i, _)| i)
            else {
                break;
            };

            let entry = &mut self.entries[min_idx];
            self.total_bytes -= entry.byte_size();
            entry.pixels.clear();
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Hit regions
// ═══════════════════════════════════════════════════════════════════════════

pub struct HitRegion {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub kind: HitKind,
}

pub enum HitKind {
    Link(String),
    Submit(usize),
    /// Reset button — node_id of the reset element.
    Reset(usize),
    /// File input — node_id of the `<input type="file">` element.
    FileInput(usize),
    /// Color input — node_id of the `<input type="color">` element.
    ColorInput(usize),
}

// ═══════════════════════════════════════════════════════════════════════════
// Persistent form controls
// ═══════════════════════════════════════════════════════════════════════════

pub struct FormControl {
    pub control_id: u32,
    pub node_id: usize,
    pub kind: FormFieldKind,
    pub name: String,
    seen: bool,
    /// Document-space position and size (for host-mode hit-testing).
    pub doc_x: i32,
    pub doc_y: i32,
    pub doc_w: i32,
    pub doc_h: i32,
}

// ═══════════════════════════════════════════════════════════════════════════
// Display list — flat, Y-sorted draw commands
// ═══════════════════════════════════════════════════════════════════════════

/// A single draw command in absolute document coordinates.
struct DrawCmd {
    /// Absolute document X.
    x: i32,
    /// Absolute document Y.
    y: i32,
    /// Width of the drawn element.
    w: i32,
    /// Height of the drawn element.
    h: i32,
    /// The drawing operation.
    kind: DrawKind,
    /// Optional clip rect (from parent with overflow:hidden).
    /// (clip_x, clip_y, clip_w, clip_h) — commands are clipped to this rect.
    clip: Option<(i32, i32, i32, i32)>,
}

enum DrawKind {
    /// Fill a rectangle with a solid or alpha-blended color.
    Rect { color: u32 },
    /// Fill a rounded rectangle with corner radii.
    RoundedRect { color: u32, radii: [i32; 4] }, // [tl, tr, br, bl]
    /// Draw a dashed/dotted horizontal or vertical border line.
    DashedLine {
        color: u32,
        dash_len: i32,
        gap_len: i32,
        vertical: bool,
    },
    /// Draw a text string.
    Text {
        color: u32,
        font_id: u32,
        font_size: u16,
        text: String,
    },
    /// Blit an image (looked up from ImageCache by src URL at rasterize time).
    Image {
        src: String,
        object_fit: crate::style::ObjectFit,
        object_position_x: i32,
        object_position_x_is_percent: bool,
        object_position_y: i32,
        object_position_y_is_percent: bool,
    },
}

/// A flat display list built from the layout tree in correct paint order.
///
/// Commands are emitted by walking the tree in CSS2 Appendix E stacking
/// order (negative z-index stacking contexts first, then in-flow content,
/// then positive z-index stacking contexts).  The resulting command list
/// is already in back-to-front paint order — no post-hoc sort needed.
pub(crate) struct DisplayList {
    cmds: Vec<DrawCmd>,
    /// Current clip rect during flatten (None = no clipping).
    clip_stack: Vec<(i32, i32, i32, i32)>,
    /// Maximum command height seen — used as search margin for binary search.
    max_h: i32,
    /// Optional document-space Y cull range for fast initial paints.
    cull_y_range: Option<(i32, i32)>,
}

impl DisplayList {
    pub fn new() -> Self {
        Self {
            cmds: Vec::new(),
            max_h: 0,
            clip_stack: Vec::new(),
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
            cull_y_range: None,
        };
        dl.flatten(root, 0, 0);
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
            cull_y_range: Some((y_start, y_end)),
        };
        dl.flatten(root, 0, 0);
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

            match &cmd.kind {
                DrawKind::Rect { color } => {
                    fill_rect_buf(buf, stride, buf_h, cx, cy, cw, ch, *color);
                }
                DrawKind::RoundedRect { color, radii } => {
                    fill_rounded_rect_buf(buf, stride, buf_h, cx, cy, cw, ch, *color, *radii);
                }
                DrawKind::DashedLine {
                    color,
                    dash_len,
                    gap_len,
                    vertical,
                } => {
                    fill_dashed_buf(
                        buf, stride, buf_h, cx, cy, cw, ch, *color, *dash_len, *gap_len, *vertical,
                    );
                }
                DrawKind::Text {
                    color,
                    font_id,
                    font_size,
                    text,
                } => {
                    // Text clipping is harder — for now, draw at original position
                    // (the fill_rect clipping handles most visual cases).
                    libfont_client::draw_string_buf(
                        buf, stride, buf_h, cmd.x, draw_y, *color, *font_id, *font_size, text,
                    );
                }
                DrawKind::Image {
                    src,
                    object_fit,
                    object_position_x,
                    object_position_x_is_percent,
                    object_position_y,
                    object_position_y_is_percent,
                } => {
                    if let Some(entry) = images.get_ref(src) {
                        if !entry.has_pixels() {
                            continue;
                        }
                        blit_image_scaled(
                            buf,
                            stride,
                            buf_h,
                            cmd.x,
                            draw_y,
                            cmd.w,
                            cmd.h,
                            (cx, cy, cw, ch),
                            &entry.pixels,
                            entry.width,
                            entry.height,
                            *object_fit,
                            *object_position_x,
                            *object_position_x_is_percent,
                            *object_position_y,
                            *object_position_y_is_percent,
                        );
                    }
                }
            }
        }
    }

    /// Recursively flatten the layout tree into draw commands.
    /// Children are processed in CSS2 Appendix E stacking order when the
    /// parent creates a stacking context.
    fn flatten(&mut self, bx: &LayoutBox, offset_x: i32, offset_y: i32) {
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

        // Position: sticky — clamp Y so the element doesn't scroll above its
        // sticky threshold (top offset from the viewport top).  The element
        // stays in normal flow until scrolling would move it past the threshold,
        // then it "sticks" at that position.
        let orig_abs_y = if bx.is_sticky {
            let viewport_top = 0; // TODO: adjust for nested scroll containers
            let sticky_threshold = viewport_top + bx.sticky_top;
            if orig_abs_y < sticky_threshold {
                sticky_threshold
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
                orig_abs_y,
                bx.height,
                bx.transform_origin_y,
                bx.transform_origin_y_is_percent,
            );
            let new_w = (bx.width as f32 * sx) as i32;
            let new_h = (bx.height as f32 * sy) as i32;
            let new_x = cx + ((orig_abs_x - cx) as f32 * sx) as i32;
            let new_y = cy + ((orig_abs_y - cy) as f32 * sy) as i32;
            (new_x, new_y, new_w, new_h)
        } else {
            (orig_abs_x, orig_abs_y, bx.width, bx.height)
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

        // Background.
        if bx.bg_color != 0 && bx.bg_color != 0x00000000 {
            if has_radius {
                self.push(
                    abs_x,
                    abs_y,
                    draw_w,
                    draw_h,
                    DrawKind::RoundedRect {
                        color: bx.bg_color,
                        radii,
                    },
                );
            } else {
                self.push(
                    abs_x,
                    abs_y,
                    draw_w,
                    draw_h,
                    DrawKind::Rect { color: bx.bg_color },
                );
            }
        }

        // Background image / gradient.
        self.emit_background_image(abs_x, abs_y, bx);

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
                let font_id = if bx.custom_font_id != 0 {
                    bx.custom_font_id
                } else if bx.bold {
                    1u32
                } else if bx.italic {
                    3u32
                } else {
                    0u32
                };
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
        // Native-widget controls (Select, Range) are rendered by the anyui
        // toolkit — skip canvas painting for those.
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
                // Select and Range use native widgets now.
                FormFieldKind::Select | FormFieldKind::Range => {}
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
                self.flatten(&bx.children[idx], cx, cy);
            }

            // Non-stacking-context children in document order
            for child in &bx.children {
                if !child.creates_stacking_context {
                    self.flatten(child, cx, cy);
                }
            }

            // Non-negative z-index stacking contexts (ascending)
            pos.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            for &(_, idx) in &pos {
                self.flatten(&bx.children[idx], cx, cy);
            }
        } else {
            // No children create stacking contexts — document order.
            for child in &bx.children {
                self.flatten(child, cx, cy);
            }
        }

        if pushed_clip {
            self.clip_stack.pop();
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
        match &bx.background_image {
            BackgroundImageVal::LinearGradient { angle_deg, stops } => {
                if stops.len() < 2 || bx.width <= 0 || bx.height <= 0 {
                    return;
                }
                let angle = *angle_deg;
                let is_horizontal = angle == 90 || angle == 270;
                let is_vertical = angle == 0 || angle == 180;

                if is_horizontal || is_vertical {
                    // Fast path: axis-aligned gradients rendered as stripe rects.
                    let dimension = if is_horizontal { bx.width } else { bx.height };
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
                            let sx = abs_x + i * stripe_size;
                            let sw = if i == stripe_count - 1 {
                                bx.width - i * stripe_size
                            } else {
                                stripe_size
                            };
                            self.push(sx, abs_y, sw, bx.height, DrawKind::Rect { color });
                        } else {
                            let sy = abs_y + i * stripe_size;
                            let sh = if i == stripe_count - 1 {
                                bx.height - i * stripe_size
                            } else {
                                stripe_size
                            };
                            self.push(abs_x, sy, bx.width, sh, DrawKind::Rect { color });
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
                    let w_f = bx.width as f32;
                    let h_f = bx.height as f32;
                    let half_w = w_f / 2.0;
                    let half_h = h_f / 2.0;
                    let grad_len = (dx.abs() * w_f + dy.abs() * h_f).max(1.0);

                    // Render as horizontal scan-line stripes, max 64 for perf.
                    let stripe_count = bx.height.min(64).max(2);
                    let stripe_h = bx.height / stripe_count;
                    if stripe_h <= 0 {
                        return;
                    }

                    for i in 0..stripe_count {
                        let cy =
                            (i * bx.height / stripe_count) as f32 + stripe_h as f32 / 2.0 - half_h;
                        let cx = 0.0_f32; // center of scanline
                        let proj = (cx * dx + cy * dy) / grad_len + 0.5;
                        let t = (proj * 10000.0).max(0.0).min(10000.0) as i32;
                        let color = interpolate_gradient_color(stops, t);
                        let sy = abs_y + i * stripe_h;
                        let sh = if i == stripe_count - 1 {
                            bx.height - i * stripe_h
                        } else {
                            stripe_h
                        };
                        self.push(abs_x, sy, bx.width, sh, DrawKind::Rect { color });
                    }
                }
            }
            BackgroundImageVal::Url(ref src) => {
                if !src.is_empty() {
                    self.push(
                        abs_x,
                        abs_y,
                        bx.width,
                        bx.height,
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

    /// Emit draw commands for a submit/button element.
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
        let (tw, _) = libfont_client::measure(0, font_size, &label_text);
        let tx = x + (bx.width - tw as i32) / 2;
        let ty = y + (bx.height - font_size as i32) / 2;
        self.push(
            tx,
            ty,
            tw as i32,
            font_size as i32,
            DrawKind::Text {
                color: text_color,
                font_id: 0,
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

    #[inline]
    fn push(&mut self, x: i32, y: i32, w: i32, h: i32, kind: DrawKind) {
        if let Some((y_start, y_end)) = self.cull_y_range {
            if y + h <= y_start || y >= y_end {
                return;
            }
        }
        if h > self.max_h {
            self.max_h = h;
        }
        let clip = self.clip_stack.last().copied();
        self.cmds.push(DrawCmd {
            x,
            y,
            w,
            h,
            kind,
            clip,
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tile cache (pixel data)
// ═══════════════════════════════════════════════════════════════════════════

const TILE_HEIGHT: u32 = 256;
const MAX_CACHED_TILES: usize = 40;
const BUFFER_ZONE: i32 = 768;
const MAX_TILE_CANVASES: usize = 30;
const MAX_TILES_PER_TICK: usize = 8;
const INITIAL_VISIBLE_EXTRA_ROWS: u32 = 1;

struct CachedTile {
    row: u32,
    pixels: Vec<u32>,
    generation: u64,
}

struct TileCache {
    tiles: Vec<CachedTile>,
    generation: u64,
    /// Pool of reusable pixel buffers (avoids alloc per tile).
    free_bufs: Vec<Vec<u32>>,
}

impl TileCache {
    fn new() -> Self {
        Self {
            tiles: Vec::new(),
            generation: 0,
            free_bufs: Vec::new(),
        }
    }

    fn get(&self, row: u32) -> Option<&[u32]> {
        self.tiles
            .iter()
            .find(|t| t.row == row)
            .map(|t| t.pixels.as_slice())
    }

    fn insert(&mut self, row: u32, pixels: Vec<u32>) {
        self.generation += 1;
        let gen = self.generation;

        if let Some(tile) = self.tiles.iter_mut().find(|t| t.row == row) {
            // Return old buffer to pool before replacing.
            let old = core::mem::replace(&mut tile.pixels, pixels);
            if self.free_bufs.len() < 8 {
                self.free_bufs.push(old);
            }
            tile.generation = gen;
            return;
        }

        if self.tiles.len() >= MAX_CACHED_TILES {
            let min_idx = self
                .tiles
                .iter()
                .enumerate()
                .min_by_key(|(_, t)| t.generation)
                .map(|(i, _)| i)
                .unwrap_or(0);
            let evicted = self.tiles.swap_remove(min_idx);
            if self.free_bufs.len() < 8 {
                self.free_bufs.push(evicted.pixels);
            }
        }

        self.tiles.push(CachedTile {
            row,
            pixels,
            generation: gen,
        });
    }

    fn invalidate_all(&mut self) {
        for tile in self.tiles.drain(..) {
            if self.free_bufs.len() < 8 {
                self.free_bufs.push(tile.pixels);
            }
        }
        self.generation = 0;
    }

    /// Take a buffer from the pool or allocate a new one.
    fn take_buf(&mut self, pixel_count: usize, clear_color: u32) -> Vec<u32> {
        if let Some(mut buf) = self.free_bufs.pop() {
            buf.resize(pixel_count, clear_color);
            for px in buf.iter_mut() {
                *px = clear_color;
            }
            buf
        } else {
            let mut buf = Vec::with_capacity(pixel_count);
            buf.resize(pixel_count, clear_color);
            buf
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tile canvas
// ═══════════════════════════════════════════════════════════════════════════

struct TileCanvas {
    row: u32,
    canvas: ui::Canvas,
}

// ═══════════════════════════════════════════════════════════════════════════
// Renderer
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) struct Renderer {
    tile_canvases: Vec<TileCanvas>,
    tile_cache: TileCache,
    doc_w: u32,
    doc_h: u32,
    pub hit_regions: Vec<HitRegion>,
    pub form_controls: Vec<FormControl>,
    pub link_map: Vec<(u32, String)>,
    link_cb: Option<ui::Callback>,
    link_cb_ud: u64,
    submit_cb: Option<ui::Callback>,
    submit_cb_ud: u64,
    last_scroll_y: i32,
    /// The display list — built once after layout, used for all tile rasterization.
    display_list: DisplayList,
    display_list_complete: bool,
    display_list_y_range: Option<(i32, i32)>,
}

impl Renderer {
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

    fn progressive_band_range(scroll_y: i32, viewport_h: u32, doc_h: u32) -> (i32, i32) {
        let vp = viewport_h.max(1) as i32;
        let start = (scroll_y - vp * PROGRESSIVE_BAND_VIEWPORTS_BEFORE).max(0);
        let end = (scroll_y + vp * PROGRESSIVE_BAND_VIEWPORTS_AFTER).min(doc_h as i32).max(start + 1);
        (start, end)
    }

    fn prioritized_tile_rows(
        first_row: u32,
        last_row: u32,
        scroll_y: i32,
        viewport_h: u32,
    ) -> Vec<u32> {
        if last_row < first_row {
            return Vec::new();
        }
        let mut rows: Vec<u32> = (first_row..=last_row).collect();
        let vp_center_row = ((scroll_y + viewport_h as i32 / 2).max(0) as u32) / TILE_HEIGHT;
        rows.sort_by_key(|row| {
            if *row > vp_center_row {
                *row - vp_center_row
            } else {
                vp_center_row - *row
            }
        });
        rows
    }

    pub fn new() -> Self {
        Self {
            tile_canvases: Vec::new(),
            tile_cache: TileCache::new(),
            doc_w: 0,
            doc_h: 0,
            hit_regions: Vec::new(),
            form_controls: Vec::new(),
            link_map: Vec::new(),
            link_cb: None,
            link_cb_ud: 0,
            submit_cb: None,
            submit_cb_ud: 0,
            last_scroll_y: 0,
            display_list: DisplayList::new(),
            display_list_complete: true,
            display_list_y_range: None,
        }
    }

    fn visible_tile_row_range(scroll_y: i32, viewport_h: u32, doc_h: u32) -> (u32, u32) {
        let visible_y_start = scroll_y.max(0);
        let visible_y_end = (scroll_y + viewport_h as i32).min(doc_h as i32);
        let first_row = visible_y_start as u32 / TILE_HEIGHT;
        let last_visible_row = if visible_y_end > 0 {
            ((visible_y_end - 1) as u32) / TILE_HEIGHT
        } else {
            first_row
        };
        let start = first_row.saturating_sub(INITIAL_VISIBLE_EXTRA_ROWS);
        let max_row = if doc_h > 0 {
            (doc_h - 1) / TILE_HEIGHT
        } else {
            0
        };
        let end = (last_visible_row + INITIAL_VISIBLE_EXTRA_ROWS).min(max_row);
        (start, end)
    }

    pub fn tile_hit_coords(&self, ctrl_id: u32) -> Option<(i32, i32)> {
        for tc in &self.tile_canvases {
            if tc.canvas.id() == ctrl_id {
                let (mx, my, _) = tc.canvas.get_mouse();
                let doc_y = my + (tc.row * TILE_HEIGHT) as i32;
                return Some((mx, doc_y));
            }
        }
        None
    }

    pub fn control_count(&self) -> usize {
        self.form_controls.len()
    }

    /// Soft clear: reset hit regions, invalidate tile cache, destroy canvases.
    pub fn clear(&mut self) {
        self.hit_regions.clear();
        self.link_map.clear();
        self.tile_cache.invalidate_all();
        for tc in self.tile_canvases.drain(..) {
            ui::Control::from_id(tc.canvas.id()).remove();
        }
        for fc in &mut self.form_controls {
            fc.seen = false;
        }
        self.display_list.clear();
        self.display_list_complete = true;
        self.display_list_y_range = None;
    }

    /// Hard clear: destroy everything.
    pub fn clear_all(&mut self) {
        for fc in &self.form_controls {
            if fc.control_id != 0 {
                ui::Control::from_id(fc.control_id).remove();
            }
        }
        self.form_controls.clear();
        for tc in self.tile_canvases.drain(..) {
            ui::Control::from_id(tc.canvas.id()).remove();
        }
        self.doc_w = 0;
        self.doc_h = 0;
        self.hit_regions.clear();
        self.link_map.clear();
        self.tile_cache.invalidate_all();
        self.link_cb = None;
        self.link_cb_ud = 0;
        self.submit_cb = None;
        self.submit_cb_ud = 0;
        self.last_scroll_y = 0;
        self.display_list.clear();
        self.display_list_complete = true;
        self.display_list_y_range = None;
    }

    pub fn hit_test_link_at(&self, x: i32, doc_y: i32) -> Option<&str> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Link(ref url) = region.kind {
                    return Some(url.as_str());
                }
            }
        }
        None
    }

    pub fn hit_test_submit_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Submit(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_reset_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Reset(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_file_input_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::FileInput(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_color_input_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::ColorInput(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    // ─────────────────────────────────────────────────────────────────────
    // Full render (relayout path)
    // ─────────────────────────────────────────────────────────────────────

    pub fn render(
        &mut self,
        root: &LayoutBox,
        parent: &ui::View,
        images: &ImageCache,
        doc_w: u32,
        doc_h: u32,
        viewport_h: u32,
        scroll_y: i32,
        bg_color: u32,
        link_cb: Option<ui::Callback>,
        link_cb_ud: u64,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
        allow_progressive_display_list: bool,
    ) -> bool {
        crate::debug_surf!(
            "[render] full render start ({}x{}, vp_h={}, scroll_y={})",
            doc_w,
            doc_h,
            viewport_h,
            scroll_y
        );

        let w = doc_w.max(1);
        let clear_color = if bg_color != 0 { bg_color } else { 0xFFFFFFFF };

        self.doc_w = w;
        self.doc_h = doc_h;
        self.link_cb = link_cb;
        self.link_cb_ud = link_cb_ud;
        self.submit_cb = submit_cb;
        self.submit_cb_ud = submit_cb_ud;
        self.last_scroll_y = scroll_y;

        // 1. Invalidate tile cache (layout has changed).
        self.tile_cache.invalidate_all();

        // 4. Compute visible tile rows.
        let render_y_start = (scroll_y - BUFFER_ZONE).max(0);
        let render_y_end = (scroll_y + viewport_h as i32 + BUFFER_ZONE).min(doc_h as i32);
        let first_row = render_y_start as u32 / TILE_HEIGHT;
        let last_row = if render_y_end > 0 {
            ((render_y_end - 1) as u32) / TILE_HEIGHT
        } else {
            0
        };
        let prioritized_rows =
            Self::prioritized_tile_rows(first_row, last_row, scroll_y, viewport_h);
        let (visible_first_row, visible_last_row) =
            Self::visible_tile_row_range(scroll_y, viewport_h, doc_h);
        let immediate_rows =
            Self::prioritized_tile_rows(visible_first_row, visible_last_row, scroll_y, viewport_h);

        let fast_initial_display_list = doc_h > viewport_h.saturating_mul(3)
            || root.subtree_bottom > viewport_h as i32 * 3;
        let (initial_visible_y_start, initial_visible_y_end) =
            Self::progressive_band_range(scroll_y, viewport_h, doc_h);

        // 3.5 For very tall documents, build only the visible display list first.
        // This makes the first styled paint arrive quickly; the full list is
        // materialized later on demand when the user scrolls outside the
        // initial viewport band.
        if fast_initial_display_list {
            self.walk_controls_visible(
                root,
                0,
                0,
                parent,
                self.submit_cb,
                self.submit_cb_ud,
                initial_visible_y_start,
                initial_visible_y_end,
            );
            self.display_list =
                DisplayList::build_visible(root, initial_visible_y_start, initial_visible_y_end);
            self.display_list_complete = false;
            self.display_list_y_range = Some((initial_visible_y_start, initial_visible_y_end));
            crate::debug_surf!(
                "[render] initial visible display list: {} commands in [{}..{})",
                self.display_list.cmds.len(),
                initial_visible_y_start,
                initial_visible_y_end
            );
        } else {
            if !allow_progressive_display_list {
                crate::debug_surf!(
                    "[render] progressive display list disabled for correctness-sensitive initial render"
                );
            }
            // 2. Walk full tree for form controls + hit regions.
            self.walk_controls(root, 0, 0, parent, self.submit_cb, self.submit_cb_ud);

            // 3. Build display list (flat, Y-sorted draw commands).
            self.display_list = DisplayList::build(root);
            self.display_list_complete = true;
            self.display_list_y_range = None;
            crate::debug_surf!(
                "[render] display list: {} commands, max_h={}",
                self.display_list.cmds.len(),
                self.display_list.max_h
            );
        }

        // 5. Rasterize visible tiles using the display list.
        for row in immediate_rows.iter().copied() {
            let tile_buf = self.rasterize_tile_dl(images, w, row, doc_h, clear_color);
            self.tile_cache.insert(row, tile_buf);
            self.create_tile_canvas(row, w, doc_h, parent);
        }

        // 6. Bring form controls in front of tile canvases so they are
        //    visible and interactive (canvases were just added on top).
        for fc in &self.form_controls {
            if fc.control_id != 0 && fc.seen {
                ui::Control::from_id(fc.control_id).bring_to_front();
            }
        }

        // 7. GC unseen form controls.
        self.form_controls.retain(|fc| {
            if !fc.seen && fc.control_id != 0 {
                ui::Control::from_id(fc.control_id).remove();
                false
            } else {
                fc.seen || fc.control_id == 0
            }
        });

        crate::debug_surf!(
            "[render] full render done: {} tile canvases, {} hit_regions, {} form_controls",
            self.tile_canvases.len(),
            self.hit_regions.len(),
            self.form_controls.len()
        );
        !self.display_list_complete || immediate_rows.len() < prioritized_rows.len()
    }

    /// Paint-only refresh path.
    ///
    /// Reuses the existing display list / controls / hit regions and only
    /// invalidates tile pixels for the current viewport. This is the fast path
    /// for late image arrivals and pure paint mutations after layout is already
    /// stable.
    pub fn repaint(
        &mut self,
        root: &LayoutBox,
        parent: &ui::View,
        images: &ImageCache,
        doc_w: u32,
        doc_h: u32,
        viewport_h: u32,
        scroll_y: i32,
        bg_color: u32,
    ) -> bool {
        let w = doc_w.max(1);
        let clear_color = if bg_color != 0 { bg_color } else { 0xFFFFFFFF };

        self.doc_w = w;
        self.doc_h = doc_h;
        self.last_scroll_y = scroll_y;

        self.tile_cache.invalidate_all();
        for tc in self.tile_canvases.drain(..) {
            ui::Control::from_id(tc.canvas.id()).remove();
        }

        let (visible_first_row, visible_last_row) =
            Self::visible_tile_row_range(scroll_y, viewport_h, doc_h);
        let immediate_rows =
            Self::prioritized_tile_rows(visible_first_row, visible_last_row, scroll_y, viewport_h);

        if !self.display_list_complete {
            let (visible_y_start, visible_y_end) =
                Self::progressive_band_range(scroll_y, viewport_h, doc_h);
            let needs_full_list = match self.display_list_y_range {
                Some((built_y_start, built_y_end)) => {
                    visible_y_start < built_y_start || visible_y_end > built_y_end
                }
                None => true,
            };
            if needs_full_list {
                crate::debug_surf!(
                    "[render] repaint expanding visible display list to [{}..{})",
                    visible_y_start,
                    visible_y_end
                );
                self.hit_regions.clear();
                self.link_map.clear();
                for fc in &mut self.form_controls {
                    fc.seen = false;
                }
                self.walk_controls_visible(
                    root,
                    0,
                    0,
                    parent,
                    self.submit_cb,
                    self.submit_cb_ud,
                    visible_y_start,
                    visible_y_end,
                );
                self.display_list =
                    DisplayList::build_visible(root, visible_y_start, visible_y_end);
                self.display_list_complete = false;
                self.display_list_y_range = Some((visible_y_start, visible_y_end));
            }
        }

        for row in immediate_rows.iter().copied() {
            let tile_buf = self.rasterize_tile_dl(images, w, row, doc_h, clear_color);
            self.tile_cache.insert(row, tile_buf);
            self.create_tile_canvas(row, w, doc_h, parent);
        }

        for fc in &self.form_controls {
            if fc.control_id != 0 && fc.seen {
                ui::Control::from_id(fc.control_id).bring_to_front();
            }
        }

        !self.display_list_complete
    }

    // ─────────────────────────────────────────────────────────────────────
    // Scroll render (fast path)
    // ─────────────────────────────────────────────────────────────────────

    pub fn render_scroll(
        &mut self,
        root: &LayoutBox,
        parent: &ui::View,
        images: &ImageCache,
        doc_w: u32,
        doc_h: u32,
        viewport_h: u32,
        scroll_y: i32,
        bg_color: u32,
        _link_cb: Option<ui::Callback>,
        _link_cb_ud: u64,
    ) -> bool {
        let w = doc_w.max(1);
        let clear_color = if bg_color != 0 { bg_color } else { 0xFFFFFFFF };

        self.doc_w = w;
        self.doc_h = doc_h;
        self.last_scroll_y = scroll_y;

        let render_y_start = (scroll_y - BUFFER_ZONE).max(0);
        let render_y_end = (scroll_y + viewport_h as i32 + BUFFER_ZONE).min(doc_h as i32);
        let first_row = render_y_start as u32 / TILE_HEIGHT;
        let last_row = if render_y_end > 0 {
            ((render_y_end - 1) as u32) / TILE_HEIGHT
        } else {
            0
        };
        let prioritized_rows =
            Self::prioritized_tile_rows(first_row, last_row, scroll_y, viewport_h);

        if !self.display_list_complete {
            let (band_y_start, band_y_end) =
                Self::progressive_band_range(scroll_y, viewport_h, doc_h);
            let needs_band_expand = match self.display_list_y_range {
                Some((built_y_start, built_y_end)) => {
                    band_y_start < built_y_start || band_y_end > built_y_end
                }
                None => true,
            };
            if needs_band_expand {
                crate::debug_surf!(
                    "[render] expanding visible display list for scroll range [{}..{})",
                    band_y_start,
                    band_y_end
                );
                self.hit_regions.clear();
                self.link_map.clear();
                for fc in &mut self.form_controls {
                    fc.seen = false;
                }
                self.walk_controls_visible(
                    root,
                    0,
                    0,
                    parent,
                    self.submit_cb,
                    self.submit_cb_ud,
                    band_y_start,
                    band_y_end,
                );
                self.display_list = DisplayList::build_visible(root, band_y_start, band_y_end);
                self.display_list_complete = false;
                self.display_list_y_range = Some((band_y_start, band_y_end));
                // Keep already rasterized tiles/canvases. Expanding the band only
                // adds more commands outside the previous range; it should not
                // destroy the current viewport and force a full repaint/jank spike.
            }
        }

        let mut rasterized = 0usize;
        let mut pending = false;
        for row in prioritized_rows {
            if self.tile_canvases.iter().any(|tc| tc.row == row) {
                continue;
            }

            if self.tile_cache.get(row).is_none() {
                if rasterized >= MAX_TILES_PER_TICK {
                    pending = true;
                    continue;
                }
                let tile_buf = self.rasterize_tile_dl(images, w, row, doc_h, clear_color);
                self.tile_cache.insert(row, tile_buf);
                rasterized += 1;
            }

            self.create_tile_canvas(row, w, doc_h, parent);
        }

        // Bring form controls in front of newly added tile canvases.
        if rasterized > 0 {
            for fc in &self.form_controls {
                if fc.control_id != 0 && fc.seen {
                    ui::Control::from_id(fc.control_id).bring_to_front();
                }
            }
        }

        // Evict distant tile canvases.
        let keep_first = first_row.saturating_sub(4);
        let keep_last = (last_row + 4).min(if doc_h > 0 {
            (doc_h - 1) / TILE_HEIGHT
        } else {
            0
        });
        self.tile_canvases.retain(|tc| {
            if tc.row < keep_first || tc.row > keep_last {
                ui::Control::from_id(tc.canvas.id()).remove();
                false
            } else {
                true
            }
        });

        while self.tile_canvases.len() > MAX_TILE_CANVASES {
            let vp_center_row = ((scroll_y + viewport_h as i32 / 2).max(0) as u32) / TILE_HEIGHT;
            let farthest_idx = self
                .tile_canvases
                .iter()
                .enumerate()
                .max_by_key(|(_, tc)| {
                    if tc.row > vp_center_row {
                        tc.row - vp_center_row
                    } else {
                        vp_center_row - tc.row
                    }
                })
                .map(|(i, _)| i)
                .unwrap_or(0);
            let tc = self.tile_canvases.swap_remove(farthest_idx);
            ui::Control::from_id(tc.canvas.id()).remove();
        }

        pending
    }

    // ─────────────────────────────────────────────────────────────────────
    // Internal helpers
    // ─────────────────────────────────────────────────────────────────────

    /// Rasterize a tile using the display list (binary search + linear scan).
    fn rasterize_tile_dl(
        &mut self,
        images: &ImageCache,
        doc_w: u32,
        row: u32,
        doc_h: u32,
        clear_color: u32,
    ) -> Vec<u32> {
        let tile_y_start = (row * TILE_HEIGHT) as i32;
        let tile_y_end = (tile_y_start + TILE_HEIGHT as i32).min(doc_h as i32);

        let pixel_count = (doc_w as usize) * (TILE_HEIGHT as usize);
        let mut buf = self.tile_cache.take_buf(pixel_count, clear_color);

        self.display_list.rasterize_tile(
            images,
            buf.as_mut_ptr(),
            doc_w,
            TILE_HEIGHT,
            tile_y_start,
            tile_y_end,
        );

        buf
    }

    fn create_tile_canvas(&mut self, row: u32, doc_w: u32, doc_h: u32, parent: &ui::View) {
        let pixels = match self.tile_cache.get(row) {
            Some(px) => px,
            None => return,
        };

        let tile_y = (row * TILE_HEIGHT) as i32;
        let tile_h = TILE_HEIGHT
            .min(doc_h.saturating_sub(row * TILE_HEIGHT))
            .max(1);

        let c = ui::Canvas::new(doc_w, tile_h);
        c.set_position(0, tile_y);
        c.set_size(doc_w, tile_h);
        if let Some(cb) = self.link_cb {
            c.on_click_raw(cb, self.link_cb_ud);
            c.on_event_raw(ui::EVENT_MOUSE_MOVE, cb, self.link_cb_ud);
            c.on_event_raw(ui::EVENT_MOUSE_DOWN, cb, self.link_cb_ud);
            c.on_event_raw(ui::EVENT_MOUSE_UP, cb, self.link_cb_ud);
            c.on_event_raw(ui::EVENT_MOUSE_LEAVE, cb, self.link_cb_ud);
        }
        parent.add(&c);
        c.copy_pixels_from(pixels);

        self.tile_canvases.push(TileCanvas { row, canvas: c });
    }

    fn register_control_selector_events(&self, control_id: u32) {
        let Some(cb) = self.link_cb else {
            return;
        };
        let ctrl = ui::Control::from_id(control_id);
        ctrl.on_focus_raw(cb, self.link_cb_ud);
        ctrl.on_blur_raw(cb, self.link_cb_ud);
        ctrl.on_event_raw(ui::EVENT_MOUSE_ENTER, cb, self.link_cb_ud);
        ctrl.on_event_raw(ui::EVENT_MOUSE_LEAVE, cb, self.link_cb_ud);
        ctrl.on_event_raw(ui::EVENT_MOUSE_DOWN, cb, self.link_cb_ud);
        ctrl.on_event_raw(ui::EVENT_MOUSE_UP, cb, self.link_cb_ud);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Walk: form controls + hit regions (unchanged)
    // ─────────────────────────────────────────────────────────────────────

    fn walk_controls(
        &mut self,
        bx: &LayoutBox,
        offset_x: i32,
        offset_y: i32,
        parent: &ui::View,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
    ) {
        if bx.visibility_hidden {
            return;
        }

        let (abs_x, abs_y) = if bx.is_fixed {
            (bx.x, bx.y)
        } else {
            (offset_x + bx.x, offset_y + bx.y)
        };

        if let Some(ref text) = bx.text {
            if !text.is_empty() && bx.form_field.is_none() {
                if let Some(ref url) = bx.link_url {
                    self.hit_regions.push(HitRegion {
                        x: abs_x,
                        y: abs_y,
                        w: bx.width,
                        h: bx.height,
                        kind: HitKind::Link(url.clone()),
                    });
                }
            }
        }

        if let Some(kind) = bx.form_field {
            self.emit_form_control(kind, bx, abs_x, abs_y, parent, submit_cb, submit_cb_ud);
        }

        for child in &bx.children {
            self.walk_controls(child, abs_x, abs_y, parent, submit_cb, submit_cb_ud);
        }
    }

    fn walk_controls_visible(
        &mut self,
        bx: &LayoutBox,
        offset_x: i32,
        offset_y: i32,
        parent: &ui::View,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
        visible_y_start: i32,
        visible_y_end: i32,
    ) {
        if bx.visibility_hidden {
            return;
        }

        let (abs_x, abs_y) = if bx.is_fixed {
            (bx.x, bx.y)
        } else {
            (offset_x + bx.x, offset_y + bx.y)
        };

        if !bx.subtree_has_viewport_positioned {
            let subtree_abs_top = abs_y + bx.subtree_top;
            let subtree_abs_bottom = abs_y + bx.subtree_bottom;
            if subtree_abs_bottom <= visible_y_start || subtree_abs_top >= visible_y_end {
                return;
            }
        }

        let overlaps_visible_band =
            abs_y < visible_y_end && abs_y + bx.height.max(1) > visible_y_start;

        if overlaps_visible_band {
            if let Some(ref text) = bx.text {
                if !text.is_empty() && bx.form_field.is_none() {
                    if let Some(ref url) = bx.link_url {
                        self.hit_regions.push(HitRegion {
                            x: abs_x,
                            y: abs_y,
                            w: bx.width,
                            h: bx.height,
                            kind: HitKind::Link(url.clone()),
                        });
                    }
                }
            }

            if let Some(kind) = bx.form_field {
                self.emit_form_control(kind, bx, abs_x, abs_y, parent, submit_cb, submit_cb_ud);
            }
        }

        for child in &bx.children {
            self.walk_controls_visible(
                child,
                abs_x,
                abs_y,
                parent,
                submit_cb,
                submit_cb_ud,
                visible_y_start,
                visible_y_end,
            );
        }
    }

    fn emit_form_control(
        &mut self,
        kind: FormFieldKind,
        bx: &LayoutBox,
        x: i32,
        y: i32,
        parent: &ui::View,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
    ) {
        let node_id = bx.node_id.unwrap_or(0);

        let w = bx.width;
        let h = bx.height;

        match kind {
            FormFieldKind::TextInput | FormFieldKind::Password => {
                let bg = self.default_control_bg(bx);
                let fg = self.default_control_fg(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(bg);
                    ctrl.set_text_color(fg);
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    // If this input has a datalist, use AutoCompleteTextField.
                    let id = if let Some(ref suggestions) = bx.form_datalist {
                        let atf = ui::AutoCompleteTextField::new();
                        atf.set_position(x, y);
                        atf.set_size(w as u32, h as u32);
                        atf.set_color(bg);
                        atf.set_text_color(fg);
                        atf.set_enabled(!bx.form_disabled);
                        atf.set_suggestions(suggestions);
                        if let Some(ref ph) = bx.form_placeholder {
                            atf.set_placeholder(ph);
                        }
                        if let Some(ref val) = bx.form_value {
                            atf.set_text(val);
                        }
                        if let Some(cb) = submit_cb {
                            atf.on_submit_raw(cb, submit_cb_ud);
                        }
                        parent.add(&atf);
                        atf.id()
                    } else {
                        let tf = ui::TextField::new();
                        if kind == FormFieldKind::Password {
                            tf.set_password_mode(true);
                        }
                        tf.set_position(x, y);
                        tf.set_size(w as u32, h as u32);
                        tf.set_color(bg);
                        tf.set_text_color(fg);
                        tf.set_enabled(!bx.form_disabled);
                        if let Some(ref ph) = bx.form_placeholder {
                            tf.set_placeholder(ph);
                        }
                        if let Some(ref val) = bx.form_value {
                            tf.set_text(val);
                        }
                        if let Some(cb) = submit_cb {
                            tf.on_submit_raw(cb, submit_cb_ud);
                        }
                        parent.add(&tf);
                        tf.id()
                    };
                    let id = id;

                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            FormFieldKind::Submit | FormFieldKind::ButtonEl => {
                self.hit_regions.push(HitRegion {
                    x,
                    y,
                    w,
                    h,
                    kind: HitKind::Submit(node_id),
                });
            }

            FormFieldKind::Reset => {
                self.hit_regions.push(HitRegion {
                    x,
                    y,
                    w,
                    h,
                    kind: HitKind::Reset(node_id),
                });
            }

            FormFieldKind::Checkbox => {
                let accent = self.effective_accent_color(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(accent);
                    ctrl.set_state(if bx.form_checked { 1 } else { 0 });
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let cb = ui::Checkbox::new("");
                    cb.set_position(x, y);
                    cb.set_size(w as u32, h as u32);
                    cb.set_color(accent);
                    cb.set_state(if bx.form_checked { 1 } else { 0 });
                    cb.set_enabled(!bx.form_disabled);
                    parent.add(&cb);
                    let id = cb.id();

                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            FormFieldKind::Radio => {
                let accent = self.effective_accent_color(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(accent);
                    ctrl.set_state(if bx.form_checked { 1 } else { 0 });
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let rb = ui::RadioButton::new("");
                    rb.set_position(x, y);
                    rb.set_size(w as u32, h as u32);
                    rb.set_color(accent);
                    rb.set_state(if bx.form_checked { 1 } else { 0 });
                    rb.set_enabled(!bx.form_disabled);
                    parent.add(&rb);
                    let id = rb.id();

                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            FormFieldKind::Textarea => {
                let bg = self.default_control_bg(bx);
                let fg = self.default_control_fg(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(bg);
                    ctrl.set_text_color(fg);
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let ta = ui::TextArea::new();
                    ta.set_position(x, y);
                    ta.set_size(w as u32, h as u32);
                    ta.set_color(bg);
                    ta.set_text_color(fg);
                    ta.set_enabled(!bx.form_disabled);
                    parent.add(&ta);
                    let id = ta.id();

                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            FormFieldKind::Hidden => {
                if !self
                    .form_controls
                    .iter()
                    .any(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    self.form_controls.push(FormControl {
                        control_id: 0,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                } else {
                    if let Some(fc) = self
                        .form_controls
                        .iter_mut()
                        .find(|fc| fc.node_id == node_id && fc.kind == kind)
                    {
                        fc.seen = true;
                    }
                }
            }

            FormFieldKind::Select => {
                let bg = self.default_control_bg(bx);
                let fg = self.default_control_fg(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(bg);
                    ctrl.set_text_color(fg);
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let items = bx.form_options.as_deref().unwrap_or("");
                    let dd = ui::DropDown::new(items);
                    dd.set_position(x, y);
                    dd.set_size(w as u32, h as u32);
                    dd.set_color(bg);
                    dd.set_text_color(fg);
                    if bx.form_selected_index >= 0 {
                        dd.set_selected_index(bx.form_selected_index as u32);
                    }
                    dd.set_enabled(!bx.form_disabled);
                    parent.add(&dd);
                    let id = dd.id();

                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            FormFieldKind::Range => {
                let accent = self.effective_accent_color(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(accent);
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    // Parse percentage from form_value (encoded as 0..1000).
                    let pct_i: u32 = bx
                        .form_value
                        .as_deref()
                        .and_then(|s| {
                            if s == "X" {
                                Some(100u32)
                            } else {
                                s.parse::<u32>().ok().map(|v| v / 10)
                            }
                        })
                        .unwrap_or(50);
                    let slider = ui::Slider::new(pct_i);
                    slider.set_position(x, y);
                    slider.set_size(w as u32, h as u32);
                    slider.set_color(accent);
                    slider.set_enabled(!bx.form_disabled);
                    parent.add(&slider);
                    let id = slider.id();

                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            // Progress, Meter — display-list painted only (read-only indicators).
            FormFieldKind::Progress | FormFieldKind::Meter => {}

            // Number — TextField with placeholder.
            FormFieldKind::Number => {
                let bg = self.default_control_bg(bx);
                let fg = self.default_control_fg(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(bg);
                    ctrl.set_text_color(fg);
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let tf = ui::TextField::new();
                    tf.set_position(x, y);
                    tf.set_size(w as u32, h as u32);
                    tf.set_color(bg);
                    tf.set_text_color(fg);
                    if let Some(ref ph) = bx.form_placeholder {
                        tf.set_placeholder(ph);
                    }
                    if let Some(ref val) = bx.form_value {
                        tf.set_text(val);
                    }
                    tf.set_enabled(!bx.form_disabled);
                    if let Some(cb) = submit_cb {
                        tf.on_submit_raw(cb, submit_cb_ud);
                    }
                    parent.add(&tf);
                    let id = tf.id();
                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            // Date, Time, DatetimeLocal, Month, Week — native DateTimePicker.
            FormFieldKind::Date
            | FormFieldKind::Time
            | FormFieldKind::DatetimeLocal
            | FormFieldKind::Month
            | FormFieldKind::Week => {
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let picker = match kind {
                        FormFieldKind::Time => {
                            let tp = ui::TimePicker::new();
                            // Parse "HH:MM" from value.
                            if let Some(ref val) = bx.form_value {
                                if let Some((h, m)) = parse_time_value(val) {
                                    tp.set_time(h, m);
                                }
                            }
                            tp.set_position(x, y);
                            tp.set_size(w as u32, h as u32);
                            tp.set_enabled(!bx.form_disabled);
                            parent.add(&tp);
                            tp.id()
                        }
                        FormFieldKind::Date | FormFieldKind::Month | FormFieldKind::Week => {
                            let dp = ui::DatePicker::new();
                            if let Some(ref val) = bx.form_value {
                                if let Some((y, m, d)) = parse_date_value(val) {
                                    dp.set_date(d, m, y);
                                }
                            }
                            dp.set_position(x, y);
                            dp.set_size(w as u32, h as u32);
                            dp.set_enabled(!bx.form_disabled);
                            parent.add(&dp);
                            dp.id()
                        }
                        _ => {
                            // DatetimeLocal
                            let dtp = ui::DateTimePicker::new();
                            if let Some(ref val) = bx.form_value {
                                // Parse "YYYY-MM-DDThh:mm"
                                if let (Some((y, mo, d)), Some((h, mi))) = (
                                    parse_date_value(&val.split('T').next().unwrap_or("")),
                                    val.split('T').nth(1).and_then(|t| parse_time_value(t)),
                                ) {
                                    dtp.set_datetime(d, mo, y, h, mi);
                                }
                            }
                            dtp.set_position(x, y);
                            dtp.set_size(w as u32, h as u32);
                            dtp.set_enabled(!bx.form_disabled);
                            parent.add(&dtp);
                            dtp.id()
                        }
                    };
                    self.form_controls.push(FormControl {
                        control_id: picker,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            // Color input — native ColorWell with color picker dialog.
            FormFieldKind::Color => {
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let cw = ui::ColorWell::new();
                    cw.set_position(x, y);
                    cw.set_size(w as u32, h as u32);
                    let val = bx.form_value.as_deref().unwrap_or("#000000");
                    let color = parse_color_value(val);
                    cw.set_selected_color(color);
                    if bx.form_disabled {
                        cw.set_enabled(false);
                    }
                    parent.add(&cw);
                    let id = cw.id();
                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            // File input — button + filename display.
            FormFieldKind::File => {
                self.hit_regions.push(HitRegion {
                    x,
                    y,
                    w,
                    h,
                    kind: HitKind::FileInput(node_id),
                });
                // Store a hidden control for form data.
                if !self
                    .form_controls
                    .iter()
                    .any(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    self.form_controls.push(FormControl {
                        control_id: 0,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                } else if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    fc.seen = true;
                }
            }

            FormFieldKind::Reset => {
                // Already handled above in the hit_regions push.
            }
        }
    }

    /// Hit-test for a focusable form control at document coordinates.
    /// Returns the control_id of the first matching control.
    pub fn hit_test_form_at(&self, x: i32, doc_y: i32) -> Option<u32> {
        for fc in &self.form_controls {
            if fc.control_id == 0 {
                continue;
            }
            match fc.kind {
                FormFieldKind::TextInput
                | FormFieldKind::Password
                | FormFieldKind::Textarea
                | FormFieldKind::Number
                | FormFieldKind::Date
                | FormFieldKind::Time
                | FormFieldKind::DatetimeLocal
                | FormFieldKind::Month
                | FormFieldKind::Week
                | FormFieldKind::Color
                | FormFieldKind::Select
                | FormFieldKind::Range => {}
                _ => continue,
            }
            if x >= fc.doc_x
                && x < fc.doc_x + fc.doc_w
                && doc_y >= fc.doc_y
                && doc_y < fc.doc_y + fc.doc_h
            {
                return Some(fc.control_id);
            }
        }
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Buffer drawing helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Fill a rectangle directly in the ARGB pixel buffer with clipping.
/// Parse a CSS color hex value (#RGB, #RRGGBB) into ARGB u32.
/// Parse "YYYY-MM-DD" into (year, month, day).
fn parse_date_value(s: &str) -> Option<(u32, u32, u32)> {
    let b = s.as_bytes();
    // Expect at least YYYY-MM-DD (10 chars).
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let year = parse_uint(&b[0..4])?;
    let month = parse_uint(&b[5..7])?;
    let day = parse_uint(&b[8..10])?;
    Some((year, month, day))
}

/// Parse "HH:MM" into (hour, minute).
fn parse_time_value(s: &str) -> Option<(u32, u32)> {
    let b = s.as_bytes();
    if b.len() < 5 || b[2] != b':' {
        return None;
    }
    let hour = parse_uint(&b[0..2])?;
    let minute = parse_uint(&b[3..5])?;
    Some((hour, minute))
}

fn parse_uint(b: &[u8]) -> Option<u32> {
    let mut n: u32 = 0;
    for &c in b {
        if c < b'0' || c > b'9' {
            return None;
        }
        n = n * 10 + (c - b'0') as u32;
    }
    Some(n)
}

pub fn parse_color_value(s: &str) -> u32 {
    let s = s.trim();
    let hex = if s.starts_with('#') { &s[1..] } else { s };
    let (r, g, b) = match hex.len() {
        3 => {
            let r = hex_nibble(hex.as_bytes()[0]) * 17;
            let g = hex_nibble(hex.as_bytes()[1]) * 17;
            let b = hex_nibble(hex.as_bytes()[2]) * 17;
            (r, g, b)
        }
        6 => {
            let r = hex_nibble(hex.as_bytes()[0]) * 16 + hex_nibble(hex.as_bytes()[1]);
            let g = hex_nibble(hex.as_bytes()[2]) * 16 + hex_nibble(hex.as_bytes()[3]);
            let b = hex_nibble(hex.as_bytes()[4]) * 16 + hex_nibble(hex.as_bytes()[5]);
            (r, g, b)
        }
        _ => (0, 0, 0),
    };
    0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

fn hex_nibble(c: u8) -> u32 {
    match c {
        b'0'..=b'9' => (c - b'0') as u32,
        b'a'..=b'f' => (c - b'a' + 10) as u32,
        b'A'..=b'F' => (c - b'A' + 10) as u32,
        _ => 0,
    }
}

fn fill_rect_buf(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u32,
) {
    if w <= 0 || h <= 0 || buf.is_null() {
        return;
    }
    let s = stride as i32;
    let bh = buf_h as i32;

    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(s);
    let y1 = (y + h).min(bh);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let cw = (x1 - x0) as usize;
    let alpha = (color >> 24) & 0xFF;
    unsafe {
        for row in y0..y1 {
            let offset = row as usize * stride as usize + x0 as usize;
            let ptr = buf.add(offset);
            if alpha >= 255 {
                // Fast path: 4-pixel unrolled opaque fill.
                let mut i = 0usize;
                let cw4 = cw & !3;
                while i < cw4 {
                    *ptr.add(i) = color;
                    *ptr.add(i + 1) = color;
                    *ptr.add(i + 2) = color;
                    *ptr.add(i + 3) = color;
                    i += 4;
                }
                while i < cw {
                    *ptr.add(i) = color;
                    i += 1;
                }
            } else if alpha > 0 {
                let inv_a = 255 - alpha;
                let sr = (color >> 16) & 0xFF;
                let sg = (color >> 8) & 0xFF;
                let sb = color & 0xFF;
                for i in 0..cw {
                    let dst = *ptr.add(i);
                    let dr = (dst >> 16) & 0xFF;
                    let dg = (dst >> 8) & 0xFF;
                    let db = dst & 0xFF;
                    let r = (sr * alpha + dr * inv_a) / 255;
                    let g = (sg * alpha + dg * inv_a) / 255;
                    let b = (sb * alpha + db * inv_a) / 255;
                    *ptr.add(i) = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}

/// Blit image pixels into the buffer with scaling and clipping.
fn blit_image_buf(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    dx: i32,
    dy: i32,
    dw: i32,
    dh: i32,
    src: &[u32],
    src_w: u32,
    src_h: u32,
) {
    if dw <= 0 || dh <= 0 || src.is_empty() || src_w == 0 || src_h == 0 || buf.is_null() {
        return;
    }
    let s = stride as i32;
    let bh = buf_h as i32;

    let x0 = dx.max(0);
    let y0 = dy.max(0);
    let x1 = (dx + dw).min(s);
    let y1 = (dy + dh).min(bh);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    unsafe {
        for row in y0..y1 {
            let sy = ((row - dy) as u64 * src_h as u64 / dh as u64) as usize;
            if sy >= src_h as usize {
                continue;
            }
            let dst_offset = row as usize * stride as usize;
            let src_row = sy * src_w as usize;
            for col in x0..x1 {
                let sx = ((col - dx) as u64 * src_w as u64 / dw as u64) as usize;
                if sx >= src_w as usize {
                    continue;
                }
                let src_idx = src_row + sx;
                if src_idx >= src.len() {
                    continue;
                }
                let pixel = src[src_idx];
                let alpha = (pixel >> 24) & 0xFF;
                let dst_idx = dst_offset + col as usize;
                if alpha >= 255 {
                    *buf.add(dst_idx) = pixel;
                } else if alpha > 0 {
                    let dst = *buf.add(dst_idx);
                    let inv_a = 255 - alpha;
                    let r = (((pixel >> 16) & 0xFF) * alpha + ((dst >> 16) & 0xFF) * inv_a) / 255;
                    let g = (((pixel >> 8) & 0xFF) * alpha + ((dst >> 8) & 0xFF) * inv_a) / 255;
                    let b = ((pixel & 0xFF) * alpha + (dst & 0xFF) * inv_a) / 255;
                    *buf.add(dst_idx) = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}

fn blit_image_buf_clipped(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    dx: i32,
    dy: i32,
    dw: i32,
    dh: i32,
    clip: (i32, i32, i32, i32),
    src: &[u32],
    src_w: u32,
    src_h: u32,
) {
    if dw <= 0 || dh <= 0 || src.is_empty() || src_w == 0 || src_h == 0 || buf.is_null() {
        return;
    }
    let s = stride as i32;
    let bh = buf_h as i32;
    let (clip_x, clip_y, clip_w, clip_h) = clip;
    let x0 = dx.max(clip_x).max(0);
    let y0 = dy.max(clip_y).max(0);
    let x1 = (dx + dw).min(clip_x + clip_w).min(s);
    let y1 = (dy + dh).min(clip_y + clip_h).min(bh);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    unsafe {
        for row in y0..y1 {
            let sy = ((row - dy) as u64 * src_h as u64 / dh as u64) as usize;
            if sy >= src_h as usize {
                continue;
            }
            let dst_offset = row as usize * stride as usize;
            let src_row = sy * src_w as usize;
            for col in x0..x1 {
                let sx = ((col - dx) as u64 * src_w as u64 / dw as u64) as usize;
                if sx >= src_w as usize {
                    continue;
                }
                let src_idx = src_row + sx;
                if src_idx >= src.len() {
                    continue;
                }
                let pixel = src[src_idx];
                let alpha = (pixel >> 24) & 0xFF;
                let dst_idx = dst_offset + col as usize;
                if alpha >= 255 {
                    *buf.add(dst_idx) = pixel;
                } else if alpha > 0 {
                    let dst = *buf.add(dst_idx);
                    let inv_a = 255 - alpha;
                    let r = (((pixel >> 16) & 0xFF) * alpha + ((dst >> 16) & 0xFF) * inv_a) / 255;
                    let g = (((pixel >> 8) & 0xFF) * alpha + ((dst >> 8) & 0xFF) * inv_a) / 255;
                    let b = ((pixel & 0xFF) * alpha + (dst & 0xFF) * inv_a) / 255;
                    *buf.add(dst_idx) = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}

fn resolve_axis_origin(start: i32, size: i32, value: i32, is_percent: bool) -> i32 {
    if is_percent {
        start + (size as i64 * value as i64 / 10000) as i32
    } else {
        start + value
    }
}

fn resolve_object_position_offset(free_space: i32, value: i32, is_percent: bool) -> i32 {
    if is_percent {
        (free_space as i64 * value as i64 / 10000) as i32
    } else {
        value
    }
}

fn fit_contain_size(dw: i32, dh: i32, src_w: u32, src_h: u32) -> (i32, i32) {
    let sw = src_w as i64;
    let sh = src_h as i64;
    let dw64 = dw as i64;
    let dh64 = dh as i64;
    if sw * dh64 > sh * dw64 {
        (dw, (sh * dw64 / sw).max(1) as i32)
    } else {
        ((sw * dh64 / sh).max(1) as i32, dh)
    }
}

fn fit_cover_size(dw: i32, dh: i32, src_w: u32, src_h: u32) -> (i32, i32) {
    let sw = src_w as i64;
    let sh = src_h as i64;
    let dw64 = dw as i64;
    let dh64 = dh as i64;
    if sw * dh64 < sh * dw64 {
        (dw, (sh * dw64 / sw).max(1) as i32)
    } else {
        ((sw * dh64 / sh).max(1) as i32, dh)
    }
}

/// Blit image with object-fit semantics.
fn blit_image_scaled(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    dx: i32,
    dy: i32,
    dw: i32,
    dh: i32,
    clip: (i32, i32, i32, i32),
    src: &[u32],
    src_w: u32,
    src_h: u32,
    fit: crate::style::ObjectFit,
    pos_x: i32,
    pos_x_is_percent: bool,
    pos_y: i32,
    pos_y_is_percent: bool,
) {
    use crate::style::ObjectFit;
    if dw <= 0 || dh <= 0 || src.is_empty() || src_w == 0 || src_h == 0 {
        return;
    }
    match fit {
        ObjectFit::Fill => {
            blit_image_buf_clipped(buf, stride, buf_h, dx, dy, dw, dh, clip, src, src_w, src_h);
        }
        ObjectFit::Contain | ObjectFit::ScaleDown => {
            let (fw, fh) = fit_contain_size(dw, dh, src_w, src_h);
            let (fw, fh) = if matches!(fit, ObjectFit::ScaleDown)
                && src_w as i32 <= dw
                && src_h as i32 <= dh
            {
                (src_w as i32, src_h as i32)
            } else {
                (fw, fh)
            };
            let ox = dx + resolve_object_position_offset(dw - fw, pos_x, pos_x_is_percent);
            let oy = dy + resolve_object_position_offset(dh - fh, pos_y, pos_y_is_percent);
            blit_image_buf_clipped(buf, stride, buf_h, ox, oy, fw, fh, clip, src, src_w, src_h);
        }
        ObjectFit::Cover => {
            let (fw, fh) = fit_cover_size(dw, dh, src_w, src_h);
            let ox = dx + resolve_object_position_offset(dw - fw, pos_x, pos_x_is_percent);
            let oy = dy + resolve_object_position_offset(dh - fh, pos_y, pos_y_is_percent);
            blit_image_buf_clipped(buf, stride, buf_h, ox, oy, fw, fh, clip, src, src_w, src_h);
        }
        ObjectFit::None => {
            let nw = src_w as i32;
            let nh = src_h as i32;
            let ox = dx + resolve_object_position_offset(dw - nw, pos_x, pos_x_is_percent);
            let oy = dy + resolve_object_position_offset(dh - nh, pos_y, pos_y_is_percent);
            blit_image_buf_clipped(buf, stride, buf_h, ox, oy, nw, nh, clip, src, src_w, src_h);
        }
    }
}

// ---------------------------------------------------------------------------
// Gradient and shadow helpers
// ---------------------------------------------------------------------------

/// Interpolate a color along a gradient at position `t` (0..10000).
fn interpolate_gradient_color(stops: &[crate::style::GradientStop], t: i32) -> u32 {
    if stops.is_empty() {
        return 0xFF000000;
    }
    if stops.len() == 1 {
        return stops[0].color;
    }

    // Find the two stops that bracket `t`.
    let t_clamped = t.max(0).min(10000);
    let mut prev = &stops[0];
    for stop in &stops[1..] {
        if stop.position >= t_clamped {
            // Interpolate between prev and stop.
            let range = stop.position - prev.position;
            if range <= 0 {
                return stop.color;
            }
            let frac = ((t_clamped - prev.position) * 255 / range) as u32;
            return lerp_color(prev.color, stop.color, frac);
        }
        prev = stop;
    }
    stops.last().map(|s| s.color).unwrap_or(0xFF000000)
}

/// Linear interpolation between two ARGB colors. `frac` is 0..255.
fn lerp_color(c0: u32, c1: u32, frac: u32) -> u32 {
    let inv = 255 - frac;
    let a = (((c0 >> 24) & 0xFF) * inv + ((c1 >> 24) & 0xFF) * frac) / 255;
    let r = (((c0 >> 16) & 0xFF) * inv + ((c1 >> 16) & 0xFF) * frac) / 255;
    let g = (((c0 >> 8) & 0xFF) * inv + ((c1 >> 8) & 0xFF) * frac) / 255;
    let b = ((c0 & 0xFF) * inv + (c1 & 0xFF) * frac) / 255;
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Apply an alpha value (0..255) to an existing color.
fn alpha_blend(color: u32, alpha: u32) -> u32 {
    let existing_a = (color >> 24) & 0xFF;
    let new_a = (existing_a * alpha / 255).min(255);
    (new_a << 24) | (color & 0x00FFFFFF)
}

/// Darken a color by a percentage (0..100).
fn darken_color(color: u32, amount: u32) -> u32 {
    let a = (color >> 24) & 0xFF;
    let r = ((color >> 16) & 0xFF) * (100 - amount) / 100;
    let g = ((color >> 8) & 0xFF) * (100 - amount) / 100;
    let b = (color & 0xFF) * (100 - amount) / 100;
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Lighten a color by a percentage (0..100).
fn lighten_color(color: u32, amount: u32) -> u32 {
    let a = (color >> 24) & 0xFF;
    let r = (((color >> 16) & 0xFF) + (255 - ((color >> 16) & 0xFF)) * amount / 100).min(255);
    let g = (((color >> 8) & 0xFF) + (255 - ((color >> 8) & 0xFF)) * amount / 100).min(255);
    let b = ((color & 0xFF) + (255 - (color & 0xFF)) * amount / 100).min(255);
    (a << 24) | (r << 16) | (g << 8) | b
}

// ---------------------------------------------------------------------------
// Rounded rectangle rendering
// ---------------------------------------------------------------------------

/// Fill a rounded rectangle. `radii` = [top-left, top-right, bottom-right, bottom-left].
/// Uses a simple per-pixel distance check against corner circles.
fn fill_rounded_rect_buf(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u32,
    radii: [i32; 4],
) {
    if w <= 0 || h <= 0 || buf.is_null() {
        return;
    }
    let s = stride as i32;
    let bh = buf_h as i32;

    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(s);
    let y1 = (y + h).min(bh);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let alpha = (color >> 24) & 0xFF;
    if alpha == 0 {
        return;
    }

    let [rtl, rtr, rbr, rbl] = radii;

    unsafe {
        for row in y0..y1 {
            let ry = row - y; // relative y within rect
            let offset = row as usize * stride as usize;
            for col in x0..x1 {
                let rx = col - x; // relative x within rect

                // Check if this pixel falls inside a rounded corner.
                let inside = is_inside_rounded_rect(rx, ry, w, h, rtl, rtr, rbr, rbl);
                if !inside {
                    continue;
                }

                let dst_idx = offset + col as usize;
                if alpha >= 255 {
                    *buf.add(dst_idx) = color;
                } else {
                    let dst = *buf.add(dst_idx);
                    let inv_a = 255 - alpha;
                    let r = (((color >> 16) & 0xFF) * alpha + ((dst >> 16) & 0xFF) * inv_a) / 255;
                    let g = (((color >> 8) & 0xFF) * alpha + ((dst >> 8) & 0xFF) * inv_a) / 255;
                    let b = ((color & 0xFF) * alpha + (dst & 0xFF) * inv_a) / 255;
                    *buf.add(dst_idx) = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}

/// Check if point (px, py) relative to rect origin is inside a rounded rect.
#[inline]
fn is_inside_rounded_rect(
    px: i32,
    py: i32,
    w: i32,
    h: i32,
    rtl: i32,
    rtr: i32,
    rbr: i32,
    rbl: i32,
) -> bool {
    // Top-left corner
    if px < rtl && py < rtl {
        let dx = rtl - px;
        let dy = rtl - py;
        return dx * dx + dy * dy <= rtl * rtl;
    }
    // Top-right corner
    if px >= w - rtr && py < rtr {
        let dx = px - (w - rtr - 1);
        let dy = rtr - py;
        return dx * dx + dy * dy <= rtr * rtr;
    }
    // Bottom-right corner
    if px >= w - rbr && py >= h - rbr {
        let dx = px - (w - rbr - 1);
        let dy = py - (h - rbr - 1);
        return dx * dx + dy * dy <= rbr * rbr;
    }
    // Bottom-left corner
    if px < rbl && py >= h - rbl {
        let dx = rbl - px;
        let dy = py - (h - rbl - 1);
        return dx * dx + dy * dy <= rbl * rbl;
    }
    true
}

// ---------------------------------------------------------------------------
// Dashed / dotted border rendering
// ---------------------------------------------------------------------------

/// Fill a dashed/dotted line pattern within the given rect.
fn fill_dashed_buf(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u32,
    dash_len: i32,
    gap_len: i32,
    vertical: bool,
) {
    if w <= 0 || h <= 0 || buf.is_null() || dash_len <= 0 {
        return;
    }
    let cycle = dash_len + gap_len;
    if cycle <= 0 {
        return;
    }

    if vertical {
        // Vertical dashed line: iterate along height, fill dash segments.
        let mut pos = 0;
        while pos < h {
            let seg_len = dash_len.min(h - pos);
            if seg_len > 0 {
                fill_rect_buf(buf, stride, buf_h, x, y + pos, w, seg_len, color);
            }
            pos += cycle;
        }
    } else {
        // Horizontal dashed line: iterate along width, fill dash segments.
        let mut pos = 0;
        while pos < w {
            let seg_len = dash_len.min(w - pos);
            if seg_len > 0 {
                fill_rect_buf(buf, stride, buf_h, x + pos, y, seg_len, h, color);
            }
            pos += cycle;
        }
    }
}

// ---------------------------------------------------------------------------
// Trig approximations (no_std)
// ---------------------------------------------------------------------------

/// Sine approximation using Bhaskara I's formula. Input in radians.
fn sin_approx(x: f32) -> f32 {
    // Normalize to [0, 2*PI)
    let pi = core::f32::consts::PI;
    let two_pi = 2.0 * pi;
    let mut a = x % two_pi;
    if a < 0.0 {
        a += two_pi;
    }

    let sign = if a > pi {
        a -= pi;
        -1.0
    } else {
        1.0
    };

    // Bhaskara I approximation for [0, PI]:
    // sin(x) ≈ 16x(PI-x) / (5*PI^2 - 4x(PI-x))
    let num = 16.0 * a * (pi - a);
    let den = 5.0 * pi * pi - 4.0 * a * (pi - a);
    if den.abs() < 0.001 {
        return 0.0;
    }
    sign * num / den
}

/// Cosine approximation: cos(x) = sin(x + PI/2).
fn cos_approx(x: f32) -> f32 {
    sin_approx(x + core::f32::consts::FRAC_PI_2)
}

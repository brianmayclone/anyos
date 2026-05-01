use alloc::string::String;
use alloc::vec::Vec;

use crate::layout::FormFieldKind;
use crate::style::{BackgroundImageVal, BackgroundRepeatVal, BackgroundSizeVal};

// ═══════════════════════════════════════════════════════════════════════════
// Hit regions
// ═══════════════════════════════════════════════════════════════════════════

pub(super) struct HitRegion {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) w: i32,
    pub(super) h: i32,
    pub(super) kind: HitKind,
}

pub enum HitKind {
    Link(String),
    Submit(usize),
    /// Reset button — node_id of the reset element.
    Reset(usize),
    Select(usize),
    Checkbox(usize),
    Radio(usize),
    Range(usize),
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
    pub(super) seen: bool,
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
pub(super) struct DrawCmd {
    /// Document-space paint bounds used for culling and clip intersection.
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) w: i32,
    pub(super) h: i32,
    /// Untransformed source rect in document coordinates.
    pub(super) src_x: i32,
    pub(super) src_y: i32,
    pub(super) src_w: i32,
    pub(super) src_h: i32,
    /// The drawing operation.
    pub(super) kind: DrawKind,
    /// Optional clip rect (from parent with overflow:hidden).
    /// (clip_x, clip_y, clip_w, clip_h) — commands are clipped to this rect.
    pub(super) clip: Option<(i32, i32, i32, i32)>,
    /// Active CSS mask layers inherited from ancestors and this element.
    pub(super) masks: Vec<MaskLayer>,
    /// Active rotation transforms inherited from ancestors and this element.
    pub(super) rotations: Vec<DrawRotation>,
}

#[derive(Clone, Copy)]
pub(super) struct DrawRotation {
    pub(super) origin_x: i32,
    pub(super) origin_y: i32,
    pub(super) angle_deg100: i32,
}

#[derive(Clone)]
pub(super) struct MaskLayer {
    pub(super) clip_rect: (i32, i32, i32, i32),
    pub(super) origin_rect: (i32, i32, i32, i32),
    pub(super) image: BackgroundImageVal,
    pub(super) size: BackgroundSizeVal,
    pub(super) repeat: BackgroundRepeatVal,
    pub(super) position_x: i32,
    pub(super) position_x_is_percent: bool,
    pub(super) position_y: i32,
    pub(super) position_y_is_percent: bool,
}

pub(super) enum DrawKind {
    /// Fill a rectangle with a solid or alpha-blended color.
    Rect { color: u32 },
    /// Fill a rounded rectangle with corner radii.
    RoundedRect { color: u32, radii: [i32; 4] }, // [tl, tr, br, bl]
    /// Stroke a rounded rectangle border.
    RoundedBorder {
        color: u32,
        radii: [i32; 4],
        widths: [i32; 4], // [top, right, bottom, left]
    },
    /// Fill a triangle with local coordinates relative to the draw command rect.
    Triangle {
        color: u32,
        p0: (i32, i32),
        p1: (i32, i32),
        p2: (i32, i32),
    },
    /// Draw a dashed/dotted horizontal or vertical border line.
    DashedLine {
        color: u32,
        dash_len: i32,
        gap_len: i32,
        vertical: bool,
    },
    RadialGradient {
        center_x: i32,
        center_y: i32,
        stops: Vec<crate::style::GradientStop>,
    },
    /// Draw a text string.
    Text {
        color: u32,
        font_id: u32,
        font_size: u16,
        scale_x_percent: i32,
        synthetic_bold: bool,
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
    pub(super) cmds: Vec<DrawCmd>,
    /// Per-256px tile-row index into `cmds`.  Indices stay in paint order
    /// because they are appended while scanning `cmds` front-to-back.
    pub(super) tile_cmds: Vec<Vec<usize>>,
    /// Current clip rect during flatten (None = no clipping).
    pub(super) clip_stack: Vec<(i32, i32, i32, i32)>,
    /// Current CSS mask layers during flatten.
    pub(super) mask_stack: Vec<MaskLayer>,
    /// Active rotation transforms during flatten.
    pub(super) rotation_stack: Vec<DrawRotation>,
    /// Maximum command height seen — used as search margin for binary search.
    pub(super) max_h: i32,
    /// Optional document-space Y cull range for fast initial paints.
    pub(super) cull_y_range: Option<(i32, i32)>,
}

#[derive(Clone, Copy)]
pub(super) struct StickyContext {
    pub(super) top: i32,
    pub(super) height: i32,
}

//! Layout engine for the Surf web browser.
//!
//! Takes a DOM tree (`Dom`) and per-node computed styles (`ComputedStyle`)
//! and produces a tree of `LayoutBox`es with absolute positions and sizes.
//!
//! Sub-modules:
//!   - `block`: Block-level layout (`build_block`)
//!   - `flex`: Flexbox layout (`layout_flex`)
//!   - `inline`: Inline/text layout, form element fragments
//!   - `form`: Form field position collection

pub mod block;
pub mod flex;
pub mod form;
pub mod grid;
pub mod inline;
pub mod table;

use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::{Dom, NodeId, Tag};
use crate::style::{
    AlignItems, ClearVal, ComputedStyle, Direction, Display, FloatVal, FontStyleVal, FontWeight,
    InlineAxisAlignment, ListStyle, ListStylePosition, OverflowVal, Position, PseudoStyles,
    TextAlignVal, TextDeco, TextTransform,
};
use crate::ImageCache;

// Re-export sub-module public items.
use block::{build_block, build_block_with_budget};
pub use form::{collect_form_positions, FormFieldPos};
use inline::{layout_inline_content, layout_inline_content_with_pseudo};

// ---------------------------------------------------------------------------
// Public data structures
// ---------------------------------------------------------------------------

pub struct LayoutBox {
    pub node_id: Option<NodeId>,
    pub box_type: BoxType,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub margin: Edges,
    pub padding: Edges,
    pub border_width: i32,
    pub children: Vec<LayoutBox>,
    /// Text content for text runs.
    pub text: Option<String>,
    pub font_size: i32,
    pub bold: bool,
    pub italic: bool,
    pub color: u32,
    pub bg_color: u32,
    pub accent_color: u32,
    pub uses_dark_color_scheme: bool,
    pub appearance_none: bool,
    pub border_color: u32,
    pub border_radius: i32,
    pub text_decoration: TextDeco,
    pub text_align: TextAlignVal,
    pub link_url: Option<String>,
    pub list_marker: Option<String>,
    pub list_marker_inside: bool,
    pub is_hr: bool,
    /// Image source URL for `<img>` elements.
    pub image_src: Option<String>,
    pub image_width: Option<i32>,
    pub image_height: Option<i32>,
    /// Form field kind (for `<input>`, `<button>`, `<textarea>`, `<select>`).
    pub form_field: Option<FormFieldKind>,
    /// Placeholder text for form text inputs.
    pub form_placeholder: Option<String>,
    /// Default value for form text inputs.
    pub form_value: Option<String>,
    /// Pipe-separated option labels for `<select>` (used by native DropDown).
    pub form_options: Option<String>,
    /// Index of the initially selected `<option>` (0-based).
    pub form_selected_index: i32,
    /// Pipe-separated option values for `<select>` (parallel to form_options).
    pub form_option_values: Option<String>,
    /// Whether `<select>` has `multiple` attribute.
    pub form_multiple: bool,
    /// `size` attribute for `<select>` (number of visible rows, 0 = default dropdown).
    pub form_size: u32,
    /// `disabled` attribute.
    pub form_disabled: bool,
    /// `checked` state for checkbox/radio controls.
    pub form_checked: bool,
    /// `readonly` attribute.
    pub form_readonly: bool,
    /// `required` attribute.
    pub form_required: bool,
    /// `min` attribute (for number/range/date/time).
    pub form_min: Option<String>,
    /// `max` attribute (for number/range/date/time).
    pub form_max: Option<String>,
    /// `step` attribute (for number/range/date/time).
    pub form_step: Option<String>,
    /// `pattern` attribute (regex constraint).
    pub form_pattern: Option<String>,
    /// `maxlength` attribute (-1 = unset).
    pub form_maxlength: i32,
    /// `minlength` attribute (-1 = unset).
    pub form_minlength: i32,
    /// Pipe-separated datalist suggestions (from `<datalist>` referenced by `list` attr).
    pub form_datalist: Option<String>,
    /// If true, children that extend outside this box should be clipped.
    pub overflow_hidden: bool,
    /// If true, this box is invisible but still takes up space.
    pub visibility_hidden: bool,
    /// Opacity: 0..255 (255 = fully opaque).
    pub opacity: i32,
    /// If true, this box is `position:fixed` and its x/y are viewport-relative.
    /// The renderer will ignore accumulated parent offsets and use x/y directly.
    pub is_fixed: bool,
    /// If true, this box is `position:absolute` or `position:fixed` — out of
    /// normal flow.  Used by `intrinsic_width()` to skip these children.
    pub is_out_of_flow: bool,
    /// Hypothetical in-flow static-position rectangle for abs/fixed alignment.
    pub static_position_x: Option<i32>,
    pub static_position_y: Option<i32>,
    pub static_position_width: Option<i32>,
    pub static_position_height: Option<i32>,
    /// Per-side border widths (litehtml-style).
    pub border_top_width: i32,
    pub border_right_width: i32,
    pub border_bottom_width: i32,
    pub border_left_width: i32,
    /// Per-side border colors.
    pub border_top_color: u32,
    pub border_right_color: u32,
    pub border_bottom_color: u32,
    pub border_left_color: u32,
    /// Per-corner radii.
    pub border_top_left_radius: i32,
    pub border_top_right_radius: i32,
    pub border_bottom_right_radius: i32,
    pub border_bottom_left_radius: i32,
    /// Outline.
    pub outline_width: i32,
    pub outline_color: u32,
    pub outline_offset: i32,
    /// Box shadows.
    pub box_shadows: Vec<crate::style::BoxShadowVal>,
    /// Text shadows.
    pub text_shadows: Vec<crate::style::TextShadowVal>,
    /// Text overflow mode.
    pub text_overflow_ellipsis: bool,
    /// Background image / gradient.
    pub background_image: crate::style::BackgroundImageVal,
    pub mask_image: crate::style::BackgroundImageVal,
    pub background_size: crate::style::BackgroundSizeVal,
    pub background_repeat: crate::style::BackgroundRepeatVal,
    pub background_clip: crate::style::BackgroundClipVal,
    pub mask_size: crate::style::BackgroundSizeVal,
    pub mask_repeat: crate::style::BackgroundRepeatVal,
    pub mask_clip: crate::style::BackgroundClipVal,
    pub mask_origin: crate::style::BackgroundClipVal,
    pub mask_position_x: i32,
    pub mask_position_x_is_percent: bool,
    pub mask_position_y: i32,
    pub mask_position_y_is_percent: bool,
    /// Letter spacing (px).
    pub letter_spacing: i32,
    /// Z-index for stacking context.
    pub z_index: i32,
    /// Whether z-index is `auto` (no explicit integer set).
    pub z_index_auto: bool,
    /// Whether this box creates a new stacking context (CSS2 §9.9.1).
    /// True for: positioned elements with explicit z-index, opacity < 1,
    /// elements with transform, etc.
    pub creates_stacking_context: bool,
    /// Per-side border styles.
    pub border_top_style: crate::style::BorderStyleVal,
    pub border_right_style: crate::style::BorderStyleVal,
    pub border_bottom_style: crate::style::BorderStyleVal,
    pub border_left_style: crate::style::BorderStyleVal,
    /// CSS filter effects.
    pub filter: crate::style::FilterVal,
    /// Clip path.
    pub clip_path: crate::style::ClipPathVal,
    /// Text decoration color (0 = use text color).
    pub text_decoration_color: u32,
    /// Text decoration style.
    pub text_decoration_style: crate::style::TextDecorationStyle,
    /// Text decoration thickness (0 = auto).
    pub text_decoration_thickness: i32,
    /// Text underline offset (0 = auto).
    pub text_underline_offset: i32,
    /// Object-fit for `<img>` elements.
    pub object_fit: crate::style::ObjectFit,
    pub object_position_x: i32,
    pub object_position_x_is_percent: bool,
    pub object_position_y: i32,
    pub object_position_y_is_percent: bool,
    /// Custom font ID from web fonts (0 = use system font based on bold/italic).
    pub custom_font_id: u32,
    /// If true, this box is `position:sticky` — the renderer should clamp its Y
    /// position based on the scroll offset and `sticky_top`.
    pub is_sticky: bool,
    /// The `top` value for sticky positioning (distance from viewport top when stuck).
    pub sticky_top: i32,
    /// CSS `clip: rect(top, right, bottom, left)` for absolute elements.
    /// Values are in px relative to the element's own top-left corner.
    pub clip_rect: Option<[i32; 4]>,
    /// CSS transform scale X (×1000 fixed-point, 1000 = 1.0).
    pub transform_sx: i32,
    /// CSS transform scale Y (×1000 fixed-point, 1000 = 1.0).
    pub transform_sy: i32,
    /// CSS transform origin X (px or pct*100 of box width).
    pub transform_origin_x: i32,
    pub transform_origin_x_is_percent: bool,
    /// CSS transform origin Y (px or pct*100 of box height).
    pub transform_origin_y: i32,
    pub transform_origin_y_is_percent: bool,
    /// CSS transform rotation in degrees (×100 fixed-point).
    pub transform_rotate: i32,
    /// CSS backdrop-filter blur radius (px).  0 = no effect.
    pub backdrop_filter_blur: i32,
    /// Maximum Y extent of this subtree (relative to parent origin, like `y`).
    /// Computed by `compute_subtree_bottom()` after layout.  Used by the
    /// tile rasterizer to cull entire subtrees that are outside the tile.
    pub subtree_top: i32,
    pub subtree_bottom: i32,
    /// True when this subtree contains fixed or sticky positioning that can
    /// move descendants independently of the normal parent offset chain.
    /// Visible-band culling must stay conservative in that case.
    pub subtree_has_viewport_positioned: bool,
    /// Scroll offset for overflow:auto/scroll containers (set via JS scrollTop).
    pub scroll_top: i32,
    /// Scroll offset for overflow:auto/scroll containers (set via JS scrollLeft).
    pub scroll_left: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoxType {
    Block,
    Inline,
    InlineBlock,
    Anonymous,
    LineBox,
}

/// Kind of HTML form field for interactive rendering.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FormFieldKind {
    TextInput,
    Password,
    Submit,
    Checkbox,
    Radio,
    Hidden,
    ButtonEl,
    Textarea,
    Range,
    Progress,
    Select,
    /// `<input type="number">` — text field + spinner buttons.
    Number,
    /// `<input type="color">` — color picker swatch.
    Color,
    /// `<input type="file">` — file picker button + label.
    File,
    /// `<input type="date">`.
    Date,
    /// `<input type="time">`.
    Time,
    /// `<input type="datetime-local">`.
    DatetimeLocal,
    /// `<input type="month">`.
    Month,
    /// `<input type="week">`.
    Week,
    /// `<meter>` element.
    Meter,
    /// `<input type="reset">` or `<button type="reset">`.
    Reset,
}

#[derive(Clone, Copy, Default)]
pub struct Edges {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

// ---------------------------------------------------------------------------
// Layout box constructors (pub(super) for sub-modules)
// ---------------------------------------------------------------------------

impl LayoutBox {
    pub(super) fn new(node_id: Option<NodeId>, box_type: BoxType) -> Self {
        LayoutBox {
            node_id,
            box_type,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            margin: Edges::default(),
            padding: Edges::default(),
            border_width: 0,
            children: Vec::new(),
            text: None,
            font_size: 16,
            bold: false,
            italic: false,
            color: 0xFF000000,
            bg_color: 0,
            accent_color: 0,
            uses_dark_color_scheme: false,
            appearance_none: false,
            border_color: 0,
            border_radius: 0,
            text_decoration: TextDeco::None,
            text_align: TextAlignVal::Left,
            link_url: None,
            list_marker: None,
            list_marker_inside: false,
            is_hr: false,
            image_src: None,
            image_width: None,
            image_height: None,
            form_field: None,
            form_placeholder: None,
            form_value: None,
            form_options: None,
            form_selected_index: -1,
            form_option_values: None,
            form_multiple: false,
            form_size: 0,
            form_disabled: false,
            form_checked: false,
            form_readonly: false,
            form_required: false,
            form_min: None,
            form_max: None,
            form_step: None,
            form_pattern: None,
            form_maxlength: -1,
            form_minlength: -1,
            form_datalist: None,
            overflow_hidden: false,
            visibility_hidden: false,
            opacity: 255,
            is_fixed: false,
            is_out_of_flow: false,
            static_position_x: None,
            static_position_y: None,
            static_position_width: None,
            static_position_height: None,
            // Per-side borders
            border_top_width: 0,
            border_right_width: 0,
            border_bottom_width: 0,
            border_left_width: 0,
            border_top_color: 0,
            border_right_color: 0,
            border_bottom_color: 0,
            border_left_color: 0,
            border_top_left_radius: 0,
            border_top_right_radius: 0,
            border_bottom_right_radius: 0,
            border_bottom_left_radius: 0,
            // Outline
            outline_width: 0,
            outline_color: 0,
            outline_offset: 0,
            // Shadows
            box_shadows: Vec::new(),
            text_shadows: Vec::new(),
            // Text overflow
            text_overflow_ellipsis: false,
            // Background image
            background_image: crate::style::BackgroundImageVal::None,
            mask_image: crate::style::BackgroundImageVal::None,
            background_size: crate::style::BackgroundSizeVal::Auto,
            background_repeat: crate::style::BackgroundRepeatVal::Repeat,
            background_clip: crate::style::BackgroundClipVal::BorderBox,
            mask_size: crate::style::BackgroundSizeVal::Auto,
            mask_repeat: crate::style::BackgroundRepeatVal::Repeat,
            mask_clip: crate::style::BackgroundClipVal::BorderBox,
            mask_origin: crate::style::BackgroundClipVal::BorderBox,
            mask_position_x: 0,
            mask_position_x_is_percent: true,
            mask_position_y: 0,
            mask_position_y_is_percent: true,
            // Letter spacing
            letter_spacing: 0,
            z_index: 0,
            z_index_auto: true,
            creates_stacking_context: false,
            border_top_style: crate::style::BorderStyleVal::None,
            border_right_style: crate::style::BorderStyleVal::None,
            border_bottom_style: crate::style::BorderStyleVal::None,
            border_left_style: crate::style::BorderStyleVal::None,
            filter: crate::style::FilterVal::none(),
            clip_path: crate::style::ClipPathVal::None,
            text_decoration_color: 0,
            text_decoration_style: crate::style::TextDecorationStyle::Solid,
            text_decoration_thickness: 0,
            text_underline_offset: 0,
            object_fit: crate::style::ObjectFit::Fill,
            object_position_x: 5000,
            object_position_x_is_percent: true,
            object_position_y: 5000,
            object_position_y_is_percent: true,
            custom_font_id: 0,
            is_sticky: false,
            sticky_top: 0,
            backdrop_filter_blur: 0,
            subtree_top: 0,
            subtree_bottom: 0,
            subtree_has_viewport_positioned: false,
            clip_rect: None,
            transform_sx: 1000,
            transform_sy: 1000,
            transform_origin_x: 5000,
            transform_origin_x_is_percent: true,
            transform_origin_y: 5000,
            transform_origin_y_is_percent: true,
            transform_rotate: 0,
            scroll_top: 0,
            scroll_left: 0,
        }
    }

    pub(super) fn new_text(
        text: String,
        font_size: i32,
        bold: bool,
        italic: bool,
        color: u32,
    ) -> Self {
        let mut b = LayoutBox::new(None, BoxType::Inline);
        b.text = Some(text);
        b.font_size = font_size;
        b.bold = bold;
        b.italic = italic;
        b.color = color;
        b
    }
}

// ---------------------------------------------------------------------------
// Shared helpers (pub(super) for sub-modules)
// ---------------------------------------------------------------------------

pub(crate) fn resolve_font_id(custom_font_id: u32, bold: bool, italic: bool) -> u32 {
    if custom_font_id != 0 {
        custom_font_id
    } else if bold {
        1
    } else if italic {
        3
    } else {
        0
    }
}

pub(super) fn measure_text(
    text: &str,
    font_size: i32,
    custom_font_id: u32,
    bold: bool,
    italic: bool,
) -> (i32, i32) {
    let font_id = resolve_font_id(custom_font_id, bold, italic);
    let (w, h) = libfont_client::measure(font_id, font_size.max(1) as u16, text);
    (w as i32, h as i32)
}

pub(super) fn font_size_px(style: &ComputedStyle) -> i32 {
    let s = style.font_size;
    if s <= 0 {
        16
    } else {
        s
    }
}

pub(super) fn is_bold(style: &ComputedStyle) -> bool {
    matches!(style.font_weight, FontWeight::Bold)
}

pub(super) fn is_italic(style: &ComputedStyle) -> bool {
    matches!(style.font_style, FontStyleVal::Italic)
}

pub(super) fn edges_from(top: i32, right: i32, bottom: i32, left: i32) -> Edges {
    Edges {
        top,
        right,
        bottom,
        left,
    }
}

pub(super) fn link_href(dom: &Dom, node_id: NodeId) -> Option<String> {
    if dom.tag(node_id) == Some(Tag::A) {
        dom.attr(node_id, "href").map(|s| String::from(s))
    } else {
        None
    }
}

pub(super) fn inherited_link(dom: &Dom, node_id: NodeId) -> Option<String> {
    let mut cur = Some(node_id);
    while let Some(id) = cur {
        if let Some(href) = link_href(dom, id) {
            return Some(href);
        }
        cur = dom.get(id).parent;
    }
    None
}

/// Returns `(marker_string, inside)` for a `<li>` node.
/// `inside` is true when `list-style-position: inside` is set.
pub(super) fn list_marker_for(dom: &Dom, node_id: NodeId, style: &ComputedStyle) -> Option<String> {
    if dom.tag(node_id) != Some(Tag::Li) {
        return None;
    }
    let inside = style.list_style_position == ListStylePosition::Inside;
    // Suffix: space after the marker (wider gap for inside to separate from text)
    let suffix = if inside { " " } else { " " };
    match style.list_style {
        ListStyle::Disc => Some(concat_str("\u{2022}", suffix)), // • BULLET
        ListStyle::Circle => Some(concat_str("\u{25CB}", suffix)), // ○ WHITE CIRCLE
        ListStyle::Square => Some(concat_str("\u{25A0}", suffix)), // ■ BLACK SQUARE
        ListStyle::Decimal
        | ListStyle::LowerAlpha
        | ListStyle::UpperAlpha
        | ListStyle::LowerLatin
        | ListStyle::UpperLatin
        | ListStyle::LowerRoman
        | ListStyle::UpperRoman => {
            let idx = li_index(dom, node_id);
            let mut s = String::new();
            match style.list_style {
                ListStyle::LowerAlpha | ListStyle::LowerLatin => format_alpha(&mut s, idx, false),
                ListStyle::UpperAlpha | ListStyle::UpperLatin => format_alpha(&mut s, idx, true),
                ListStyle::LowerRoman => format_roman(&mut s, idx, false),
                ListStyle::UpperRoman => format_roman(&mut s, idx, true),
                _ => format_decimal(&mut s, idx),
            }
            s.push('.');
            s.push_str(suffix);
            Some(s)
        }
        ListStyle::None => None,
    }
}

fn concat_str(a: &str, b: &str) -> String {
    let mut s = String::new();
    s.push_str(a);
    s.push_str(b);
    s
}

/// Count this `<li>`'s 1-based index among its `<li>` siblings.
fn li_index(dom: &Dom, node_id: NodeId) -> u32 {
    let Some(parent) = dom.get(node_id).parent else {
        return 1;
    };
    // Honour the `start` attribute on the parent <ol>.
    let start: u32 = dom
        .attr(parent, "start")
        .and_then(|s| s.parse::<i32>().ok())
        .map(|v| v.max(1) as u32)
        .unwrap_or(1);
    let mut idx = start;
    for &sib in &dom.get(parent).children {
        if sib == node_id {
            break;
        }
        if dom.tag(sib) == Some(Tag::Li) {
            idx += 1;
        }
    }
    idx
}

fn format_decimal(out: &mut String, mut n: u32) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        out.push(buf[i] as char);
    }
}

/// a–z, then aa–az, ba–bz, … (CSS `lower-alpha` / `lower-latin`)
fn format_alpha(out: &mut String, n: u32, upper: bool) {
    if n == 0 {
        out.push(if upper { 'A' } else { 'a' });
        return;
    }
    let base: u32 = 26;
    // Convert to bijective base-26 (1=a, 26=z, 27=aa, …)
    let mut chars: [u8; 8] = [0; 8];
    let mut len = 0;
    let mut v = n;
    while v > 0 {
        v -= 1;
        chars[len] = (v % base) as u8;
        v /= base;
        len += 1;
    }
    for i in (0..len).rev() {
        let c = chars[i] + if upper { b'A' } else { b'a' };
        out.push(c as char);
    }
}

/// Standard roman numerals (I–MMMCMXCIX = 1–3999); fallback to decimal beyond that.
fn format_roman(out: &mut String, n: u32, upper: bool) {
    if n == 0 || n > 3999 {
        format_decimal(out, n);
        return;
    }
    const VALS: &[u32] = &[1000, 900, 500, 400, 100, 90, 50, 40, 10, 9, 5, 4, 1];
    const SYMS: &[&str] = &[
        "M", "CM", "D", "CD", "C", "XC", "L", "XL", "X", "IX", "V", "IV", "I",
    ];
    let mut v = n;
    for (&val, &sym) in VALS.iter().zip(SYMS.iter()) {
        while v >= val {
            if upper {
                out.push_str(sym);
            } else {
                for c in sym.chars() {
                    out.push(c.to_ascii_lowercase());
                }
            }
            v -= val;
        }
    }
}

/// Generate the synthetic image-cache key for an inline `<svg>` node.
///
/// Must match the key produced by `surf::resources::inline_svg_key`.
pub(crate) fn svg_inline_key(node_id: NodeId) -> String {
    let mut s = String::from("__svg_");
    let mut n = node_id;
    if n == 0 {
        s.push('0');
    } else {
        let mut buf = [0u8; 20];
        let mut pos = 20usize;
        while n > 0 {
            pos -= 1;
            buf[pos] = b'0' + (n % 10) as u8;
            n /= 10;
        }
        for &b in &buf[pos..] {
            s.push(b as char);
        }
    }
    s.push_str("__");
    s
}

pub(super) fn image_dimensions(
    dom: &Dom,
    node_id: NodeId,
    max_width: i32,
    images: &ImageCache,
) -> (i32, i32) {
    // Get natural dimensions from image cache (actual decoded image size).
    let src = dom.image_url(node_id);
    let natural = src
        .as_deref()
        .and_then(|s| images.get_ref(s))
        .map(|e| (e.width.min(65535) as i32, e.height.min(65535) as i32));

    // HTML attributes override natural size; fall back to natural; then 300x150.
    let w = dom
        .attr(node_id, "width")
        .and_then(parse_attr_int)
        .or(natural.map(|(w, _)| w))
        .unwrap_or(300);
    let h = dom
        .attr(node_id, "height")
        .and_then(parse_attr_int)
        .or(natural.map(|(_, h)| h))
        .unwrap_or(150);

    // Scale down proportionally if wider than container.
    if w > max_width && max_width > 0 && w > 0 {
        let scaled_h = (h as i64 * max_width as i64 / w as i64) as i32;
        (max_width, scaled_h.max(1))
    } else {
        (w, h)
    }
}

/// Parse a positive integer from an HTML attribute value.
///
/// Uses saturating arithmetic to prevent overflow; caps the result at
/// 65535 to keep layout dimensions safely within i32 range even after
/// subsequent multiplications (e.g. `cols * 8`).
pub(super) fn parse_attr_int(s: &str) -> Option<i32> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut val: i32 = 0;
    for &b in bytes {
        if b.is_ascii_digit() {
            val = val.saturating_mul(10).saturating_add((b - b'0') as i32);
            if val > 65535 {
                val = 65535;
                break;
            }
        } else {
            break;
        }
    }
    if val > 0 {
        Some(val)
    } else {
        None
    }
}

pub(super) fn is_ascii_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

pub(super) fn ascii_lower_str<'a>(s: &str, buf: &'a mut [u8; 16]) -> &'a str {
    let len = s.len().min(16);
    for i in 0..len {
        let b = s.as_bytes()[i];
        buf[i] = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
    }
    core::str::from_utf8(&buf[..len]).unwrap_or("")
}

pub(super) fn size_attr_width(dom: &Dom, node_id: NodeId, default: i32) -> i32 {
    if let Some(size) = dom.attr(node_id, "size") {
        if let Some(s) = parse_attr_int(size) {
            return (s * 8).max(40).min(600);
        }
    }
    default
}

// ---------------------------------------------------------------------------
// CSS Counter resolution
// ---------------------------------------------------------------------------

/// Resolved counter values for each DOM node.
/// `values[node_id]` is a Vec of `(counter_name, value)` pairs giving the
/// counter values that are "current" when that node's content is rendered.
pub struct CounterValues {
    pub per_node: Vec<Vec<(String, i32)>>,
}

impl CounterValues {
    pub fn empty(count: usize) -> Self {
        let mut per_node = Vec::with_capacity(count);
        for _ in 0..count {
            per_node.push(Vec::new());
        }
        CounterValues { per_node }
    }

    pub fn get(&self, node_id: NodeId, name: &str) -> i32 {
        if node_id >= self.per_node.len() {
            return 0;
        }
        for &(ref n, v) in self.per_node[node_id].iter().rev() {
            if n.as_str() == name {
                return v;
            }
        }
        0
    }
}

/// Walk the DOM in document order (pre-order) to compute CSS counter values.
/// Each node's counter values reflect the state AFTER counter-reset / counter-increment
/// of that node have been applied.
pub fn compute_counter_values(dom: &Dom, styles: &[ComputedStyle]) -> CounterValues {
    compute_counter_values_budgeted(dom, styles, None)
}

pub fn compute_counter_values_budgeted(
    dom: &Dom,
    styles: &[ComputedStyle],
    node_limit: Option<usize>,
) -> CounterValues {
    let n = dom.nodes.len();
    let tracked = node_limit.unwrap_or(n).min(n);
    let mut cv = CounterValues::empty(tracked);
    // Current counter state: list of (name, value). Most recent entry wins.
    let mut state: Vec<(String, i32)> = Vec::new();
    walk_counters(dom, styles, 0, &mut state, &mut cv, node_limit);
    cv
}

fn walk_counters(
    dom: &Dom,
    styles: &[ComputedStyle],
    node_id: NodeId,
    state: &mut Vec<(String, i32)>,
    cv: &mut CounterValues,
    node_limit: Option<usize>,
) {
    if node_limit.map(|limit| node_id >= limit).unwrap_or(false) {
        return;
    }
    if node_id < styles.len() {
        let style = &styles[node_id];

        // counter-reset: creates/resets counters in current scope
        if let Some(ref cr) = style.counter_reset {
            // Format: "name1 [value1] name2 [value2] ..."
            let parts: Vec<&str> = cr.split_whitespace().collect();
            let mut i = 0;
            while i < parts.len() {
                let name = parts[i].to_ascii_lowercase();
                let val = if i + 1 < parts.len() {
                    parts[i + 1].parse::<i32>().unwrap_or(0)
                } else {
                    0
                };
                // Find and update existing entry, or add new one
                let mut found = false;
                for entry in state.iter_mut().rev() {
                    if entry.0 == name {
                        entry.1 = val;
                        found = true;
                        break;
                    }
                }
                if !found {
                    state.push((name, val));
                }
                i += if i + 1 < parts.len() && parts[i + 1].parse::<i32>().is_ok() {
                    2
                } else {
                    1
                };
            }
        }

        // counter-increment: increments counters
        if let Some(ref ci) = style.counter_increment {
            let parts: Vec<&str> = ci.split_whitespace().collect();
            let mut i = 0;
            while i < parts.len() {
                let name = parts[i].to_ascii_lowercase();
                let inc = if i + 1 < parts.len() {
                    parts[i + 1].parse::<i32>().unwrap_or(1)
                } else {
                    1
                };
                let mut found = false;
                for entry in state.iter_mut().rev() {
                    if entry.0 == name {
                        entry.1 += inc;
                        found = true;
                        break;
                    }
                }
                if !found {
                    state.push((name, inc));
                }
                i += if i + 1 < parts.len() && parts[i + 1].parse::<i32>().is_ok() {
                    2
                } else {
                    1
                };
            }
        }

        // Record current state snapshot for this node
        if node_id < cv.per_node.len() {
            cv.per_node[node_id] = state.clone();
        }
    }

    let children: Vec<NodeId> = dom.get(node_id).children.iter().copied().collect();
    for child in children {
        walk_counters(dom, styles, child, state, cv, node_limit);
    }
}

/// Resolve `\x01COUNTER:name\x01` markers in a content string using counter values.
pub fn resolve_counters_in_content(text: &str, node_id: NodeId, cv: &CounterValues) -> String {
    if !text.contains('\x01') {
        return String::from(text);
    }
    let mut out = String::with_capacity(text.len() + 4);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\x01' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != b'\x01' {
                i += 1;
            }
            let marker = core::str::from_utf8(&bytes[start..i]).unwrap_or("");
            if let Some(name) = marker.strip_prefix("COUNTER:") {
                let val = cv.get(node_id, name);
                // Format as decimal
                let mut num = val;
                if num == 0 {
                    out.push('0');
                } else {
                    let neg = num < 0;
                    if neg {
                        num = -num;
                    }
                    let mut digits = [0u8; 12];
                    let mut d = 0;
                    while num > 0 {
                        digits[d] = (num % 10) as u8 + b'0';
                        num /= 10;
                        d += 1;
                    }
                    if neg {
                        out.push('-');
                    }
                    for k in (0..d).rev() {
                        out.push(digits[k] as char);
                    }
                }
            }
            if i < bytes.len() {
                i += 1;
            } // skip closing \x01
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Build a layout tree from the DOM and computed styles.
pub fn layout(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &mut PseudoStyles,
    viewport_width: i32,
    viewport_height: i32,
    images: &ImageCache,
) -> LayoutBox {
    layout_with_budget(
        dom,
        styles,
        pseudo,
        viewport_width,
        viewport_height,
        images,
        None,
        None,
    )
}

/// Build a layout tree from the DOM and computed styles, optionally stopping
/// normal-flow traversal once the initial document Y exceeds `layout_budget_bottom`.
pub fn layout_with_budget(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &mut PseudoStyles,
    viewport_width: i32,
    viewport_height: i32,
    images: &ImageCache,
    layout_budget_bottom: Option<i32>,
    style_budget_nodes: Option<usize>,
) -> LayoutBox {
    crate::debug_surf!(
        "[layout] layout start: {} nodes, viewport_width={}",
        dom.nodes.len(),
        viewport_width
    );
    #[cfg(feature = "debug_surf")]
    crate::debug_surf!(
        "[layout]   RSP=0x{:X} heap=0x{:X}",
        crate::debug_rsp(),
        crate::debug_heap_pos()
    );

    // Counter()/pseudo-content resolution is surprisingly expensive on large
    // pages because it snapshots counter state per node. For the staged first
    // render we intentionally skip this pre-pass and accept that some list
    // markers/generated content may finalize a little later once the deferred
    // full layout catches up.
    if style_budget_nodes.is_none() {
        let cv = compute_counter_values_budgeted(dom, styles, style_budget_nodes);
        let n = pseudo.before.len();
        for id in 0..n {
            if let Some(ref mut ps) = pseudo.before[id] {
                if let Some(ref text) = ps.content.clone() {
                    if text.contains('\x01') {
                        ps.content = Some(resolve_counters_in_content(text, id, &cv));
                    }
                }
            }
            if let Some(ref mut ps) = pseudo.after[id] {
                if let Some(ref text) = ps.content.clone() {
                    if text.contains('\x01') {
                        ps.content = Some(resolve_counters_in_content(text, id, &cv));
                    }
                }
            }
        }
    }

    let body_id = dom.find_body().unwrap_or(0);
    let style = &styles[body_id];

    let mut root = LayoutBox::new(Some(body_id), BoxType::Block);
    // Body width: explicit width if set, else viewport width.
    root.width = if let Some(w) = style.width {
        w
    } else if let Some(pct) = style.width_pct {
        (viewport_width as i64 * pct as i64 / 10000) as i32
    } else {
        viewport_width
    };
    root.bg_color = style.background_color;
    root.mask_image = style.mask_image.clone();
    root.background_clip = style.background_clip;
    root.mask_size = style.mask_size;
    root.mask_repeat = style.mask_repeat;
    root.mask_clip = style.mask_clip;
    root.mask_origin = style.mask_origin;
    root.mask_position_x = style.mask_position_x;
    root.mask_position_x_is_percent = style.mask_position_x_is_percent;
    root.mask_position_y = style.mask_position_y;
    root.mask_position_y_is_percent = style.mask_position_y_is_percent;
    root.color = style.color;
    root.accent_color = style.accent_color;
    root.uses_dark_color_scheme = style.color_scheme == crate::style::ColorSchemeVal::Dark;
    root.appearance_none = style.appearance == crate::style::AppearanceVal::None;
    root.padding = edges_from(
        style.padding_top,
        style.padding_right,
        style.padding_bottom,
        style.padding_left,
    );
    root.margin = edges_from(
        style.margin_top,
        style.margin_right,
        style.margin_bottom,
        style.margin_left,
    );

    let content_width = root.width
        - root.padding.left
        - root.padding.right
        - root.margin.left
        - root.margin.right;

    let children = &dom.get(body_id).children;
    let child_ids: Vec<NodeId> = children.iter().copied().collect();
    crate::debug_surf!(
        "[layout] body has {} direct children, content_width={}",
        child_ids.len(),
        content_width
    );

    // Dispatch to the correct layout mode based on body's display property.
    let height = if matches!(style.display, Display::Flex | Display::InlineFlex) {
        flex::layout_flex(
            dom,
            styles,
            pseudo,
            &child_ids,
            content_width,
            viewport_height,
            &mut root,
            images,
            viewport_width,
        )
    } else if matches!(style.display, Display::Grid | Display::InlineGrid) {
        grid::layout_grid(
            dom,
            styles,
            pseudo,
            &child_ids,
            content_width,
            &mut root,
            images,
            viewport_width,
            None,
            None,
            None,
            None,
        )
    } else {
        // Pass body's definite content height (or 0 if auto) so children
        // with `height: %` and `position: absolute; top:0; bottom:0` resolve correctly.
        let body_definite_h = style.height.unwrap_or(0).max(0);
        layout_children(
            dom,
            styles,
            pseudo,
            &child_ids,
            content_width,
            &mut root,
            body_id,
            images,
            viewport_width,
            viewport_height,
            body_definite_h,
            0,
            layout_budget_bottom,
        )
    };

    root.height = height + root.padding.top + root.padding.bottom;

    resolve_absolute_alignment(&mut root, styles, viewport_width, viewport_height);

    // Post-pass: compute subtree_bottom for tile rasterizer culling.
    compute_subtree_bottom(&mut root);

    crate::debug_surf!("[layout] layout done: root height={}", root.height);
    #[cfg(feature = "debug_surf")]
    crate::debug_surf!(
        "[layout]   RSP=0x{:X} heap=0x{:X}",
        crate::debug_rsp(),
        crate::debug_heap_pos()
    );
    root
}

fn self_alignment_offset(align: AlignItems, available: i32, size: i32) -> i32 {
    match align {
        AlignItems::Center => (available - size) / 2,
        AlignItems::FlexEnd => available - size,
        _ => 0,
    }
}

fn resolve_inline_alignment(
    keyword: Option<InlineAxisAlignment>,
    fallback_keyword: Option<InlineAxisAlignment>,
    explicit: Option<AlignItems>,
    fallback: AlignItems,
    direction: Direction,
) -> AlignItems {
    match keyword.or(fallback_keyword) {
        Some(InlineAxisAlignment::Start) | Some(InlineAxisAlignment::FirstBaseline) => match direction {
            Direction::Ltr => AlignItems::FlexStart,
            Direction::Rtl => AlignItems::FlexEnd,
        },
        Some(InlineAxisAlignment::End) | Some(InlineAxisAlignment::LastBaseline) => {
            match direction {
                Direction::Ltr => AlignItems::FlexEnd,
                Direction::Rtl => AlignItems::FlexStart,
            }
        }
        Some(InlineAxisAlignment::Left) => AlignItems::FlexStart,
        Some(InlineAxisAlignment::Right) => AlignItems::FlexEnd,
        Some(InlineAxisAlignment::Center) => AlignItems::Center,
        Some(InlineAxisAlignment::Stretch) => AlignItems::Stretch,
        None => explicit.unwrap_or(fallback),
    }
}

fn resolve_absolute_alignment(
    root: &mut LayoutBox,
    styles: &[ComputedStyle],
    viewport_w: i32,
    viewport_h: i32,
) {
    let cb_x = root.border_width + root.padding.left;
    let cb_y = root.border_width + root.padding.top;
    let cb_w = (root.width - root.padding.left - root.padding.right - root.border_width * 2).max(0);
    let cb_h =
        (root.height - root.padding.top - root.padding.bottom - root.border_width * 2).max(0);
    resolve_absolute_alignment_rec(
        root,
        styles,
        0,
        0,
        cb_x,
        cb_y,
        cb_w,
        cb_h,
        viewport_w,
        viewport_h,
    );
}

fn resolve_absolute_alignment_rec(
    bx: &mut LayoutBox,
    styles: &[ComputedStyle],
    abs_x: i32,
    abs_y: i32,
    cb_abs_x: i32,
    cb_abs_y: i32,
    cb_w: i32,
    cb_h: i32,
    viewport_w: i32,
    viewport_h: i32,
) {
    let mut next_cb_abs_x = cb_abs_x;
    let mut next_cb_abs_y = cb_abs_y;
    let mut next_cb_w = cb_w;
    let mut next_cb_h = cb_h;

    if let Some(node_id) = bx.node_id {
        if styles[node_id].position != Position::Static {
            next_cb_abs_x = abs_x + bx.border_width + bx.padding.left;
            next_cb_abs_y = abs_y + bx.border_width + bx.padding.top;
            next_cb_w =
                (bx.width - bx.padding.left - bx.padding.right - bx.border_width * 2).max(0);
            next_cb_h =
                (bx.height - bx.padding.top - bx.padding.bottom - bx.border_width * 2).max(0);
        }
    }

    let parent_content_abs_x = abs_x + bx.border_width + bx.padding.left;
    let parent_content_abs_y = abs_y + bx.border_width + bx.padding.top;
    let parent_border_abs_x = abs_x;
    let parent_border_abs_y = abs_y;
    let parent_border_w = bx.width;
    let parent_border_h = bx.height;
    for child in &mut bx.children {
        let mut child_abs_x = if child.is_fixed { child.x } else { abs_x + child.x };
        let mut child_abs_y = if child.is_fixed { child.y } else { abs_y + child.y };
        let static_start_abs_x = child
            .static_position_x
            .map(|x| if child.is_fixed { x } else { abs_x + x })
            .unwrap_or(parent_border_abs_x);
        let static_start_abs_y = child
            .static_position_y
            .map(|y| if child.is_fixed { y } else { abs_y + y })
            .unwrap_or(parent_content_abs_y);
        let static_size_x = child.static_position_width.unwrap_or(parent_border_w);
        let static_size_y = child
            .static_position_height
            .unwrap_or(bx.border_width.max(0) * 2);

        if !child.is_fixed && child.is_out_of_flow {
            if let Some(node_id) = child.node_id {
                let style = &styles[node_id];
                if style.position == Position::Absolute {
                    let justify = if style.justify_self_is_normal {
                        AlignItems::FlexStart
                    } else {
                        resolve_inline_alignment(
                            style.justify_self_inline,
                            None,
                            style.justify_self,
                            AlignItems::Stretch,
                            style.direction,
                        )
                    };
                    let align = if style.align_self_is_normal {
                        AlignItems::FlexStart
                    } else {
                        style.align_self.unwrap_or(AlignItems::Stretch)
                    };

                    let resolve_axis = |start_auto: bool,
                                        end_auto: bool,
                                        start: i32,
                                        end: i32,
                                        size: &mut i32,
                                        margin_start: &mut i32,
                                        margin_end: &mut i32,
                                        auto_margin_start: bool,
                                        auto_margin_end: bool,
                                        normal_start_align: AlignItems,
                                        cb_start_abs: i32,
                                        cb_size: i32,
                                        static_start_abs: i32,
                                        static_size: i32,
                                        allow_stretch: bool|
                     -> i32 {
                        if !start_auto && !end_auto {
                            let available = (cb_size - start - end).max(0);
                            if allow_stretch && normal_start_align == AlignItems::Stretch {
                                *size = (available - *margin_start - *margin_end).max(0);
                            }
                            let remaining = (available - *size - *margin_start - *margin_end).max(0);
                            if auto_margin_start && auto_margin_end {
                                *margin_start = remaining / 2;
                                *margin_end = remaining - *margin_start;
                            } else if auto_margin_start {
                                *margin_start = remaining;
                            } else if auto_margin_end {
                                *margin_end = remaining;
                            }
                            cb_start_abs + start + *margin_start
                        } else if !start_auto {
                            cb_start_abs + start + *margin_start
                        } else if !end_auto {
                            cb_start_abs + cb_size - end - *size - *margin_end
                        } else {
                            let available = static_size.max(0);
                            static_start_abs
                                + *margin_start
                                + self_alignment_offset(
                                    normal_start_align,
                                    available,
                                    *size + *margin_start + *margin_end,
                                )
                        }
                    };

                    let mut width = child.width;
                    let mut ml = child.margin.left;
                    let mut mr = child.margin.right;
                    let desired_abs_x = resolve_axis(
                        style.left_offset.is_none(),
                        style.right_offset.is_none(),
                        style.left_offset.unwrap_or(0),
                        style.right_offset.unwrap_or(0),
                        &mut width,
                        &mut ml,
                        &mut mr,
                        style.margin_left_auto,
                        style.margin_right_auto,
                        justify,
                        next_cb_abs_x,
                        next_cb_w,
                        static_start_abs_x,
                        static_size_x,
                        style.width.is_none() && style.width_pct.is_none() && style.width_calc.is_none(),
                    );
                    child.width = width;
                    child.margin.left = ml;
                    child.margin.right = mr;

                    let mut height = child.height;
                    let mut mt = child.margin.top;
                    let mut mb = child.margin.bottom;
                    let desired_abs_y = resolve_axis(
                        style.top.is_none(),
                        style.bottom_offset.is_none(),
                        style.top.unwrap_or(0),
                        style.bottom_offset.unwrap_or(0),
                        &mut height,
                        &mut mt,
                        &mut mb,
                        style.margin_top_auto,
                        style.margin_bottom_auto,
                        align,
                        next_cb_abs_y,
                        next_cb_h,
                        static_start_abs_y,
                        static_size_y,
                        style.height.is_none() && style.height_pct.is_none() && style.height_calc.is_none(),
                    );
                    child.height = height;
                    child.margin.top = mt;
                    child.margin.bottom = mb;

                    child.x = desired_abs_x - abs_x;
                    child.y = desired_abs_y - abs_y;
                    child_abs_x = desired_abs_x;
                    child_abs_y = desired_abs_y;
                }
            }
        }

        resolve_absolute_alignment_rec(
            child,
            styles,
            child_abs_x,
            child_abs_y,
            next_cb_abs_x,
            next_cb_abs_y,
            next_cb_w,
            next_cb_h,
            viewport_w,
            viewport_h,
        );
    }
}

/// Compute subtree extents for every node in the tree.
///
/// `subtree_top` / `subtree_bottom` are the minimum/maximum Y extents
/// (relative to parent, same space as `y`) of the node and all its
/// descendants. The tile rasterizer uses this to skip entire subtrees that
/// are fully above or below the visible band.
pub(crate) fn compute_subtree_bottom(bx: &mut LayoutBox) {
    // In renderer space:
    //   absolute subtree top    = abs_y + subtree_top
    //   absolute subtree bottom = abs_y + subtree_bottom
    let mut min_t = 0;
    let mut max_b = bx.height;
    let mut has_viewport_positioned = bx.is_fixed || bx.is_sticky;
    for child in &mut bx.children {
        compute_subtree_bottom(child);
        let ct = child.y + child.subtree_top;
        let cb = child.y + child.subtree_bottom;
        if ct < min_t {
            min_t = ct;
        }
        if cb > max_b {
            max_b = cb;
        }
        has_viewport_positioned |= child.subtree_has_viewport_positioned;
    }
    bx.subtree_top = min_t;
    bx.subtree_bottom = max_b;
    bx.subtree_has_viewport_positioned = has_viewport_positioned;
}

// ---------------------------------------------------------------------------
// Block flow orchestration
// ---------------------------------------------------------------------------

/// Layout a list of child node IDs within the given available width.
/// Appends resulting `LayoutBox`es to `parent.children` and returns the total
/// height consumed.
///
/// `viewport_w` is the full viewport width; required for correct `position:fixed`
/// sizing and placement (independent of the current containing block width).
pub(super) fn layout_children(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    child_ids: &[NodeId],
    available_width: i32,
    parent: &mut LayoutBox,
    _parent_node: NodeId,
    images: &ImageCache,
    viewport_w: i32,
    viewport_h: i32,
    parent_height: i32,
    abs_y: i32,
    layout_budget_bottom: Option<i32>,
) -> i32 {
    layout_children_ex_with_budget(
        dom,
        styles,
        pseudo,
        child_ids,
        available_width,
        parent,
        _parent_node,
        images,
        viewport_w,
        viewport_h,
        parent_height,
        None,
        None,
        abs_y,
        layout_budget_bottom,
    )
}

/// Like `layout_children` but also accepts optional pre-built block pseudo-element boxes
/// (from the parent's `::before` / `::after`) that must be placed INSIDE the flow.
pub(super) fn layout_children_ex_with_budget(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    child_ids: &[NodeId],
    available_width: i32,
    parent: &mut LayoutBox,
    _parent_node: NodeId,
    images: &ImageCache,
    viewport_w: i32,
    viewport_h: i32,
    parent_height: i32,
    before_block: Option<LayoutBox>,
    after_block: Option<LayoutBox>,
    abs_y: i32,
    layout_budget_bottom: Option<i32>,
) -> i32 {
    // Children start after the border and padding on the top-left.
    let bw = parent.border_width;
    let mut cursor_y: i32 = bw + parent.padding.top;
    let mut prev_margin_bottom: i32 = 0;
    let mut float_ctx = FloatContext::new(available_width);

    // Place ::before block pseudo-element at the start of the flow.
    if let Some(mut b) = before_block {
        let mt = b.margin.top;
        let mb = b.margin.bottom;
        b.x = bw + parent.padding.left + b.margin.left;
        b.y = cursor_y + mt;
        cursor_y += b.height + mt + mb;
        prev_margin_bottom = mb;
        parent.children.push(b);
    }

    // Collect absolutely/fixed-positioned children to lay out after normal flow.
    let mut deferred_abs: Vec<(NodeId, i32, i32, i32, i32)> = Vec::new();

    let mut i = 0;
    while i < child_ids.len() {
        if layout_budget_bottom
            .map(|budget_bottom| abs_y + cursor_y >= budget_bottom && !parent.children.is_empty())
            .unwrap_or(false)
        {
            break;
        }

        let cid = child_ids[i];
        let style = &styles[cid];

        if style.display == Display::None {
            i += 1;
            continue;
        }
        if dom.attr(_parent_node, "id") == Some("item") {
            crate::debug_surf!(
                "[layout:children] parent=item child={} has_tag={} display={:?}",
                cid,
                dom.tag(cid).is_some(),
                style.display
            );
        }

        // display: contents — skip the element box, promote children.
        // Exception: SVG elements must NOT promote their children, because the
        // HTML parser stores SVG inner markup as a raw Text node.  Promoting
        // that text would render it as visible characters on the page.
        if style.display == Display::Contents && dom.tag(cid) != Some(Tag::Svg) {
            let grandchildren: Vec<NodeId> = dom.get(cid).children.iter().copied().collect();
            let h = layout_children(
                dom,
                styles,
                pseudo,
                &grandchildren,
                available_width,
                parent,
                cid,
                images,
                viewport_w,
                viewport_h,
                parent_height,
                abs_y + cursor_y,
                layout_budget_bottom,
            );
            cursor_y += h;
            i += 1;
            continue;
        }

        // Skip absolute/fixed from normal flow — position them after.
        if matches!(style.position, Position::Absolute | Position::Fixed) {
            let li = float_ctx.left_intrusion_at(cursor_y, 1);
            let ri = float_ctx.right_intrusion_at(cursor_y, 1);
            let static_width = (available_width - li - ri).max(0);
            let collapsed = if prev_margin_bottom > style.margin_top {
                prev_margin_bottom
            } else {
                style.margin_top
            };
            let static_y = if cursor_y == bw + parent.padding.top {
                cursor_y + style.margin_top
            } else {
                cursor_y + collapsed - prev_margin_bottom
            };
            let static_x = bw + parent.padding.left + li;
            deferred_abs.push((cid, static_x, static_y, static_width, 0));
            i += 1;
            continue;
        }

        // Handle `clear` property — advance cursor past cleared floats.
        if style.clear != ClearVal::None {
            let clear_to = float_ctx.clear_y(style.clear);
            if clear_to > cursor_y {
                cursor_y = clear_to;
            }
        }

        let is_block = is_block_level(dom, cid, style);
        if dom.attr(_parent_node, "id") == Some("item") {
            crate::debug_surf!(
                "[layout:children] parent=item child={} is_block={}",
                cid,
                is_block
            );
        }

        if is_block {
            let float_val = style.float;

            // ── Floated elements ──
            if float_val != FloatVal::None {
                let stf_width = shrink_to_fit_width(
                    dom,
                    styles,
                    pseudo,
                    cid,
                    available_width,
                    images,
                    viewport_w,
                );
                let mut placed = if is_table_element(dom, cid) {
                    table::layout_table(dom, styles, pseudo, cid, stf_width, images, viewport_w)
                } else {
                    build_block(dom, styles, pseudo, cid, stf_width, images, viewport_w, parent_height)
                };

                let total_w = placed.width + placed.margin.left + placed.margin.right;
                let total_h = placed.height + placed.margin.top + placed.margin.bottom;
                let place_y = float_ctx.find_y_for_float(float_val, total_w, total_h, cursor_y);
                let relative_x = placed.x;
                let relative_y = placed.y;

                let li = float_ctx.left_intrusion_at(place_y, total_h);
                let ri = float_ctx.right_intrusion_at(place_y, total_h);

                if float_val == FloatVal::Left {
                    placed.x = bw + parent.padding.left + li + placed.margin.left + relative_x;
                } else {
                    let right_edge = available_width - ri;
                    placed.x =
                        bw + parent.padding.left + right_edge - placed.width - placed.margin.right
                            + relative_x;
                }
                placed.y = place_y + placed.margin.top + relative_y;

                float_ctx.add(PlacedFloat {
                    side: float_val,
                    x: placed.x - placed.margin.left,
                    y: place_y,
                    width: total_w,
                    height: total_h,
                });

                parent.children.push(placed);
                i += 1;
                continue;
            }

            // ── Normal block flow ──
            let li = float_ctx.left_intrusion_at(cursor_y, 1);
            let ri = float_ctx.right_intrusion_at(cursor_y, 1);
            let effective_avail = (available_width - li - ri).max(0);
            let parent_style = &styles[_parent_node];
            let child_style = &styles[cid];
            let parent_justify = if parent_style.justify_items_specified {
                resolve_inline_alignment(
                    parent_style.justify_items_inline,
                    None,
                    Some(parent_style.justify_items),
                    AlignItems::Stretch,
                    child_style.direction,
                )
            } else {
                AlignItems::FlexStart
            };
            let justify = if child_style.justify_self_is_normal {
                AlignItems::FlexStart
            } else {
                resolve_inline_alignment(
                    child_style.justify_self_inline,
                    parent_style.justify_items_inline,
                    child_style.justify_self,
                    parent_justify,
                    child_style.direction,
                )
            };
            let has_explicit_self_alignment = parent_style.justify_items_specified
                || child_style.justify_self_is_normal
                || child_style.justify_self.is_some()
                || child_style.justify_self_inline.is_some();
            let is_widget_like = matches!(
                dom.tag(cid),
                Some(Tag::Input | Tag::Select | Tag::Textarea | Tag::Button)
            );
            let use_fit_content_width = justify != AlignItems::Stretch
                && child_style.width.is_none()
                && child_style.width_pct.is_none()
                && child_style.width_calc.is_none()
                && !child_style.width_max_content
                && !child_style.width_min_content
                && !child_style.width_fit_content
                && (has_explicit_self_alignment || is_widget_like);
            let child_avail = if use_fit_content_width {
                shrink_to_fit_width(dom, styles, pseudo, cid, effective_avail, images, viewport_w)
            } else {
                effective_avail
            };

            let child_box = if is_table_element(dom, cid) {
                table::layout_table(
                    dom,
                    styles,
                    pseudo,
                    cid,
                    child_avail,
                    images,
                    viewport_w,
                )
            } else {
                let child_margin_top = style.margin_top;
                let collapsed = if prev_margin_bottom > child_margin_top {
                    prev_margin_bottom
                } else {
                    child_margin_top
                };
                let child_y = if cursor_y == bw + parent.padding.top {
                    cursor_y + child_margin_top
                } else {
                    cursor_y + collapsed - prev_margin_bottom
                };
                build_block_with_budget(
                    dom,
                    styles,
                    pseudo,
                    cid,
                    child_avail,
                    images,
                    viewport_w,
                    parent_height,
                    abs_y + child_y,
                    layout_budget_bottom,
                )
            };

            let collapsed = if prev_margin_bottom > child_box.margin.top {
                prev_margin_bottom
            } else {
                child_box.margin.top
            };
            let placed_y = if cursor_y == bw + parent.padding.top {
                cursor_y + child_box.margin.top
            } else {
                cursor_y + collapsed - prev_margin_bottom
            };

            let mut placed = child_box;
            let relative_x = placed.x;
            let relative_y = placed.y;
            if use_fit_content_width {
                let remaining =
                    (effective_avail - placed.width - placed.margin.left - placed.margin.right)
                        .max(0);
                if child_style.margin_left_auto && child_style.margin_right_auto {
                    placed.margin.left = remaining / 2;
                    placed.margin.right = remaining - placed.margin.left;
                } else if child_style.margin_left_auto {
                    placed.margin.left += remaining;
                } else if child_style.margin_right_auto {
                    placed.margin.right += remaining;
                }
            }
            let stretch_self = justify == AlignItems::Stretch
                && has_explicit_self_alignment
                && child_style.width.is_none()
                && child_style.width_pct.is_none()
                && child_style.width_calc.is_none()
                && !child_style.width_max_content
                && !child_style.width_min_content
                && !child_style.width_fit_content;

            if stretch_self {
                placed.width =
                    (effective_avail - placed.margin.left - placed.margin.right).max(0);
            }

            let total_child_w = placed.width + placed.margin.left + placed.margin.right;
            let justify_offset = match justify {
                AlignItems::Center => (effective_avail - total_child_w).max(0) / 2,
                AlignItems::FlexEnd => (effective_avail - total_child_w).max(0),
                _ => 0,
            };
            placed.x = bw + parent.padding.left + li + placed.margin.left + justify_offset + relative_x;

            // Keep legacy `text-align` fallback when self-alignment remains at start.
            let parent_align = parent_style.text_align;
            if justify == AlignItems::Stretch || justify == AlignItems::FlexStart {
                if parent_align == TextAlignVal::Center {
                    if total_child_w < effective_avail {
                        placed.x =
                            bw + parent.padding.left
                                + li
                                + (effective_avail - total_child_w) / 2
                                + relative_x;
                    }
                } else if parent_align == TextAlignVal::Right {
                    if total_child_w < effective_avail {
                        placed.x = bw + parent.padding.left
                            + li
                            + effective_avail
                            - total_child_w
                            + relative_x;
                    }
                }
            }

            placed.y = placed_y + relative_y;
            cursor_y = placed_y + placed.height + placed.margin.bottom;
            prev_margin_bottom = placed.margin.bottom;

            parent.children.push(placed);
            if layout_budget_bottom
                .map(|budget_bottom| abs_y + cursor_y >= budget_bottom)
                .unwrap_or(false)
            {
                break;
            }
            i += 1;
        } else {
            // ── Inline run ──
            let run_start = i;
            while i < child_ids.len() {
                let sid = child_ids[i];
                let ss = &styles[sid];
                if ss.display == Display::None {
                    i += 1;
                    continue;
                }
                if is_block_level(dom, sid, ss) {
                    break;
                }
                i += 1;
            }
            let inline_ids: Vec<NodeId> = child_ids[run_start..i].iter().copied().collect();

            // CSS §9.2.1: Whitespace-only text nodes between block-level siblings do NOT
            // generate boxes and must not advance the block cursor.
            // Check: if every node in this inline run is either display:none or a
            // whitespace-only text node, and there are no pseudo-element contributions,
            // skip the run entirely.
            {
                let has_no_pseudo = {
                    let before_ps = if _parent_node < pseudo.before.len() {
                        pseudo.before[_parent_node].as_ref()
                    } else {
                        None
                    };
                    let after_ps = if _parent_node < pseudo.after.len() {
                        pseudo.after[_parent_node].as_ref()
                    } else {
                        None
                    };
                    before_ps.is_none() && after_ps.is_none()
                };
                let all_whitespace = inline_ids.iter().all(|&nid| {
                    if styles[nid].display == Display::None {
                        return true;
                    }
                    match &dom.get(nid).node_type {
                        crate::dom::NodeType::Text(t) => {
                            t.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
                        }
                        _ => false,
                    }
                });
                if all_whitespace && has_no_pseudo {
                    prev_margin_bottom = 0;
                    continue;
                }
            }

            // Query float intrusions for inline content.
            let li = float_ctx.left_intrusion_at(cursor_y, 1);
            let ri = float_ctx.right_intrusion_at(cursor_y, 1);
            let inline_avail = (available_width - li - ri).max(0);

            let parent_style = &styles[_parent_node];
            let parent_align = parent_style.text_align;

            // Check if parent block node has inline ::before/::after pseudo-elements.
            // These are only injected into the FIRST inline run (for ::before) and LAST (for ::after).
            let is_first_run = run_start == 0
                || child_ids[..run_start]
                    .iter()
                    .all(|&id| styles[id].display == Display::None);
            let is_last_run = i >= child_ids.len()
                || child_ids[i..].iter().all(|&id| {
                    let s = &styles[id];
                    s.display == Display::None || is_block_level(dom, id, s)
                });

            let before_inline_ps = if is_first_run && _parent_node < pseudo.before.len() {
                pseudo.before[_parent_node].as_ref().filter(|ps| {
                    !matches!(
                        ps.display,
                        Display::Block
                            | Display::FlowRoot
                            | Display::InlineBlock
                            | Display::Flex
                            | Display::InlineFlex
                            | Display::Grid
                            | Display::InlineGrid
                    )
                })
            } else {
                None
            };
            let after_inline_ps = if is_last_run && _parent_node < pseudo.after.len() {
                pseudo.after[_parent_node].as_ref().filter(|ps| {
                    !matches!(
                        ps.display,
                        Display::Block
                            | Display::FlowRoot
                            | Display::InlineBlock
                            | Display::Flex
                            | Display::InlineFlex
                            | Display::Grid
                            | Display::InlineGrid
                    )
                })
            } else {
                None
            };

            let line_boxes = layout_inline_content_with_pseudo(
                dom,
                styles,
                pseudo,
                &inline_ids,
                inline_avail,
                bw + parent.padding.left + li,
                images,
                parent_align,
                parent_style.line_height,
                viewport_w,
                before_inline_ps,
                after_inline_ps,
            );
            for lb in line_boxes {
                let h = lb.height;
                let mut placed = lb;
                placed.y = cursor_y;
                cursor_y += h;
                parent.children.push(placed);
            }
            prev_margin_bottom = 0;
            if layout_budget_bottom
                .map(|budget_bottom| abs_y + cursor_y >= budget_bottom)
                .unwrap_or(false)
            {
                break;
            }
        }
    }

    // Position absolutely/fixed elements out of flow.
    for &(abs_id, static_x, static_y, static_w, static_h) in &deferred_abs {
        let abs_style = &styles[abs_id];
        let is_fixed_pos = abs_style.position == Position::Fixed;

        // Containing block size for the absolute element.
        let cb_width = if is_fixed_pos { viewport_w } else { available_width };
        let cb_height = if is_fixed_pos { viewport_h } else { parent_height };

        // CSS §10.3.7: For absolute elements with width:auto and BOTH left and right
        // specified, width = cb_width - left - right (- margins, treated as 0).
        // This pre-computes the width to pass into build_block.
        let sizing_width = if abs_style.width.is_none()
            && abs_style.width_pct.is_none()
            && abs_style.left_offset.is_some()
            && abs_style.right_offset.is_some()
        {
            let l = abs_style.left_offset.unwrap_or(0);
            let r = abs_style.right_offset.unwrap_or(0);
            (cb_width - l - r).max(0)
        } else {
            cb_width
        };

        let mut abs_box = if is_table_element(dom, abs_id) {
            table::layout_table(
                dom,
                styles,
                pseudo,
                abs_id,
                sizing_width,
                images,
                viewport_w,
            )
        } else {
            build_block(
                dom,
                styles,
                pseudo,
                abs_id,
                sizing_width,
                images,
                viewport_w,
                cb_height,
            )
        };

        // Note: height for abs elements with top+bottom is now computed inside
        // build_block (CSS §10.6.4), so children can be laid out correctly.

        if is_fixed_pos {
            // position:fixed — coordinates are viewport-relative.
            // The renderer honours `is_fixed = true` by ignoring accumulated parent offsets.
            let t = abs_style.top.unwrap_or(0);
            let l = abs_style.left_offset.unwrap_or(0);

            abs_box.x = l + abs_box.margin.left;
            abs_box.y = t + abs_box.margin.top;

            if abs_style.left_offset.is_none() {
                if let Some(r) = abs_style.right_offset {
                    abs_box.x = (viewport_w - r - abs_box.width - abs_box.margin.right).max(0);
                }
            }
            if abs_style.top.is_none() {
                if let Some(b) = abs_style.bottom_offset {
                    abs_box.y = (viewport_h - b - abs_box.height - abs_box.margin.bottom).max(0);
                }
            }

            abs_box.is_fixed = true;
            abs_box.is_out_of_flow = true;
        } else {
            // position:absolute — coordinates relative to the direct containing block (parent box).
            let t = abs_style.top.unwrap_or(0);
            let l = abs_style.left_offset.unwrap_or(0);
            let content_x = bw + parent.padding.left;
            let content_y = bw + parent.padding.top;

            abs_box.x = content_x + l + abs_box.margin.left;
            abs_box.y = content_y + t + abs_box.margin.top;

            if abs_style.left_offset.is_none() {
                if let Some(r) = abs_style.right_offset {
                    abs_box.x =
                        content_x + available_width - r - abs_box.width - abs_box.margin.right;
                }
            }
            if abs_style.top.is_none() {
                if let Some(b) = abs_style.bottom_offset {
                    abs_box.y = cursor_y - b - abs_box.height - abs_box.margin.bottom;
                }
            }
        }

        abs_box.is_out_of_flow = true;
        abs_box.static_position_x = Some(static_x);
        abs_box.static_position_y = Some(static_y);
        abs_box.static_position_width = Some(static_w);
        abs_box.static_position_height = Some(static_h);
        parent.children.push(abs_box);
    }

    // Place ::after block pseudo-element at the end of the flow.
    if let Some(mut b) = after_block {
        let mt = b.margin.top;
        let mb = b.margin.bottom;
        b.x = bw + parent.padding.left + b.margin.left;
        b.y = cursor_y + mt;
        cursor_y += b.height + mt + mb;
        parent.children.push(b);
    }

    // BFC (Block Formatting Context) containment (CSS2 §9.4.1, CSS Display §3):
    // Elements that establish a new BFC must contain their own floated children.
    // If any floats extend below cursor_y, expand cursor_y to cover them.
    let parent_is_bfc = if _parent_node < styles.len() {
        let s = &styles[_parent_node];
        // overflow: hidden/scroll/auto establishes a BFC.
        let has_overflow_bfc = !matches!(s.overflow_x, OverflowVal::Visible)
            || !matches!(s.overflow_y, OverflowVal::Visible);
        // display: flow-root / inline-block / flex / inline-flex / grid / inline-grid.
        let has_display_bfc = matches!(
            s.display,
            Display::FlowRoot
                | Display::InlineBlock
                | Display::Flex
                | Display::InlineFlex
                | Display::Grid
                | Display::InlineGrid
        );
        // Floated or absolutely positioned elements also establish a BFC.
        let has_float_bfc = s.float != crate::style::FloatVal::None;
        let has_pos_bfc = matches!(
            s.position,
            crate::style::Position::Absolute | crate::style::Position::Fixed
        );
        let has_align_content_bfc = !s.align_content_is_normal
            && !matches!(
                s.display,
                Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid
            );
        has_overflow_bfc
            || has_display_bfc
            || has_float_bfc
            || has_pos_bfc
            || has_align_content_bfc
    } else {
        false
    };

    if parent_is_bfc && !float_ctx.floats.is_empty() {
        let float_bottom = float_ctx
            .floats
            .iter()
            .map(|f| f.y + f.height)
            .max()
            .unwrap_or(0);
        if float_bottom > cursor_y {
            cursor_y = float_bottom;
        }
    }

    cursor_y
}

/// Apply text-transform to a string.
pub(super) fn apply_text_transform(text: &str, transform: TextTransform) -> String {
    match transform {
        TextTransform::None => String::from(text),
        TextTransform::Uppercase => {
            let mut out = String::with_capacity(text.len());
            for ch in text.chars() {
                for c in ch.to_uppercase() {
                    out.push(c);
                }
            }
            out
        }
        TextTransform::Lowercase => {
            let mut out = String::with_capacity(text.len());
            for ch in text.chars() {
                for c in ch.to_lowercase() {
                    out.push(c);
                }
            }
            out
        }
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(text.len());
            let mut prev_ws = true;
            for ch in text.chars() {
                if prev_ws && ch.is_alphabetic() {
                    for c in ch.to_uppercase() {
                        out.push(c);
                    }
                } else {
                    out.push(ch);
                }
                prev_ws = ch.is_whitespace();
            }
            out
        }
    }
}

/// Determine whether a node should generate a block-level box.
fn is_block_level(dom: &Dom, node_id: NodeId, style: &ComputedStyle) -> bool {
    // CSS §9.7: If `float` has a value other than `none`, the computed `display`
    // is forced to block-level (`inline`/`inline-block` → `block`).
    if style.float != crate::style::FloatVal::None
        && !matches!(style.position, crate::style::Position::Absolute | crate::style::Position::Fixed)
    {
        return true;
    }
    if matches!(
        style.display,
        Display::Block | Display::FlowRoot | Display::Flex | Display::Grid | Display::ListItem
    ) {
        return true;
    }
    if let Some(tag) = dom.tag(node_id) {
        if tag == Tag::Hr || tag == Tag::Table {
            return true;
        }
        if tag.is_block()
            && style.display != Display::Inline
            && style.display != Display::InlineFlex
            && style.display != Display::InlineBlock
            && style.display != Display::InlineGrid
        {
            return true;
        }
    }
    false
}

fn is_table_element(dom: &Dom, node_id: NodeId) -> bool {
    matches!(dom.tag(node_id), Some(Tag::Table))
}

/// Check whether `node_id` has an `<svg>` ancestor.  SVG inner markup is
/// stored as raw text by the HTML parser — it must never be rendered as
/// visible characters.  Walking the full ancestor chain (not just the
/// immediate parent) handles nested SVG elements like `<g>`, `<defs>`, etc.
pub(crate) fn is_inside_svg(dom: &Dom, node_id: NodeId) -> bool {
    let mut cur = dom.nodes.get(node_id).and_then(|n| n.parent);
    while let Some(pid) = cur {
        if dom.tag(pid) == Some(Tag::Svg) {
            return true;
        }
        cur = dom.nodes.get(pid).and_then(|n| n.parent);
    }
    false
}

// ---------------------------------------------------------------------------
// Float context — tracks placed floats for correct flow-around behaviour.
// ---------------------------------------------------------------------------

struct PlacedFloat {
    side: FloatVal, // Left or Right
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

struct FloatContext {
    floats: Vec<PlacedFloat>,
    container_width: i32,
}

impl FloatContext {
    fn new(container_width: i32) -> Self {
        FloatContext {
            floats: Vec::new(),
            container_width,
        }
    }

    /// Total width consumed by left floats overlapping the given Y band.
    fn left_intrusion_at(&self, y: i32, h: i32) -> i32 {
        let mut max_right = 0i32;
        for f in &self.floats {
            if f.side == FloatVal::Left && f.y < y + h && f.y + f.height > y {
                let right = f.x + f.width;
                if right > max_right {
                    max_right = right;
                }
            }
        }
        max_right
    }

    /// Total width consumed by right floats overlapping the given Y band.
    fn right_intrusion_at(&self, y: i32, h: i32) -> i32 {
        let mut max_left = self.container_width;
        for f in &self.floats {
            if f.side == FloatVal::Right && f.y < y + h && f.y + f.height > y {
                if f.x < max_left {
                    max_left = f.x;
                }
            }
        }
        self.container_width - max_left
    }

    /// Available horizontal space at a given Y band.
    fn available_width_at(&self, y: i32, h: i32) -> i32 {
        let li = self.left_intrusion_at(y, h);
        let ri = self.right_intrusion_at(y, h);
        (self.container_width - li - ri).max(0)
    }

    /// Y position past which all floats matching `clear` are cleared.
    fn clear_y(&self, clear: ClearVal) -> i32 {
        let mut max_bottom = 0i32;
        for f in &self.floats {
            let dominated = match clear {
                ClearVal::Left => f.side == FloatVal::Left,
                ClearVal::Right => f.side == FloatVal::Right,
                ClearVal::Both => true,
                ClearVal::None => false,
            };
            if dominated {
                let bot = f.y + f.height;
                if bot > max_bottom {
                    max_bottom = bot;
                }
            }
        }
        max_bottom
    }

    /// Find the Y position where a float of `width` can be placed.
    /// Scans downward from `start_y` in 1-px increments until there's room.
    fn find_y_for_float(&self, _side: FloatVal, width: i32, height: i32, start_y: i32) -> i32 {
        let mut y = start_y;
        loop {
            let li = self.left_intrusion_at(y, height);
            let ri = self.right_intrusion_at(y, height);
            let avail = self.container_width - li - ri;
            if avail >= width {
                return y;
            }
            y += 1;
            if y > start_y + 10000 {
                return y;
            } // safety cap
        }
    }

    fn add(&mut self, pf: PlacedFloat) {
        self.floats.push(pf);
    }
}

/// Compute max-content width (natural width with no wrapping) for a DOM node.
/// Used by `width: max-content`. Delegates to the flex measure helper.
pub(super) fn intrinsic_width(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    node_id: NodeId,
    available_width: i32,
    images: &ImageCache,
    viewport_w: i32,
) -> i32 {
    flex::measure_max_content(dom, styles, pseudo, node_id, images, viewport_w)
        .min(available_width)
        .max(0)
}

/// Compute min-content width (minimum width without breaking words) for a DOM node.
/// Used by `width: min-content`. Approximated as the longest single text word.
pub(super) fn intrinsic_min_width(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    node_id: NodeId,
    images: &ImageCache,
    viewport_w: i32,
) -> i32 {
    measure_min_content(dom, styles, pseudo, node_id, images, viewport_w)
}

/// Recursively measure the minimum content width (longest unbreakable run).
fn measure_min_content(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    node_id: NodeId,
    images: &ImageCache,
    viewport_w: i32,
) -> i32 {
    use crate::dom::NodeType;
    let st = &styles[node_id];
    if st.display == Display::None {
        return 0;
    }

    // Explicit width → that IS the min-content width.
    if let Some(w) = st.width {
        if w > 0 {
            return w;
        }
    }
    if st.width_min_content || st.width_max_content {
        return 0;
    }

    let pad_border = st.padding_left
        + st.padding_right
        + st.border_width * 2
        + st.border_left.width
        + st.border_right.width;

    if let NodeType::Text(ref text) = dom.nodes[node_id].node_type {
        if is_inside_svg(dom, node_id) {
            return 0;
        }
        // Find the longest word (non-breaking run).
        let fs = st.font_size.max(1);
        let bold = matches!(st.font_weight, crate::style::FontWeight::Bold);
        let mut max_w = 0i32;
        for word in text.split_whitespace() {
            let custom_font_id = st
                .font_family
                .as_ref()
                .and_then(|family| crate::lookup_web_font(family))
                .unwrap_or(0);
            let italic = matches!(st.font_style, crate::style::FontStyleVal::Italic);
            let (w, _) = measure_text(word, fs, custom_font_id, bold, italic);
            if w > max_w {
                max_w = w;
            }
        }
        return max_w;
    }

    // Image.
    if dom.tag(node_id) == Some(crate::dom::Tag::Img) || dom.has_tag_name(node_id, "a-img") {
        return flex::measure_max_content(dom, styles, pseudo, node_id, images, viewport_w);
    }

    // Recurse into children.
    let mut max_child_w = 0i32;
    for &cid in &dom.nodes[node_id].children {
        let cw = measure_min_content(dom, styles, pseudo, cid, images, viewport_w);
        if cw > max_child_w {
            max_child_w = cw;
        }
    }
    max_child_w + pad_border
}

/// Compute shrink-to-fit width for a float element.
/// Per CSS spec, the float's width = min(max-content, max(min-content, available)).
/// We approximate this as max-content width capped at max_width.
fn shrink_to_fit_width(
    dom: &Dom,
    styles: &[ComputedStyle],
    pseudo: &PseudoStyles,
    node_id: NodeId,
    max_width: i32,
    images: &ImageCache,
    viewport_w: i32,
) -> i32 {
    let style = &styles[node_id];
    // If explicit width is set, use it.
    if let Some(w) = style.width {
        if w > 0 {
            return w.min(max_width);
        }
    }
    // Percentage width.
    if let Some(pct) = style.width_pct {
        let w = (max_width as i64 * pct as i64 / 10000) as i32;
        if w > 0 {
            return w.min(max_width);
        }
    }
    // Otherwise, use max-content width (natural width without forced line-wrapping).
    // This is the correct CSS shrink-to-fit algorithm: it prevents block children
    // (like <figcaption>) from expanding the float to the full container width.
    let mc = flex::measure_max_content(dom, styles, pseudo, node_id, images, viewport_w);
    let pad_border = style.padding_left
        + style.padding_right
        + style.border_left.width
        + style.border_right.width
        + style.border_width * 2;
    (mc + pad_border).max(1).min(max_width)
}

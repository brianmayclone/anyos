// Grid helpers
// ---------------------------------------------------------------------------

/// Decode a `CssValue` into a list of `GridTrackSize` (for `grid-template-*`).
///
/// Single-token values such as `CssValue::Length(100, Unit::Fr)` are wrapped in
/// a one-element Vec; multi-token values arrive as `CssValue::Keyword`.
fn decode_track_list(val: &CssValue) -> Vec<GridTrackSize> {
    match val {
        CssValue::Keyword(kw) => parse_track_list(kw),
        CssValue::Auto => vec![GridTrackSize::Auto],
        CssValue::Length(v, Unit::Fr) => vec![GridTrackSize::Fr(*v)],
        CssValue::Length(v, Unit::Px) => vec![GridTrackSize::Px(v / 100)],
        CssValue::Length(v, Unit::Percent) | CssValue::Percentage(v) => {
            vec![GridTrackSize::Percent(*v)]
        }
        _ => Vec::new(),
    }
}

/// Decode a `CssValue` into a single `GridTrackSize` (for `grid-auto-*`).
fn decode_single_track(val: &CssValue) -> GridTrackSize {
    match val {
        CssValue::Keyword(kw) => parse_single_track(kw),
        CssValue::Auto => GridTrackSize::Auto,
        CssValue::Length(v, Unit::Fr) => GridTrackSize::Fr(*v),
        CssValue::Length(v, Unit::Px) => GridTrackSize::Px(v / 100),
        CssValue::Length(v, Unit::Percent) | CssValue::Percentage(v) => GridTrackSize::Percent(*v),
        _ => GridTrackSize::Auto,
    }
}

/// Parse a CSS track-list string such as `"100px 1fr auto"` or
/// `"repeat(3, 1fr)"` into a `Vec<GridTrackSize>`.
fn parse_track_list(s: &str) -> Vec<GridTrackSize> {
    let mut tracks = Vec::new();
    let s = s.trim();

    // Handle repeat(count, size) — supports numeric counts and auto-fill/auto-fit.
    if s.starts_with("repeat(") {
        let inner = s.trim_start_matches("repeat(").trim_end_matches(')');
        let mut parts = inner.splitn(2, ',');
        let count_str = parts.next().unwrap_or("1").trim();
        let size_str = parts.next().unwrap_or("auto").trim();

        // Handle auto-fill / auto-fit keywords.
        if count_str == "auto-fill" || count_str == "auto-fit" {
            let min_px = parse_minmax_min(size_str);
            let track = if count_str == "auto-fill" {
                GridTrackSize::AutoFill { min_px }
            } else {
                GridTrackSize::AutoFit { min_px }
            };
            tracks.push(track);
            return tracks;
        }

        // Numeric repeat count.
        let count: usize = count_str.parse().unwrap_or(1).max(1);
        let track = parse_single_track(size_str);
        for _ in 0..count {
            tracks.push(track.clone());
        }
        return tracks;
    }

    // Space-separated list of track sizes (respecting parentheses).
    let tokens = split_whitespace_respecting_parens(s);
    for token in &tokens {
        tracks.push(parse_single_track(token));
    }
    tracks
}

/// Split a string on whitespace, but keep parenthesized groups together.
/// E.g. "12.25rem minmax(0, 1fr)" → ["12.25rem", "minmax(0, 1fr)"]
fn split_whitespace_respecting_parens(s: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut depth: u32 = 0;
    let mut i = 0;
    // Skip leading whitespace.
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    start = i;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b' ' | b'\t' if depth == 0 => {
                if start < i {
                    tokens.push(&s[start..i]);
                }
                // Skip whitespace.
                while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                    i += 1;
                }
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        tokens.push(&s[start..]);
    }
    tokens
}

fn apply_padding_shorthand(style: &mut ComputedStyle, value: &str, parent_fs: i32, root_fs: i32) {
    let parts = split_whitespace_respecting_parens(value);
    if parts.is_empty() {
        return;
    }
    let (top, right, bottom, left) = match parts.len() {
        1 => (parts[0], parts[0], parts[0], parts[0]),
        2 => (parts[0], parts[1], parts[0], parts[1]),
        3 => (parts[0], parts[1], parts[2], parts[1]),
        _ => (parts[0], parts[1], parts[2], parts[3]),
    };
    apply_padding_side(
        &mut style.padding_top,
        &mut style.padding_top_pct,
        top,
        &Property::PaddingTop,
        parent_fs,
        root_fs,
    );
    apply_padding_side(
        &mut style.padding_right,
        &mut style.padding_right_pct,
        right,
        &Property::PaddingRight,
        parent_fs,
        root_fs,
    );
    apply_padding_side(
        &mut style.padding_bottom,
        &mut style.padding_bottom_pct,
        bottom,
        &Property::PaddingBottom,
        parent_fs,
        root_fs,
    );
    apply_padding_side(
        &mut style.padding_left,
        &mut style.padding_left_pct,
        left,
        &Property::PaddingLeft,
        parent_fs,
        root_fs,
    );
}

fn apply_padding_side(
    px_slot: &mut i32,
    pct_slot: &mut Option<i32>,
    value: &str,
    property: &Property,
    parent_fs: i32,
    root_fs: i32,
) {
    let parsed = crate::css::parse_value(property, value);
    if let CssValue::Percentage(v) = parsed {
        *pct_slot = Some(v);
    } else if let Some(px) = resolve_length(&parsed, parent_fs, root_fs) {
        *px_slot = px;
        *pct_slot = None;
    }
}

/// Extract the minimum pixel value from `minmax(300px, 1fr)` or similar.
/// Falls back to 0 if the syntax is not recognized.
fn parse_minmax_min(s: &str) -> i32 {
    let s = s.trim();
    if s.starts_with("minmax(") {
        let inner = s.trim_start_matches("minmax(").trim_end_matches(')');
        if let Some((min_str, _max_str)) = inner.split_once(',') {
            let min_str = min_str.trim();
            if let Some(px_val) = min_str.strip_suffix("px") {
                return px_val.trim().parse::<f32>().unwrap_or(0.0) as i32;
            }
            if let Some(pct_val) = min_str.strip_suffix('%') {
                // Store percentage as negative to distinguish from px.
                return -(pct_val.trim().parse::<f32>().unwrap_or(0.0) as i32);
            }
        }
    }
    // Not minmax(), try as a plain track size.
    match parse_single_track(s) {
        GridTrackSize::Px(px) => px,
        _ => 0,
    }
}

/// Parse a single track size token (`"100px"`, `"1fr"`, `"50%"`, `"auto"`,
/// `"minmax(200px, 1fr)"`).
pub(crate) fn parse_single_track(token: &str) -> GridTrackSize {
    let token = token.trim();
    if token.eq_ignore_ascii_case("subgrid") {
        return GridTrackSize::Subgrid;
    }
    if token == "auto" || token.is_empty() {
        return GridTrackSize::Auto;
    }
    // Handle minmax(min, max).
    if token.starts_with("minmax(") {
        let inner = token.trim_start_matches("minmax(").trim_end_matches(')');
        if let Some((min_str, max_str)) = inner.split_once(',') {
            let min_str = min_str.trim();
            let max_str = max_str.trim();
            // Parse min component → pixel value (0 for min-content/auto).
            let min_px = if min_str == "0" {
                0
            } else if min_str == "min-content" || min_str == "max-content" || min_str == "auto" {
                0
            } else if let Some(v) = min_str.strip_suffix("px") {
                v.parse::<f32>().map(|f| f as i32).unwrap_or(0)
            } else if let Some(v) = min_str.strip_suffix("rem") {
                v.parse::<f32>().map(|f| (f * 16.0) as i32).unwrap_or(0)
            } else {
                0
            };
            // Parse max component.
            if let Some(fr_v) = max_str.strip_suffix("fr") {
                let fr = fr_v
                    .parse::<f32>()
                    .map(|f| (f * 100.0) as i32)
                    .unwrap_or(100);
                return GridTrackSize::Minmax {
                    min_px,
                    max_px: fr,
                    max_is_fr: true,
                };
            }
            // Non-fr max: treat as a track size with a minimum floor.
            let max_track = parse_single_track(max_str);
            return match max_track {
                GridTrackSize::Px(px) => GridTrackSize::Minmax {
                    min_px,
                    max_px: px,
                    max_is_fr: false,
                },
                GridTrackSize::Auto | GridTrackSize::MaxContent => GridTrackSize::Minmax {
                    min_px,
                    max_px: -1,
                    max_is_fr: false,
                },
                other => other,
            };
        }
        return GridTrackSize::Auto;
    }
    if let Some(fr_val) = token.strip_suffix("fr") {
        if let Ok(v) = fr_val.parse::<f32>() {
            return GridTrackSize::Fr((v * 100.0) as i32);
        }
    }
    if let Some(pct_val) = token.strip_suffix('%') {
        if let Ok(v) = pct_val.parse::<f32>() {
            return GridTrackSize::Percent((v * 100.0) as i32);
        }
    }
    if let Some(px_val) = token.strip_suffix("px") {
        if let Ok(v) = px_val.parse::<f32>() {
            return GridTrackSize::Px(v as i32);
        }
    }
    if let Some(rem_val) = token.strip_suffix("rem") {
        if let Ok(v) = rem_val.parse::<f32>() {
            // 1rem = 16px (root font-size default).
            return GridTrackSize::Px((v * 16.0) as i32);
        }
    }
    if let Some(em_val) = token.strip_suffix("em") {
        if let Ok(v) = em_val.parse::<f32>() {
            // 1em ≈ 16px (approximation — grid tracks don't have font context).
            return GridTrackSize::Px((v * 16.0) as i32);
        }
    }
    // Handle fit-content(value): min(max-content, max(min-content, value))
    // Approximated as Minmax { min_px: 0, max_px: value, max_is_fr: false }.
    if token.starts_with("fit-content(") && token.ends_with(')') {
        let inner = &token["fit-content(".len()..token.len() - 1];
        let max_px = if let Some(v) = inner.trim().strip_suffix("px") {
            v.parse::<f32>().unwrap_or(0.0) as i32
        } else if let Some(v) = inner.trim().strip_suffix("rem") {
            (v.parse::<f32>().unwrap_or(0.0) * 16.0) as i32
        } else if let Some(v) = inner.trim().strip_suffix("em") {
            (v.parse::<f32>().unwrap_or(0.0) * 16.0) as i32
        } else {
            0
        };
        return GridTrackSize::Minmax {
            min_px: 0,
            max_px,
            max_is_fr: false,
        };
    }
    // Handle min-content / max-content keywords.
    if token == "min-content" {
        return GridTrackSize::MinContent;
    }
    if token == "max-content" {
        return GridTrackSize::MaxContent;
    }
    GridTrackSize::Auto
}

/// Parse a single `GridLine` from a string token (`"auto"`, `"2"`, `"span 3"`, `"areaName"`).
fn parse_grid_line(s: &str) -> GridLine {
    let s = s.trim();
    if s.is_empty() || s == "auto" {
        return GridLine::Auto;
    }
    if let Some(rest) = s.strip_prefix("span ") {
        if let Ok(n) = rest.trim().parse::<i32>() {
            return GridLine::Span(n.max(1));
        }
    }
    if let Ok(n) = s.parse::<i32>() {
        return GridLine::Index(n);
    }
    // Named grid area — store the name for resolution at layout time.
    if s.chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return GridLine::Named(String::from(s));
    }
    GridLine::Auto
}

/// Parse `"start / end"` shorthand into a pair of `GridLine` values.
fn parse_grid_line_pair(s: &str) -> (GridLine, GridLine) {
    let mut it = s.splitn(2, '/');
    let start = parse_grid_line(it.next().unwrap_or("auto"));
    let end = parse_grid_line(it.next().unwrap_or("auto"));
    (start, end)
}

/// Extract an integer from a `CssValue::Number` (fixed-point ×100).
fn try_integer(val: &CssValue) -> Option<i32> {
    if let CssValue::Number(v) = val {
        return Some(v / 100);
    }
    None
}

/// Parse an `align-items` / `justify-items` keyword into `AlignItems`.
fn parse_align_items_kw(kw: &str) -> AlignItems {
    let kw = kw.trim();
    let kw = kw
        .strip_prefix("safe ")
        .or_else(|| kw.strip_prefix("unsafe "))
        .unwrap_or(kw)
        .trim();
    match kw {
        "flex-start" | "start" | "self-start" | "left" => AlignItems::FlexStart,
        "flex-end" | "end" | "self-end" | "right" | "last baseline" => AlignItems::FlexEnd,
        "center" => AlignItems::Center,
        "baseline" | "first baseline" => AlignItems::Baseline,
        _ => AlignItems::Stretch,
    }
}

fn parse_inline_axis_alignment_kw(kw: &str) -> Option<InlineAxisAlignment> {
    let kw = kw.trim();
    let kw = kw
        .strip_prefix("safe ")
        .or_else(|| kw.strip_prefix("unsafe "))
        .unwrap_or(kw)
        .trim();
    match kw {
        "flex-start" | "start" | "self-start" => Some(InlineAxisAlignment::Start),
        "flex-end" | "end" | "self-end" => Some(InlineAxisAlignment::End),
        "left" | "legacy" => Some(InlineAxisAlignment::Left),
        "right" => Some(InlineAxisAlignment::Right),
        "center" | "anchor-center" => Some(InlineAxisAlignment::Center),
        "stretch" | "normal" => Some(InlineAxisAlignment::Stretch),
        "baseline" | "first baseline" => Some(InlineAxisAlignment::FirstBaseline),
        "last baseline" => Some(InlineAxisAlignment::LastBaseline),
        _ => None,
    }
}

fn parse_self_alignment_kw(kw: &str) -> Option<Option<AlignItems>> {
    let kw = kw.trim();
    let kw = kw
        .strip_prefix("safe ")
        .or_else(|| kw.strip_prefix("unsafe "))
        .unwrap_or(kw)
        .trim();
    match kw {
        "auto" => Some(None),
        "flex-start" | "start" | "self-start" | "left" => Some(Some(AlignItems::FlexStart)),
        "flex-end" | "end" | "self-end" | "right" | "last baseline" => {
            Some(Some(AlignItems::FlexEnd))
        }
        "center" | "anchor-center" => Some(Some(AlignItems::Center)),
        "stretch" | "normal" => Some(Some(AlignItems::Stretch)),
        "baseline" | "first baseline" => Some(Some(AlignItems::Baseline)),
        "legacy" => Some(Some(AlignItems::FlexStart)),
        _ => None,
    }
}

fn parse_place_items_inline_value(
    kw: &str,
) -> (Option<InlineAxisAlignment>, Option<InlineAxisAlignment>) {
    let mut it = kw.split_whitespace();
    let first = it.next();
    let second = it.next();
    let align = first.and_then(parse_inline_axis_alignment_kw);
    let justify = second
        .and_then(parse_inline_axis_alignment_kw)
        .or_else(|| first.and_then(parse_inline_axis_alignment_kw));
    (align, justify)
}

fn parse_place_self_inline_value(
    kw: &str,
) -> (Option<InlineAxisAlignment>, Option<InlineAxisAlignment>) {
    parse_place_items_inline_value(kw)
}

fn parse_align_content_kw(kw: &str) -> Option<AlignContent> {
    let kw = kw.trim();
    let kw = kw
        .strip_prefix("safe ")
        .or_else(|| kw.strip_prefix("unsafe "))
        .unwrap_or(kw)
        .trim();
    match kw {
        "flex-start" | "start" | "baseline" | "first baseline" => Some(AlignContent::FlexStart),
        "flex-end" | "end" | "last baseline" => Some(AlignContent::FlexEnd),
        "center" | "anchor-center" => Some(AlignContent::Center),
        "space-between" => Some(AlignContent::SpaceBetween),
        "space-around" => Some(AlignContent::SpaceAround),
        "space-evenly" => Some(AlignContent::SpaceEvenly),
        "stretch" | "normal" => Some(AlignContent::Stretch),
        _ => None,
    }
}

fn parse_justify_content_kw(kw: &str) -> Option<JustifyContent> {
    let kw = kw.trim();
    let kw = kw
        .strip_prefix("safe ")
        .or_else(|| kw.strip_prefix("unsafe "))
        .unwrap_or(kw)
        .trim();
    match kw {
        "flex-start" | "start" | "left" => Some(JustifyContent::FlexStart),
        "flex-end" | "end" | "right" => Some(JustifyContent::FlexEnd),
        "center" | "anchor-center" => Some(JustifyContent::Center),
        "space-between" => Some(JustifyContent::SpaceBetween),
        "space-around" => Some(JustifyContent::SpaceAround),
        "space-evenly" => Some(JustifyContent::SpaceEvenly),
        _ => None,
    }
}

fn parse_place_items_value(kw: &str) -> (AlignItems, AlignItems) {
    let mut parts = kw.split_whitespace();
    let first = parts.next().unwrap_or("stretch");
    let second = parts.next().unwrap_or(first);
    (parse_align_items_kw(first), parse_align_items_kw(second))
}

fn parse_place_self_value(kw: &str) -> (Option<AlignItems>, Option<AlignItems>) {
    let mut parts = kw.split_whitespace();
    let first = parts.next().unwrap_or("auto");
    let second = parts.next().unwrap_or(first);
    (
        parse_self_alignment_kw(first).unwrap_or(None),
        parse_self_alignment_kw(second).unwrap_or(None),
    )
}

fn parse_place_content_value(kw: &str) -> (AlignContent, JustifyContent) {
    let mut parts = kw.split_whitespace();
    let first = parts.next().unwrap_or("stretch");
    let second = parts.next().unwrap_or(first);
    (
        parse_align_content_kw(first).unwrap_or(AlignContent::Stretch),
        parse_justify_content_kw(second).unwrap_or(JustifyContent::FlexStart),
    )
}

fn parse_overflow_keyword(kw: &str) -> OverflowVal {
    match kw {
        "visible" => OverflowVal::Visible,
        "hidden" => OverflowVal::Hidden,
        "scroll" => OverflowVal::Scroll,
        "auto" => OverflowVal::Auto,
        _ => OverflowVal::Visible,
    }
}

// ---------------------------------------------------------------------------

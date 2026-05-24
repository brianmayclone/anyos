// Declaration application
// ---------------------------------------------------------------------------

fn apply_inset_side(
    value: &CssValue,
    offset: &mut Option<i32>,
    calc: &mut Option<(i32, i32)>,
    parent_fs: i32,
    root_fs: i32,
) {
    if matches!(value, CssValue::Auto) {
        *offset = None;
        *calc = None;
    } else if let CssValue::Calc(px, pct) = value {
        *offset = if *pct == 0 { Some(px / 100) } else { None };
        *calc = Some((*px, *pct));
    } else if let CssValue::Percentage(v) = value {
        *offset = None;
        *calc = Some((0, *v));
    } else if let Some(px) = resolve_length(value, parent_fs, root_fs) {
        *offset = Some(px);
        *calc = None;
    }
}

/// Resolve a CSS length value to pixels.
///
/// `CssValue::Length` stores fixed-point * 100: "16px" -> Length(1600, Px),
/// "1.5em" -> Length(150, Em), "2rem" -> Length(200, Rem).
///
/// Conversion formulas (v = stored value):
///   Px:  pixels = v / 100
///   Em:  pixels = v * parent_fs / 100
///   Rem: pixels = v * root_fs / 100
///   Pt:  pixels = v * 4 / 300   (1pt ~= 1.333px)
pub fn apply_declaration(
    style: &mut ComputedStyle,
    decl: &Declaration,
    parent_style: Option<&ComputedStyle>,
    parent_fs: i32,
    root_fs: i32,
) {
    match decl.property {
        Property::Display => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.display = parent.display;
                }
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.display = match kw.as_str() {
                    "block" => Display::Block,
                    "inline" => Display::Inline,
                    "inline-block" => Display::InlineBlock,
                    "list-item" => Display::ListItem,
                    "table" => Display::Block,
                    "inline-table" => Display::InlineBlock,
                    "table-row" => Display::TableRow,
                    "table-cell" => Display::TableCell,
                    "flex" => Display::Flex,
                    "inline-flex" => Display::InlineFlex,
                    "grid" => Display::Grid,
                    "inline-grid" => Display::InlineGrid,
                    "flow-root" => Display::FlowRoot,
                    "none" => Display::None,
                    "contents" => Display::Contents,
                    _ => style.display,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.display = Display::None;
            }
        }
        Property::Color => match decl.value {
            CssValue::Color(c) => {
                style.color = c;
            }
            CssValue::CurrentColor => {}
            CssValue::Inherit => {
                if let Some(parent) = parent_style {
                    style.color = parent.color;
                }
            }
            _ => {}
        },
        Property::BackgroundColor | Property::Background => match decl.value {
            CssValue::Color(c) => {
                style.background_color = c;
                style.background_color_is_current = false;
            }
            CssValue::None => {
                style.background_color = 0x00000000;
                style.background_color_is_current = false;
            }
            CssValue::CurrentColor => {
                style.background_color_is_current = true;
                style.background_color = style.color;
            }
            CssValue::Inherit => {
                if let Some(parent) = parent_style {
                    style.background_color = parent.background_color;
                    style.background_color_is_current = parent.background_color_is_current;
                }
            }
            _ => {}
        },
        Property::AccentColor => match decl.value {
            CssValue::Color(c) => {
                style.accent_color = c;
            }
            CssValue::CurrentColor => {
                style.accent_color = style.color;
            }
            CssValue::Auto | CssValue::None => {
                style.accent_color = 0;
            }
            _ => {}
        },
        Property::FontSize => {
            if let CssValue::Percentage(v) = decl.value {
                let px = (parent_fs as i64 * v as i64 / 10000) as i32;
                if px > 0 {
                    style.font_size = px;
                }
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                if px > 0 {
                    style.font_size = px;
                }
            }
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_size = match kw.as_str() {
                    "xx-small" => 9,
                    "x-small" => 10,
                    "small" => 13,
                    "medium" => 16,
                    "large" => 18,
                    "x-large" => 24,
                    "xx-large" => 32,
                    "smaller" => (parent_fs * 5 + 3) / 6, // ~0.833x
                    "larger" => (parent_fs * 6 + 2) / 5,  // ~1.2x
                    _ => style.font_size,
                };
            }
        }
        Property::FontWeight => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_weight = match kw.as_str() {
                    "bold" | "bolder" => FontWeight::Bold,
                    "normal" | "lighter" => FontWeight::Normal,
                    _ => style.font_weight,
                };
            }
            if let CssValue::Number(v) = decl.value {
                style.font_weight = if v / 100 >= 700 {
                    FontWeight::Bold
                } else {
                    FontWeight::Normal
                };
            }
        }
        Property::FontStyle => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_style = match kw.as_str() {
                    "italic" | "oblique" => FontStyleVal::Italic,
                    _ => FontStyleVal::Normal,
                };
            }
        }
        Property::Direction => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.direction = match kw.as_str() {
                    "rtl" => Direction::Rtl,
                    _ => Direction::Ltr,
                };
            }
        }
        Property::WritingMode => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.writing_mode = match kw.as_str() {
                    "vertical-lr" => WritingMode::VerticalLr,
                    "vertical-rl" => WritingMode::VerticalRl,
                    "sideways-lr" => WritingMode::SidewaysLr,
                    "sideways-rl" => WritingMode::SidewaysRl,
                    _ => WritingMode::HorizontalTb,
                };
            }
        }
        Property::TextAlign => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_align = match kw.as_str() {
                    "center" => TextAlignVal::Center,
                    "right" => TextAlignVal::Right,
                    "end" => {
                        if style.direction == Direction::Rtl {
                            TextAlignVal::Left
                        } else {
                            TextAlignVal::Right
                        }
                    }
                    "justify" => TextAlignVal::Justify,
                    "start" | "match-parent" => {
                        if style.direction == Direction::Rtl {
                            TextAlignVal::Right
                        } else {
                            TextAlignVal::Left
                        }
                    }
                    _ => TextAlignVal::Left,
                };
            } else if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.text_align = parent.text_align;
                }
            }
        }
        Property::TextDecoration => match decl.value {
            CssValue::Keyword(ref kw) => {
                style.text_decoration = match kw.as_str() {
                    "underline" => TextDeco::Underline,
                    "line-through" => TextDeco::LineThrough,
                    "overline" => TextDeco::Overline,
                    "none" => TextDeco::None,
                    _ => style.text_decoration,
                };
            }
            CssValue::None => {
                style.text_decoration = TextDeco::None;
            }
            CssValue::Inherit => {
                if let Some(parent) = parent_style {
                    style.text_decoration = parent.text_decoration;
                }
            }
            _ => {}
        },
        Property::LineHeight => {
            // line-height: <number> means multiple of font_size (not pixels).
            if let CssValue::Number(v) = decl.value {
                // v is fixed-point * 100, e.g. "1.5" -> 150
                style.line_height = (style.font_size * v) / 100;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.line_height = px;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                if kw == "normal" {
                    style.line_height = (style.font_size * 6 + 2) / 5;
                }
            } else if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.line_height = parent.line_height;
                }
            }
        }
        Property::Width => {
            // Clear all width variants first.
            style.width_max_content = false;
            style.width_min_content = false;
            style.width_fit_content = false;
            match decl.value {
                CssValue::Auto => {
                    style.width = Option::None;
                    style.width_pct = Option::None;
                    style.width_calc = Option::None;
                }
                CssValue::Percentage(v) => {
                    style.width_pct = Some(v);
                    style.width = Option::None;
                    style.width_calc = Option::None;
                }
                CssValue::Calc(px, pct) => {
                    style.width_calc = Some((px, pct));
                    style.width = Option::None;
                    style.width_pct = Option::None;
                }
                CssValue::Keyword(ref kw) => match kw.as_str() {
                    "max-content" | "-webkit-max-content" | "-moz-max-content" => {
                        style.width_max_content = true;
                        style.width = Option::None;
                        style.width_pct = Option::None;
                        style.width_calc = Option::None;
                    }
                    "min-content" | "-webkit-min-content" | "-moz-min-content" => {
                        style.width_min_content = true;
                        style.width = Option::None;
                        style.width_pct = Option::None;
                        style.width_calc = Option::None;
                    }
                    "fit-content" | "-webkit-fit-content" | "-moz-fit-content" => {
                        style.width_fit_content = true;
                        style.width = Option::None;
                        style.width_pct = Option::None;
                        style.width_calc = Option::None;
                    }
                    _ => {
                        if let Some(px) = resolve_length(&decl.value, style.font_size, root_fs) {
                            style.width = Some(px);
                            style.width_pct = Option::None;
                            style.width_calc = Option::None;
                        }
                    }
                },
                _ => {
                    if let Some(px) = resolve_length(&decl.value, style.font_size, root_fs) {
                        style.width = Some(px);
                        style.width_pct = Option::None;
                        style.width_calc = Option::None;
                    }
                }
            }
        }
        Property::Height => match decl.value {
            CssValue::Auto => {
                style.height = Option::None;
                style.height_pct = Option::None;
                style.height_calc = Option::None;
            }
            CssValue::Percentage(v) => {
                style.height_pct = Some(v);
                style.height = Option::None;
                style.height_calc = Option::None;
            }
            CssValue::Calc(px, pct) => {
                style.height_calc = Some((px, pct));
                style.height = Option::None;
                style.height_pct = Option::None;
            }
            _ => {
                if let Some(px) = resolve_length(&decl.value, style.font_size, root_fs) {
                    style.height = Some(px);
                    style.height_pct = Option::None;
                    style.height_calc = Option::None;
                }
            }
        },
        Property::MaxWidth => {
            match decl.value {
                CssValue::None => {
                    style.max_width = Option::None;
                    style.max_width_calc = Option::None;
                }
                CssValue::Percentage(v) => {
                    // Store percentage as negative marker; layout resolves against container.
                    style.max_width = Some(-(v.max(1)));
                    style.max_width_calc = Option::None;
                }
                CssValue::Calc(px, pct) => {
                    style.max_width = Option::None;
                    style.max_width_calc = Some((px, pct));
                }
                _ => {
                    if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                        style.max_width = Some(px);
                        style.max_width_calc = Option::None;
                    }
                }
            }
        }
        Property::MinWidth => {
            if let CssValue::Percentage(v) = decl.value {
                style.min_width = -(v.max(1));
                style.min_width_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                style.min_width = 0;
                style.min_width_calc = Some((px, pct));
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.min_width = px;
                style.min_width_calc = Option::None;
            }
        }
        Property::MaxHeight => match decl.value {
            CssValue::None => {
                style.max_height = Option::None;
                style.max_height_calc = Option::None;
            }
            CssValue::Calc(px, pct) => {
                style.max_height = Option::None;
                style.max_height_calc = Some((px, pct));
            }
            _ => {
                if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                    style.max_height = Some(px);
                    style.max_height_calc = Option::None;
                }
            }
        },
        Property::MinHeight => {
            if let CssValue::Calc(px, pct) = decl.value {
                style.min_height = 0;
                style.min_height_calc = Some((px, pct));
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.min_height = px;
                style.min_height_calc = Option::None;
            }
        }
        // Margin properties — track `auto` for centering.
        Property::Margin => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_top_auto = true;
                style.margin_left_auto = true;
                style.margin_bottom_auto = true;
                style.margin_right_auto = true;
                style.margin_top_calc = Option::None;
                style.margin_right_calc = Option::None;
                style.margin_bottom_calc = Option::None;
                style.margin_left_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                let calc = Some((px, pct));
                style.margin_top = 0;
                style.margin_right = 0;
                style.margin_bottom = 0;
                style.margin_left = 0;
                style.margin_top_calc = calc;
                style.margin_right_calc = calc;
                style.margin_bottom_calc = calc;
                style.margin_left_calc = calc;
                style.margin_top_auto = false;
                style.margin_left_auto = false;
                style.margin_bottom_auto = false;
                style.margin_right_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_top = px;
                style.margin_right = px;
                style.margin_bottom = px;
                style.margin_left = px;
                style.margin_top_calc = Option::None;
                style.margin_right_calc = Option::None;
                style.margin_bottom_calc = Option::None;
                style.margin_left_calc = Option::None;
                style.margin_top_auto = false;
                style.margin_left_auto = false;
                style.margin_bottom_auto = false;
                style.margin_right_auto = false;
            }
        }
        Property::MarginTop => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_top_auto = true;
                style.margin_top_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                style.margin_top = 0;
                style.margin_top_calc = Some((px, pct));
                style.margin_top_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_top = px;
                style.margin_top_calc = Option::None;
                style.margin_top_auto = false;
            }
        }
        Property::MarginRight => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_right_auto = true;
                style.margin_right_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                style.margin_right = 0;
                style.margin_right_calc = Some((px, pct));
                style.margin_right_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_right = px;
                style.margin_right_calc = Option::None;
                style.margin_right_auto = false;
            }
        }
        Property::MarginBottom => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_bottom_auto = true;
                style.margin_bottom_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                style.margin_bottom = 0;
                style.margin_bottom_calc = Some((px, pct));
                style.margin_bottom_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_bottom = px;
                style.margin_bottom_calc = Option::None;
                style.margin_bottom_auto = false;
            }
        }
        Property::MarginLeft => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_left_auto = true;
                style.margin_left_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                style.margin_left = 0;
                style.margin_left_calc = Some((px, pct));
                style.margin_left_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_left = px;
                style.margin_left_calc = Option::None;
                style.margin_left_auto = false;
            }
        }
        // Shorthand padding.
        Property::Padding => {
            if let CssValue::Keyword(ref value) = decl.value {
                apply_padding_shorthand(style, value, parent_fs, root_fs);
            } else if let CssValue::Percentage(v) = decl.value {
                style.padding_top_pct = Some(v);
                style.padding_right_pct = Some(v);
                style.padding_bottom_pct = Some(v);
                style.padding_left_pct = Some(v);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_top = px;
                style.padding_right = px;
                style.padding_bottom = px;
                style.padding_left = px;
                style.padding_top_pct = None;
                style.padding_right_pct = None;
                style.padding_bottom_pct = None;
                style.padding_left_pct = None;
            }
        }
        Property::PaddingTop => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.padding_top = parent.padding_top;
                    style.padding_top_pct = parent.padding_top_pct;
                }
            } else if let CssValue::Percentage(v) = decl.value {
                style.padding_top_pct = Some(v);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_top = px;
                style.padding_top_pct = None;
            }
        }
        Property::PaddingRight => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.padding_right = parent.padding_right;
                    style.padding_right_pct = parent.padding_right_pct;
                }
            } else if let CssValue::Percentage(v) = decl.value {
                style.padding_right_pct = Some(v);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_right = px;
                style.padding_right_pct = None;
            }
        }
        Property::PaddingBottom => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.padding_bottom = parent.padding_bottom;
                    style.padding_bottom_pct = parent.padding_bottom_pct;
                }
            } else if let CssValue::Percentage(v) = decl.value {
                style.padding_bottom_pct = Some(v);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_bottom = px;
                style.padding_bottom_pct = None;
            }
        }
        Property::PaddingLeft => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.padding_left = parent.padding_left;
                    style.padding_left_pct = parent.padding_left_pct;
                }
            } else if let CssValue::Percentage(v) = decl.value {
                style.padding_left_pct = Some(v);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_left = px;
                style.padding_left_pct = None;
            }
        }
        Property::BorderWidth => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_width = px;
                style.border_top.width = px;
                style.border_right.width = px;
                style.border_bottom.width = px;
                style.border_left.width = px;
            }
            if let CssValue::Keyword(ref kw) = decl.value {
                let w = match kw.as_str() {
                    "thin" => 1,
                    "medium" => 3,
                    "thick" => 5,
                    _ => style.border_width,
                };
                style.border_width = w;
                style.border_top.width = w;
                style.border_right.width = w;
                style.border_bottom.width = w;
                style.border_left.width = w;
            }
        }
        Property::BorderColor => {
            let c = match decl.value {
                CssValue::Color(c) => Some(c),
                CssValue::CurrentColor => Some(if style.color != 0 {
                    style.color
                } else {
                    0xFF000000
                }),
                _ => None,
            };
            if let Some(c) = c {
                style.border_color = c;
                style.border_top.color = c;
                style.border_right.color = c;
                style.border_bottom.color = c;
                style.border_left.color = c;
            }
        }
        Property::BorderStyle => {
            let sv = resolve_border_style_val(&decl.value);
            style.border_top.style = sv;
            style.border_right.style = sv;
            style.border_bottom.style = sv;
            style.border_left.style = sv;
        }
        Property::BorderRadius => {
            if let Some(px) = resolve_border_radius(&decl.value, parent_fs, root_fs) {
                style.border_radius = px;
                style.border_top_left_radius = px;
                style.border_top_right_radius = px;
                style.border_bottom_right_radius = px;
                style.border_bottom_left_radius = px;
            }
        }
        // Shorthand border: just pick up width and color from the value.
        Property::Border
        | Property::BorderTop
        | Property::BorderRight
        | Property::BorderBottom
        | Property::BorderLeft => {
            if let CssValue::Color(c) = decl.value {
                style.border_color = c;
                style.border_top.color = c;
                style.border_right.color = c;
                style.border_bottom.color = c;
                style.border_left.color = c;
            }
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_width = px;
                style.border_top.width = px;
                style.border_right.width = px;
                style.border_bottom.width = px;
                style.border_left.width = px;
            }
        }
        Property::ListStyleType => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.list_style = match kw.as_str() {
                    "disc" => ListStyle::Disc,
                    "circle" => ListStyle::Circle,
                    "square" => ListStyle::Square,
                    "decimal" | "decimal-leading-zero" => ListStyle::Decimal,
                    "none" => ListStyle::None,
                    "lower-alpha" | "lower-latin" => ListStyle::LowerAlpha,
                    "upper-alpha" | "upper-latin" => ListStyle::UpperAlpha,
                    "lower-roman" => ListStyle::LowerRoman,
                    "upper-roman" => ListStyle::UpperRoman,
                    _ => style.list_style,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.list_style = ListStyle::None;
            }
        }
        Property::ListStylePosition => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.list_style_position = match kw.as_str() {
                    "inside" => ListStylePosition::Inside,
                    _ => ListStylePosition::Outside,
                };
            }
        }
        Property::WhiteSpace => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.white_space = match kw.as_str() {
                    "pre" => WhiteSpace::Pre,
                    "nowrap" => WhiteSpace::Nowrap,
                    "pre-wrap" => WhiteSpace::PreWrap,
                    _ => WhiteSpace::Normal,
                };
            }
        }
        Property::Position => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.position = match kw.as_str() {
                    "static" => Position::Static,
                    "relative" => Position::Relative,
                    "absolute" => Position::Absolute,
                    "fixed" => Position::Fixed,
                    "sticky" => Position::Sticky,
                    _ => style.position,
                };
            }
        }
        Property::Top => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.top = parent.top;
                    style.top_calc = parent.top_calc;
                }
            } else {
                apply_inset_side(
                    &decl.value,
                    &mut style.top,
                    &mut style.top_calc,
                    parent_fs,
                    root_fs,
                );
            }
        }
        Property::Right => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.right_offset = parent.right_offset;
                    style.right_calc = parent.right_calc;
                }
            } else {
                apply_inset_side(
                    &decl.value,
                    &mut style.right_offset,
                    &mut style.right_calc,
                    parent_fs,
                    root_fs,
                );
            }
        }
        Property::Bottom => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.bottom_offset = parent.bottom_offset;
                    style.bottom_calc = parent.bottom_calc;
                }
            } else {
                apply_inset_side(
                    &decl.value,
                    &mut style.bottom_offset,
                    &mut style.bottom_calc,
                    parent_fs,
                    root_fs,
                );
            }
        }
        Property::Left => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.left_offset = parent.left_offset;
                    style.left_calc = parent.left_calc;
                }
            } else {
                apply_inset_side(
                    &decl.value,
                    &mut style.left_offset,
                    &mut style.left_calc,
                    parent_fs,
                    root_fs,
                );
            }
        }
        Property::ZIndex => match decl.value {
            CssValue::Number(v) => {
                style.z_index = v / 100;
                style.z_index_auto = false;
            }
            CssValue::Auto | CssValue::Inherit => {
                style.z_index = 0;
                style.z_index_auto = true;
            }
            _ => {
                if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                    style.z_index = px;
                    style.z_index_auto = false;
                }
            }
        },
        Property::FlexDirection => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.flex_direction = match kw.as_str() {
                    "row" => FlexDirection::Row,
                    "row-reverse" => FlexDirection::RowReverse,
                    "column" => FlexDirection::Column,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    _ => style.flex_direction,
                };
            }
        }
        Property::FlexWrap => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.flex_wrap = match kw.as_str() {
                    "nowrap" => FlexWrap::Nowrap,
                    "wrap" => FlexWrap::Wrap,
                    "wrap-reverse" => FlexWrap::WrapReverse,
                    _ => style.flex_wrap,
                };
            }
        }
        Property::FlexFlow => {
            if let CssValue::Keyword(ref kw) = decl.value {
                for part in kw.split_whitespace() {
                    match part {
                        "row" => style.flex_direction = FlexDirection::Row,
                        "row-reverse" => style.flex_direction = FlexDirection::RowReverse,
                        "column" => style.flex_direction = FlexDirection::Column,
                        "column-reverse" => style.flex_direction = FlexDirection::ColumnReverse,
                        "nowrap" => style.flex_wrap = FlexWrap::Nowrap,
                        "wrap" => style.flex_wrap = FlexWrap::Wrap,
                        "wrap-reverse" => style.flex_wrap = FlexWrap::WrapReverse,
                        _ => {}
                    }
                }
            }
        }
        Property::JustifyContent => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.justify_content = match kw.as_str() {
                    "flex-start" | "start" | "left" => JustifyContent::FlexStart,
                    "flex-end" | "end" | "right" => JustifyContent::FlexEnd,
                    "center" => JustifyContent::Center,
                    "space-between" => JustifyContent::SpaceBetween,
                    "space-around" => JustifyContent::SpaceAround,
                    "space-evenly" => JustifyContent::SpaceEvenly,
                    _ => style.justify_content,
                };
            }
        }
        Property::AlignItems => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.align_items = match kw.as_str() {
                    "flex-start" | "start" => AlignItems::FlexStart,
                    "flex-end" | "end" => AlignItems::FlexEnd,
                    "center" => AlignItems::Center,
                    "stretch" => AlignItems::Stretch,
                    "baseline" => AlignItems::Baseline,
                    _ => style.align_items,
                };
            }
        }
        Property::AlignSelf => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if let Some(v) = parse_self_alignment_kw(kw) {
                    style.align_self = v;
                    style.align_self_is_normal = kw.trim() == "normal";
                }
            }
        }
        Property::JustifySelf => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if let Some(v) = parse_self_alignment_kw(kw) {
                    style.justify_self = v;
                    style.justify_self_is_normal = kw.trim() == "normal";
                    style.justify_self_inline = parse_inline_axis_alignment_kw(kw);
                }
            }
        }
        Property::PlaceItems => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (align, justify) = parse_place_items_value(kw);
                style.align_items = align;
                style.justify_items = justify;
                style.justify_items_specified = true;
                style.justify_items_inline = parse_place_items_inline_value(kw).1;
            }
        }
        Property::PlaceSelf => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (align, justify) = parse_place_self_value(kw);
                style.align_self = align;
                style.justify_self = justify;
                style.align_self_is_normal = kw.split_whitespace().next() == Some("normal");
                style.justify_self_is_normal = kw.split_whitespace().nth(1) == Some("normal")
                    || (kw.split_whitespace().nth(1).is_none()
                        && kw.split_whitespace().next() == Some("normal"));
                style.justify_self_inline = parse_place_self_inline_value(kw).1;
            }
        }
        Property::PlaceContent => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (align, justify) = parse_place_content_value(kw);
                style.align_content = align;
                style.align_content_is_normal = kw.split_whitespace().next() == Some("normal");
                style.justify_content = justify;
            }
        }
        Property::FlexGrow => {
            if let CssValue::Number(v) = decl.value {
                style.flex_grow = v;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.flex_grow = px * 100;
            }
        }
        Property::FlexShrink => {
            if let CssValue::Number(v) = decl.value {
                style.flex_shrink = v;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.flex_shrink = px * 100;
            }
        }
        Property::FlexBasis => {
            if matches!(decl.value, CssValue::Auto) {
                style.flex_basis = Option::None;
                style.flex_basis_pct = Option::None;
            } else if let CssValue::Length(v, Unit::Percent) = &decl.value {
                // Percentage flex-basis: resolved at layout time against container main size.
                // Stored as percent × 100 (e.g. 100% → 10000), matching width_pct convention.
                style.flex_basis_pct = Some(*v);
                style.flex_basis = Option::None;
            } else if let CssValue::Percentage(v) = &decl.value {
                // Percentage(v) is also stored as percent × 100, just like Length(_, Percent).
                style.flex_basis_pct = Some(*v);
                style.flex_basis = Option::None;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.flex_basis = Some(px);
                style.flex_basis_pct = Option::None;
            }
        }
        Property::RowGap => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.row_gap = px;
            }
        }
        Property::ColumnGap => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.column_gap = px;
            }
        }
        Property::Order => {
            if let CssValue::Number(v) = decl.value {
                style.order = v / 100;
            }
        }
        Property::BoxSizing => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.box_sizing = match kw.as_str() {
                    "border-box" => BoxSizing::BorderBox,
                    "content-box" => BoxSizing::ContentBox,
                    _ => style.box_sizing,
                };
            }
        }
        Property::Float => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.float = match kw.as_str() {
                    "left" => FloatVal::Left,
                    "right" => FloatVal::Right,
                    "none" => FloatVal::None,
                    _ => style.float,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.float = FloatVal::None;
            }
        }
        Property::Clear => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.clear = match kw.as_str() {
                    "left" => ClearVal::Left,
                    "right" => ClearVal::Right,
                    "both" => ClearVal::Both,
                    "none" => ClearVal::None,
                    _ => style.clear,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.clear = ClearVal::None;
            }
        }
        Property::Opacity => {
            if let CssValue::Number(v) = decl.value {
                // v is fixed-point * 100: "0.5" → 50, "1" → 100
                style.opacity = ((v * 255) / 100).max(0).min(255);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.opacity = (px * 255).max(0).min(255);
            }
        }
        Property::Visibility => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.visibility = match kw.as_str() {
                    "visible" => Visibility::Visible,
                    "hidden" => Visibility::Hidden,
                    "collapse" => Visibility::Collapse,
                    _ => style.visibility,
                };
            }
        }
        Property::TextTransform => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_transform = match kw.as_str() {
                    "uppercase" => TextTransform::Uppercase,
                    "lowercase" => TextTransform::Lowercase,
                    "capitalize" => TextTransform::Capitalize,
                    "none" => TextTransform::None,
                    _ => style.text_transform,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.text_transform = TextTransform::None;
            }
        }
        Property::OverflowX => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.overflow_x = parse_overflow_keyword(kw);
            }
        }
        Property::OverflowY => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.overflow_y = parse_overflow_keyword(kw);
            }
        }
        // Transitions
        Property::Transition => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.transitions = parse_transition_shorthand(kw);
            }
        }
        Property::TransitionProperty => {
            // Set property names on existing TransitionDef entries, or create one.
            if let CssValue::Keyword(ref kw) = decl.value {
                let names: Vec<&str> = kw.split(',').map(|s| s.trim()).collect();
                style
                    .transitions
                    .resize_with(names.len().max(style.transitions.len()), || TransitionDef {
                        property: String::new(),
                        duration_ms: 0,
                        timing: TimingFunction::Ease,
                        delay_ms: 0,
                    });
                for (i, name) in names.iter().enumerate() {
                    if i < style.transitions.len() {
                        style.transitions[i].property = name.to_ascii_lowercase();
                    }
                }
            }
        }
        Property::TransitionDuration => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let ms = parse_time_ms(kw);
                if style.transitions.is_empty() {
                    style.transitions.push(TransitionDef {
                        property: String::from("all"),
                        duration_ms: ms,
                        timing: TimingFunction::Ease,
                        delay_ms: 0,
                    });
                } else {
                    for t in &mut style.transitions {
                        t.duration_ms = ms;
                    }
                }
            }
        }
        Property::TransitionTimingFunction => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let tf = parse_timing_function(kw);
                if style.transitions.is_empty() {
                    style.transitions.push(TransitionDef {
                        property: String::from("all"),
                        duration_ms: 0,
                        timing: tf,
                        delay_ms: 0,
                    });
                } else {
                    for t in &mut style.transitions {
                        t.timing = tf;
                    }
                }
            }
        }
        Property::TransitionDelay => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let ms = parse_time_ms(kw);
                if style.transitions.is_empty() {
                    style.transitions.push(TransitionDef {
                        property: String::from("all"),
                        duration_ms: 0,
                        timing: TimingFunction::Ease,
                        delay_ms: ms,
                    });
                } else {
                    for t in &mut style.transitions {
                        t.delay_ms = ms;
                    }
                }
            }
        }
        // Animations
        Property::Animation => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.animations = parse_animation_shorthand(kw);
            }
        }
        Property::AnimationName => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let names: Vec<&str> = kw.split(',').map(|s| s.trim()).collect();
                style
                    .animations
                    .resize_with(names.len().max(style.animations.len()), || AnimationDef {
                        name: String::new(),
                        duration_ms: 0,
                        timing: TimingFunction::Ease,
                        delay_ms: 0,
                        iteration_count: 1,
                        alternate: false,
                    });
                for (i, name) in names.iter().enumerate() {
                    if i < style.animations.len() {
                        style.animations[i].name = name.to_ascii_lowercase();
                    }
                }
            }
        }
        Property::AnimationDuration => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let ms = parse_time_ms(kw);
                if style.animations.is_empty() {
                    style.animations.push(AnimationDef {
                        name: String::new(),
                        duration_ms: ms,
                        timing: TimingFunction::Ease,
                        delay_ms: 0,
                        iteration_count: 1,
                        alternate: false,
                    });
                } else {
                    for a in &mut style.animations {
                        a.duration_ms = ms;
                    }
                }
            }
        }
        Property::AnimationTimingFunction => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let tf = parse_timing_function(kw);
                for a in &mut style.animations {
                    a.timing = tf;
                }
            }
        }
        Property::AnimationDelay => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let ms = parse_time_ms(kw);
                for a in &mut style.animations {
                    a.delay_ms = ms;
                }
            }
        }
        Property::AnimationIterationCount => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let count = if kw == "infinite" {
                    0
                } else {
                    kw.parse::<u32>().unwrap_or(1)
                };
                for a in &mut style.animations {
                    a.iteration_count = count;
                }
            } else if let CssValue::Number(v) = decl.value {
                let count = (v / 100) as u32;
                for a in &mut style.animations {
                    a.iteration_count = count;
                }
            }
        }
        Property::AnimationDirection => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let alt = kw == "alternate" || kw == "alternate-reverse";
                for a in &mut style.animations {
                    a.alternate = alt;
                }
            }
        }
        Property::AnimationFillMode | Property::AnimationPlayState => {}
        Property::TextIndent => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.text_indent = px;
            }
        }
        Property::VerticalAlign => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.vertical_align = match kw.as_str() {
                    "baseline" => VerticalAlign::Baseline,
                    "top" => VerticalAlign::Top,
                    "middle" => VerticalAlign::Middle,
                    "bottom" => VerticalAlign::Bottom,
                    "text-top" => VerticalAlign::TextTop,
                    "text-bottom" => VerticalAlign::TextBottom,
                    "sub" => VerticalAlign::Sub,
                    "super" => VerticalAlign::Super,
                    _ => style.vertical_align,
                };
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.vertical_align = VerticalAlign::Length(px);
            }
        }
        Property::FontFamily => {
            match decl.value {
                CssValue::Keyword(ref kw) => {
                    style.font_family = Some(kw.clone());
                }
                CssValue::Inherit => {
                    if let Some(parent) = parent_style {
                        style.font_family = parent.font_family.clone();
                    }
                }
                _ => {}
            }
        }
        Property::LetterSpacing => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if kw == "normal" {
                    style.letter_spacing = 0;
                }
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.letter_spacing = px;
            }
        }
        Property::WordSpacing => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if kw == "normal" {
                    style.word_spacing = 0;
                }
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.word_spacing = px;
            }
        }
        Property::WordBreak => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.word_break = match kw.as_str() {
                    "break-all" => WordBreak::BreakAll,
                    "keep-all" => WordBreak::KeepAll,
                    _ => WordBreak::Normal,
                };
            }
        }
        Property::OverflowWrap => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.overflow_wrap = match kw.as_str() {
                    "break-word" => OverflowWrapVal::BreakWord,
                    "anywhere" => OverflowWrapVal::Anywhere,
                    _ => OverflowWrapVal::Normal,
                };
            }
        }
        Property::TextOverflow => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_overflow = match kw.as_str() {
                    "ellipsis" => TextOverflowVal::Ellipsis,
                    _ => TextOverflowVal::Clip,
                };
            }
        }
        // Per-side border widths
        Property::BorderTopWidth => {
            resolve_border_width(&decl.value, parent_fs, root_fs, &mut style.border_top.width);
            style.border_width = style.border_top.width; // sync unified
        }
        Property::BorderRightWidth => {
            resolve_border_width(
                &decl.value,
                parent_fs,
                root_fs,
                &mut style.border_right.width,
            );
        }
        Property::BorderBottomWidth => {
            resolve_border_width(
                &decl.value,
                parent_fs,
                root_fs,
                &mut style.border_bottom.width,
            );
        }
        Property::BorderLeftWidth => {
            resolve_border_width(
                &decl.value,
                parent_fs,
                root_fs,
                &mut style.border_left.width,
            );
        }
        // Per-side border colors
        Property::BorderTopColor => {
            if let CssValue::Color(c) = decl.value {
                style.border_top.color = c;
            }
        }
        Property::BorderRightColor => {
            if let CssValue::Color(c) = decl.value {
                style.border_right.color = c;
            }
        }
        Property::BorderBottomColor => {
            if let CssValue::Color(c) = decl.value {
                style.border_bottom.color = c;
            }
        }
        Property::BorderLeftColor => {
            if let CssValue::Color(c) = decl.value {
                style.border_left.color = c;
            }
        }
        // Per-side border styles
        Property::BorderTopStyle => {
            style.border_top.style = resolve_border_style_val(&decl.value);
        }
        Property::BorderRightStyle => {
            style.border_right.style = resolve_border_style_val(&decl.value);
        }
        Property::BorderBottomStyle => {
            style.border_bottom.style = resolve_border_style_val(&decl.value);
        }
        Property::BorderLeftStyle => {
            style.border_left.style = resolve_border_style_val(&decl.value);
        }
        // Per-corner border radius
        Property::BorderTopLeftRadius => {
            if let Some(px) = resolve_border_radius(&decl.value, parent_fs, root_fs) {
                style.border_top_left_radius = px;
            }
        }
        Property::BorderTopRightRadius => {
            if let Some(px) = resolve_border_radius(&decl.value, parent_fs, root_fs) {
                style.border_top_right_radius = px;
            }
        }
        Property::BorderBottomRightRadius => {
            if let Some(px) = resolve_border_radius(&decl.value, parent_fs, root_fs) {
                style.border_bottom_right_radius = px;
            }
        }
        Property::BorderBottomLeftRadius => {
            if let Some(px) = resolve_border_radius(&decl.value, parent_fs, root_fs) {
                style.border_bottom_left_radius = px;
            }
        }
        // Outline
        Property::OutlineWidth => {
            resolve_border_width(&decl.value, parent_fs, root_fs, &mut style.outline_width);
        }
        Property::OutlineColor => {
            if let CssValue::Color(c) = decl.value {
                style.outline_color = c;
            }
        }
        Property::OutlineStyle => {
            style.outline_style = resolve_border_style_val(&decl.value);
        }
        Property::OutlineOffset => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.outline_offset = px;
            }
        }
        // Shadows
        Property::BoxShadow => {
            if matches!(decl.value, CssValue::None) {
                style.box_shadows.clear();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.box_shadows = parse_box_shadows(kw, parent_fs, root_fs);
            }
        }
        Property::TextShadow => {
            if matches!(decl.value, CssValue::None) {
                style.text_shadows.clear();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.text_shadows = parse_text_shadows(kw, parent_fs, root_fs);
            }
        }
        // Background extensions
        Property::BackgroundImage => {
            if matches!(decl.value, CssValue::None) {
                style.background_image = BackgroundImageVal::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                if let Some(parsed) = parse_background_image_val(kw) {
                    style.background_image = parsed;
                }
            }
        }
        Property::BackgroundSize => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.background_size = match kw.as_str() {
                    "cover" => BackgroundSizeVal::Cover,
                    "contain" => BackgroundSizeVal::Contain,
                    "auto" => BackgroundSizeVal::Auto,
                    _ => {
                        // Try "Wpx Hpx" or "W% H%"
                        let parts: Vec<&str> = kw.split_whitespace().collect();
                        if parts.len() >= 2 {
                            let w = parse_bg_size_dim(parts[0], parent_fs, root_fs);
                            let h = parse_bg_size_dim(parts[1], parent_fs, root_fs);
                            BackgroundSizeVal::Explicit(w, h)
                        } else if parts.len() == 1 {
                            let w = parse_bg_size_dim(parts[0], parent_fs, root_fs);
                            BackgroundSizeVal::Explicit(w, -1)
                        } else {
                            BackgroundSizeVal::Auto
                        }
                    }
                };
            }
            if matches!(decl.value, CssValue::Auto) {
                style.background_size = BackgroundSizeVal::Auto;
            }
        }
        Property::BackgroundRepeat => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.background_repeat = match kw.as_str() {
                    "repeat-x" => BackgroundRepeatVal::RepeatX,
                    "repeat-y" => BackgroundRepeatVal::RepeatY,
                    "no-repeat" => BackgroundRepeatVal::NoRepeat,
                    _ => BackgroundRepeatVal::Repeat,
                };
            }
        }
        Property::BackgroundClip => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.background_clip = match kw.as_str() {
                    "padding-box" => BackgroundClipVal::PaddingBox,
                    "content-box" => BackgroundClipVal::ContentBox,
                    "text" => BackgroundClipVal::Text,
                    _ => BackgroundClipVal::BorderBox,
                };
            }
        }
        Property::BackgroundPosition => {
            // Simplified: just parse keywords or lengths
            if let CssValue::Keyword(ref kw) = decl.value {
                let parts: Vec<&str> = kw.split_whitespace().collect();
                if !parts.is_empty() {
                    style.background_position_x =
                        parse_bg_position_part(parts[0], parent_fs, root_fs);
                }
                if parts.len() >= 2 {
                    style.background_position_y =
                        parse_bg_position_part(parts[1], parent_fs, root_fs);
                } else if parts.len() == 1 {
                    // CSS Backgrounds: one-value background-position means
                    // horizontal position plus vertical center.
                    style.background_position_y = 5000;
                }
            }
        }
        // Content
        Property::Content => {
            if matches!(decl.value, CssValue::None) {
                style.content = Option::None;
                style.content_url = Option::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                // Use the full content value parser for proper multi-value handling.
                let (text, url) = parse_content_value(kw.as_str());
                style.content = text;
                style.content_url = url;
            }
        }
        Property::ObjectFit => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.object_fit = match kw.as_str() {
                    "fill" => ObjectFit::Fill,
                    "contain" => ObjectFit::Contain,
                    "cover" => ObjectFit::Cover,
                    "none" => ObjectFit::None,
                    "scale-down" => ObjectFit::ScaleDown,
                    _ => style.object_fit,
                };
            }
        }
        Property::ObjectPosition => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (x, x_is_percent, y, y_is_percent) =
                    parse_position_pair(kw, parent_fs, root_fs, 5000, true, 5000, true);
                style.object_position_x = x;
                style.object_position_x_is_percent = x_is_percent;
                style.object_position_y = y;
                style.object_position_y_is_percent = y_is_percent;
            }
        }
        Property::Transform => {
            // Parse transform functions: translate(x,y), translateX(x), translateY(y)
            if matches!(decl.value, CssValue::None)
                || matches!(decl.value, CssValue::Keyword(ref k) if k == "none")
            {
                style.transform_tx = 0;
                style.transform_ty = 0;
                style.transform_tx_pct = 0;
                style.transform_ty_pct = 0;
                style.transform_sx = 1000;
                style.transform_sy = 1000;
                style.transform_rotate = 0;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                let s = kw.as_str();
                let mut tx = 0i32;
                let mut ty = 0i32;
                let mut tx_pct = 0i32;
                let mut ty_pct = 0i32;
                if !s.contains('(') {
                    let parts: Vec<&str> = s.split_whitespace().collect();
                    let looks_like_translate = parts.iter().any(|part| {
                        let p = part.trim();
                        p == "0"
                            || p.ends_with('%')
                            || p.ends_with("px")
                            || p.ends_with("em")
                            || p.ends_with("rem")
                    });
                    if looks_like_translate {
                        if let Some(x) = parts.first() {
                            let (px, pct) = parse_transform_translate_component(x, parent_fs);
                            tx = px;
                            tx_pct = pct;
                        }
                        if let Some(y) = parts.get(1) {
                            let (px, pct) = parse_transform_translate_component(y, parent_fs);
                            ty = px;
                            ty_pct = pct;
                        }
                        style.transform_tx = tx;
                        style.transform_ty = ty;
                        style.transform_tx_pct = tx_pct;
                        style.transform_ty_pct = ty_pct;
                        return;
                    }
                }
                let mut pos = 0usize;
                let bytes = s.as_bytes();
                while pos < bytes.len() {
                    // Skip whitespace
                    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
                        pos += 1;
                    }
                    if pos >= bytes.len() {
                        break;
                    }
                    // Read function name
                    let name_start = pos;
                    while pos < bytes.len() && bytes[pos] != b'(' && bytes[pos] != b' ' {
                        pos += 1;
                    }
                    let fname = core::str::from_utf8(&bytes[name_start..pos]).unwrap_or("");
                    if pos < bytes.len() && bytes[pos] == b'(' {
                        pos += 1; // skip '('
                                  // Read args until ')'
                        let args_start = pos;
                        while pos < bytes.len() && bytes[pos] != b')' {
                            pos += 1;
                        }
                        let args = core::str::from_utf8(&bytes[args_start..pos]).unwrap_or("");
                        if pos < bytes.len() {
                            pos += 1;
                        } // skip ')'
                        match fname {
                            "translateX" | "translatex" => {
                                let (px, pct) =
                                    parse_transform_translate_component(args.trim(), parent_fs);
                                tx += px;
                                tx_pct += pct;
                            }
                            "translateY" | "translatey" => {
                                let (px, pct) =
                                    parse_transform_translate_component(args.trim(), parent_fs);
                                ty += px;
                                ty_pct += pct;
                            }
                            "translate" => {
                                let parts: Vec<&str> = if args.contains(',') {
                                    args.split(',').collect()
                                } else {
                                    args.split_whitespace().collect()
                                };
                                if !parts.is_empty() {
                                    let (px, pct) = parse_transform_translate_component(
                                        parts[0].trim(),
                                        parent_fs,
                                    );
                                    tx += px;
                                    tx_pct += pct;
                                }
                                if parts.len() > 1 {
                                    let (px, pct) = parse_transform_translate_component(
                                        parts[1].trim(),
                                        parent_fs,
                                    );
                                    ty += px;
                                    ty_pct += pct;
                                }
                            }
                            "scale" => {
                                // scale(sx) or scale(sx, sy)
                                let parts: Vec<&str> = args.split(',').collect();
                                if let Some(sx_str) = parts.first() {
                                    if let Ok(sx) = sx_str.trim().parse::<f32>() {
                                        style.transform_sx = (sx * 1000.0) as i32;
                                        style.transform_sy = if let Some(sy_str) = parts.get(1) {
                                            if let Ok(sy) = sy_str.trim().parse::<f32>() {
                                                (sy * 1000.0) as i32
                                            } else {
                                                style.transform_sx
                                            }
                                        } else {
                                            style.transform_sx
                                        };
                                    }
                                }
                            }
                            "scaleX" | "scalex" => {
                                if let Ok(sx) = args.trim().parse::<f32>() {
                                    style.transform_sx = (sx * 1000.0) as i32;
                                }
                            }
                            "scaleY" | "scaley" => {
                                if let Ok(sy) = args.trim().parse::<f32>() {
                                    style.transform_sy = (sy * 1000.0) as i32;
                                }
                            }
                            "rotate" => {
                                let s = args.trim();
                                let deg = if s.ends_with("deg") {
                                    s.trim_end_matches("deg").parse::<f32>().ok()
                                } else if s.ends_with("rad") {
                                    s.trim_end_matches("rad")
                                        .parse::<f32>()
                                        .ok()
                                        .map(|r| r * 180.0 / 3.14159265)
                                } else if s.ends_with("turn") {
                                    s.trim_end_matches("turn")
                                        .parse::<f32>()
                                        .ok()
                                        .map(|t| t * 360.0)
                                } else {
                                    s.parse::<f32>().ok()
                                };
                                if let Some(d) = deg {
                                    style.transform_rotate = (d * 100.0) as i32;
                                }
                            }
                            _ => {}
                        }
                    } else {
                        break;
                    }
                }
                style.transform_tx = tx;
                style.transform_ty = ty;
                style.transform_tx_pct = tx_pct;
                style.transform_ty_pct = ty_pct;
            }
        }
        Property::Translate => {
            if matches!(decl.value, CssValue::None)
                || matches!(decl.value, CssValue::Keyword(ref k) if k == "none")
            {
                style.transform_tx = 0;
                style.transform_ty = 0;
                style.transform_tx_pct = 0;
                style.transform_ty_pct = 0;
            } else if let Some((tx, ty, tx_pct, ty_pct)) =
                parse_individual_translate(&decl.value, parent_fs, root_fs)
            {
                style.transform_tx = tx;
                style.transform_ty = ty;
                style.transform_tx_pct = tx_pct;
                style.transform_ty_pct = ty_pct;
            }
        }
        Property::Scale => {
            if matches!(decl.value, CssValue::None)
                || matches!(decl.value, CssValue::Keyword(ref k) if k == "none")
            {
                style.transform_sx = 1000;
                style.transform_sy = 1000;
            } else if let Some((sx, sy)) = parse_individual_scale(&decl.value) {
                style.transform_sx = sx;
                style.transform_sy = sy;
            }
        }
        Property::Rotate => {
            if matches!(decl.value, CssValue::None)
                || matches!(decl.value, CssValue::Keyword(ref k) if k == "none")
            {
                style.transform_rotate = 0;
            } else if let Some(deg100) = parse_individual_rotate(&decl.value) {
                style.transform_rotate = deg100;
            }
        }
        Property::TransformOrigin => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (x, x_is_percent, y, y_is_percent) =
                    parse_position_pair(kw, parent_fs, root_fs, 5000, true, 5000, true);
                style.transform_origin_x = x;
                style.transform_origin_x_is_percent = x_is_percent;
                style.transform_origin_y = y;
                style.transform_origin_y_is_percent = y_is_percent;
            }
        }
        Property::AlignContent => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if let Some(v) = parse_align_content_kw(kw) {
                    style.align_content = v;
                    style.align_content_is_normal = kw.trim() == "normal";
                }
            }
        }
        // Properties we parse but do not yet resolve:
        Property::BorderCollapse => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.border_collapse = kw == "collapse";
            }
        }
        Property::BorderSpacing => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_spacing_x = px;
                style.border_spacing_y = px;
            } else if let CssValue::Keyword(ref raw) = decl.value {
                let mut parts = raw.split_ascii_whitespace();
                if let Some(first) = parts.next() {
                    if let Some(px) = resolve_length(
                        &crate::css::parse_value(&Property::BorderSpacing, first),
                        parent_fs,
                        root_fs,
                    ) {
                        style.border_spacing_x = px;
                        style.border_spacing_y = parts
                            .next()
                            .and_then(|second| {
                                resolve_length(
                                    &crate::css::parse_value(&Property::BorderSpacing, second),
                                    parent_fs,
                                    root_fs,
                                )
                            })
                            .unwrap_or(px);
                    }
                }
            }
        }
        Property::TableLayout => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.table_layout_fixed = kw == "fixed";
            }
        }
        // Filter
        Property::Filter => {
            if matches!(decl.value, CssValue::None) {
                style.filter = FilterVal::none();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.filter = parse_filter_value(kw, parent_fs, root_fs);
            }
        }
        // Aspect ratio
        Property::AspectRatio => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if kw == "auto" {
                    style.aspect_ratio = 0;
                } else if let Some(pos) = kw.find('/') {
                    // "16 / 9" or "auto 16/9" or "16/9 auto" format
                    // Strip optional "auto" keyword (CSS Sizing §5.1.2).
                    let w_str = kw[..pos]
                        .trim()
                        .trim_start_matches("auto")
                        .trim_end_matches("auto")
                        .trim();
                    let h_str = kw[pos + 1..]
                        .trim()
                        .trim_start_matches("auto")
                        .trim_end_matches("auto")
                        .trim();
                    if let (Some(w), Some(h)) =
                        (try_parse_simple_float(w_str), try_parse_simple_float(h_str))
                    {
                        if h > 0 {
                            style.aspect_ratio = w * 100 / h;
                        }
                    }
                } else if let Some(v) =
                    try_parse_simple_float(kw.trim().trim_start_matches("auto").trim())
                {
                    style.aspect_ratio = v;
                }
            } else if let CssValue::Number(v) = decl.value {
                style.aspect_ratio = v;
            }
        }
        // Text decoration sub-properties
        Property::TextDecorationColor => {
            if let CssValue::Color(c) = decl.value {
                style.text_decoration_color = c;
            }
        }
        Property::TextDecorationStyle => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_decoration_style = match kw.as_str() {
                    "solid" => TextDecorationStyle::Solid,
                    "double" => TextDecorationStyle::Double,
                    "dotted" => TextDecorationStyle::Dotted,
                    "dashed" => TextDecorationStyle::Dashed,
                    "wavy" => TextDecorationStyle::Wavy,
                    _ => style.text_decoration_style,
                };
            }
        }
        Property::TextDecorationThickness => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.text_decoration_thickness = px;
            }
        }
        Property::ColorScheme => {
            if matches!(decl.value, CssValue::Auto) {
                style.color_scheme = ColorSchemeVal::Auto;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                let mut resolved = ColorSchemeVal::Auto;
                for part in kw.split_whitespace() {
                    match part {
                        "dark" => {
                            resolved = ColorSchemeVal::Dark;
                            break;
                        }
                        "light" => {
                            resolved = ColorSchemeVal::Light;
                            break;
                        }
                        "normal" => {
                            resolved = ColorSchemeVal::Auto;
                            break;
                        }
                        "only" => {
                            continue;
                        }
                        _ => {}
                    }
                }
                style.color_scheme = resolved;
            }
        }
        Property::ContainerType => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.container_type = if kw.contains("inline-size") {
                    ContainerTypeVal::InlineSize
                } else if kw.contains("size") {
                    ContainerTypeVal::Size
                } else {
                    ContainerTypeVal::Normal
                };
            } else if matches!(decl.value, CssValue::None) {
                style.container_type = ContainerTypeVal::Normal;
            }
        }
        Property::ContainerName => {
            if matches!(decl.value, CssValue::None) {
                style.container_names.clear();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.container_names = kw
                    .split_whitespace()
                    .filter(|part| !part.is_empty() && *part != "none")
                    .map(String::from)
                    .collect();
            }
        }
        Property::TextUnderlineOffset => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.text_underline_offset = px;
            }
        }
        Property::ScrollBehavior => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.scroll_behavior = if kw.eq_ignore_ascii_case("smooth") {
                    ScrollBehaviorVal::Smooth
                } else {
                    ScrollBehaviorVal::Auto
                };
            }
        }
        // Font variant
        Property::FontVariant => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_variant = match kw.as_str() {
                    "small-caps" => FontVariantVal::SmallCaps,
                    _ => FontVariantVal::Normal,
                };
            }
        }
        // Tab size
        Property::TabSize => {
            if let CssValue::Number(v) = decl.value {
                style.tab_size = (v / 100).max(1);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.tab_size = px.max(1);
            }
        }
        // Clip path
        Property::ClipPath => {
            if matches!(decl.value, CssValue::None) {
                style.clip_path = ClipPathVal::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.clip_path = parse_clip_path_value(kw, parent_fs, root_fs);
            }
        }
        Property::Clip => {
            // `clip: rect(top, right, bottom, left)` for absolutely-positioned elements.
            // `clip: auto` clears the clip rect.
            if matches!(decl.value, CssValue::Auto) || matches!(decl.value, CssValue::None) {
                style.clip_rect = Option::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.clip_rect = parse_clip_rect(kw, parent_fs, root_fs);
            }
        }
        // CSS counters
        Property::CounterReset => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.counter_reset = Some(kw.clone());
            } else if matches!(decl.value, CssValue::None) {
                style.counter_reset = Option::None;
            }
        }
        Property::CounterIncrement => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.counter_increment = Some(kw.clone());
            } else if matches!(decl.value, CssValue::None) {
                style.counter_increment = Option::None;
            }
        }
        // Inset shorthand is expanded before reaching here.
        Property::Inset => {
            apply_inset_side(
                &decl.value,
                &mut style.top,
                &mut style.top_calc,
                parent_fs,
                root_fs,
            );
            apply_inset_side(
                &decl.value,
                &mut style.right_offset,
                &mut style.right_calc,
                parent_fs,
                root_fs,
            );
            apply_inset_side(
                &decl.value,
                &mut style.bottom_offset,
                &mut style.bottom_calc,
                parent_fs,
                root_fs,
            );
            apply_inset_side(
                &decl.value,
                &mut style.left_offset,
                &mut style.left_calc,
                parent_fs,
                root_fs,
            );
        }
        Property::InsetInline => {
            apply_inset_side(
                &decl.value,
                &mut style.left_offset,
                &mut style.left_calc,
                parent_fs,
                root_fs,
            );
            apply_inset_side(
                &decl.value,
                &mut style.right_offset,
                &mut style.right_calc,
                parent_fs,
                root_fs,
            );
        }
        Property::InsetBlock => {
            apply_inset_side(
                &decl.value,
                &mut style.top,
                &mut style.top_calc,
                parent_fs,
                root_fs,
            );
            apply_inset_side(
                &decl.value,
                &mut style.bottom_offset,
                &mut style.bottom_calc,
                parent_fs,
                root_fs,
            );
        }
        Property::Overflow => {
            // `overflow` shorthand: one or two keywords.
            // One value → both axes. Two values → overflow-x overflow-y.
            if let CssValue::Keyword(ref kw) = decl.value {
                let parts: Vec<&str> = kw.split_whitespace().collect();
                if parts.len() == 1 {
                    let v = parse_overflow_keyword(parts[0]);
                    style.overflow_x = v;
                    style.overflow_y = v;
                } else if parts.len() >= 2 {
                    style.overflow_x = parse_overflow_keyword(parts[0]);
                    style.overflow_y = parse_overflow_keyword(parts[1]);
                }
            }
        }
        Property::BorderStyle | Property::Flex | Property::Cursor | Property::Outline => {}
        Property::Gap => {
            // gap: <row-gap> <column-gap>?
            // Single value → both row and column gap
            // Two values → row then column
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.row_gap = px;
                style.column_gap = px;
            } else if let CssValue::Keyword(ref s) = decl.value {
                let parts: Vec<&str> = s.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Some(v1) = crate::css::try_parse_dimension_pub(parts[0]) {
                        if let Some(rg) = resolve_length(&v1, parent_fs, root_fs) {
                            style.row_gap = rg;
                        }
                    }
                    if let Some(v2) = crate::css::try_parse_dimension_pub(parts[1]) {
                        if let Some(cg) = resolve_length(&v2, parent_fs, root_fs) {
                            style.column_gap = cg;
                        }
                    }
                } else if parts.len() == 1 {
                    if let Some(v) = crate::css::try_parse_dimension_pub(parts[0]) {
                        if let Some(g) = resolve_length(&v, parent_fs, root_fs) {
                            style.row_gap = g;
                            style.column_gap = g;
                        }
                    }
                }
            }
        }
        // Grid container properties
        Property::GridTemplateColumns => {
            style.grid_template_columns = decode_track_list(&decl.value);
        }
        Property::GridTemplateRows => {
            style.grid_template_rows = decode_track_list(&decl.value);
        }
        Property::GridTemplateAreas => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_template_areas = parse_grid_template_areas_value(kw);
            }
        }
        // GridTemplate shorthand is expanded before reaching here.
        Property::GridTemplate => {}
        Property::GridAutoColumns => {
            style.grid_auto_columns = decode_single_track(&decl.value);
        }
        Property::GridAutoRows => {
            style.grid_auto_rows = decode_single_track(&decl.value);
        }
        Property::GridAutoFlow => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_auto_flow_column = kw.contains("column");
            }
        }
        Property::JustifyItems => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.justify_items = parse_align_items_kw(kw);
                style.justify_items_specified = true;
                style.justify_items_inline = parse_inline_axis_alignment_kw(kw);
            }
        }
        // Grid item placement
        Property::GridColumn => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (start, end) = parse_grid_line_pair(kw);
                style.grid_column_start = start;
                style.grid_column_end = end;
            }
        }
        Property::GridColumnStart => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_column_start = parse_grid_line(kw);
            } else if let Some(n) = try_integer(&decl.value) {
                style.grid_column_start = GridLine::Index(n);
            }
        }
        Property::GridColumnEnd => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_column_end = parse_grid_line(kw);
            } else if let Some(n) = try_integer(&decl.value) {
                style.grid_column_end = GridLine::Index(n);
            }
        }
        Property::GridRow => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (start, end) = parse_grid_line_pair(kw);
                style.grid_row_start = start;
                style.grid_row_end = end;
            }
        }
        Property::GridRowStart => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_row_start = parse_grid_line(kw);
            } else if let Some(n) = try_integer(&decl.value) {
                style.grid_row_start = GridLine::Index(n);
            }
        }
        Property::GridRowEnd => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_row_end = parse_grid_line(kw);
            } else if let Some(n) = try_integer(&decl.value) {
                style.grid_row_end = GridLine::Index(n);
            }
        }
        Property::GridArea => {
            // CSS Grid §8.2: `grid-area: row-start / col-start / row-end / col-end`
            // If fewer than 4 values:
            //   1 value:  all four set to that value
            //   2 values: row-end = row-start, col-end = col-start
            //   3 values: col-end = col-start
            if let CssValue::Keyword(ref kw) = decl.value {
                let parts: Vec<&str> = kw.splitn(4, '/').collect();
                let trimmed: Vec<&str> = parts.iter().map(|s| s.trim()).collect();
                let n = trimmed.len();
                let row_s = parse_grid_line(trimmed[0]);
                let col_s = if n >= 2 {
                    parse_grid_line(trimmed[1])
                } else {
                    row_s.clone()
                };
                let row_e = if n >= 3 {
                    parse_grid_line(trimmed[2])
                } else {
                    row_s.clone()
                };
                let col_e = if n >= 4 {
                    parse_grid_line(trimmed[3])
                } else {
                    col_s.clone()
                };
                style.grid_row_start = row_s;
                style.grid_column_start = col_s;
                style.grid_row_end = row_e;
                style.grid_column_end = col_e;
            }
        }
        Property::CustomProperty(_) => {
            // Custom properties stored separately in resolve_styles; no-op here.
        }
        Property::MaskImage => {
            if matches!(decl.value, CssValue::None) {
                style.mask_image = BackgroundImageVal::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                if let Some(parsed) = parse_background_image_val(kw) {
                    style.mask_image = parsed;
                }
            }
        }
        Property::MaskSize => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.mask_size = match kw.as_str() {
                    "cover" => BackgroundSizeVal::Cover,
                    "contain" => BackgroundSizeVal::Contain,
                    "auto" => BackgroundSizeVal::Auto,
                    _ => {
                        let parts: Vec<&str> = kw.split_whitespace().collect();
                        if parts.len() >= 2 {
                            let w = parse_bg_size_dim(parts[0], parent_fs, root_fs);
                            let h = parse_bg_size_dim(parts[1], parent_fs, root_fs);
                            BackgroundSizeVal::Explicit(w, h)
                        } else if parts.len() == 1 {
                            let w = parse_bg_size_dim(parts[0], parent_fs, root_fs);
                            BackgroundSizeVal::Explicit(w, -1)
                        } else {
                            BackgroundSizeVal::Auto
                        }
                    }
                };
            }
            if matches!(decl.value, CssValue::Auto) {
                style.mask_size = BackgroundSizeVal::Auto;
            }
        }
        Property::MaskRepeat => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.mask_repeat = match kw.as_str() {
                    "repeat-x" => BackgroundRepeatVal::RepeatX,
                    "repeat-y" => BackgroundRepeatVal::RepeatY,
                    "no-repeat" => BackgroundRepeatVal::NoRepeat,
                    _ => BackgroundRepeatVal::Repeat,
                };
            }
        }
        Property::MaskClip => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.mask_clip = match kw.as_str() {
                    "padding-box" => BackgroundClipVal::PaddingBox,
                    "content-box" => BackgroundClipVal::ContentBox,
                    "text" => BackgroundClipVal::Text,
                    _ => BackgroundClipVal::BorderBox,
                };
            }
        }
        Property::MaskOrigin => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.mask_origin = match kw.as_str() {
                    "padding-box" => BackgroundClipVal::PaddingBox,
                    "content-box" => BackgroundClipVal::ContentBox,
                    "text" => BackgroundClipVal::Text,
                    _ => BackgroundClipVal::BorderBox,
                };
            }
        }
        Property::MaskPosition => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (x, x_is_percent, y, y_is_percent) =
                    parse_position_pair(kw, parent_fs, root_fs, 0, true, 0, true);
                style.mask_position_x = x;
                style.mask_position_x_is_percent = x_is_percent;
                style.mask_position_y = y;
                style.mask_position_y_is_percent = y_is_percent;
            }
        }
        Property::PointerEvents => {
            match decl.value {
                CssValue::None => style.pointer_events = PointerEventsVal::None,
                CssValue::Auto => style.pointer_events = PointerEventsVal::Auto,
                CssValue::Inherit => {
                    if let Some(parent) = parent_style {
                        style.pointer_events = parent.pointer_events;
                    }
                }
                CssValue::Keyword(ref kw) => {
                    style.pointer_events = match kw.as_str() {
                        "none" => PointerEventsVal::None,
                        _ => PointerEventsVal::Auto,
                    };
                }
                _ => {}
            }
        }
        Property::UserSelect => {
            if let CssValue::Keyword(ref kw) = decl.value {
                match kw.as_str() {
                    "none" => style.user_select = UserSelectVal::None,
                    "text" => style.user_select = UserSelectVal::Text,
                    "all" => style.user_select = UserSelectVal::All,
                    _ => style.user_select = UserSelectVal::Auto,
                }
            }
        }
        Property::BackdropFilter => {
            if matches!(decl.value, CssValue::None) {
                style.backdrop_filter = FilterVal::none();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.backdrop_filter = parse_filter_value(kw, parent_fs, root_fs);
            }
        }
        Property::Appearance => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.appearance = if kw.eq_ignore_ascii_case("none") {
                    AppearanceVal::None
                } else {
                    AppearanceVal::Auto
                };
            } else if matches!(decl.value, CssValue::None) {
                style.appearance = AppearanceVal::None;
            }
        }
        // CSS Logical Properties — expand to physical sides (LTR assumption)
        Property::PaddingInline => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_left = px;
                style.padding_right = px;
            }
        }
        Property::PaddingBlock => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_top = px;
                style.padding_bottom = px;
            }
        }
        Property::MarginInline => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_left_auto = true;
                style.margin_right_auto = true;
                style.margin_left_calc = Option::None;
                style.margin_right_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                let calc = Some((px, pct));
                style.margin_left = 0;
                style.margin_right = 0;
                style.margin_left_calc = calc;
                style.margin_right_calc = calc;
                style.margin_left_auto = false;
                style.margin_right_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_left = px;
                style.margin_right = px;
                style.margin_left_calc = Option::None;
                style.margin_right_calc = Option::None;
                style.margin_left_auto = false;
                style.margin_right_auto = false;
            }
        }
        Property::MarginBlock => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_top_auto = true;
                style.margin_bottom_auto = true;
                style.margin_top_calc = Option::None;
                style.margin_bottom_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                let calc = Some((px, pct));
                style.margin_top = 0;
                style.margin_bottom = 0;
                style.margin_top_calc = calc;
                style.margin_bottom_calc = calc;
                style.margin_top_auto = false;
                style.margin_bottom_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_top = px;
                style.margin_bottom = px;
                style.margin_top_calc = Option::None;
                style.margin_bottom_calc = Option::None;
                style.margin_top_auto = false;
                style.margin_bottom_auto = false;
            }
        }
        // Parsed but not visually applied (accepted to prevent "unknown property" skips)
        Property::Resize => {}
    }
}

// ---------------------------------------------------------------------------

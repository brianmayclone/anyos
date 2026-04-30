fn parse_declarations(p: &mut Parser) -> Vec<Declaration> {
    let mut decls = Vec::new();

    loop {
        p.skip_whitespace();
        if p.eof() || p.peek() == b'}' {
            break;
        }

        // Check for CSS nesting: if the next char is a selector start (.#&*[)
        // or if we see a combinator, skip to the nested block.
        let ch = p.peek();
        if ch == b'.'
            || ch == b'#'
            || ch == b'&'
            || ch == b'*'
            || ch == b'['
            || ch == b'>'
            || ch == b'+'
            || ch == b'~'
        {
            // Skip nested rule (CSS nesting).
            while !p.eof() && p.peek() != b'{' && p.peek() != b'}' {
                p.pos += 1;
            }
            if p.peek() == b'{' {
                p.skip_block();
            }
            continue;
        }

        let prop_name = p.read_ident();
        if prop_name.is_empty() {
            // Skip garbage character
            p.pos += 1;
            continue;
        }

        p.skip_whitespace();
        if p.peek() != b':' {
            // Could be a nested rule (CSS nesting) — skip the entire block.
            // Also handles selectors that look like property names (e.g. ".child { ... }").
            while !p.eof() && p.peek() != b';' && p.peek() != b'}' && p.peek() != b'{' {
                p.pos += 1;
            }
            if p.peek() == b'{' {
                p.skip_block(); // Skip the nested { ... } block
            } else if p.peek() == b';' {
                p.pos += 1;
            }
            continue;
        }
        p.pos += 1; // consume ':'

        p.skip_whitespace();

        // Read value until ';' or '}'
        let value_str = read_value_str(p);

        if p.peek() == b';' {
            p.pos += 1;
        }

        let trimmed = value_str.trim();
        let (trimmed, important) = strip_important(trimmed);
        let decl_ast = CssDeclarationAst {
            name: prop_name,
            value: parse_value_ast(trimmed),
            important,
        };
        for decl in lower_declaration_ast(&decl_ast) {
            decls.push(decl);
        }
    }

    decls
}

/// Strip `!important` from end of a CSS value string.
fn strip_important(s: &str) -> (&str, bool) {
    let bytes = s.as_bytes();
    if bytes.len() < 10 {
        return (s, false);
    }
    // Check last 10 chars case-insensitively for "!important"
    let end = &bytes[bytes.len() - 10..];
    let matches = end[0] == b'!'
        && (end[1] == b'i' || end[1] == b'I')
        && (end[2] == b'm' || end[2] == b'M')
        && (end[3] == b'p' || end[3] == b'P')
        && (end[4] == b'o' || end[4] == b'O')
        && (end[5] == b'r' || end[5] == b'R')
        && (end[6] == b't' || end[6] == b'T')
        && (end[7] == b'a' || end[7] == b'A')
        && (end[8] == b'n' || end[8] == b'N')
        && (end[9] == b't' || end[9] == b'T');
    if matches {
        let trimmed = s[..s.len() - 10].trim_end();
        (trimmed, true)
    } else {
        (s, false)
    }
}

fn read_value_str(p: &mut Parser) -> String {
    let start = p.pos;
    let mut paren_depth: u32 = 0;
    while !p.eof() {
        let ch = p.peek();
        if ch == b'(' {
            paren_depth += 1;
            p.pos += 1;
        } else if ch == b')' {
            if paren_depth > 0 {
                paren_depth -= 1;
            }
            p.pos += 1;
        } else if (ch == b';' || ch == b'}') && paren_depth == 0 {
            break;
        } else {
            p.pos += 1;
        }
    }
    let bytes = &p.input[start..p.pos];
    String::from_utf8_lossy(bytes).into_owned()
}

// ---------------------------------------------------------------------------
// Inline style parser
// ---------------------------------------------------------------------------

pub fn parse_inline_style(style: &str) -> Vec<Declaration> {
    let mut p = Parser::new(style);
    parse_declarations(&mut p)
}

// ---------------------------------------------------------------------------
// Property name matching
// ---------------------------------------------------------------------------

pub fn parse_property(name: &str) -> Option<Property> {
    // Convert to lowercase for comparison
    let mut buf = [0u8; 40];
    let len = name.len().min(40);
    for (i, &b) in name.as_bytes()[..len].iter().enumerate() {
        buf[i] = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
    }
    let lower = core::str::from_utf8(&buf[..len]).ok()?;

    match lower {
        "display" => Some(Property::Display),
        "color" | "-webkit-text-fill-color" => Some(Property::Color),
        "background-color" => Some(Property::BackgroundColor),
        "background" => Some(Property::Background),
        "font-size" => Some(Property::FontSize),
        "font-weight" => Some(Property::FontWeight),
        "font-style" => Some(Property::FontStyle),
        "direction" => Some(Property::Direction),
        "writing-mode" => Some(Property::WritingMode),
        "text-align" => Some(Property::TextAlign),
        "text-decoration" => Some(Property::TextDecoration),
        "text-indent" => Some(Property::TextIndent),
        "line-height" => Some(Property::LineHeight),
        "vertical-align" => Some(Property::VerticalAlign),
        "width" => Some(Property::Width),
        "height" => Some(Property::Height),
        "max-width" => Some(Property::MaxWidth),
        "min-width" => Some(Property::MinWidth),
        "max-height" => Some(Property::MaxHeight),
        "min-height" => Some(Property::MinHeight),
        "margin" => Some(Property::Margin),
        "margin-top" => Some(Property::MarginTop),
        "margin-right" => Some(Property::MarginRight),
        "margin-bottom" => Some(Property::MarginBottom),
        "margin-left" => Some(Property::MarginLeft),
        "padding" => Some(Property::Padding),
        "padding-top" => Some(Property::PaddingTop),
        "padding-right" => Some(Property::PaddingRight),
        "padding-bottom" => Some(Property::PaddingBottom),
        "padding-left" => Some(Property::PaddingLeft),
        "border" => Some(Property::Border),
        "border-top" => Some(Property::BorderTop),
        "border-right" => Some(Property::BorderRight),
        "border-bottom" => Some(Property::BorderBottom),
        "border-left" => Some(Property::BorderLeft),
        "border-color" => Some(Property::BorderColor),
        "border-width" => Some(Property::BorderWidth),
        "border-style" => Some(Property::BorderStyle),
        "border-radius" => Some(Property::BorderRadius),
        "border-collapse" => Some(Property::BorderCollapse),
        "border-spacing" => Some(Property::BorderSpacing),
        // Per-side border width
        "border-top-width" => Some(Property::BorderTopWidth),
        "border-right-width" => Some(Property::BorderRightWidth),
        "border-bottom-width" => Some(Property::BorderBottomWidth),
        "border-left-width" => Some(Property::BorderLeftWidth),
        // Per-side border color
        "border-top-color" => Some(Property::BorderTopColor),
        "border-right-color" => Some(Property::BorderRightColor),
        "border-bottom-color" => Some(Property::BorderBottomColor),
        "border-left-color" => Some(Property::BorderLeftColor),
        // Per-side border style
        "border-top-style" => Some(Property::BorderTopStyle),
        "border-right-style" => Some(Property::BorderRightStyle),
        "border-bottom-style" => Some(Property::BorderBottomStyle),
        "border-left-style" => Some(Property::BorderLeftStyle),
        // Per-corner border radius
        "border-top-left-radius" => Some(Property::BorderTopLeftRadius),
        "border-top-right-radius" => Some(Property::BorderTopRightRadius),
        "border-bottom-right-radius" => Some(Property::BorderBottomRightRadius),
        "border-bottom-left-radius" => Some(Property::BorderBottomLeftRadius),
        "list-style-type" => Some(Property::ListStyleType),
        "list-style" => Some(Property::ListStyleType),
        "list-style-position" => Some(Property::ListStylePosition),
        "white-space" => Some(Property::WhiteSpace),
        "overflow" => Some(Property::Overflow),
        "overflow-x" => Some(Property::OverflowX),
        "overflow-y" => Some(Property::OverflowY),
        // Positioning
        "position" => Some(Property::Position),
        "top" => Some(Property::Top),
        "right" => Some(Property::Right),
        "bottom" => Some(Property::Bottom),
        "left" => Some(Property::Left),
        "z-index" => Some(Property::ZIndex),
        // Flexbox
        "flex-direction" => Some(Property::FlexDirection),
        "flex-wrap" => Some(Property::FlexWrap),
        "flex-flow" => Some(Property::FlexFlow),
        "justify-content" => Some(Property::JustifyContent),
        "align-items" => Some(Property::AlignItems),
        "align-self" => Some(Property::AlignSelf),
        "justify-self" => Some(Property::JustifySelf),
        "place-items" => Some(Property::PlaceItems),
        "place-self" => Some(Property::PlaceSelf),
        "place-content" => Some(Property::PlaceContent),
        "align-content" => Some(Property::AlignContent),
        "flex-grow" => Some(Property::FlexGrow),
        "flex-shrink" => Some(Property::FlexShrink),
        "flex-basis" => Some(Property::FlexBasis),
        "flex" => Some(Property::Flex),
        "gap" => Some(Property::Gap),
        "row-gap" => Some(Property::RowGap),
        "column-gap" => Some(Property::ColumnGap),
        "order" => Some(Property::Order),
        // Box model
        "box-sizing" => Some(Property::BoxSizing),
        // Float
        "float" => Some(Property::Float),
        "clear" => Some(Property::Clear),
        // Visual
        "opacity" => Some(Property::Opacity),
        "visibility" => Some(Property::Visibility),
        "text-transform" => Some(Property::TextTransform),
        "cursor" => Some(Property::Cursor),
        "table-layout" => Some(Property::TableLayout),
        // Typography
        "font-family" => Some(Property::FontFamily),
        "letter-spacing" => Some(Property::LetterSpacing),
        "word-spacing" => Some(Property::WordSpacing),
        "word-break" => Some(Property::WordBreak),
        "overflow-wrap" | "word-wrap" => Some(Property::OverflowWrap),
        "text-overflow" => Some(Property::TextOverflow),
        // Outline
        "outline" => Some(Property::Outline),
        "outline-color" => Some(Property::OutlineColor),
        "outline-style" => Some(Property::OutlineStyle),
        "outline-width" => Some(Property::OutlineWidth),
        "outline-offset" => Some(Property::OutlineOffset),
        // Shadows
        "box-shadow" => Some(Property::BoxShadow),
        "text-shadow" => Some(Property::TextShadow),
        // Background extensions
        "background-image" => Some(Property::BackgroundImage),
        "background-position" => Some(Property::BackgroundPosition),
        "background-repeat" => Some(Property::BackgroundRepeat),
        "background-size" => Some(Property::BackgroundSize),
        // Transform
        "transform" => Some(Property::Transform),
        "transform-origin" => Some(Property::TransformOrigin),
        // Content
        "content" => Some(Property::Content),
        "object-fit" => Some(Property::ObjectFit),
        // Filter
        "filter" | "-webkit-filter" => Some(Property::Filter),
        // Layout
        "aspect-ratio" => Some(Property::AspectRatio),
        "inset" => Some(Property::Inset),
        "clip-path" | "-webkit-clip-path" => Some(Property::ClipPath),
        "clip" => Some(Property::Clip),
        // Text decoration sub-properties
        "text-decoration-color" => Some(Property::TextDecorationColor),
        "text-decoration-style" => Some(Property::TextDecorationStyle),
        "text-decoration-thickness" => Some(Property::TextDecorationThickness),
        "text-underline-offset" => Some(Property::TextUnderlineOffset),
        // Typography extras
        "font-variant" => Some(Property::FontVariant),
        "tab-size" | "-moz-tab-size" => Some(Property::TabSize),
        // Counters
        "counter-reset" => Some(Property::CounterReset),
        "counter-increment" => Some(Property::CounterIncrement),
        // Transitions
        "transition" => Some(Property::Transition),
        "transition-property" => Some(Property::TransitionProperty),
        "transition-duration" => Some(Property::TransitionDuration),
        "transition-timing-function" => Some(Property::TransitionTimingFunction),
        "transition-delay" => Some(Property::TransitionDelay),
        // Animations
        "animation" => Some(Property::Animation),
        "animation-name" => Some(Property::AnimationName),
        "animation-duration" => Some(Property::AnimationDuration),
        "animation-timing-function" => Some(Property::AnimationTimingFunction),
        "animation-delay" => Some(Property::AnimationDelay),
        "animation-iteration-count" => Some(Property::AnimationIterationCount),
        "animation-direction" => Some(Property::AnimationDirection),
        "animation-fill-mode" => Some(Property::AnimationFillMode),
        "animation-play-state" => Some(Property::AnimationPlayState),
        // Grid
        "grid-template-columns" => Some(Property::GridTemplateColumns),
        "grid-template-rows" => Some(Property::GridTemplateRows),
        "grid-template-areas" => Some(Property::GridTemplateAreas),
        "grid-template" => Some(Property::GridTemplate),
        "grid-auto-columns" => Some(Property::GridAutoColumns),
        "grid-auto-rows" => Some(Property::GridAutoRows),
        "grid-auto-flow" => Some(Property::GridAutoFlow),
        "justify-items" => Some(Property::JustifyItems),
        "grid-column" => Some(Property::GridColumn),
        "grid-column-start" => Some(Property::GridColumnStart),
        "grid-column-end" => Some(Property::GridColumnEnd),
        "grid-row" => Some(Property::GridRow),
        "grid-row-start" => Some(Property::GridRowStart),
        "grid-row-end" => Some(Property::GridRowEnd),
        "grid-area" => Some(Property::GridArea),
        // Mask
        "mask-image" | "-webkit-mask-image" | "mask" | "-webkit-mask" => Some(Property::MaskImage),
        "mask-position" | "-webkit-mask-position" => Some(Property::MaskPosition),
        "mask-repeat" | "-webkit-mask-repeat" => Some(Property::MaskRepeat),
        "mask-size" | "-webkit-mask-size" => Some(Property::MaskSize),
        "mask-clip" | "-webkit-mask-clip" => Some(Property::MaskClip),
        "mask-origin" | "-webkit-mask-origin" => Some(Property::MaskOrigin),
        // Pointer events
        "pointer-events" => Some(Property::PointerEvents),
        // User interaction
        "user-select" | "-webkit-user-select" | "-moz-user-select" | "-ms-user-select" => {
            Some(Property::UserSelect)
        }
        // Backdrop filter
        "backdrop-filter" | "-webkit-backdrop-filter" => Some(Property::BackdropFilter),
        // CSS Logical Properties
        "padding-inline" => Some(Property::PaddingInline),
        "padding-inline-start" => Some(Property::PaddingLeft),
        "padding-inline-end" => Some(Property::PaddingRight),
        "padding-block" => Some(Property::PaddingBlock),
        "padding-block-start" => Some(Property::PaddingTop),
        "padding-block-end" => Some(Property::PaddingBottom),
        "margin-inline" => Some(Property::MarginInline),
        "margin-inline-start" => Some(Property::MarginLeft),
        "margin-inline-end" => Some(Property::MarginRight),
        "margin-block" => Some(Property::MarginBlock),
        "margin-block-start" => Some(Property::MarginTop),
        "margin-block-end" => Some(Property::MarginBottom),
        "inset-inline" => Some(Property::InsetInline),
        "inset-inline-start" => Some(Property::Left),
        "inset-inline-end" => Some(Property::Right),
        "inset-block" => Some(Property::InsetBlock),
        "inset-block-start" => Some(Property::Top),
        "inset-block-end" => Some(Property::Bottom),
        "border-inline-width" => Some(Property::BorderWidth),
        "border-block-width" => Some(Property::BorderWidth),
        // Font shorthand
        "font" => Some(Property::FontFamily),
        // Additional properties
        "appearance" | "-webkit-appearance" | "-moz-appearance" => Some(Property::Appearance),
        "accent-color" => Some(Property::AccentColor),
        "background-clip" | "-webkit-background-clip" => Some(Property::BackgroundClip),
        "color-scheme" => Some(Property::ColorScheme),
        "container-type" => Some(Property::ContainerType),
        "container-name" => Some(Property::ContainerName),
        "text-decoration-line" => Some(Property::TextDecoration),
        "scroll-behavior" => Some(Property::ScrollBehavior),
        "resize" => Some(Property::Resize),
        "object-position" => Some(Property::ObjectPosition),
        "translate" => Some(Property::Translate),
        "scale" => Some(Property::Scale),
        "rotate" => Some(Property::Rotate),
        _ => Option::None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_inline_style;
    use crate::css::{CssValue, Property};

    #[test]
    fn inline_font_shorthand_expands_family_size_and_line_height() {
        let decls = parse_inline_style("font: 5px/1 Ahem");
        assert!(decls.iter().any(|d| {
            d.property == Property::FontSize && matches!(d.value, CssValue::Length(_, _))
        }));
        assert!(decls.iter().any(|d| {
            d.property == Property::LineHeight && matches!(d.value, CssValue::Number(100))
        }));
        assert!(decls.iter().any(|d| {
            d.property == Property::FontFamily
                && matches!(d.value, CssValue::Keyword(ref kw) if kw == "Ahem")
        }));
    }

    #[test]
    fn css_math_lengths_keep_fixed_point_scale() {
        let decls = parse_inline_style("font-size: clamp(48px, 80px, 112px); width: min(24px, 32px); height: max(12px, 18px)");
        assert!(decls.iter().any(|d| {
            d.property == Property::FontSize
                && matches!(d.value, CssValue::Length(8000, crate::css::Unit::Px))
        }));
        assert!(decls.iter().any(|d| {
            d.property == Property::Width
                && matches!(d.value, CssValue::Length(2400, crate::css::Unit::Px))
        }));
        assert!(decls.iter().any(|d| {
            d.property == Property::Height
                && matches!(d.value, CssValue::Length(1800, crate::css::Unit::Px))
        }));
    }
}

// ---------------------------------------------------------------------------
// Value parser
// ---------------------------------------------------------------------------

// Inheritance (only unset inheritable properties)
// ---------------------------------------------------------------------------

fn inherit_unset(child: &mut ComputedStyle, parent: &ComputedStyle, set: u32) {
    if set & SET_COLOR == 0 {
        child.color = parent.color;
    }
    if set & SET_FONT_SIZE == 0 {
        child.font_size = parent.font_size;
    }
    if set & SET_FONT_WEIGHT == 0 {
        child.font_weight = parent.font_weight;
    }
    if set & SET_FONT_STYLE == 0 {
        child.font_style = parent.font_style;
    }
    if set & SET_FONT_FAMILY == 0 {
        child.font_family = parent.font_family.clone();
    }
    if set & SET_DIRECTION == 0 {
        child.direction = parent.direction;
    }
    if set & SET_WRITING_MODE == 0 {
        child.writing_mode = parent.writing_mode;
    }
    if set & SET_TEXT_ALIGN == 0 {
        child.text_align = parent.text_align;
    }
    if set & SET_LINE_HEIGHT == 0 {
        child.line_height = parent.line_height;
    }
    if set & SET_WHITE_SPACE == 0 {
        child.white_space = parent.white_space;
    }
    if set & SET_LIST_STYLE == 0 {
        child.list_style = parent.list_style;
    }
    if set & SET_LIST_STYLE_POS == 0 {
        child.list_style_position = parent.list_style_position;
    }
    if set & SET_TEXT_DECO == 0 {
        child.text_decoration = parent.text_decoration;
    }
    if set & SET_VISIBILITY == 0 {
        child.visibility = parent.visibility;
    }
    if set & SET_TEXT_TRANSFORM == 0 {
        child.text_transform = parent.text_transform;
    }
    if set & SET_LETTER_SPACING == 0 {
        child.letter_spacing = parent.letter_spacing;
    }
    if set & SET_WORD_SPACING == 0 {
        child.word_spacing = parent.word_spacing;
    }
    if set & SET_WORD_BREAK == 0 {
        child.word_break = parent.word_break;
    }
    if set & SET_OVERFLOW_WRAP == 0 {
        child.overflow_wrap = parent.overflow_wrap;
    }
    if set & SET_ACCENT_COLOR == 0 {
        child.accent_color = parent.accent_color;
    }
    if set & SET_COLOR_SCHEME == 0 {
        child.color_scheme = parent.color_scheme;
    }
    if set & SET_POINTER_EVENTS == 0 {
        child.pointer_events = parent.pointer_events;
    }
}

fn inherit_unset_in_dom_order(dom: &Dom, styles: &mut [ComputedStyle], set_flags: &[u32]) {
    fn walk(dom: &Dom, styles: &mut [ComputedStyle], set_flags: &[u32], node_id: usize) {
        if node_id >= styles.len() {
            return;
        }
        let parent_style = dom.nodes[node_id]
            .parent
            .and_then(|pid| styles.get(pid))
            .cloned();
        if let Some(parent) = parent_style {
            let flags = set_flags.get(node_id).copied().unwrap_or(0);
            inherit_unset(&mut styles[node_id], &parent, flags);
            if flags & SET_LINE_HEIGHT == 0 {
                styles[node_id].line_height = (styles[node_id].font_size * 6 + 2) / 5;
            }
        }

        let children = dom.nodes[node_id].children.clone();
        for child_id in children {
            walk(dom, styles, set_flags, child_id);
        }
    }

    for (node_id, node) in dom.nodes.iter().enumerate() {
        if node.parent.is_none() {
            walk(dom, styles, set_flags, node_id);
        }
    }
}

/// Map a CSS property to the inheritable-set bitflag (0 if not inheritable).
fn decl_set_flag(prop: &Property) -> u32 {
    match prop {
        Property::Color => SET_COLOR,
        Property::AccentColor => SET_ACCENT_COLOR,
        Property::ColorScheme => SET_COLOR_SCHEME,
        Property::FontSize => SET_FONT_SIZE,
        Property::FontWeight => SET_FONT_WEIGHT,
        Property::FontStyle => SET_FONT_STYLE,
        Property::FontFamily => SET_FONT_FAMILY,
        Property::Direction => SET_DIRECTION,
        Property::WritingMode => SET_WRITING_MODE,
        Property::TextAlign => SET_TEXT_ALIGN,
        Property::LineHeight => SET_LINE_HEIGHT,
        Property::WhiteSpace => SET_WHITE_SPACE,
        Property::ListStyleType => SET_LIST_STYLE,
        Property::ListStylePosition => SET_LIST_STYLE_POS,
        Property::TextDecoration => SET_TEXT_DECO,
        Property::Visibility => SET_VISIBILITY,
        Property::TextTransform => SET_TEXT_TRANSFORM,
        Property::LetterSpacing => SET_LETTER_SPACING,
        Property::WordSpacing => SET_WORD_SPACING,
        Property::WordBreak => SET_WORD_BREAK,
        Property::OverflowWrap => SET_OVERFLOW_WRAP,
        Property::PointerEvents => SET_POINTER_EVENTS,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------

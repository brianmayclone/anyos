/// User-agent stylesheet: hardcoded browser defaults per HTML tag.
/// Returns the base style AND a bitfield indicating which inheritable
/// properties the UA explicitly sets (so inheritance does not clobber them).
fn ua_style_and_flags(tag: Tag) -> (ComputedStyle, u32) {
    let mut s = default_style();
    let mut flags: u32 = 0;
    match tag {
        Tag::Body => {
            s.margin_top = 8;
            s.margin_right = 8;
            s.margin_bottom = 8;
            s.margin_left = 8;
        }
        Tag::H1 => {
            s.font_size = 32;
            s.font_weight = FontWeight::Bold;
            s.margin_top = 21;
            s.margin_bottom = 21;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H2 => {
            s.font_size = 24;
            s.font_weight = FontWeight::Bold;
            s.margin_top = 19;
            s.margin_bottom = 19;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H3 => {
            s.font_size = 19;
            s.font_weight = FontWeight::Bold;
            s.margin_top = 18;
            s.margin_bottom = 18;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H4 => {
            s.font_size = 16;
            s.font_weight = FontWeight::Bold;
            s.margin_top = 21;
            s.margin_bottom = 21;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H5 => {
            s.font_size = 13;
            s.font_weight = FontWeight::Bold;
            s.margin_top = 22;
            s.margin_bottom = 22;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H6 => {
            s.font_size = 11;
            s.font_weight = FontWeight::Bold;
            s.margin_top = 24;
            s.margin_bottom = 24;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::P => {
            s.margin_top = 16;
            s.margin_bottom = 16;
        }
        Tag::A => {
            s.display = Display::Inline;
            s.color = 0xFF007AFF;
            s.text_decoration = TextDeco::Underline;
            flags |= SET_COLOR | SET_TEXT_DECO;
        }
        Tag::Em | Tag::I => {
            s.display = Display::Inline;
            s.font_style = FontStyleVal::Italic;
            flags |= SET_FONT_STYLE;
        }
        Tag::Strong | Tag::B => {
            s.display = Display::Inline;
            s.font_weight = FontWeight::Bold;
            flags |= SET_FONT_WEIGHT;
        }
        Tag::U => {
            s.display = Display::Inline;
            s.text_decoration = TextDeco::Underline;
            flags |= SET_TEXT_DECO;
        }
        Tag::Code => {
            s.display = Display::Inline;
        }
        Tag::Pre => {
            s.white_space = WhiteSpace::Pre;
            flags |= SET_WHITE_SPACE;
        }
        Tag::Blockquote => {
            s.margin_left = 40;
        }
        Tag::Ul => {
            s.margin_top = 16;
            s.margin_bottom = 16;
            s.padding_left = 40;
            // UA list-style: disc is inherited by <li> children.
            // Setting the flag here prevents <ul> from inheriting list-style from its
            // ancestors; <li> children inherit from <ul> because <li> has no flag.
            s.list_style = ListStyle::Disc;
            flags |= SET_LIST_STYLE;
        }
        Tag::Ol => {
            s.margin_top = 16;
            s.margin_bottom = 16;
            s.padding_left = 40;
            s.list_style = ListStyle::Decimal;
            flags |= SET_LIST_STYLE;
        }
        Tag::Li => {
            s.display = Display::ListItem;
            // No SET_LIST_STYLE flag: <li> inherits list-style from its parent (<ul>/<ol>).
            // This allows `list-style: none` on the parent to propagate via CSS inheritance.
            s.list_style = ListStyle::Disc; // fallback if orphan (no <ul>/<ol> parent)
        }
        Tag::Hr => {
            s.border_width = 1;
            s.margin_top = 8;
            s.margin_bottom = 8;
        }
        Tag::Img | Tag::Picture | Tag::Br | Tag::Span | Tag::Label => {
            s.display = Display::Inline;
        }
        Tag::Button => {
            // HTML form controls are replaced/flow-root style inline boxes in
            // browser UA styles. Treating rich <button> content as plain inline
            // breaks modern pill controls such as Google's KI button, where
            // absolutely positioned layers and centered children depend on the
            // button having its own inline-block box.
            //
            // We model styled rich buttons as inline-flex: this matches the
            // visual behavior sites generally rely on from native buttons
            // (content centered in the control) while still allowing author
            // CSS to override `display`.
            s.display = Display::InlineFlex;
            s.align_items = AlignItems::Center;
            s.justify_content = JustifyContent::Center;
            s.box_sizing = BoxSizing::BorderBox;
        }
        Tag::Input | Tag::Select | Tag::Textarea => {
            s.display = Display::InlineBlock;
        }
        Tag::Table => {}
        Tag::Tr => {
            s.display = Display::TableRow;
        }
        Tag::Td => {
            s.display = Display::TableCell;
        }
        Tag::Th => {
            s.display = Display::TableCell;
            s.font_weight = FontWeight::Bold;
            flags |= SET_FONT_WEIGHT;
        }
        Tag::Head
        | Tag::Title
        | Tag::Meta
        | Tag::Link
        | Tag::Style
        | Tag::Script
        | Tag::Noscript
        | Tag::Template => {
            s.display = Display::None;
        }
        // Inline semantic text elements
        Tag::Small => {
            s.display = Display::Inline;
            s.font_size = 13;
            flags |= SET_FONT_SIZE;
        }
        Tag::S | Tag::Del => {
            s.display = Display::Inline;
            s.text_decoration = TextDeco::LineThrough;
            flags |= SET_TEXT_DECO;
        }
        Tag::Ins => {
            s.display = Display::Inline;
            s.text_decoration = TextDeco::Underline;
            flags |= SET_TEXT_DECO;
        }
        Tag::Mark => {
            s.display = Display::Inline;
            s.background_color = 0xFFFFFF00; // yellow highlight
            s.color = 0xFF000000;
            flags |= SET_COLOR;
        }
        Tag::Sub
        | Tag::Sup
        | Tag::Kbd
        | Tag::Samp
        | Tag::Var
        | Tag::Abbr
        | Tag::Cite
        | Tag::Dfn
        | Tag::Q
        | Tag::Time
        | Tag::Bdi
        | Tag::Bdo
        | Tag::Data
        | Tag::Ruby
        | Tag::Rt
        | Tag::Rp
        | Tag::Wbr
        | Tag::Nobr
        | Tag::Tt => {
            s.display = Display::Inline;
        }
        // Definition list
        Tag::Dl => {
            s.margin_top = 16;
            s.margin_bottom = 16;
        }
        Tag::Dt => {
            s.font_weight = FontWeight::Bold;
            flags |= SET_FONT_WEIGHT;
        }
        Tag::Dd => {
            s.margin_left = 40;
        }
        // Figure
        Tag::Figure => {
            s.margin_top = 16;
            s.margin_bottom = 16;
            s.margin_left = 40;
            s.margin_right = 40;
        }
        Tag::Figcaption => {
            s.text_align = TextAlignVal::Center;
            flags |= SET_TEXT_ALIGN;
        }
        // Details/Summary
        Tag::Details => {}
        Tag::Summary => {
            s.display = Display::Block;
            s.font_weight = FontWeight::Bold;
            flags |= SET_FONT_WEIGHT;
        }
        // Dialog
        Tag::Dialog => {
            s.display = Display::Block;
            s.position = Position::Absolute;
        }
        // Sectioning
        Tag::Aside | Tag::Hgroup | Tag::Address => {}
        // Table extensions
        Tag::Tfoot => {
            s.display = Display::TableRow;
        }
        Tag::Caption => {
            s.text_align = TextAlignVal::Center;
            flags |= SET_TEXT_ALIGN;
        }
        // Form elements
        Tag::Fieldset => {
            s.border_width = 1;
            s.padding_top = 8;
            s.padding_right = 12;
            s.padding_bottom = 8;
            s.padding_left = 12;
        }
        Tag::Legend => {
            s.display = Display::Inline;
            s.font_weight = FontWeight::Bold;
            flags |= SET_FONT_WEIGHT;
        }
        Tag::Optgroup => {}
        Tag::Datalist | Tag::Output => {
            s.display = Display::Inline;
        }
        Tag::Progress | Tag::Meter => {
            s.display = Display::Inline;
        }
        // Deprecated
        Tag::Center => {
            s.text_align = TextAlignVal::Center;
            flags |= SET_TEXT_ALIGN;
        }
        Tag::Font => {
            s.display = Display::Inline;
        }
        Tag::Marquee => {
            s.display = Display::InlineBlock;
            s.overflow_x = OverflowVal::Hidden;
            s.overflow_y = OverflowVal::Hidden;
            s.white_space = WhiteSpace::Nowrap;
            flags |= SET_WHITE_SPACE;
        }
        // Block-level elements that just use defaults.
        Tag::Div
        | Tag::Section
        | Tag::Article
        | Tag::Header
        | Tag::Footer
        | Tag::Nav
        | Tag::Main
        | Tag::Form
        | Tag::Thead
        | Tag::Tbody => {}
        // Custom/unknown elements (Web Components etc.) default to inline per HTML spec.
        // CSS can always override with display:block/flex/grid as needed.
        Tag::Unknown => {
            s.display = Display::Inline;
        }
        _ => {}
    }
    (s, flags)
}

/// Public convenience: returns only the `ComputedStyle` (no flags).
pub fn user_agent_styles(tag: Tag) -> ComputedStyle {
    ua_style_and_flags(tag).0
}

// ---------------------------------------------------------------------------

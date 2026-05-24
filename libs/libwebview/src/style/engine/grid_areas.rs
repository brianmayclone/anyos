// Grid template areas parsing
// ---------------------------------------------------------------------------

/// Parse `grid-template-areas` value into named grid areas.
/// Example: `'header header' 'sidebar content' 'footer footer'`
/// Returns a list of GridArea with 1-based line numbers.
fn parse_grid_template_areas_value(s: &str) -> Vec<GridArea> {
    let mut areas: Vec<GridArea> = Vec::new();
    let mut row: i32 = 1;

    // Extract each quoted row string.
    let mut pos = 0;
    let bytes = s.as_bytes();
    while pos < bytes.len() {
        // Find start of quoted string.
        if bytes[pos] == b'\'' || bytes[pos] == b'"' {
            let quote = bytes[pos];
            pos += 1;
            let start = pos;
            while pos < bytes.len() && bytes[pos] != quote {
                pos += 1;
            }
            let row_str = core::str::from_utf8(&bytes[start..pos]).unwrap_or("");
            if pos < bytes.len() {
                pos += 1;
            } // skip closing quote

            // Parse cells in this row.
            let cells: Vec<&str> = row_str.split_whitespace().collect();
            for (col_idx, &name) in cells.iter().enumerate() {
                if name == "." {
                    continue;
                } // empty cell
                let col = col_idx as i32 + 1; // 1-based

                // Check if this area already exists — extend it.
                if let Some(existing) = areas.iter_mut().find(|a| a.name == name) {
                    // Extend the area to cover this cell.
                    if row + 1 > existing.row_end {
                        existing.row_end = row + 1;
                    }
                    if col + 1 > existing.col_end {
                        existing.col_end = col + 1;
                    }
                    if row < existing.row_start {
                        existing.row_start = row;
                    }
                    if col < existing.col_start {
                        existing.col_start = col;
                    }
                } else {
                    areas.push(GridArea {
                        name: String::from(name),
                        row_start: row,
                        col_start: col,
                        row_end: row + 1,
                        col_end: col + 1,
                    });
                }
            }
            row += 1;
        } else {
            pos += 1;
        }
    }
    areas
}

#[cfg(test)]
mod layout_regression_tests {
    use super::*;

    #[test]
    fn resolves_negative_margins_for_replaced_elements() {
        let dom = crate::html::parse(r#"<img id="t1" src="x">"#);
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            img { margin: 10px; }
            #t1 {
                padding-left: 20px;
                margin-left: -10px;
                padding-bottom: 20px;
                margin-bottom: -10px;
            }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 800, 600, &mut inline_style_cache);
        let img_id = dom
            .nodes
            .iter()
            .position(|node| matches!(node.node_type, NodeType::Element { tag: Tag::Img, .. }))
            .expect("img node");
        let style = &styles[img_id];

        assert_eq!(style.margin_top, 10);
        assert_eq!(style.margin_right, 10);
        assert_eq!(style.margin_bottom, -10);
        assert_eq!(style.margin_left, -10);
        assert_eq!(style.padding_left, 20);
        assert_eq!(style.padding_bottom, 20);
    }

    #[test]
    fn calc_division_by_negative_number_after_var_resolution() {
        let dom = crate::html::parse(r#"<section id="home"></section>"#);
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            :root {
                --viewport-width: 800px;
                --viewport-height: 600px;
                --padding-width: 15px;
                --border-width: 6px;
            }
            section {
                width: var(--viewport-width);
                height: var(--viewport-height);
                margin-top: calc(var(--viewport-height) / -2 - var(--padding-width) - var(--border-width));
                margin-left: calc(var(--viewport-width) / -2 - var(--padding-width) - var(--border-width));
            }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 900, &mut inline_style_cache);
        let section_id = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    node.node_type,
                    NodeType::Element {
                        tag: Tag::Section,
                        ..
                    }
                )
            })
            .expect("section node");
        let style = &styles[section_id];

        assert_eq!(style.width, Some(800));
        assert_eq!(style.height, Some(600));
        assert_eq!(style.margin_top, -321);
        assert_eq!(style.margin_left, -421);
    }

    #[test]
    fn border_shorthand_resolves_var_width_and_var_color() {
        let dom = crate::html::parse(r#"<section id="home"></section>"#);
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            :root {
                --border-width: 6px;
                --foreground: rgb(235, 235, 235);
            }
            section {
                border: var(--border-width) solid var(--foreground);
            }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 900, &mut inline_style_cache);
        let section_id = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    node.node_type,
                    NodeType::Element {
                        tag: Tag::Section,
                        ..
                    }
                )
            })
            .expect("section node");
        let style = &styles[section_id];

        assert_eq!(style.border_width, 6);
        assert_eq!(style.border_top.width, 6);
        assert!(matches!(style.border_top.style, BorderStyleVal::Solid));
        assert_eq!(style.border_top.color, 0xFFEBEBEB);
    }

    #[test]
    fn picture_defaults_to_inline_like_replaced_media_container() {
        let dom = crate::html::parse(r#"<picture><img src="x"></picture>"#);
        let stylesheet = crate::css::parse_stylesheet("");
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 800, 600, &mut inline_style_cache);
        let picture_id = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    node.node_type,
                    NodeType::Element {
                        tag: Tag::Picture,
                        ..
                    }
                )
            })
            .expect("picture node");

        assert!(matches!(styles[picture_id].display, Display::Inline));
    }

    #[test]
    fn early_inheritance_seed_preserves_explicit_ua_styles() {
        let dom = crate::html::parse(r#"<center><div id="child">x</div></center><h1>Title</h1>"#);
        let stylesheet = crate::css::parse_stylesheet("center { background: #eee; }");
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 800, 600, &mut inline_style_cache);

        let child_id = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { tag: Tag::Div, attrs }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "child")
                )
            })
            .expect("child div");
        let h1_id = dom
            .nodes
            .iter()
            .position(|node| matches!(node.node_type, NodeType::Element { tag: Tag::H1, .. }))
            .expect("h1");

        assert!(matches!(styles[child_id].text_align, TextAlignVal::Center));
        assert_eq!(styles[h1_id].font_size, 32);
        assert!(matches!(styles[h1_id].font_weight, FontWeight::Bold));
    }

    #[test]
    fn dialog_without_open_is_not_rendered() {
        let dom = crate::html::parse(r#"<dialog class="modal-dialog">Modal</dialog>"#);
        let stylesheet = crate::css::parse_stylesheet("dialog { display: block; }");
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 800, 600, &mut inline_style_cache);
        let dialog_id = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    node.node_type,
                    NodeType::Element {
                        tag: Tag::Dialog,
                        ..
                    }
                )
            })
            .expect("dialog node");

        assert!(matches!(styles[dialog_id].display, Display::None));
    }

    #[test]
    fn skip_link_is_offscreen_until_focused() {
        let dom =
            crate::html::parse(r##"<a class="skip-link" href="#main">Weiter zum Hauptinhalt</a>"##);
        let stylesheet = crate::css::parse_stylesheet(
            ".skip-link { display: flex; position: absolute; padding: 24px; }",
        );
        let prepared = PreparedStylesheets::prepare(&[&stylesheet], 800, 600);
        let link_id = dom
            .nodes
            .iter()
            .position(|node| matches!(node.node_type, NodeType::Element { tag: Tag::A, .. }))
            .expect("skip link node");
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles_prepared_with_state(
            &dom,
            &prepared,
            800,
            600,
            &mut inline_style_cache,
            &SelectorState::default(),
        );

        assert_eq!(styles[link_id].left_offset, Some(-10000));
        assert_eq!(styles[link_id].width, Some(1));
        assert_eq!(styles[link_id].height, Some(1));

        let mut focused = SelectorState::default();
        focused.focused_node = Some(link_id);
        inline_style_cache.clear();
        let (focused_styles, _) = resolve_styles_prepared_with_state(
            &dom,
            &prepared,
            800,
            600,
            &mut inline_style_cache,
            &focused,
        );

        assert_ne!(focused_styles[link_id].left_offset, Some(-10000));
        assert_eq!(focused_styles[link_id].padding_left, 24);
    }

    #[test]
    fn responsive_escaped_tailwind_classes_override_base_display() {
        let dom = crate::html::parse(
            r#"<div id="md" class="hidden md:flex"></div><div id="xl" class="hidden xl:inline"></div>"#,
        );
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            .hidden { display: none; }
            @media (min-width: 768px) { .md\:flex { display: flex; } }
            @media (min-width: 1280px) { .xl\:inline { display: inline; } }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 700, &mut inline_style_cache);
        let md_id = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "md")
                )
            })
            .expect("md node");
        let xl_id = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "xl")
                )
            })
            .expect("xl node");

        assert!(matches!(styles[md_id].display, Display::Flex));
        assert!(matches!(styles[xl_id].display, Display::Inline));
    }

    #[test]
    fn descendant_display_rule_requires_matching_ancestor() {
        let dom = crate::html::parse(
            r#"
            <div>
                <div id="closed" class="oMByyf"></div>
            </div>
            <div class="KWUYAe">
                <div id="open" class="oMByyf"></div>
            </div>
            "#,
        );
        let stylesheet = crate::css::parse_stylesheet(
            ".oMByyf { display: none; } .KWUYAe .oMByyf { display: block; }",
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 700, &mut inline_style_cache);
        let find = |id_value: &str| {
            dom.nodes
                .iter()
                .position(|node| {
                    matches!(
                        &node.node_type,
                        NodeType::Element { attrs, .. }
                            if attrs.iter().any(|a| a.name == "id" && a.value == id_value)
                    )
                })
                .expect("node")
        };

        assert!(matches!(styles[find("closed")].display, Display::None));
        assert!(matches!(styles[find("open")].display, Display::Block));
    }

    #[test]
    fn body_attribute_rule_custom_properties_inherit_to_children() {
        let dom = crate::html::parse(
            r#"
            <body data-color-brand="bild">
                <nav><span id="nav-text">STARTSEITE</span></nav>
            </body>
            "#,
        );
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            body[data-color-brand=bild] {
                --navi-font: Gotham XNarrow, Arial Narrow, sans-serif;
            }
            nav span { font-family: var(--navi-font); }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 700, &mut inline_style_cache);
        let nav_text = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "nav-text")
                )
            })
            .expect("nav text");

        assert_eq!(
            styles[nav_text].font_family.as_deref(),
            Some("Gotham XNarrow, Arial Narrow, sans-serif")
        );
    }

    #[test]
    fn font_family_inherit_copies_parent_family() {
        let dom = crate::html::parse(
            r#"
            <body data-color-brand="bild">
                <a class="nav_btn">
                    <span id="nav-text" class="nav_btn__text">KAUFBERATER SPORT</span>
                </a>
            </body>
            "#,
        );
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            body[data-color-brand=bild] {
                --label-font: Gotham XNarrow, Arial Narrow, sans-serif;
            }
            .nav_btn { font-family: var(--label-font); }
            .nav_btn__text { font-family: inherit; }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 700, &mut inline_style_cache);
        let nav_text = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "nav-text")
                )
            })
            .expect("nav text");

        assert_eq!(
            styles[nav_text].font_family.as_deref(),
            Some("Gotham XNarrow, Arial Narrow, sans-serif")
        );
    }

    #[test]
    fn custom_element_property_var_resolves_for_overflow_x() {
        let dom = crate::html::parse(
            r#"
            <a-scroll-container>
                <div id="scroller" class="scroll-container"></div>
            </a-scroll-container>
            "#,
        );
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            a-scroll-container {
                --ho-scroll-container-overflow-x: scroll;
            }
            a-scroll-container .scroll-container {
                overflow-x: var(--ho-scroll-container-overflow-x);
            }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 700, &mut inline_style_cache);
        let scroller = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "scroller")
                )
            })
            .expect("scroller node");

        assert!(matches!(styles[scroller].overflow_x, OverflowVal::Scroll));
    }

    #[test]
    fn layered_root_custom_properties_resolve_for_tailwind_utilities() {
        let dom = crate::html::parse(
            r#"
            <main id="app" class="min-h-screen bg-surface-950 text-white">
                CoreVM
            </main>
            "#,
        );
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            @layer theme {
                :root, :host {
                    --color-white: #fff;
                    --color-surface-950: #020617;
                }
            }
            @layer utilities {
                .min-h-screen { min-height: 100vh; }
                .bg-surface-950 { background-color: var(--color-surface-950); }
                .text-white { color: var(--color-white); }
            }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1280, 900, &mut inline_style_cache);
        let app = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "app")
                )
            })
            .expect("app node");

        assert_eq!(styles[app].color, 0xFFFFFFFF);
        assert_eq!(styles[app].background_color, 0xFF020617);
        assert_eq!(styles[app].min_height, 900);
    }

    #[test]
    fn custom_property_chain_resolves_nested_var_inside_calc() {
        let dom = crate::html::parse(r#"<svg id="icon" class="icon"></svg>"#);
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            :root {
                --font-size-base: 1rem;
                --scaling-factor-xxxs: 27/40;
                --baseline-down-04: calc(var(--font-size-base) * var(--scaling-factor-xxxs));
                --text-xxs: var(--baseline-down-04);
            }
            .icon {
                width: var(--text-xxs);
                height: var(--text-xxs);
            }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1280, 900, &mut inline_style_cache);
        let icon = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "icon")
                )
            })
            .expect("icon node");

        assert_eq!(styles[icon].width, Some(10));
        assert_eq!(styles[icon].height, Some(10));
    }

    #[test]
    fn tailwind_display_fallback_handles_missing_responsive_rules() {
        let dom = crate::html::parse(
            r#"<div id="mobile" class="flex md:hidden"></div><div id="desktop" class="hidden xl:inline"></div>"#,
        );
        let stylesheet = crate::css::parse_stylesheet("");
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 700, &mut inline_style_cache);
        let find = |id_value: &str| {
            dom.nodes
                .iter()
                .position(|node| {
                    matches!(
                        &node.node_type,
                        NodeType::Element { attrs, .. }
                            if attrs.iter().any(|a| a.name == "id" && a.value == id_value)
                    )
                })
                .expect("node")
        };

        assert!(matches!(styles[find("mobile")].display, Display::None));
        assert!(matches!(styles[find("desktop")].display, Display::Inline));
    }
}

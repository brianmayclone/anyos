// Resolve styles for entire DOM
// ---------------------------------------------------------------------------

/// Compute the final resolved style for every node in the DOM.
/// Returns a `Vec<ComputedStyle>` indexed by `NodeId`.
pub fn resolve_styles(
    dom: &Dom,
    stylesheets: &[&Stylesheet],
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<Option<Vec<Declaration>>>,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    let prepared = PreparedStylesheets::prepare(stylesheets, viewport_width, viewport_height);
    resolve_styles_prepared_with_state(
        dom,
        &prepared,
        viewport_width,
        viewport_height,
        inline_style_cache,
        &SelectorState::default(),
    )
}

pub fn resolve_styles_prepared(
    dom: &Dom,
    prepared: &PreparedStylesheets,
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<Option<Vec<Declaration>>>,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    resolve_styles_prepared_with_state(
        dom,
        prepared,
        viewport_width,
        viewport_height,
        inline_style_cache,
        &SelectorState::default(),
    )
}

pub fn resolve_styles_prepared_with_state(
    dom: &Dom,
    prepared: &PreparedStylesheets,
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<Option<Vec<Declaration>>>,
    selector_state: &SelectorState,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    resolve_styles_prepared_impl(
        dom,
        prepared,
        viewport_width,
        viewport_height,
        inline_style_cache,
        None,
        selector_state,
    )
}

pub fn resolve_styles_prepared_budgeted(
    dom: &Dom,
    prepared: &PreparedStylesheets,
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<Option<Vec<Declaration>>>,
    node_budget: usize,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    resolve_styles_prepared_budgeted_with_state(
        dom,
        prepared,
        viewport_width,
        viewport_height,
        inline_style_cache,
        node_budget,
        &SelectorState::default(),
    )
}

pub fn resolve_styles_prepared_budgeted_with_state(
    dom: &Dom,
    prepared: &PreparedStylesheets,
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<Option<Vec<Declaration>>>,
    node_budget: usize,
    selector_state: &SelectorState,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    resolve_styles_prepared_impl(
        dom,
        prepared,
        viewport_width,
        viewport_height,
        inline_style_cache,
        Some(node_budget),
        selector_state,
    )
}

fn resolve_styles_prepared_impl(
    dom: &Dom,
    prepared: &PreparedStylesheets,
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<Option<Vec<Declaration>>>,
    node_budget: Option<usize>,
    selector_state: &SelectorState,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    set_viewport_size(viewport_width, viewport_height);

    let count = dom.nodes.len();
    let resolved_count = node_budget.unwrap_or(count).min(count);
    let budgeted = resolved_count < count;
    if inline_style_cache.len() < resolved_count {
        inline_style_cache.resize_with(resolved_count, || None);
    }
    crate::debug_surf!(
        "[style] resolve_styles: {} nodes, {} stylesheets, resolved_count={}, budgeted={}",
        count,
        prepared.rules.len(),
        resolved_count,
        budgeted
    );
    #[cfg(feature = "debug_surf")]
    crate::debug_surf!(
        "[style]   RSP=0x{:X} heap=0x{:X}",
        crate::debug_rsp(),
        crate::debug_heap_pos()
    );

    let mut styles: Vec<ComputedStyle> = Vec::with_capacity(resolved_count.max(1));
    let root_font_size: i32 = 16;

    let all_rules = prepared.as_rule_refs();
    crate::debug_surf!(
        "[style] collected {} applicable rules (once)",
        all_rules.len()
    );

    crate::debug_surf!(
        "[style] rule index: {} wildcard, {} id-buckets, {} class-buckets",
        prepared.rule_index.wildcard.len(),
        prepared.rule_index.by_id.len(),
        prepared.rule_index.by_class.len()
    );

    // Reusable scratch buffers for per-node matching (avoids repeated alloc/free).
    let mut matches: Vec<((u32, u32, u32), usize)> = Vec::with_capacity(64);
    let mut candidates: Vec<usize> = Vec::with_capacity(128);
    let mut seen_bitset: Vec<u64> = Vec::with_capacity((all_rules.len() + 63) / 64);

    // Separate storage for custom properties (--name: value).
    // Only nodes that DEFINE custom properties have non-empty entries.
    // var() references are resolved on-demand by walking the DOM parent chain,
    // eliminating the per-node clone that caused heap-stack collision on large
    // pages (~54 MiB for chip.de's 6228 nodes).
    let mut custom_props: Vec<Vec<(String, String)>> = vec![Vec::new(); resolved_count];
    let mut set_flags_by_node: Vec<u32> = Vec::with_capacity(resolved_count.max(1));
    let mut active_containers: Vec<ActiveContainer> = Vec::new();

    for id in 0..resolved_count {
        #[cfg(feature = "debug_surf")]
        {
            if id < 5 || id % 1000 == 0 {
                crate::debug_surf!(
                    "[style] node {}/{} RSP=0x{:X} heap=0x{:X}",
                    id,
                    count,
                    crate::debug_rsp(),
                    crate::debug_heap_pos()
                );
            }
        }

        let node = &dom.nodes[id];
        while let Some(container) = active_containers.last() {
            if is_ancestor_or_self(dom, container.node_id, id) {
                break;
            }
            active_containers.pop();
        }
        let parent_fs = node
            .parent
            .map_or(16, |pid| if pid < id { styles[pid].font_size } else { 16 });
        let available_inline_size = active_containers
            .last()
            .map(|container| container.inline_size)
            .unwrap_or(viewport_width);
        let available_block_size = active_containers
            .last()
            .map(|container| container.block_size)
            .unwrap_or(viewport_height);
        let parent_style = node.parent.and_then(|pid| styles.get(pid));

        // Phase 1: Start from UA defaults (elements) or initial values (text).
        let (mut style, mut set_flags) = match &node.node_type {
            NodeType::Element { tag, .. } => ua_style_and_flags(*tag),
            NodeType::Text(_) => {
                let mut s = default_style();
                s.display = Display::Inline;
                (s, 0u32)
            }
        };

        // Seed inheritable runtime-critical values early so relative units and
        // currentColor-dependent properties see the same baseline that later
        // inheritance will preserve.
        if let Some(parent) = parent_style {
            if set_flags & SET_FONT_SIZE == 0 {
                style.font_size = parent.font_size;
            }
            if set_flags & SET_COLOR == 0 {
                style.color = parent.color;
            }
            if set_flags & SET_FONT_WEIGHT == 0 {
                style.font_weight = parent.font_weight;
            }
            if set_flags & SET_FONT_STYLE == 0 {
                style.font_style = parent.font_style;
            }
            if set_flags & SET_DIRECTION == 0 {
                style.direction = parent.direction;
            }
            if set_flags & SET_TEXT_ALIGN == 0 {
                style.text_align = parent.text_align;
            }
            if set_flags & SET_WHITE_SPACE == 0 {
                style.white_space = parent.white_space;
            }
            if set_flags & SET_TEXT_TRANSFORM == 0 {
                style.text_transform = parent.text_transform;
            }
            if set_flags & SET_LETTER_SPACING == 0 {
                style.letter_spacing = parent.letter_spacing;
            }
            if set_flags & SET_WORD_SPACING == 0 {
                style.word_spacing = parent.word_spacing;
            }
            if set_flags & SET_WORD_BREAK == 0 {
                style.word_break = parent.word_break;
            }
            if set_flags & SET_OVERFLOW_WRAP == 0 {
                style.overflow_wrap = parent.overflow_wrap;
            }
        }

        // UA override: [hidden] → display:none (per HTML spec).
        if let NodeType::Element { attrs, .. } = &node.node_type {
            if attrs
                .iter()
                .any(|a| eq_ignore_ascii_case(&a.name, "hidden"))
            {
                style.display = Display::None;
            }
        }

        // Common accessibility utility classes keep text available to screen
        // readers while moving it out of visual layout. Treat them as hidden
        // for the visual renderer so labels and page titles do not leak.
        if element_has_screen_reader_only_class(dom, id) {
            style.display = Display::None;
        }

        // Heuristic for component-driven collapse UIs: collapsed content is
        // usually materialized as a `data-collapse-target` subtree and only
        // becomes visible when an ancestor gains `is-open`.
        if element_has_attr(dom, id, "data-collapse-target")
            && !ancestor_has_attr(dom, id, "is-open")
        {
            style.display = Display::None;
        }

        // UA override: <input type="hidden"> → display:none (per HTML spec).
        if let NodeType::Element { tag, attrs, .. } = &node.node_type {
            if *tag == Tag::Input {
                if attrs.iter().any(|a| {
                    eq_ignore_ascii_case(&a.name, "type")
                        && eq_ignore_ascii_case(&a.value, "hidden")
                }) {
                    style.display = Display::None;
                }
            }
        }

        // Phase 1b: Presentational hints from HTML attributes (specificity 0,
        // per HTML spec §15.3.3). Applied after UA styles but before author rules.
        if let NodeType::Element { tag, attrs, .. } = &node.node_type {
            // `align` attribute on div, main, nav, header, footer, section,
            // article, aside, hgroup, address, center, p, h1-h6, blockquote,
            // figure, figcaption, details, summary, dialog, search.
            // Maps to text-align (HTML spec §15.3.3).
            let supports_align = matches!(
                tag,
                Tag::Div
                    | Tag::Main
                    | Tag::Nav
                    | Tag::Header
                    | Tag::Footer
                    | Tag::Section
                    | Tag::Article
                    | Tag::Aside
                    | Tag::Hgroup
                    | Tag::Address
                    | Tag::Center
                    | Tag::P
                    | Tag::H1
                    | Tag::H2
                    | Tag::H3
                    | Tag::H4
                    | Tag::H5
                    | Tag::H6
                    | Tag::Blockquote
                    | Tag::Figure
                    | Tag::Figcaption
                    | Tag::Details
                    | Tag::Summary
                    | Tag::Dialog
                    | Tag::Search
                    | Tag::Td
                    | Tag::Th
            );
            if supports_align {
                for a in attrs {
                    if eq_ignore_ascii_case(&a.name, "align") {
                        let val = a.value.trim();
                        let align = if val.eq_ignore_ascii_case("left") {
                            Some(TextAlignVal::Left)
                        } else if val.eq_ignore_ascii_case("right") {
                            Some(TextAlignVal::Right)
                        } else if val.eq_ignore_ascii_case("center") {
                            Some(TextAlignVal::Center)
                        } else if val.eq_ignore_ascii_case("justify") {
                            Some(TextAlignVal::Justify)
                        } else {
                            None
                        };
                        if let Some(ta) = align {
                            style.text_align = ta;
                            set_flags |= SET_TEXT_ALIGN;
                        }
                        break;
                    }
                }
            }
        }

        // Phase 2 + 3: Apply author rules and inline styles.
        // Custom property declarations are stored in custom_props[id].
        // var() references are resolved by walking the parent chain.
        if matches!(node.node_type, NodeType::Element { .. }) {
            let (ancestors_cp, current_and_rest) = custom_props.split_at_mut(id);
            let node_cp = &mut current_and_rest[0];

            set_flags |= apply_author_rules(
                &mut style,
                parent_style,
                dom,
                id,
                &all_rules,
                prepared.layer_count,
                &prepared.rule_index,
                &mut candidates,
                &mut seen_bitset,
                &mut matches,
                parent_fs,
                root_font_size,
                node_cp,
                ancestors_cp,
                selector_state,
                &active_containers,
            );

            if let NodeType::Element { attrs, .. } = &node.node_type {
                apply_tailwind_display_fallback(&mut style, attrs, viewport_width);
                apply_tailwind_spacing_fallback(&mut style, attrs);
                apply_flexbox_grid_column_fallback(&mut style, attrs, viewport_width);
            }

            // Phase 3: Apply inline styles (highest specificity).
            // Uses a cache to avoid re-parsing style="..." on every relayout.
            if let NodeType::Element { attrs, .. } = &node.node_type {
                for a in attrs {
                    if eq_ignore_ascii_case(&a.name, "style") {
                        // Look up cached declarations for this node, or parse and cache.
                        if inline_style_cache[id].is_none() {
                            inline_style_cache[id] = Some(crate::css::parse_inline_style(&a.value));
                        }
                        let inline_decls = inline_style_cache[id].as_ref().unwrap();
                        apply_custom_props_from_decls(inline_decls, false, node_cp);
                        apply_custom_props_from_decls(inline_decls, true, node_cp);
                        apply_decls_two_pass(
                            &mut style,
                            inline_decls,
                            false,
                            parent_style,
                            dom,
                            id,
                            node_cp,
                            ancestors_cp,
                            parent_fs,
                            root_font_size,
                            &mut set_flags,
                        );
                        apply_decls_two_pass(
                            &mut style,
                            inline_decls,
                            true,
                            parent_style,
                            dom,
                            id,
                            node_cp,
                            ancestors_cp,
                            parent_fs,
                            root_font_size,
                            &mut set_flags,
                        );
                        break;
                    }
                }
            }
        }

        // HTML <dialog> elements are not rendered unless the `open` attribute is
        // present. Many sites keep modal templates in the DOM and rely on this
        // behavior; painting them breaks the page under a full-screen backdrop.
        if let NodeType::Element { tag, attrs, .. } = &node.node_type {
            if *tag == Tag::Dialog && attrs.iter().all(|a| !eq_ignore_ascii_case(&a.name, "open")) {
                style.display = Display::None;
            }

            if is_unfocused_skip_link(*tag, attrs, id, selector_state) {
                apply_visually_hidden_style(&mut style);
            } else if selector_state.focused_node != Some(id)
                && attrs.iter().any(|a| {
                    eq_ignore_ascii_case(&a.name, "class") && has_visually_hidden_class(&a.value)
                })
            {
                apply_visually_hidden_style(&mut style);
            }
        }

        fn apply_visually_hidden_style(style: &mut ComputedStyle) {
            style.position = Position::Absolute;
            style.left_offset = Some(-10000);
            style.top = Some(0);
            style.width = Some(1);
            style.height = Some(1);
            style.min_width = 0;
            style.min_height = 0;
            style.padding_top = 0;
            style.padding_right = 0;
            style.padding_bottom = 0;
            style.padding_left = 0;
            style.overflow_x = OverflowVal::Hidden;
            style.overflow_y = OverflowVal::Hidden;
            style.clip_rect = Some([0, 0, 0, 0]);
        }

        // (Phase 3b removed: custom properties are resolved on-demand via
        // parent chain walk, eliminating the per-node clone that caused
        // heap-stack collision on large pages.)

        // Phase 4: Inherit inheritable properties NOT explicitly set.
        if let Some(pid) = node.parent {
            if pid < id {
                inherit_unset(&mut style, &styles[pid], set_flags);
            }
        }

        // Phase 5: Resolve `li` list_style from parent (ol -> decimal).
        if let NodeType::Element { tag: Tag::Li, .. } = &node.node_type {
            if set_flags & SET_LIST_STYLE != 0 && style.list_style == ListStyle::Disc {
                if let Some(pid) = node.parent {
                    if dom.tag(pid) == Some(Tag::Ol) {
                        style.list_style = ListStyle::Decimal;
                    }
                }
            }
        }

        // Phase 6: Resolve auto line_height.
        // Important: an explicitly specified `line-height: 0` is valid CSS and
        // must not be treated as "unset/auto".
        if set_flags & SET_LINE_HEIGHT == 0 && style.line_height == 0 {
            style.line_height = (style.font_size * 6 + 2) / 5;
        }

        set_flags_by_node.push(set_flags);
        styles.push(style);

        if matches!(
            styles[id].container_type,
            ContainerTypeVal::InlineSize | ContainerTypeVal::Size
        ) {
            active_containers.push(ActiveContainer {
                node_id: id,
                names: styles[id].container_names.clone(),
                inline_size: estimated_content_inline_size(&styles[id], available_inline_size),
                block_size: estimated_content_block_size(&styles[id], available_block_size),
            });
        }
    }

    if budgeted {
        let mut hidden = default_style();
        hidden.display = Display::None;
        styles.resize(count, hidden);
        set_flags_by_node.resize(count, 0);
    }

    // JS-created virtual DOM nodes are materialized from React-style createElement
    // trees, where child text nodes can receive real DOM ids before their parent
    // element does.  The main resolver walks numeric node ids for performance, so
    // `pid < id` inheritance is not guaranteed for those nodes.  A cheap DOM-order
    // inheritance pass makes inheritable properties (especially color/font) match
    // the actual tree, while preserving properties explicitly set on the child.
    inherit_unset_in_dom_order(dom, &mut styles, &set_flags_by_node);

    crate::debug_surf!("[style] resolve_styles done: {} styles", styles.len());
    #[cfg(feature = "debug_surf")]
    crate::debug_surf!(
        "[style]   RSP=0x{:X} heap=0x{:X}",
        crate::debug_rsp(),
        crate::debug_heap_pos()
    );

    // ── Phase 7: Resolve ::before/::after pseudo-element styles ──
    let mut pseudo = PseudoStyles::empty(count);
    let mut pseudo_candidates: Vec<usize> = Vec::with_capacity(128);
    let mut pseudo_seen_bitset: Vec<u64> = Vec::with_capacity((all_rules.len() + 63) / 64);
    let mut pseudo_matches: Vec<((u32, u32, u32), usize)> = Vec::with_capacity(32);
    let mut pseudo_active_containers: Vec<ActiveContainer> = Vec::new();
    for id in 0..resolved_count {
        while let Some(container) = pseudo_active_containers.last() {
            if is_ancestor_or_self(dom, container.node_id, id) {
                break;
            }
            pseudo_active_containers.pop();
        }
        let node = &dom.nodes[id];
        let (tag, attrs) = match &node.node_type {
            NodeType::Element { tag, attrs } => (*tag, attrs),
            _ => continue,
        };
        let id_attr = attrs
            .iter()
            .find(|a| eq_ignore_ascii_case(&a.name, "id"))
            .map(|a| a.value.as_str());
        let class_attr = attrs
            .iter()
            .find(|a| eq_ignore_ascii_case(&a.name, "class"))
            .map(|a| a.value.as_str());
        prepared.rule_index.candidates(
            tag,
            id_attr,
            class_attr,
            &mut pseudo_candidates,
            &mut pseudo_seen_bitset,
        );
        pseudo_matches.clear();
        for &rule_idx in pseudo_candidates.iter() {
            let (rule, _order) = all_rules[rule_idx];
            if let Some(ref query) = rule.container_query {
                if !container_query_matches(query, &pseudo_active_containers) {
                    continue;
                }
            }
            for sel in &rule.selectors {
                let pe = sel.pseudo_element();
                if pe.is_none() {
                    continue;
                }
                // Check if the base selector (without pseudo-element) matches this node.
                if !selector_matches_base(sel, dom, id, selector_state) {
                    continue;
                }
                pseudo_matches.push((sel.specificity(), rule_idx));
                break;
            }
        }
        let parent_fs = styles[id].font_size;
        let root_fs = 16;
        let mut before_style: Option<ComputedStyle> = None;
        let mut after_style: Option<ComputedStyle> = None;
        sort_matches_for_phase(&mut pseudo_matches, &all_rules, false, prepared.layer_count);
        apply_pseudo_rule_matches(
            &mut before_style,
            &mut after_style,
            id,
            &styles,
            &pseudo_matches,
            &all_rules,
            false,
            parent_fs,
            root_fs,
        );
        sort_matches_for_phase(&mut pseudo_matches, &all_rules, true, prepared.layer_count);
        apply_pseudo_rule_matches(
            &mut before_style,
            &mut after_style,
            id,
            &styles,
            &pseudo_matches,
            &all_rules,
            true,
            parent_fs,
            root_fs,
        );
        pseudo.before[id] = before_style;
        pseudo.after[id] = after_style;
        // Resolve line_height for pseudo styles.
        if let Some(ref mut ps) = pseudo.before[id] {
            if ps.line_height == 0 {
                ps.line_height = (ps.font_size * 6 + 2) / 5;
            }
            // Only keep if content is set and non-empty.
            if ps.content.is_none() {
                pseudo.before[id] = None;
            }
        }
        if let Some(ref mut ps) = pseudo.after[id] {
            if ps.line_height == 0 {
                ps.line_height = (ps.font_size * 6 + 2) / 5;
            }
            if ps.content.is_none() {
                pseudo.after[id] = None;
            }
        }
        if matches!(
            styles[id].container_type,
            ContainerTypeVal::InlineSize | ContainerTypeVal::Size
        ) {
            let available_inline_size = pseudo_active_containers
                .last()
                .map(|container| container.inline_size)
                .unwrap_or(viewport_width);
            let available_block_size = pseudo_active_containers
                .last()
                .map(|container| container.block_size)
                .unwrap_or(viewport_height);
            pseudo_active_containers.push(ActiveContainer {
                node_id: id,
                names: styles[id].container_names.clone(),
                inline_size: estimated_content_inline_size(&styles[id], available_inline_size),
                block_size: estimated_content_block_size(&styles[id], available_block_size),
            });
        }
    }

    (styles, pseudo)
}

fn apply_author_rules(
    style: &mut ComputedStyle,
    parent_style: Option<&ComputedStyle>,
    dom: &Dom,
    node_id: NodeId,
    all_rules: &[(&Rule, usize)],
    layer_count: usize,
    rule_index: &RuleIndex,
    candidates: &mut Vec<usize>,
    seen_bitset: &mut Vec<u64>,
    matches: &mut Vec<((u32, u32, u32), usize)>,
    parent_fs: i32,
    root_fs: i32,
    node_cp: &mut Vec<(String, String)>,
    ancestors_cp: &[Vec<(String, String)>],
    selector_state: &SelectorState,
    active_containers: &[ActiveContainer],
) -> u32 {
    // Reuse the caller's matches buffer (avoids alloc/free per node).
    matches.clear();

    // Use the rule index to get only candidate rules for this node's tag/id/classes.
    let node = &dom.nodes[node_id];
    let (tag, attrs) = match &node.node_type {
        NodeType::Element { tag, attrs } => (tag, attrs),
        _ => return 0,
    };
    let id_attr = attrs
        .iter()
        .find(|a| eq_ignore_ascii_case(&a.name, "id"))
        .map(|a| a.value.as_str());
    let class_attr = attrs
        .iter()
        .find(|a| eq_ignore_ascii_case(&a.name, "class"))
        .map(|a| a.value.as_str());

    rule_index.candidates(*tag, id_attr, class_attr, candidates, seen_bitset);

    for &idx in candidates.iter() {
        let (rule, _order) = all_rules[idx];
        if let Some(ref query) = rule.container_query {
            if !container_query_matches(query, active_containers) {
                continue;
            }
        }
        for sel in &rule.selectors {
            if selector_matches(sel, dom, node_id, selector_state) {
                matches.push((sel.specificity(), idx));
                break;
            }
        }
    }

    let mut set_flags: u32 = 0;

    // Resolve custom properties before dependent declarations. Utility-first
    // CSS (Tailwind 4 etc.) often emits `background-image: var(...)` before
    // the utility rules that define the gradient stops for the same element.
    sort_matches_for_phase(matches, all_rules, false, layer_count);
    for &(_, idx) in matches.iter() {
        let (rule, _) = all_rules[idx];
        apply_custom_props_from_decls(&rule.declarations, false, node_cp);
    }
    sort_matches_for_phase(matches, all_rules, true, layer_count);
    for &(_, idx) in matches.iter() {
        let (rule, _) = all_rules[idx];
        apply_custom_props_from_decls(&rule.declarations, true, node_cp);
    }

    // Phase 1: Apply normal (non-!important) declarations.
    sort_matches_for_phase(matches, all_rules, false, layer_count);
    for &(_, idx) in matches.iter() {
        let (rule, _) = all_rules[idx];
        apply_decls_two_pass(
            style,
            &rule.declarations,
            false,
            parent_style,
            dom,
            node_id,
            node_cp,
            ancestors_cp,
            parent_fs,
            root_fs,
            &mut set_flags,
        );
    }

    // Phase 2: Apply !important declarations (override normal ones).
    sort_matches_for_phase(matches, all_rules, true, layer_count);
    for &(_, idx) in matches.iter() {
        let (rule, _) = all_rules[idx];
        apply_decls_two_pass(
            style,
            &rule.declarations,
            true,
            parent_style,
            dom,
            node_id,
            node_cp,
            ancestors_cp,
            parent_fs,
            root_fs,
            &mut set_flags,
        );
    }

    set_flags
}

fn apply_tailwind_display_fallback(
    style: &mut ComputedStyle,
    attrs: &[crate::dom::Attr],
    viewport_width: i32,
) {
    let Some(class_attr) = attrs
        .iter()
        .find(|a| eq_ignore_ascii_case(&a.name, "class"))
    else {
        return;
    };

    let mut base_display = None;
    let mut sm_display = None;
    let mut md_display = None;
    let mut lg_display = None;
    let mut xl_display = None;
    let mut xxl_display = None;
    let mut max_sm_display = None;
    let mut max_md_display = None;
    let mut max_lg_display = None;
    let mut max_xl_display = None;

    for class_name in class_attr.value.split_ascii_whitespace() {
        let mut matched_prefix = None;
        let mut utility = class_name;
        if let Some((prefix, rest)) = class_name.split_once(':') {
            matched_prefix = Some(prefix);
            utility = rest.rsplit(':').next().unwrap_or(rest);
        }
        let Some(display) = tailwind_display_utility(utility) else {
            continue;
        };
        match matched_prefix {
            Some("sm") => sm_display = Some(display),
            Some("md") => md_display = Some(display),
            Some("lg") => lg_display = Some(display),
            Some("xl") => xl_display = Some(display),
            Some("2xl") => xxl_display = Some(display),
            Some("max-sm") => max_sm_display = Some(display),
            Some("max-md") => max_md_display = Some(display),
            Some("max-lg") => max_lg_display = Some(display),
            Some("max-xl") => max_xl_display = Some(display),
            None => base_display = Some(display),
            _ => {}
        }
    }

    let mut display = base_display;
    if viewport_width >= 640 {
        display = sm_display.or(display);
    } else {
        display = max_sm_display.or(display);
    }
    if viewport_width >= 768 {
        display = md_display.or(display);
    } else {
        display = max_md_display.or(display);
    }
    if viewport_width >= 1024 {
        display = lg_display.or(display);
    } else {
        display = max_lg_display.or(display);
    }
    if viewport_width >= 1280 {
        display = xl_display.or(display);
    } else {
        display = max_xl_display.or(display);
    }
    if viewport_width >= 1536 {
        display = xxl_display.or(display);
    }
    if let Some(display) = display {
        style.display = display;
    }
}

fn tailwind_display_utility(class_name: &str) -> Option<Display> {
    match class_name {
        "hidden" => Some(Display::None),
        "block" => Some(Display::Block),
        "inline" => Some(Display::Inline),
        "inline-block" => Some(Display::InlineBlock),
        // Approximate CSS table wrappers as block boxes until the layout
        // engine has full CSS table display support.  This is especially
        // important for the ubiquitous clearfix idiom
        // `::after { display: table; clear: both; content: "" }`.
        "table" => Some(Display::Block),
        "inline-table" => Some(Display::InlineBlock),
        "flex" => Some(Display::Flex),
        "inline-flex" => Some(Display::InlineFlex),
        "grid" => Some(Display::Grid),
        "inline-grid" => Some(Display::InlineGrid),
        "flow-root" => Some(Display::FlowRoot),
        "contents" => Some(Display::Contents),
        _ => None,
    }
}

fn apply_tailwind_spacing_fallback(style: &mut ComputedStyle, attrs: &[crate::dom::Attr]) {
    let Some(class_attr) = attrs
        .iter()
        .find(|a| eq_ignore_ascii_case(&a.name, "class"))
    else {
        return;
    };

    for class_name in class_attr.value.split_ascii_whitespace() {
        let utility = class_name.rsplit(':').next().unwrap_or(class_name);

        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("gap-")) {
            style.row_gap = px;
            style.column_gap = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("gap-x-")) {
            style.column_gap = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("gap-y-")) {
            style.row_gap = px;
            continue;
        }

        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("p-")) {
            style.padding_top = px;
            style.padding_right = px;
            style.padding_bottom = px;
            style.padding_left = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("px-")) {
            style.padding_left = px;
            style.padding_right = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("py-")) {
            style.padding_top = px;
            style.padding_bottom = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("pt-")) {
            style.padding_top = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("pr-")) {
            style.padding_right = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("pb-")) {
            style.padding_bottom = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("pl-")) {
            style.padding_left = px;
            continue;
        }

        if utility == "mx-auto" {
            style.margin_left_auto = true;
            style.margin_right_auto = true;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("m-")) {
            style.margin_top = px;
            style.margin_right = px;
            style.margin_bottom = px;
            style.margin_left = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("mx-")) {
            style.margin_left = px;
            style.margin_right = px;
            style.margin_left_auto = false;
            style.margin_right_auto = false;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("my-")) {
            style.margin_top = px;
            style.margin_bottom = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("mt-")) {
            style.margin_top = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("mr-")) {
            style.margin_right = px;
            style.margin_right_auto = false;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("mb-")) {
            style.margin_bottom = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("ml-")) {
            style.margin_left = px;
            style.margin_left_auto = false;
            continue;
        }

        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("w-")) {
            style.width = Some(px);
            style.width_pct = None;
            style.width_calc = None;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("h-")) {
            style.height = Some(px);
            style.height_pct = None;
            style.height_calc = None;
            continue;
        }
        if let Some(px) = tailwind_max_width_value(utility) {
            style.max_width = Some(px);
            style.max_width_calc = None;
        }
    }
}

fn tailwind_spacing_value(rest: Option<&str>) -> Option<i32> {
    let rest = rest?;
    match rest {
        "px" => return Some(1),
        "0" => return Some(0),
        "0.5" => return Some(2),
        "1" => return Some(4),
        "1.5" => return Some(6),
        "2" => return Some(8),
        "2.5" => return Some(10),
        "3" => return Some(12),
        "3.5" => return Some(14),
        "4" => return Some(16),
        "5" => return Some(20),
        "6" => return Some(24),
        "7" => return Some(28),
        "8" => return Some(32),
        "9" => return Some(36),
        "10" => return Some(40),
        "11" => return Some(44),
        "12" => return Some(48),
        "14" => return Some(56),
        "16" => return Some(64),
        "20" => return Some(80),
        "24" => return Some(96),
        "28" => return Some(112),
        "32" => return Some(128),
        _ => {}
    }
    None
}

fn tailwind_max_width_value(class_name: &str) -> Option<i32> {
    match class_name {
        "max-w-xs" => Some(320),
        "max-w-sm" => Some(384),
        "max-w-md" => Some(448),
        "max-w-lg" => Some(512),
        "max-w-xl" => Some(576),
        "max-w-2xl" => Some(672),
        "max-w-3xl" => Some(768),
        "max-w-4xl" => Some(896),
        "max-w-5xl" => Some(1024),
        "max-w-6xl" => Some(1152),
        "max-w-7xl" => Some(1280),
        _ => None,
    }
}

fn apply_flexbox_grid_column_fallback(
    style: &mut ComputedStyle,
    attrs: &[crate::dom::Attr],
    viewport_width: i32,
) {
    let Some(class_attr) = attrs
        .iter()
        .find(|a| eq_ignore_ascii_case(&a.name, "class"))
    else {
        return;
    };

    let mut base: Option<i32> = None;
    let mut sm: Option<i32> = None;
    let mut md: Option<i32> = None;
    let mut lg: Option<i32> = None;
    let mut xl: Option<i32> = None;

    for class_name in class_attr.value.split_ascii_whitespace() {
        if let Some(cols) = parse_fb_col_class(class_name, "fb-col-") {
            base = Some(cols);
            continue;
        }
        if let Some(cols) = parse_fb_col_class(class_name, "fb-col-sm-") {
            sm = Some(cols);
            continue;
        }
        if let Some(cols) = parse_fb_col_class(class_name, "fb-col-md-") {
            md = Some(cols);
            continue;
        }
        if let Some(cols) = parse_fb_col_class(class_name, "fb-col-lg-") {
            lg = Some(cols);
            continue;
        }
        if let Some(cols) = parse_fb_col_class(class_name, "fb-col-xl-") {
            xl = Some(cols);
        }
    }

    let mut cols = base;
    if viewport_width >= 660 {
        cols = sm.or(md).or(cols);
    }
    if viewport_width >= 1000 {
        cols = lg.or(cols);
    }
    if viewport_width >= 1200 {
        cols = xl.or(cols);
    }

    let Some(cols) = cols else {
        return;
    };
    let pct = ((cols.clamp(1, 12) as i64 * 10000 + 6) / 12) as i32;
    style.width = None;
    style.width_calc = None;
    style.width_pct = Some(pct);
}

fn parse_fb_col_class(class_name: &str, prefix: &str) -> Option<i32> {
    let rest = class_name.strip_prefix(prefix)?;
    if rest.is_empty() || rest.as_bytes().iter().any(|b| !b.is_ascii_digit()) {
        return None;
    }
    let cols = rest.parse::<i32>().ok()?;
    if (1..=12).contains(&cols) {
        Some(cols)
    } else {
        None
    }
}

/// Store a custom property in a node's custom property list.
fn store_custom_prop(cp: &mut Vec<(String, String)>, name: &str, val: &str) {
    if let Some(existing) = cp.iter_mut().find(|(k, _)| k == name) {
        existing.1.clear();
        existing.1.push_str(val);
    } else {
        cp.push((String::from(name), String::from(val)));
    }
}

fn custom_property_value_to_string(value: &CssValue) -> Option<String> {
    match value {
        CssValue::Keyword(s) => Some(s.clone()),
        CssValue::Color(c) => {
            let a = (c >> 24) & 0xff;
            let r = (c >> 16) & 0xff;
            let g = (c >> 8) & 0xff;
            let b = c & 0xff;
            if a == 255 {
                Some(format!("#{:02x}{:02x}{:02x}", r, g, b))
            } else {
                Some(format!("#{:02x}{:02x}{:02x}{:02x}", r, g, b, a))
            }
        }
        CssValue::Length(v, unit) => Some(format!("{}{}", v, unit_suffix(*unit))),
        CssValue::Percentage(v) => Some(format!("{}%", v / 100)),
        CssValue::Number(v) => Some(v.to_string()),
        CssValue::Auto => Some(String::from("auto")),
        CssValue::None => Some(String::from("none")),
        CssValue::Inherit => Some(String::from("inherit")),
        CssValue::CurrentColor => Some(String::from("currentColor")),
        CssValue::Calc(px, pct) => {
            if *pct == 0 {
                Some(format!("{}px", px))
            } else {
                Some(format!("calc({}px + {}%)", px, pct / 100))
            }
        }
        CssValue::Var(name, fallback) => {
            let mut out = String::from("var(");
            out.push_str(name);
            if let Some(fallback) = fallback {
                if let Some(fallback_text) = custom_property_value_to_string(fallback) {
                    out.push_str(", ");
                    out.push_str(&fallback_text);
                }
            }
            out.push(')');
            Some(out)
        }
    }
}

fn unit_suffix(unit: Unit) -> &'static str {
    match unit {
        Unit::Px => "px",
        Unit::Em => "em",
        Unit::Rem => "rem",
        Unit::In => "in",
        Unit::Cm => "cm",
        Unit::Mm => "mm",
        Unit::Pt => "pt",
        Unit::Pc => "pc",
        Unit::Q => "q",
        Unit::Percent => "%",
        Unit::Fr => "fr",
        Unit::Vw => "vw",
        Unit::Vh => "vh",
        Unit::Vmin => "vmin",
        Unit::Vmax => "vmax",
    }
}

/// Look up a custom property by walking the DOM parent chain.
///
/// Checks the current node's own custom properties first, then walks up
/// the ancestor chain. Returns the raw value string if found.
fn lookup_custom_property<'a>(
    name: &str,
    node_cp: &'a [(String, String)],
    dom: &Dom,
    node_id: NodeId,
    ancestors_cp: &'a [Vec<(String, String)>],
) -> Option<&'a str> {
    // Check this node's own custom properties first.
    if let Some((_, val)) = node_cp.iter().find(|(k, _)| k == name) {
        return Some(val.as_str());
    }
    // Walk up the parent chain.
    let mut cur = dom.nodes[node_id].parent;
    while let Some(pid) = cur {
        if pid < ancestors_cp.len() {
            if let Some((_, val)) = ancestors_cp[pid].iter().find(|(k, _)| k == name) {
                return Some(val.as_str());
            }
        }
        cur = dom.nodes.get(pid).and_then(|node| node.parent);
    }
    fallback_custom_property(name)
}

fn custom_property_is_unset_keyword(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.eq_ignore_ascii_case("initial")
        || trimmed.eq_ignore_ascii_case("unset")
        || trimmed.eq_ignore_ascii_case("revert")
        || trimmed.eq_ignore_ascii_case("revert-layer")
}

fn fallback_custom_property(name: &str) -> Option<&'static str> {
    match name {
        "--base" | "--text-black" | "--surface-black" | "--global-black" => Some("#333333"),
        "--black" => Some("#000000"),
        "--white" | "--global-white" | "--background-primary" | "--text-on-color-white" => {
            Some("#ffffff")
        }
        "--background" | "--background-light" | "--surface-gray-10" => Some("#eeeeef"),
        "--surface-gray-20" | "--border-bg" | "--border-medium" => Some("#e0e0e0"),
        "--surface-gray-30" | "--text-muted" => Some("#666666"),
        "--surface-brand" | "--text-brand" | "--border-brand" | "--primary" => Some("#ee001c"),
        "--text-link" => Some("#156fbc"),
        "--font-weight-bold" => Some("700"),
        "--font-weight-bolder" => Some("800"),
        "--font-weight-black" => Some("900"),
        "--font-letter-spacing" | "--font-letter-spacing-p" | "--font-letter-spacing-caps" => {
            Some("0")
        }
        "--font-family-headline"
        | "--font-family-inter-tight"
        | "--website-font"
        | "--website-paragraph" => Some("Arial"),
        "--text-xxs" | "--unified-text-xxs" => Some(".555rem"),
        "--text-xs" | "--unified-text-xs" => Some(".666rem"),
        "--text-sm" | "--unified-text-sm" => Some(".777rem"),
        "--text-md" | "--unified-text-md" => Some(".888rem"),
        "--text-base" | "--unified-text-base" => Some("1rem"),
        "--text-lg" | "--unified-text-lg" => Some("1.125rem"),
        "--text-xl" | "--unified-text-xl" => Some("1.222rem"),
        "--text-xxl" | "--unified-text-xxl" => Some("1.5rem"),
        "--headline-xxs" | "--unified-headline-xxs" => Some(".888rem"),
        "--headline-xs" | "--unified-headline-xs" => Some("1rem"),
        "--headline-sm" | "--unified-headline-sm" | "--headline-lg-mobile" => Some("1.222rem"),
        "--headline-md" | "--unified-headline-md" => Some("1.555rem"),
        "--headline-lg" | "--unified-headline-lg" => Some("1.777rem"),
        "--headline-xl" | "--unified-headline-xl" => Some("2rem"),
        "--headline-xxl" | "--unified-headline-xxl" => Some("3rem"),
        "--line-height-default"
        | "--line-height-text-xs"
        | "--line-height-text-sm"
        | "--line-height-text-md"
        | "--line-height-text-lg"
        | "--line-height-text-xl"
        | "--line-height-text-xxl"
        | "--txt-line-height-xs"
        | "--txt-line-height-sm"
        | "--txt-line-height-md"
        | "--txt-line-height-lg"
        | "--txt-line-height-xl"
        | "--unified-line-height-base"
        | "--unified-line-height-text-xxs"
        | "--unified-line-height-text-xs"
        | "--unified-line-height-text-sm"
        | "--unified-line-height-text-md"
        | "--unified-line-height-text-lg"
        | "--unified-line-height-text-xl"
        | "--unified-line-height-text-xxl" => Some("1.3"),
        "--line-height-hl-xxs"
        | "--line-height-hl-xs"
        | "--line-height-hl-sm"
        | "--line-height-hl-md"
        | "--line-height-hl-lg"
        | "--line-height-hl-xl"
        | "--line-height-hl-xxl"
        | "--unified-line-height-hl-xxs"
        | "--unified-line-height-hl-xs"
        | "--unified-line-height-hl-sm"
        | "--unified-line-height-hl-md"
        | "--unified-line-height-hl-lg"
        | "--unified-line-height-hl-xl"
        | "--unified-line-height-hl-xxl" => Some("1.2"),
        "--spacing-xxs" => Some(".125rem"),
        "--spacing-xs" => Some(".25rem"),
        "--spacing-sm" => Some(".5rem"),
        "--spacing-md" => Some("1rem"),
        "--spacing-lg" => Some("1.5rem"),
        "--spacing-xl" => Some("2rem"),
        "--spacing-xxl" => Some("4rem"),
        "--spacing" => Some(".25rem"),
        "--container-xs" => Some("20rem"),
        "--container-sm" => Some("24rem"),
        "--container-md" => Some("28rem"),
        "--container-lg" => Some("32rem"),
        "--container-xl" => Some("36rem"),
        "--container-2xl" => Some("42rem"),
        "--container-3xl" => Some("48rem"),
        "--container-4xl" => Some("56rem"),
        "--container-5xl" => Some("64rem"),
        "--container-6xl" => Some("72rem"),
        "--container-7xl" => Some("80rem"),
        "--scaling-factor-xxxs" => Some("27/40"),
        "--scaling-factor-xxs" => Some("3/4"),
        "--scaling-factor-xs" => Some("27/32"),
        "--scaling-factor-sm" => Some("1"),
        "--scaling-factor-base" => Some("9/8"),
        "--scaling-factor-md" => Some("27/20"),
        "--scaling-factor-lg" => Some("27/16"),
        "--scaling-factor-xl" => Some("2/1"),
        "--scaling-factor-xxl" => Some("9/4"),
        "--scaling-factor-xxxl" => Some("27/8"),
        "--baseline-down-04" => Some("calc(1rem * 27/40)"),
        "--baseline-down-03" => Some("calc(1rem * 3/4)"),
        "--baseline-down-02" => Some("calc(1rem * 27/32)"),
        "--baseline-down-01" => Some("1rem"),
        "--baseline" => Some("calc(1rem * 9/8)"),
        "--baseline-up-01" => Some("calc(1rem * 27/20)"),
        "--baseline-up-02" => Some("calc(1rem * 27/16)"),
        "--baseline-up-03" => Some("calc(1rem * 2)"),
        "--baseline-up-04" => Some("calc(1rem * 9/4)"),
        "--baseline-up-05" => Some("calc(1rem * 27/8)"),
        "--grid-spacing" | "--container-spacing" => Some("20px"),
        "--column-gap" => Some("1.25rem"),
        "--article-content-width" => Some("800px"),
        "--full-content-width" | "--container-width" => Some("956px"),
        "--border-sm" => Some("1px"),
        _ => None,
    }
}

/// Resolve var() references by walking the DOM parent chain.
fn resolve_var_in_decl(
    decl: &Declaration,
    dom: &Dom,
    node_id: NodeId,
    node_cp: &[(String, String)],
    ancestors_cp: &[Vec<(String, String)>],
) -> Declaration {
    if matches!(decl.value, CssValue::Var(_, _)) {
        if let Some(value) = resolve_css_var_value(
            &decl.value,
            &decl.property,
            dom,
            node_id,
            node_cp,
            ancestors_cp,
            0,
        ) {
            return Declaration {
                property: decl.property.clone(),
                value,
                important: decl.important,
            };
        }
    }
    decl.clone()
}

fn resolve_css_var_value(
    value: &CssValue,
    property: &Property,
    dom: &Dom,
    node_id: NodeId,
    node_cp: &[(String, String)],
    ancestors_cp: &[Vec<(String, String)>],
    depth: usize,
) -> Option<CssValue> {
    if depth > 8 {
        return None;
    }
    match value {
        CssValue::Var(name, fallback) => {
            if let Some(val) = lookup_custom_property(name, node_cp, dom, node_id, ancestors_cp) {
                if custom_property_is_unset_keyword(val) {
                    return if let Some(fb) = fallback {
                        resolve_css_var_value(
                            fb,
                            property,
                            dom,
                            node_id,
                            node_cp,
                            ancestors_cp,
                            depth + 1,
                        )
                    } else {
                        None
                    };
                }
                let resolved_val = if val.contains("var(") {
                    resolve_nested_vars(val, dom, node_id, node_cp, ancestors_cp)
                } else {
                    String::from(val)
                };
                let parsed = crate::css::parse_value(property, &resolved_val);
                if matches!(parsed, CssValue::Var(_, _)) {
                    resolve_css_var_value(
                        &parsed,
                        property,
                        dom,
                        node_id,
                        node_cp,
                        ancestors_cp,
                        depth + 1,
                    )
                } else {
                    Some(parsed)
                }
            } else if let Some(fb) = fallback {
                if matches!(**fb, CssValue::Var(_, _)) {
                    resolve_css_var_value(
                        fb,
                        property,
                        dom,
                        node_id,
                        node_cp,
                        ancestors_cp,
                        depth + 1,
                    )
                } else {
                    Some((**fb).clone())
                }
            } else {
                None
            }
        }
        other => Some(other.clone()),
    }
}

/// Check if a declaration has nested var() inside a function value (e.g. rgb(R G B/var(--x,1))).
fn has_nested_var(decl: &Declaration) -> bool {
    if let CssValue::Keyword(ref s) = decl.value {
        s.contains("var(")
    } else {
        false
    }
}

/// Resolve nested var() references within a value string, e.g.
/// "rgb(31 30 28/var(--tw-bg-opacity,1))" → "rgb(31 30 28/1)"
fn resolve_nested_vars(
    value: &str,
    dom: &Dom,
    node_id: NodeId,
    node_cp: &[(String, String)],
    ancestors_cp: &[Vec<(String, String)>],
) -> String {
    let mut result = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 4 <= bytes.len() && &bytes[i..i + 4] == b"var(" {
            // Find matching closing paren, respecting nesting
            let start = i + 4;
            let mut depth: u32 = 1;
            let mut end = start;
            while end < bytes.len() && depth > 0 {
                if bytes[end] == b'(' {
                    depth += 1;
                }
                if bytes[end] == b')' {
                    depth -= 1;
                }
                if depth > 0 {
                    end += 1;
                }
            }
            let inner = &value[start..end]; // content between var( and )
                                            // Split on first comma for fallback
            let (var_name, fallback) = if let Some(comma) = inner.find(',') {
                (inner[..comma].trim(), Some(inner[comma + 1..].trim()))
            } else {
                (inner.trim(), None)
            };
            // Look up the variable
            if let Some(val) = lookup_custom_property(var_name, node_cp, dom, node_id, ancestors_cp)
            {
                let use_fallback = custom_property_is_unset_keyword(val);
                if use_fallback {
                    if let Some(fb) = fallback {
                        let resolved_fb =
                            resolve_nested_vars(fb, dom, node_id, node_cp, ancestors_cp);
                        result.push_str(&resolved_fb);
                    } else {
                        let stop = (end + 1).min(bytes.len());
                        result.push_str(&value[i..stop]);
                    }
                } else {
                    let resolved_val = if val.contains("var(") {
                        resolve_nested_vars(val, dom, node_id, node_cp, ancestors_cp)
                    } else {
                        String::from(val)
                    };
                    result.push_str(&resolved_val);
                }
            } else if let Some(fb) = fallback {
                // Recursively resolve vars in fallback too
                let resolved_fb = resolve_nested_vars(fb, dom, node_id, node_cp, ancestors_cp);
                result.push_str(&resolved_fb);
            } else {
                // No value, no fallback — keep original
                let stop = (end + 1).min(bytes.len());
                result.push_str(&value[i..stop]);
            }
            i = (end + 1).min(bytes.len()); // skip past closing )
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

/// Resolve a declaration that has nested var() in its Keyword value.
fn resolve_nested_var_decl(
    decl: &Declaration,
    dom: &Dom,
    node_id: NodeId,
    node_cp: &[(String, String)],
    ancestors_cp: &[Vec<(String, String)>],
) -> Declaration {
    if let CssValue::Keyword(ref s) = decl.value {
        let resolved_str = resolve_nested_vars(s, dom, node_id, node_cp, ancestors_cp);
        let resolved = crate::css::parse_value(&decl.property, &resolved_str);
        Declaration {
            property: decl.property.clone(),
            value: resolved,
            important: decl.important,
        }
    } else {
        decl.clone()
    }
}

// ---------------------------------------------------------------------------

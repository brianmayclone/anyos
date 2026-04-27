fn normalize_stylesheet_input(css: &str) -> String {
    let mut text = css.trim();
    if let Some(stripped) = text.strip_prefix("<!--") {
        text = stripped.trim_start();
    }
    if let Some(stripped) = text.strip_suffix("-->") {
        text = stripped.trim_end();
    }
    if let Some(stripped) = text.strip_prefix("<![CDATA[") {
        text = stripped.trim_start();
    }
    if let Some(stripped) = text.strip_suffix("]]>") {
        text = stripped.trim_end();
    }
    String::from(text)
}

pub fn parse_stylesheet(css: &str) -> Stylesheet {
    crate::debug_surf!("[css] parse_stylesheet: {} bytes", css.len());
    let css_text = normalize_stylesheet_input(css);
    let ast = parse_stylesheet_ast(&css_text);
    let mut rules = Vec::new();
    let mut media_rules = Vec::new();
    let mut keyframes = Vec::new();
    let mut imports = Vec::new();
    let mut font_faces = Vec::new();
    let mut layer_order = Vec::new();
    let mut layer_stack: Vec<String> = Vec::new();
    let mut anon_layer_counter: u32 = 0;

    lower_ast_items(
        &ast.items,
        &mut layer_stack,
        &mut layer_order,
        &mut anon_layer_counter,
        &mut rules,
        &mut media_rules,
        &mut keyframes,
        &mut imports,
        &mut font_faces,
    );

    crate::debug_surf!(
        "[css] parse_stylesheet done: {} rules, {} @media, {} @keyframes, {} imports",
        rules.len(),
        media_rules.len(),
        keyframes.len(),
        imports.len()
    );
    Stylesheet {
        rules,
        layer_order,
        media_rules,
        keyframes,
        imports,
        font_faces,
    }
}

fn lower_ast_items(
    items: &[CssSyntaxNode],
    layer_stack: &mut Vec<String>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
    rules: &mut Vec<Rule>,
    media_rules: &mut Vec<MediaRule>,
    keyframes: &mut Vec<KeyframeSet>,
    imports: &mut Vec<String>,
    font_faces: &mut Vec<FontFaceRule>,
) {
    for item in items {
        match item {
            CssSyntaxNode::QualifiedRule(node) => {
                if rules.len() >= MAX_CSS_RULES {
                    crate::debug_surf!("[css] RULE LIMIT REACHED: {} rules — stopping", rules.len());
                    break;
                }
                for rule in lower_qualified_rule_ast(node, layer_stack.last().map(|s| s.as_str())) {
                    if rules.len() >= MAX_CSS_RULES {
                        break;
                    }
                    rules.push(rule);
                }
            }
            CssSyntaxNode::AtRule(node) => {
                lower_at_rule_ast(
                    node,
                    layer_stack,
                    layer_order,
                    anon_layer_counter,
                    rules,
                    media_rules,
                    keyframes,
                    imports,
                    font_faces,
                );
            }
        }
    }
}

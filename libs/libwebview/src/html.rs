// html.rs — HTML tokenizer + tree-builder for surf browser
// Handles real-world HTML: entities, void elements, auto-closing, implicit structure.

use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::{Attr, Dom, NodeId, NodeType, Tag};

// ---------------------------------------------------------------------------
// Phase 1: Tokenizer
// ---------------------------------------------------------------------------

enum Token {
    Doctype,
    StartTag {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    EndTag {
        name: String,
    },
    Text(String),
    Comment,
}

/// Decode a single HTML entity starting after the `&`.
/// Returns (decoded char(s), bytes consumed including the `;`).
fn decode_entity(s: &[u8]) -> (char, usize) {
    // Numeric: &#NNN; or &#xHH;
    if s.first() == Some(&b'#') {
        let (radix, start) = if s.get(1) == Some(&b'x') || s.get(1) == Some(&b'X') {
            (16, 2)
        } else {
            (10, 1)
        };
        let mut val: u32 = 0;
        let mut i = start;
        while i < s.len() && s[i] != b';' {
            let d = match s[i] {
                b'0'..=b'9' => (s[i] - b'0') as u32,
                b'a'..=b'f' if radix == 16 => (s[i] - b'a' + 10) as u32,
                b'A'..=b'F' if radix == 16 => (s[i] - b'A' + 10) as u32,
                _ => break,
            };
            val = val * radix + d;
            i += 1;
        }
        let consumed = if i < s.len() && s[i] == b';' { i + 1 } else { i };
        let ch = char::from_u32(val).unwrap_or('\u{FFFD}');
        return (ch, consumed);
    }

    // Named entities
    static NAMED: &[(&[u8], char)] = &[
        (b"amp", '&'),
        (b"lt", '<'),
        (b"gt", '>'),
        (b"quot", '"'),
        (b"apos", '\''),
        (b"nbsp", '\u{00A0}'),
        (b"copy", '\u{00A9}'),
        (b"reg", '\u{00AE}'),
        (b"trade", '\u{2122}'),
        (b"ndash", '\u{2013}'),
        (b"mdash", '\u{2014}'),
        (b"laquo", '\u{00AB}'),
        (b"raquo", '\u{00BB}'),
        (b"bull", '\u{2022}'),
        (b"middot", '\u{00B7}'),
        (b"hellip", '\u{2026}'),
        (b"lsquo", '\u{2018}'),
        (b"rsquo", '\u{2019}'),
        (b"ldquo", '\u{201C}'),
        (b"rdquo", '\u{201D}'),
        (b"euro", '\u{20AC}'),
        (b"pound", '\u{00A3}'),
        (b"yen", '\u{00A5}'),
        (b"cent", '\u{00A2}'),
        (b"times", '\u{00D7}'),
        (b"divide", '\u{00F7}'),
        (b"rarr", '\u{2192}'),
        (b"larr", '\u{2190}'),
        (b"uarr", '\u{2191}'),
        (b"darr", '\u{2193}'),
    ];
    for &(name, ch) in NAMED {
        if s.len() >= name.len() && &s[..name.len()] == name {
            let consumed = if s.get(name.len()) == Some(&b';') {
                name.len() + 1
            } else {
                name.len()
            };
            return (ch, consumed);
        }
    }

    // Unknown entity — emit `&` literally and consume nothing
    ('&', 0)
}

/// Collect text with entity decoding from `bytes` starting at `pos`.
/// Stops at `<` or end of input.
fn collect_text(bytes: &[u8], pos: &mut usize) -> String {
    let mut out = String::new();
    while *pos < bytes.len() && bytes[*pos] != b'<' {
        if bytes[*pos] == b'&' {
            *pos += 1; // skip '&'
            let (ch, consumed) = decode_entity(&bytes[*pos..]);
            out.push(ch);
            *pos += consumed;
        } else {
            // Fast path: copy ASCII bytes until next special char
            let start = *pos;
            while *pos < bytes.len() && bytes[*pos] != b'<' && bytes[*pos] != b'&' {
                *pos += 1;
            }
            // bytes are assumed UTF-8 from the input &str
            if let Ok(s) = core::str::from_utf8(&bytes[start..*pos]) {
                out.push_str(s);
            }
        }
    }
    out
}

/// Read raw text content for `<script>` or `<style>` — stops at `</tag_name>`.
fn collect_raw_text(bytes: &[u8], pos: &mut usize, tag_name: &str) -> String {
    let mut out = String::new();
    let end_tag = {
        let mut e = String::from("</");
        e.push_str(tag_name);
        e
    };
    let end_bytes = end_tag.as_bytes();

    while *pos < bytes.len() {
        // Check for closing tag (case-insensitive)
        if bytes[*pos] == b'<'
            && *pos + end_bytes.len() < bytes.len()
            && bytes[*pos..*pos + end_bytes.len()]
                .iter()
                .zip(end_bytes.iter())
                .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
        {
            // Verify next char is '>' or whitespace
            let after = *pos + end_bytes.len();
            if after < bytes.len()
                && (bytes[after] == b'>' || bytes[after] == b' ' || bytes[after] == b'\t')
            {
                // Skip past the closing tag
                *pos = after;
                while *pos < bytes.len() && bytes[*pos] != b'>' {
                    *pos += 1;
                }
                if *pos < bytes.len() {
                    *pos += 1; // skip '>'
                }
                return out;
            }
        }
        if let Ok(s) = core::str::from_utf8(&bytes[*pos..*pos + 1]) {
            out.push_str(s);
        }
        *pos += 1;
    }
    out
}

fn skip_whitespace(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() && is_ws(bytes[*pos]) {
        *pos += 1;
    }
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0C)
}

fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':' || b == b'.'
}

/// Read a tag or attribute name (lowercase).
fn read_name(bytes: &[u8], pos: &mut usize) -> String {
    let start = *pos;
    while *pos < bytes.len() && is_name_char(bytes[*pos]) {
        *pos += 1;
    }
    let mut name = String::new();
    for &b in &bytes[start..*pos] {
        name.push((b as char).to_ascii_lowercase());
    }
    name
}

/// Parse attributes inside a tag. `pos` should be right after the tag name.
/// Returns list of (name, value) pairs and whether the tag is self-closing.
fn parse_attrs(bytes: &[u8], pos: &mut usize) -> (Vec<(String, String)>, bool) {
    let mut attrs = Vec::new();
    loop {
        skip_whitespace(bytes, pos);
        if *pos >= bytes.len() {
            break;
        }
        // Self-closing />
        if bytes[*pos] == b'/' {
            *pos += 1;
            skip_whitespace(bytes, pos);
            if *pos < bytes.len() && bytes[*pos] == b'>' {
                *pos += 1;
                return (attrs, true);
            }
            continue;
        }
        if bytes[*pos] == b'>' {
            *pos += 1;
            return (attrs, false);
        }
        // Attribute name
        let name = read_name(bytes, pos);
        if name.is_empty() {
            // Skip unknown byte to avoid infinite loop
            *pos += 1;
            continue;
        }
        skip_whitespace(bytes, pos);
        // Check for '='
        if *pos < bytes.len() && bytes[*pos] == b'=' {
            *pos += 1; // skip '='
            skip_whitespace(bytes, pos);
            let value = read_attr_value(bytes, pos);
            attrs.push((name, value));
        } else {
            // Boolean attribute (no value)
            attrs.push((name, String::new()));
        }
    }
    (attrs, false)
}

/// Read an attribute value: quoted or unquoted.
fn read_attr_value(bytes: &[u8], pos: &mut usize) -> String {
    if *pos >= bytes.len() {
        return String::new();
    }
    let quote = bytes[*pos];
    if quote == b'"' || quote == b'\'' {
        *pos += 1; // skip opening quote
        let mut val = String::new();
        while *pos < bytes.len() && bytes[*pos] != quote {
            if bytes[*pos] == b'&' {
                *pos += 1;
                let (ch, consumed) = decode_entity(&bytes[*pos..]);
                val.push(ch);
                *pos += consumed;
            } else {
                val.push(bytes[*pos] as char);
                *pos += 1;
            }
        }
        if *pos < bytes.len() {
            *pos += 1; // skip closing quote
        }
        val
    } else {
        // Unquoted value — until whitespace, >, or /
        let mut val = String::new();
        while *pos < bytes.len()
            && !is_ws(bytes[*pos])
            && bytes[*pos] != b'>'
            && bytes[*pos] != b'/'
        {
            if bytes[*pos] == b'&' {
                *pos += 1;
                let (ch, consumed) = decode_entity(&bytes[*pos..]);
                val.push(ch);
                *pos += consumed;
            } else {
                val.push(bytes[*pos] as char);
                *pos += 1;
            }
        }
        val
    }
}

pub fn tokenize(html: &str) -> Vec<Token> {
    let bytes = html.as_bytes();
    let mut pos: usize = 0;
    let mut tokens = Vec::new();

    while pos < bytes.len() {
        if bytes[pos] == b'<' {
            pos += 1; // skip '<'
            if pos >= bytes.len() {
                break;
            }

            // Comment: <!-- ... -->
            if bytes.len() - pos >= 3
                && bytes[pos] == b'!'
                && bytes[pos + 1] == b'-'
                && bytes[pos + 2] == b'-'
            {
                pos += 3;
                // Scan for -->
                loop {
                    if pos + 2 >= bytes.len() {
                        pos = bytes.len(); // unterminated comment — consume rest
                        break;
                    }
                    if bytes[pos] == b'-' && bytes[pos + 1] == b'-' && bytes[pos + 2] == b'>' {
                        pos += 3;
                        break;
                    }
                    pos += 1;
                }
                tokens.push(Token::Comment);
                continue;
            }

            // Doctype or other <!...> declaration
            if bytes[pos] == b'!' {
                pos += 1; // skip '!'
                while pos < bytes.len() && bytes[pos] != b'>' {
                    pos += 1;
                }
                if pos < bytes.len() {
                    pos += 1;
                }
                tokens.push(Token::Doctype);
                continue;
            }

            // End tag: </name>
            if bytes[pos] == b'/' {
                pos += 1;
                skip_whitespace(bytes, &mut pos);
                let name = read_name(bytes, &mut pos);
                // Skip to '>'
                while pos < bytes.len() && bytes[pos] != b'>' {
                    pos += 1;
                }
                if pos < bytes.len() {
                    pos += 1;
                }
                if !name.is_empty() {
                    tokens.push(Token::EndTag { name });
                }
                continue;
            }

            // Processing instruction: <? ... > (skip)
            if bytes[pos] == b'?' {
                while pos < bytes.len() && bytes[pos] != b'>' {
                    pos += 1;
                }
                if pos < bytes.len() {
                    pos += 1;
                }
                continue;
            }

            // Start tag
            let name = read_name(bytes, &mut pos);
            if name.is_empty() {
                // Malformed tag — emit '<' as text
                tokens.push(Token::Text(String::from("<")));
                continue;
            }

            let (attrs, self_closing) = parse_attrs(bytes, &mut pos);

            // For raw text elements, collect content now
            let is_raw = name == "script" || name == "style";
            tokens.push(Token::StartTag {
                name: name.clone(),
                attrs,
                self_closing,
            });

            if is_raw && !self_closing {
                let raw = collect_raw_text(bytes, &mut pos, &name);
                if !raw.is_empty() {
                    tokens.push(Token::Text(raw));
                }
                tokens.push(Token::EndTag { name });
            }
        } else {
            // Text content
            let text = collect_text(bytes, &mut pos);
            if !text.is_empty() {
                tokens.push(Token::Text(text));
            }
        }
    }

    tokens
}

// ---------------------------------------------------------------------------
// Phase 2: Tree Builder
// ---------------------------------------------------------------------------

/// Tags that auto-close `<p>`.
fn closes_p(tag: Tag) -> bool {
    matches!(
        tag,
        Tag::P
            | Tag::Div
            | Tag::Section
            | Tag::Article
            | Tag::Header
            | Tag::Footer
            | Tag::Nav
            | Tag::Main
            | Tag::Blockquote
            | Tag::Pre
            | Tag::Ul
            | Tag::Ol
            | Tag::Li
            | Tag::Table
            | Tag::H1
            | Tag::H2
            | Tag::H3
            | Tag::H4
            | Tag::H5
            | Tag::H6
            | Tag::Hr
            | Tag::Form
    )
}

/// Collapse whitespace in text: runs of whitespace become a single space.
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for ch in s.chars() {
        if ch.is_ascii_whitespace() {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(ch);
            in_ws = false;
        }
    }
    out
}

/// Check if a tag is on the open element stack.
fn stack_has(dom: &Dom, stack: &[NodeId], tag: Tag) -> bool {
    stack.iter().any(|&id| match &dom.nodes[id].node_type {
        NodeType::Element { tag: t, .. } => *t == tag,
        _ => false,
    })
}

/// Get the tag of a node, if it is an element.
fn node_tag(dom: &Dom, id: NodeId) -> Option<Tag> {
    match &dom.nodes[id].node_type {
        NodeType::Element { tag, .. } => Some(*tag),
        _ => None,
    }
}

/// Pop the stack back to (and including) the first element matching `tag`.
fn pop_to(dom: &Dom, stack: &mut Vec<NodeId>, tag: Tag) {
    while let Some(&top) = stack.last() {
        let t = node_tag(dom, top);
        stack.pop();
        if t == Some(tag) {
            break;
        }
    }
}

/// Check if we are inside a `<pre>` element.
fn in_pre(dom: &Dom, stack: &[NodeId]) -> bool {
    stack_has(dom, stack, Tag::Pre)
}

pub fn parse(html: &str) -> Dom {
    let tokens = tokenize(html);
    let mut dom = Dom::new();

    // Create implicit root (html-like)
    let root = dom.add_node(
        NodeType::Element {
            tag: Tag::Html,
            attrs: Vec::new(),
        },
        None,
    );

    let mut stack: Vec<NodeId> = Vec::new();
    stack.push(root);

    // Track whether we've seen explicit structural tags
    let mut saw_head = false;
    let mut saw_body = false;
    let mut head_id: Option<NodeId> = None;
    let mut body_id: Option<NodeId> = None;

    // First pass: check for explicit html/head/body
    for tok in &tokens {
        if let Token::StartTag { name, .. } = tok {
            match name.as_str() {
                "head" => saw_head = true,
                "body" => saw_body = true,
                _ => {}
            }
        }
    }

    // If no explicit structure, create implicit <head> and <body>
    if !saw_head {
        head_id = Some(dom.add_node(
            NodeType::Element {
                tag: Tag::Head,
                attrs: Vec::new(),
            },
            Some(root),
        ));
    }
    if !saw_body {
        body_id = Some(dom.add_node(
            NodeType::Element {
                tag: Tag::Body,
                attrs: Vec::new(),
            },
            Some(root),
        ));
        // Push body as default insertion point when no explicit body
        if let Some(bid) = body_id {
            stack.push(bid);
        }
    }

    for tok in tokens {
        match tok {
            Token::Doctype | Token::Comment => {
                // Skip
            }

            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => {
                let tag = Tag::from_str(&name);
                let dom_attrs: Vec<Attr> = attrs
                    .into_iter()
                    .map(|(n, v)| Attr { name: n, value: v })
                    .collect();

                // Handle structural tags
                match tag {
                    Tag::Html => {
                        // Merge attrs onto root if desired; otherwise skip creating duplicate
                        if dom_attrs.is_empty() {
                            continue;
                        }
                        // Apply attrs to root node
                        if let NodeType::Element { ref mut attrs, .. } =
                            dom.nodes[root].node_type
                        {
                            *attrs = dom_attrs;
                        }
                        continue;
                    }
                    Tag::Head => {
                        if head_id.is_none() {
                            let hid = dom.add_node(
                                NodeType::Element {
                                    tag: Tag::Head,
                                    attrs: dom_attrs,
                                },
                                Some(root),
                            );
                            head_id = Some(hid);
                            stack.push(hid);
                        } else {
                            stack.push(head_id.unwrap());
                        }
                        continue;
                    }
                    Tag::Body => {
                        if body_id.is_none() {
                            let bid = dom.add_node(
                                NodeType::Element {
                                    tag: Tag::Body,
                                    attrs: dom_attrs,
                                },
                                Some(root),
                            );
                            body_id = Some(bid);
                            // Replace implicit body in stack
                            stack.retain(|&id| node_tag(&dom, id) != Some(Tag::Head));
                            stack.push(bid);
                        } else {
                            // Pop back to body level
                            while stack.len() > 1 {
                                if node_tag(&dom, *stack.last().unwrap()) == Some(Tag::Body) {
                                    break;
                                }
                                stack.pop();
                            }
                            if stack.last().map(|&id| node_tag(&dom, id)) != Some(Some(Tag::Body))
                            {
                                stack.push(body_id.unwrap());
                            }
                        }
                        continue;
                    }
                    _ => {}
                }

                // Head-only elements go into <head>
                let is_head_element =
                    matches!(tag, Tag::Title | Tag::Meta | Tag::Link | Tag::Style)
                        && !stack_has(&dom, &stack, Tag::Body);
                if is_head_element {
                    if let Some(hid) = head_id {
                        let parent = hid;
                        let id = dom.add_node(
                            NodeType::Element {
                                tag,
                                attrs: dom_attrs,
                            },
                            Some(parent),
                        );
                        if !tag.is_void() && !self_closing {
                            stack.push(id);
                        }
                        continue;
                    }
                }

                // Auto-close <p> when block element opens inside it
                if closes_p(tag) && stack_has(&dom, &stack, Tag::P) {
                    pop_to(&dom, &mut stack, Tag::P);
                }

                // Auto-close <li> when another <li> opens
                if tag == Tag::Li {
                    if let Some(&top) = stack.last() {
                        if node_tag(&dom, top) == Some(Tag::Li) {
                            stack.pop();
                        }
                    }
                }

                // Auto-close <td>/<th> when another opens
                if tag == Tag::Td || tag == Tag::Th {
                    if let Some(&top) = stack.last() {
                        let top_tag = node_tag(&dom, top);
                        if top_tag == Some(Tag::Td) || top_tag == Some(Tag::Th) {
                            stack.pop();
                        }
                    }
                }

                // Auto-close <tr> when another <tr> opens
                if tag == Tag::Tr {
                    if let Some(&top) = stack.last() {
                        if node_tag(&dom, top) == Some(Tag::Tr) {
                            stack.pop();
                        }
                    }
                }

                // Determine parent
                let parent = stack.last().copied().unwrap_or(root);

                let id = dom.add_node(
                    NodeType::Element {
                        tag,
                        attrs: dom_attrs,
                    },
                    Some(parent),
                );

                // Push to stack unless void or self-closing
                if !tag.is_void() && !self_closing {
                    stack.push(id);
                }
            }

            Token::EndTag { name } => {
                let tag = Tag::from_str(&name);

                match tag {
                    Tag::Html | Tag::Body => {
                        // Don't actually pop these — they stay until the end
                        continue;
                    }
                    Tag::Head => {
                        // Pop head and ensure body is on stack
                        if stack_has(&dom, &stack, Tag::Head) {
                            pop_to(&dom, &mut stack, Tag::Head);
                        }
                        // Ensure body is insertion point
                        if let Some(bid) = body_id {
                            if stack.last().copied() != Some(bid) {
                                stack.push(bid);
                            }
                        }
                        continue;
                    }
                    _ => {}
                }

                // Pop stack to matching open tag
                if stack_has(&dom, &stack, tag) {
                    pop_to(&dom, &mut stack, tag);
                }
                // If not found, just ignore the end tag (error recovery)
            }

            Token::Text(text) => {
                if text.is_empty() {
                    continue;
                }

                let processed = if in_pre(&dom, &stack) {
                    text
                } else {
                    collapse_whitespace(&text)
                };

                if processed.is_empty() {
                    continue;
                }

                let parent = stack.last().copied().unwrap_or(root);
                dom.add_node(NodeType::Text(processed), Some(parent));
            }
        }
    }

    dom
}

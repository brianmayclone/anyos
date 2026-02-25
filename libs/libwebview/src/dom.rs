//! Arena-based DOM tree for the Surf web browser.
//!
//! All nodes are stored in a flat `Vec<DomNode>` and referenced by `NodeId`
//! (a plain `usize` index). This avoids recursive Box/Rc trees and keeps
//! allocation patterns simple for the anyOS bump allocator.

use alloc::string::String;
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Node identity
// ---------------------------------------------------------------------------

/// Index into `Dom::nodes`.
pub type NodeId = usize;

// ---------------------------------------------------------------------------
// DOM tree
// ---------------------------------------------------------------------------

pub struct Dom {
    pub nodes: Vec<DomNode>,
}

pub struct DomNode {
    pub node_type: NodeType,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

pub enum NodeType {
    Element { tag: Tag, attrs: Vec<Attr> },
    Text(String),
}

pub struct Attr {
    pub name: String,
    pub value: String,
}

// ---------------------------------------------------------------------------
// HTML tag enum — comprehensive HTML5 support
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    // Document structure
    Html, Head, Title, Body, Style, Link, Meta, Script, Noscript, Template,
    // Headings
    H1, H2, H3, H4, H5, H6,
    // Content sectioning
    Div, Section, Header, Footer, Nav, Main, Article, Aside, Hgroup, Address,
    // Text content
    P, Br, Hr, Pre, Blockquote, Figure, Figcaption, Details, Summary, Dialog,
    // Inline text semantics
    A, Span, Em, Strong, B, I, U, S, Code, Mark, Small, Sub, Sup,
    Kbd, Samp, Var, Abbr, Cite, Dfn, Q, Time, Del, Ins, Bdi, Bdo,
    Data, Ruby, Rt, Rp, Wbr,
    // Lists
    Ul, Ol, Li, Dl, Dt, Dd,
    // Tables
    Table, Thead, Tbody, Tfoot, Tr, Th, Td, Caption, Colgroup, Col,
    // Forms
    Form, Input, Button, Textarea, Select, Option, Optgroup, Label,
    Fieldset, Legend, Datalist, Output, Progress, Meter,
    // Media/embedded
    Img, Audio, Video, Source, Track, Canvas, Svg, Iframe, Embed, Object, Param,
    Picture, Map, Area,
    // Deprecated but still encountered
    Center, Font, Nobr, Tt,
    // Unknown fallback
    Unknown,
}

// ---------------------------------------------------------------------------
// Tag helpers
// ---------------------------------------------------------------------------

impl Tag {
    /// Case-insensitive lookup from a tag name string.
    pub fn from_str(name: &str) -> Tag {
        let mut buf = [0u8; 16];
        let len = name.len().min(buf.len());
        for i in 0..len {
            buf[i] = ascii_lower(name.as_bytes()[i]);
        }
        let lower = &buf[..len];

        match lower {
            // Document structure
            b"html" => Tag::Html, b"head" => Tag::Head, b"title" => Tag::Title,
            b"body" => Tag::Body, b"style" => Tag::Style, b"link" => Tag::Link,
            b"meta" => Tag::Meta, b"script" => Tag::Script, b"noscript" => Tag::Noscript,
            b"template" => Tag::Template,
            // Headings
            b"h1" => Tag::H1, b"h2" => Tag::H2, b"h3" => Tag::H3,
            b"h4" => Tag::H4, b"h5" => Tag::H5, b"h6" => Tag::H6,
            // Content sectioning
            b"div" => Tag::Div, b"section" => Tag::Section, b"header" => Tag::Header,
            b"footer" => Tag::Footer, b"nav" => Tag::Nav, b"main" => Tag::Main,
            b"article" => Tag::Article, b"aside" => Tag::Aside, b"hgroup" => Tag::Hgroup,
            b"address" => Tag::Address,
            // Text content
            b"p" => Tag::P, b"br" => Tag::Br, b"hr" => Tag::Hr, b"pre" => Tag::Pre,
            b"blockquote" => Tag::Blockquote, b"figure" => Tag::Figure,
            b"figcaption" => Tag::Figcaption, b"details" => Tag::Details,
            b"summary" => Tag::Summary, b"dialog" => Tag::Dialog,
            // Inline text
            b"a" => Tag::A, b"span" => Tag::Span, b"em" => Tag::Em,
            b"strong" => Tag::Strong, b"b" => Tag::B, b"i" => Tag::I,
            b"u" => Tag::U, b"s" => Tag::S, b"code" => Tag::Code,
            b"mark" => Tag::Mark, b"small" => Tag::Small,
            b"sub" => Tag::Sub, b"sup" => Tag::Sup, b"kbd" => Tag::Kbd,
            b"samp" => Tag::Samp, b"var" => Tag::Var, b"abbr" => Tag::Abbr,
            b"cite" => Tag::Cite, b"dfn" => Tag::Dfn, b"q" => Tag::Q,
            b"time" => Tag::Time, b"del" => Tag::Del, b"ins" => Tag::Ins,
            b"bdi" => Tag::Bdi, b"bdo" => Tag::Bdo, b"data" => Tag::Data,
            b"ruby" => Tag::Ruby, b"rt" => Tag::Rt, b"rp" => Tag::Rp,
            b"wbr" => Tag::Wbr,
            // Lists
            b"ul" => Tag::Ul, b"ol" => Tag::Ol, b"li" => Tag::Li,
            b"dl" => Tag::Dl, b"dt" => Tag::Dt, b"dd" => Tag::Dd,
            // Tables
            b"table" => Tag::Table, b"thead" => Tag::Thead, b"tbody" => Tag::Tbody,
            b"tfoot" => Tag::Tfoot, b"tr" => Tag::Tr, b"th" => Tag::Th,
            b"td" => Tag::Td, b"caption" => Tag::Caption,
            b"colgroup" => Tag::Colgroup, b"col" => Tag::Col,
            // Forms
            b"form" => Tag::Form, b"input" => Tag::Input, b"button" => Tag::Button,
            b"textarea" => Tag::Textarea, b"select" => Tag::Select,
            b"option" => Tag::Option, b"optgroup" => Tag::Optgroup,
            b"label" => Tag::Label, b"fieldset" => Tag::Fieldset,
            b"legend" => Tag::Legend, b"datalist" => Tag::Datalist,
            b"output" => Tag::Output, b"progress" => Tag::Progress,
            b"meter" => Tag::Meter,
            // Media/embedded
            b"img" => Tag::Img, b"audio" => Tag::Audio, b"video" => Tag::Video,
            b"source" => Tag::Source, b"track" => Tag::Track,
            b"canvas" => Tag::Canvas, b"svg" => Tag::Svg,
            b"iframe" => Tag::Iframe, b"embed" => Tag::Embed,
            b"object" => Tag::Object, b"param" => Tag::Param,
            b"picture" => Tag::Picture, b"map" => Tag::Map, b"area" => Tag::Area,
            // Deprecated
            b"center" => Tag::Center, b"font" => Tag::Font,
            b"nobr" => Tag::Nobr, b"tt" => Tag::Tt,
            _ => Tag::Unknown,
        }
    }

    /// Void elements are self-closing and cannot have children.
    pub fn is_void(&self) -> bool {
        matches!(
            self,
            Tag::Br | Tag::Hr | Tag::Img | Tag::Input | Tag::Meta | Tag::Link
                | Tag::Col | Tag::Embed | Tag::Source | Tag::Track | Tag::Wbr
                | Tag::Area | Tag::Param
        )
    }

    /// Block-level elements start on a new line and span the full width.
    pub fn is_block(&self) -> bool {
        matches!(
            self,
            Tag::Div | Tag::P
                | Tag::H1 | Tag::H2 | Tag::H3 | Tag::H4 | Tag::H5 | Tag::H6
                | Tag::Ul | Tag::Ol | Tag::Li | Tag::Dl | Tag::Dt | Tag::Dd
                | Tag::Table | Tag::Thead | Tag::Tbody | Tag::Tfoot | Tag::Tr
                | Tag::Caption | Tag::Colgroup
                | Tag::Blockquote | Tag::Pre | Tag::Figure | Tag::Figcaption
                | Tag::Section | Tag::Article | Tag::Header | Tag::Footer
                | Tag::Nav | Tag::Main | Tag::Aside | Tag::Hgroup | Tag::Address
                | Tag::Details | Tag::Summary | Tag::Dialog
                | Tag::Form | Tag::Fieldset | Tag::Legend
                | Tag::Hr | Tag::Center
                | Tag::Noscript | Tag::Canvas | Tag::Video | Tag::Audio
        )
    }

    /// Return the uppercase tag name string (e.g., "DIV", "A", "INPUT").
    pub fn tag_name(&self) -> &'static str {
        match self {
            Tag::Html => "HTML", Tag::Head => "HEAD", Tag::Title => "TITLE",
            Tag::Body => "BODY", Tag::Style => "STYLE", Tag::Link => "LINK",
            Tag::Meta => "META", Tag::Script => "SCRIPT", Tag::Noscript => "NOSCRIPT",
            Tag::Template => "TEMPLATE",
            Tag::H1 => "H1", Tag::H2 => "H2", Tag::H3 => "H3",
            Tag::H4 => "H4", Tag::H5 => "H5", Tag::H6 => "H6",
            Tag::Div => "DIV", Tag::Section => "SECTION", Tag::Header => "HEADER",
            Tag::Footer => "FOOTER", Tag::Nav => "NAV", Tag::Main => "MAIN",
            Tag::Article => "ARTICLE", Tag::Aside => "ASIDE", Tag::Hgroup => "HGROUP",
            Tag::Address => "ADDRESS",
            Tag::P => "P", Tag::Br => "BR", Tag::Hr => "HR", Tag::Pre => "PRE",
            Tag::Blockquote => "BLOCKQUOTE", Tag::Figure => "FIGURE",
            Tag::Figcaption => "FIGCAPTION", Tag::Details => "DETAILS",
            Tag::Summary => "SUMMARY", Tag::Dialog => "DIALOG",
            Tag::A => "A", Tag::Span => "SPAN", Tag::Em => "EM",
            Tag::Strong => "STRONG", Tag::B => "B", Tag::I => "I",
            Tag::U => "U", Tag::S => "S", Tag::Code => "CODE",
            Tag::Mark => "MARK", Tag::Small => "SMALL",
            Tag::Sub => "SUB", Tag::Sup => "SUP", Tag::Kbd => "KBD",
            Tag::Samp => "SAMP", Tag::Var => "VAR", Tag::Abbr => "ABBR",
            Tag::Cite => "CITE", Tag::Dfn => "DFN", Tag::Q => "Q",
            Tag::Time => "TIME", Tag::Del => "DEL", Tag::Ins => "INS",
            Tag::Bdi => "BDI", Tag::Bdo => "BDO", Tag::Data => "DATA",
            Tag::Ruby => "RUBY", Tag::Rt => "RT", Tag::Rp => "RP", Tag::Wbr => "WBR",
            Tag::Ul => "UL", Tag::Ol => "OL", Tag::Li => "LI",
            Tag::Dl => "DL", Tag::Dt => "DT", Tag::Dd => "DD",
            Tag::Table => "TABLE", Tag::Thead => "THEAD", Tag::Tbody => "TBODY",
            Tag::Tfoot => "TFOOT", Tag::Tr => "TR", Tag::Th => "TH", Tag::Td => "TD",
            Tag::Caption => "CAPTION", Tag::Colgroup => "COLGROUP", Tag::Col => "COL",
            Tag::Form => "FORM", Tag::Input => "INPUT", Tag::Button => "BUTTON",
            Tag::Textarea => "TEXTAREA", Tag::Select => "SELECT", Tag::Option => "OPTION",
            Tag::Optgroup => "OPTGROUP", Tag::Label => "LABEL",
            Tag::Fieldset => "FIELDSET", Tag::Legend => "LEGEND",
            Tag::Datalist => "DATALIST", Tag::Output => "OUTPUT",
            Tag::Progress => "PROGRESS", Tag::Meter => "METER",
            Tag::Img => "IMG", Tag::Audio => "AUDIO", Tag::Video => "VIDEO",
            Tag::Source => "SOURCE", Tag::Track => "TRACK", Tag::Canvas => "CANVAS",
            Tag::Svg => "SVG", Tag::Iframe => "IFRAME", Tag::Embed => "EMBED",
            Tag::Object => "OBJECT", Tag::Param => "PARAM", Tag::Picture => "PICTURE",
            Tag::Map => "MAP", Tag::Area => "AREA",
            Tag::Center => "CENTER", Tag::Font => "FONT", Tag::Nobr => "NOBR", Tag::Tt => "TT",
            Tag::Unknown => "UNKNOWN",
        }
    }

    /// Inline elements flow within text.
    pub fn is_inline(&self) -> bool {
        matches!(
            self,
            Tag::A | Tag::Span | Tag::Em | Tag::Strong | Tag::B | Tag::I
                | Tag::U | Tag::S | Tag::Code | Tag::Mark | Tag::Small
                | Tag::Sub | Tag::Sup | Tag::Kbd | Tag::Samp | Tag::Var
                | Tag::Abbr | Tag::Cite | Tag::Dfn | Tag::Q | Tag::Time
                | Tag::Del | Tag::Ins | Tag::Bdi | Tag::Bdo | Tag::Data
                | Tag::Ruby | Tag::Rt | Tag::Rp | Tag::Wbr
                | Tag::Img | Tag::Input | Tag::Button | Tag::Label
                | Tag::Select | Tag::Textarea | Tag::Output | Tag::Progress
                | Tag::Meter | Tag::Nobr | Tag::Tt | Tag::Font
        )
    }
}

// ---------------------------------------------------------------------------
// Dom implementation
// ---------------------------------------------------------------------------

impl Dom {
    /// Create an empty DOM with no nodes.
    pub fn new() -> Dom {
        Dom { nodes: Vec::new() }
    }

    /// Append a node to the arena, wiring up the parent/child link.
    /// Returns the `NodeId` of the new node.
    pub fn add_node(&mut self, node_type: NodeType, parent: Option<NodeId>) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(DomNode {
            node_type,
            parent,
            children: Vec::new(),
        });
        if let Some(pid) = parent {
            self.nodes[pid].children.push(id);
        }
        id
    }

    /// Get a shared reference to a node by id.
    pub fn get(&self, id: NodeId) -> &DomNode {
        &self.nodes[id]
    }

    /// Get a mutable reference to a node by id.
    pub fn get_mut(&mut self, id: NodeId) -> &mut DomNode {
        &mut self.nodes[id]
    }

    /// Look up an attribute value on an Element node (case-insensitive name
    /// match). Returns `None` for Text nodes or if the attribute is absent.
    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        match &self.nodes[id].node_type {
            NodeType::Element { attrs, .. } => {
                for a in attrs {
                    if eq_ignore_case(&a.name, name) {
                        return Some(&a.value);
                    }
                }
                None
            }
            NodeType::Text(_) => None,
        }
    }

    /// Return the `Tag` of a node if it is an Element, `None` for Text nodes.
    pub fn tag(&self, id: NodeId) -> Option<Tag> {
        match &self.nodes[id].node_type {
            NodeType::Element { tag, .. } => Some(*tag),
            NodeType::Text(_) => None,
        }
    }

    /// Recursively collect all descendant text into a single `String`.
    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(id, &mut out);
        out
    }

    /// Find the first `<body>` element in the tree (breadth-first).
    pub fn find_body(&self) -> Option<NodeId> {
        for (i, node) in self.nodes.iter().enumerate() {
            if let NodeType::Element { tag: Tag::Body, .. } = &node.node_type {
                return Some(i);
            }
        }
        None
    }

    /// Find the first `<title>` element and return its text content.
    pub fn find_title(&self) -> Option<String> {
        for (i, node) in self.nodes.iter().enumerate() {
            if let NodeType::Element { tag: Tag::Title, .. } = &node.node_type {
                let text = self.text_content(i);
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
        None
    }

    // -- mutation methods ---------------------------------------------------

    /// Set or add an attribute on an element node.
    pub fn set_attr(&mut self, id: NodeId, name: &str, value: &str) {
        if id >= self.nodes.len() { return; }
        if let NodeType::Element { attrs, .. } = &mut self.nodes[id].node_type {
            if let Some(attr) = attrs.iter_mut().find(|a| a.name == name) {
                attr.value = String::from(value);
            } else {
                attrs.push(Attr { name: String::from(name), value: String::from(value) });
            }
        }
    }

    /// Remove an attribute from an element node.
    pub fn remove_attr(&mut self, id: NodeId, name: &str) {
        if id >= self.nodes.len() { return; }
        if let NodeType::Element { attrs, .. } = &mut self.nodes[id].node_type {
            attrs.retain(|a| a.name != name);
        }
    }

    /// Replace all children with a single text node.
    pub fn set_text(&mut self, id: NodeId, text: &str) {
        if id >= self.nodes.len() { return; }
        // Clear existing children.
        self.nodes[id].children.clear();
        // Add text node if non-empty.
        if !text.is_empty() {
            let _text_id = self.add_node(NodeType::Text(String::from(text)), Some(id));
        }
    }

    /// Move a child node under a new parent (appended at end).
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if parent >= self.nodes.len() || child >= self.nodes.len() { return; }
        // Remove from old parent.
        if let Some(old_parent) = self.nodes[child].parent {
            if old_parent < self.nodes.len() {
                self.nodes[old_parent].children.retain(|&c| c != child);
            }
        }
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
    }

    /// Remove a child from a parent.
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if parent >= self.nodes.len() || child >= self.nodes.len() { return; }
        self.nodes[parent].children.retain(|&c| c != child);
        self.nodes[child].parent = None;
    }

    /// Insert new_child before ref_child under parent.
    pub fn insert_before(&mut self, parent: NodeId, new_child: NodeId, ref_child: NodeId) {
        if parent >= self.nodes.len() || new_child >= self.nodes.len() { return; }
        // Remove from old parent.
        if let Some(old_parent) = self.nodes[new_child].parent {
            if old_parent < self.nodes.len() {
                self.nodes[old_parent].children.retain(|&c| c != new_child);
            }
        }
        self.nodes[new_child].parent = Some(parent);
        if let Some(pos) = self.nodes[parent].children.iter().position(|&c| c == ref_child) {
            self.nodes[parent].children.insert(pos, new_child);
        } else {
            self.nodes[parent].children.push(new_child);
        }
    }

    // -- private helpers ----------------------------------------------------

    fn collect_text(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id].node_type {
            NodeType::Text(s) => out.push_str(s),
            NodeType::Element { .. } => {
                // Must collect children indices first to avoid holding an
                // immutable borrow on self.nodes while recursing.
                let len = self.nodes[id].children.len();
                for ci in 0..len {
                    let child = self.nodes[id].children[ci];
                    self.collect_text(child, out);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Private string helpers
// ---------------------------------------------------------------------------

fn ascii_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' {
        b + 32
    } else {
        b
    }
}

fn eq_ignore_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    for i in 0..ab.len() {
        if ascii_lower(ab[i]) != ascii_lower(bb[i]) {
            return false;
        }
    }
    true
}

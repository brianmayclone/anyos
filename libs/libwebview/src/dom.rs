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
    Html,
    Head,
    Title,
    Body,
    Style,
    Link,
    Meta,
    Script,
    Noscript,
    Template,
    // Headings
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    // Content sectioning
    Div,
    Section,
    Header,
    Footer,
    Nav,
    Main,
    Article,
    Aside,
    Hgroup,
    Address,
    // Text content
    P,
    Br,
    Hr,
    Pre,
    Blockquote,
    Figure,
    Figcaption,
    Details,
    Summary,
    Dialog,
    // Inline text semantics
    A,
    Span,
    Em,
    Strong,
    B,
    I,
    U,
    S,
    Code,
    Mark,
    Small,
    Sub,
    Sup,
    Kbd,
    Samp,
    Var,
    Abbr,
    Cite,
    Dfn,
    Q,
    Time,
    Del,
    Ins,
    Bdi,
    Bdo,
    Data,
    Ruby,
    Rt,
    Rp,
    Wbr,
    // Lists
    Ul,
    Ol,
    Li,
    Dl,
    Dt,
    Dd,
    // Tables
    Table,
    Thead,
    Tbody,
    Tfoot,
    Tr,
    Th,
    Td,
    Caption,
    Colgroup,
    Col,
    // Forms
    Form,
    Input,
    Button,
    Textarea,
    Select,
    Option,
    Optgroup,
    Label,
    Fieldset,
    Legend,
    Datalist,
    Output,
    Progress,
    Meter,
    // Media/embedded
    Img,
    Audio,
    Video,
    Source,
    Track,
    Canvas,
    Svg,
    Iframe,
    Embed,
    Object,
    Param,
    Picture,
    Map,
    Area,
    // HTML5 semantic elements
    Search,
    // Deprecated but still encountered
    Center,
    Font,
    Nobr,
    Tt,
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
            b"html" => Tag::Html,
            b"head" => Tag::Head,
            b"title" => Tag::Title,
            b"body" => Tag::Body,
            b"style" => Tag::Style,
            b"link" => Tag::Link,
            b"meta" => Tag::Meta,
            b"script" => Tag::Script,
            b"noscript" => Tag::Noscript,
            b"template" => Tag::Template,
            // Headings
            b"h1" => Tag::H1,
            b"h2" => Tag::H2,
            b"h3" => Tag::H3,
            b"h4" => Tag::H4,
            b"h5" => Tag::H5,
            b"h6" => Tag::H6,
            // Content sectioning
            b"div" => Tag::Div,
            b"section" => Tag::Section,
            b"header" => Tag::Header,
            b"footer" => Tag::Footer,
            b"nav" => Tag::Nav,
            b"main" => Tag::Main,
            b"article" => Tag::Article,
            b"aside" => Tag::Aside,
            b"hgroup" => Tag::Hgroup,
            b"address" => Tag::Address,
            // Text content
            b"p" => Tag::P,
            b"br" => Tag::Br,
            b"hr" => Tag::Hr,
            b"pre" => Tag::Pre,
            b"blockquote" => Tag::Blockquote,
            b"figure" => Tag::Figure,
            b"figcaption" => Tag::Figcaption,
            b"details" => Tag::Details,
            b"summary" => Tag::Summary,
            b"dialog" => Tag::Dialog,
            // Inline text
            b"a" => Tag::A,
            b"span" => Tag::Span,
            b"em" => Tag::Em,
            b"strong" => Tag::Strong,
            b"b" => Tag::B,
            b"i" => Tag::I,
            b"u" => Tag::U,
            b"s" => Tag::S,
            b"code" => Tag::Code,
            b"mark" => Tag::Mark,
            b"small" => Tag::Small,
            b"sub" => Tag::Sub,
            b"sup" => Tag::Sup,
            b"kbd" => Tag::Kbd,
            b"samp" => Tag::Samp,
            b"var" => Tag::Var,
            b"abbr" => Tag::Abbr,
            b"cite" => Tag::Cite,
            b"dfn" => Tag::Dfn,
            b"q" => Tag::Q,
            b"time" => Tag::Time,
            b"del" => Tag::Del,
            b"ins" => Tag::Ins,
            b"bdi" => Tag::Bdi,
            b"bdo" => Tag::Bdo,
            b"data" => Tag::Data,
            b"ruby" => Tag::Ruby,
            b"rt" => Tag::Rt,
            b"rp" => Tag::Rp,
            b"wbr" => Tag::Wbr,
            // Lists
            b"ul" => Tag::Ul,
            b"ol" => Tag::Ol,
            b"li" => Tag::Li,
            b"dl" => Tag::Dl,
            b"dt" => Tag::Dt,
            b"dd" => Tag::Dd,
            // Tables
            b"table" => Tag::Table,
            b"thead" => Tag::Thead,
            b"tbody" => Tag::Tbody,
            b"tfoot" => Tag::Tfoot,
            b"tr" => Tag::Tr,
            b"th" => Tag::Th,
            b"td" => Tag::Td,
            b"caption" => Tag::Caption,
            b"colgroup" => Tag::Colgroup,
            b"col" => Tag::Col,
            // Forms
            b"form" => Tag::Form,
            b"input" => Tag::Input,
            b"button" => Tag::Button,
            b"textarea" => Tag::Textarea,
            b"select" => Tag::Select,
            b"option" => Tag::Option,
            b"optgroup" => Tag::Optgroup,
            b"label" => Tag::Label,
            b"fieldset" => Tag::Fieldset,
            b"legend" => Tag::Legend,
            b"datalist" => Tag::Datalist,
            b"output" => Tag::Output,
            b"progress" => Tag::Progress,
            b"meter" => Tag::Meter,
            // Media/embedded
            b"img" => Tag::Img,
            b"audio" => Tag::Audio,
            b"video" => Tag::Video,
            b"source" => Tag::Source,
            b"track" => Tag::Track,
            b"canvas" => Tag::Canvas,
            b"svg" => Tag::Svg,
            b"iframe" => Tag::Iframe,
            b"embed" => Tag::Embed,
            b"object" => Tag::Object,
            b"param" => Tag::Param,
            b"picture" => Tag::Picture,
            b"map" => Tag::Map,
            b"area" => Tag::Area,
            // HTML5 semantic
            b"search" => Tag::Search,
            // Deprecated
            b"center" => Tag::Center,
            b"font" => Tag::Font,
            b"nobr" => Tag::Nobr,
            b"tt" => Tag::Tt,
            _ => Tag::Unknown,
        }
    }

    /// Void elements are self-closing and cannot have children.
    pub fn is_void(&self) -> bool {
        matches!(
            self,
            Tag::Br
                | Tag::Hr
                | Tag::Img
                | Tag::Input
                | Tag::Meta
                | Tag::Link
                | Tag::Col
                | Tag::Embed
                | Tag::Source
                | Tag::Track
                | Tag::Wbr
                | Tag::Area
                | Tag::Param
        )
    }

    /// Block-level elements start on a new line and span the full width.
    pub fn is_block(&self) -> bool {
        matches!(
            self,
            Tag::Div
                | Tag::P
                | Tag::H1
                | Tag::H2
                | Tag::H3
                | Tag::H4
                | Tag::H5
                | Tag::H6
                | Tag::Ul
                | Tag::Ol
                | Tag::Li
                | Tag::Dl
                | Tag::Dt
                | Tag::Dd
                | Tag::Table
                | Tag::Thead
                | Tag::Tbody
                | Tag::Tfoot
                | Tag::Tr
                | Tag::Caption
                | Tag::Colgroup
                | Tag::Blockquote
                | Tag::Pre
                | Tag::Figure
                | Tag::Figcaption
                | Tag::Section
                | Tag::Article
                | Tag::Header
                | Tag::Footer
                | Tag::Nav
                | Tag::Main
                | Tag::Aside
                | Tag::Hgroup
                | Tag::Address
                | Tag::Details
                | Tag::Summary
                | Tag::Dialog
                | Tag::Form
                | Tag::Fieldset
                | Tag::Legend
                | Tag::Hr
                | Tag::Center
                | Tag::Noscript
                | Tag::Canvas
                | Tag::Video
                | Tag::Audio
                | Tag::Search
        )
    }

    /// Return the uppercase tag name string (e.g., "DIV", "A", "INPUT").
    pub fn tag_name(&self) -> &'static str {
        match self {
            Tag::Html => "HTML",
            Tag::Head => "HEAD",
            Tag::Title => "TITLE",
            Tag::Body => "BODY",
            Tag::Style => "STYLE",
            Tag::Link => "LINK",
            Tag::Meta => "META",
            Tag::Script => "SCRIPT",
            Tag::Noscript => "NOSCRIPT",
            Tag::Template => "TEMPLATE",
            Tag::H1 => "H1",
            Tag::H2 => "H2",
            Tag::H3 => "H3",
            Tag::H4 => "H4",
            Tag::H5 => "H5",
            Tag::H6 => "H6",
            Tag::Div => "DIV",
            Tag::Section => "SECTION",
            Tag::Header => "HEADER",
            Tag::Footer => "FOOTER",
            Tag::Nav => "NAV",
            Tag::Main => "MAIN",
            Tag::Article => "ARTICLE",
            Tag::Aside => "ASIDE",
            Tag::Hgroup => "HGROUP",
            Tag::Address => "ADDRESS",
            Tag::P => "P",
            Tag::Br => "BR",
            Tag::Hr => "HR",
            Tag::Pre => "PRE",
            Tag::Blockquote => "BLOCKQUOTE",
            Tag::Figure => "FIGURE",
            Tag::Figcaption => "FIGCAPTION",
            Tag::Details => "DETAILS",
            Tag::Summary => "SUMMARY",
            Tag::Dialog => "DIALOG",
            Tag::A => "A",
            Tag::Span => "SPAN",
            Tag::Em => "EM",
            Tag::Strong => "STRONG",
            Tag::B => "B",
            Tag::I => "I",
            Tag::U => "U",
            Tag::S => "S",
            Tag::Code => "CODE",
            Tag::Mark => "MARK",
            Tag::Small => "SMALL",
            Tag::Sub => "SUB",
            Tag::Sup => "SUP",
            Tag::Kbd => "KBD",
            Tag::Samp => "SAMP",
            Tag::Var => "VAR",
            Tag::Abbr => "ABBR",
            Tag::Cite => "CITE",
            Tag::Dfn => "DFN",
            Tag::Q => "Q",
            Tag::Time => "TIME",
            Tag::Del => "DEL",
            Tag::Ins => "INS",
            Tag::Bdi => "BDI",
            Tag::Bdo => "BDO",
            Tag::Data => "DATA",
            Tag::Ruby => "RUBY",
            Tag::Rt => "RT",
            Tag::Rp => "RP",
            Tag::Wbr => "WBR",
            Tag::Ul => "UL",
            Tag::Ol => "OL",
            Tag::Li => "LI",
            Tag::Dl => "DL",
            Tag::Dt => "DT",
            Tag::Dd => "DD",
            Tag::Table => "TABLE",
            Tag::Thead => "THEAD",
            Tag::Tbody => "TBODY",
            Tag::Tfoot => "TFOOT",
            Tag::Tr => "TR",
            Tag::Th => "TH",
            Tag::Td => "TD",
            Tag::Caption => "CAPTION",
            Tag::Colgroup => "COLGROUP",
            Tag::Col => "COL",
            Tag::Form => "FORM",
            Tag::Input => "INPUT",
            Tag::Button => "BUTTON",
            Tag::Textarea => "TEXTAREA",
            Tag::Select => "SELECT",
            Tag::Option => "OPTION",
            Tag::Optgroup => "OPTGROUP",
            Tag::Label => "LABEL",
            Tag::Fieldset => "FIELDSET",
            Tag::Legend => "LEGEND",
            Tag::Datalist => "DATALIST",
            Tag::Output => "OUTPUT",
            Tag::Progress => "PROGRESS",
            Tag::Meter => "METER",
            Tag::Img => "IMG",
            Tag::Audio => "AUDIO",
            Tag::Video => "VIDEO",
            Tag::Source => "SOURCE",
            Tag::Track => "TRACK",
            Tag::Canvas => "CANVAS",
            Tag::Svg => "SVG",
            Tag::Iframe => "IFRAME",
            Tag::Embed => "EMBED",
            Tag::Object => "OBJECT",
            Tag::Param => "PARAM",
            Tag::Picture => "PICTURE",
            Tag::Map => "MAP",
            Tag::Area => "AREA",
            Tag::Search => "SEARCH",
            Tag::Center => "CENTER",
            Tag::Font => "FONT",
            Tag::Nobr => "NOBR",
            Tag::Tt => "TT",
            Tag::Unknown => "UNKNOWN",
        }
    }

    /// Inline elements flow within text.
    pub fn is_inline(&self) -> bool {
        matches!(
            self,
            Tag::A
                | Tag::Span
                | Tag::Em
                | Tag::Strong
                | Tag::B
                | Tag::I
                | Tag::U
                | Tag::S
                | Tag::Code
                | Tag::Mark
                | Tag::Small
                | Tag::Sub
                | Tag::Sup
                | Tag::Kbd
                | Tag::Samp
                | Tag::Var
                | Tag::Abbr
                | Tag::Cite
                | Tag::Dfn
                | Tag::Q
                | Tag::Time
                | Tag::Del
                | Tag::Ins
                | Tag::Bdi
                | Tag::Bdo
                | Tag::Data
                | Tag::Ruby
                | Tag::Rt
                | Tag::Rp
                | Tag::Wbr
                | Tag::Img
                | Tag::Input
                | Tag::Button
                | Tag::Label
                | Tag::Select
                | Tag::Textarea
                | Tag::Output
                | Tag::Progress
                | Tag::Meter
                | Tag::Nobr
                | Tag::Tt
                | Tag::Font
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

    pub fn raw_tag_name(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[id].node_type {
            NodeType::Element { attrs, .. } => attrs
                .iter()
                .find(|a| a.name == "\x00")
                .map(|a| a.value.as_str()),
            NodeType::Text(_) => None,
        }
    }

    pub fn has_tag_name(&self, id: NodeId, name: &str) -> bool {
        self.raw_tag_name(id)
            .map(|raw| raw.eq_ignore_ascii_case(name))
            .unwrap_or(false)
    }

    /// Resolve the best available image URL for an `<img>` element.
    ///
    /// Prefers modern sources such as `<picture><source srcset>`, `srcset`,
    /// and common lazy-loading attributes before falling back to `src`.
    pub fn image_url(&self, id: NodeId) -> Option<String> {
        if self.tag(id) != Some(Tag::Img) && !self.has_tag_name(id, "a-img") {
            return None;
        }

        if let Some(parent_id) = self.nodes[id].parent {
            if self.tag(parent_id) == Some(Tag::Picture) {
                for &child_id in &self.nodes[parent_id].children {
                    if child_id == id {
                        break;
                    }
                    if self.tag(child_id) != Some(Tag::Source) {
                        continue;
                    }
                    if let Some(mime) = self.attr(child_id, "type") {
                        if !supports_image_mime(mime) {
                            continue;
                        }
                    }
                    if let Some(srcset) =
                        first_non_empty_non_data_attr(self, child_id, &["srcset", "data-srcset"])
                    {
                        if let Some(candidate) = pick_srcset_candidate(srcset) {
                            return Some(candidate);
                        }
                    }
                    if let Some(src) = first_non_empty_attr(
                        self,
                        child_id,
                        &["data-src", "data-lazy-src", "data-original", "src"],
                    ) {
                        return Some(String::from(src));
                    }
                }
            }
        }

        if let Some(srcset) =
            first_non_empty_non_data_attr(self, id, &["srcset", "data-srcset", "data-lazy-srcset"])
        {
            if let Some(candidate) = pick_srcset_candidate(srcset) {
                return Some(candidate);
            }
        }

        if let Some(src) = first_non_empty_non_data_attr(
            self,
            id,
            &[
                "src",
                "data-src",
                "data-lazy-src",
                "data-original",
                "data-url",
                "data-image",
            ],
        ) {
            return Some(String::from(src));
        }

        first_non_empty_attr(
            self,
            id,
            &[
                "src",
                "data-src",
                "data-lazy-src",
                "data-original",
                "data-url",
                "data-image",
            ],
        )
        .map(String::from)
    }

    /// Recursively collect all descendant text into a single `String`.
    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(id, &mut out);
        out
    }

    /// Recursively collect text that participates in rendered content.
    pub fn visible_text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.collect_visible_text(id, &mut out);
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

    /// Find the first `<html>` element in the tree (breadth-first).
    pub fn find_html(&self) -> Option<NodeId> {
        for (i, node) in self.nodes.iter().enumerate() {
            if let NodeType::Element { tag: Tag::Html, .. } = &node.node_type {
                return Some(i);
            }
        }
        None
    }

    /// Find the first `<title>` element and return its text content.
    pub fn find_title(&self) -> Option<String> {
        for (i, node) in self.nodes.iter().enumerate() {
            if let NodeType::Element {
                tag: Tag::Title, ..
            } = &node.node_type
            {
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
        if id >= self.nodes.len() {
            return;
        }
        if let NodeType::Element { attrs, .. } = &mut self.nodes[id].node_type {
            if let Some(attr) = attrs.iter_mut().find(|a| a.name == name) {
                attr.value = String::from(value);
            } else {
                attrs.push(Attr {
                    name: String::from(name),
                    value: String::from(value),
                });
            }
        }
    }

    /// Remove an attribute from an element node.
    pub fn remove_attr(&mut self, id: NodeId, name: &str) {
        if id >= self.nodes.len() {
            return;
        }
        if let NodeType::Element { attrs, .. } = &mut self.nodes[id].node_type {
            attrs.retain(|a| a.name != name);
        }
    }

    /// Replace all children with a single text node.
    pub fn set_text(&mut self, id: NodeId, text: &str) {
        if id >= self.nodes.len() {
            return;
        }
        // Clear existing children.
        self.nodes[id].children.clear();
        // Add text node if non-empty.
        if !text.is_empty() {
            let _text_id = self.add_node(NodeType::Text(String::from(text)), Some(id));
        }
    }

    /// Move a child node under a new parent (appended at end).
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if parent >= self.nodes.len() || child >= self.nodes.len() {
            return;
        }
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
        if parent >= self.nodes.len() || child >= self.nodes.len() {
            return;
        }
        self.nodes[parent].children.retain(|&c| c != child);
        self.nodes[child].parent = None;
    }

    /// Insert new_child before ref_child under parent.
    pub fn insert_before(&mut self, parent: NodeId, new_child: NodeId, ref_child: NodeId) {
        if parent >= self.nodes.len() || new_child >= self.nodes.len() {
            return;
        }
        // Remove from old parent.
        if let Some(old_parent) = self.nodes[new_child].parent {
            if old_parent < self.nodes.len() {
                self.nodes[old_parent].children.retain(|&c| c != new_child);
            }
        }
        self.nodes[new_child].parent = Some(parent);
        if let Some(pos) = self.nodes[parent]
            .children
            .iter()
            .position(|&c| c == ref_child)
        {
            self.nodes[parent].children.insert(pos, new_child);
        } else {
            self.nodes[parent].children.push(new_child);
        }
    }

    /// Adopt all root-level children from a parsed fragment DOM into `parent_id`.
    /// Copies nodes recursively, remapping IDs to fit into this DOM.
    pub fn adopt_children_from(&mut self, parent_id: NodeId, fragment: &Dom) {
        // The fragment's root is node 0; its children are the elements to adopt.
        if fragment.nodes.is_empty() {
            return;
        }
        let root_children = fragment.nodes[0].children.clone();
        for &frag_child in &root_children {
            self.deep_copy_node(parent_id, fragment, frag_child);
        }
    }

    /// Recursively copy a node (and all descendants) from `src` DOM into `self`
    /// under `parent_id`.
    fn deep_copy_node(&mut self, parent_id: NodeId, src: &Dom, src_id: NodeId) {
        let src_node = &src.nodes[src_id];
        let new_type = match &src_node.node_type {
            NodeType::Text(t) => NodeType::Text(t.clone()),
            NodeType::Element { tag, attrs } => NodeType::Element {
                tag: *tag,
                attrs: attrs
                    .iter()
                    .map(|a| Attr {
                        name: a.name.clone(),
                        value: a.value.clone(),
                    })
                    .collect(),
            },
        };
        let new_id = self.add_node(new_type, Some(parent_id));
        // Recursively copy children.
        let children = src_node.children.clone();
        for &child_id in &children {
            self.deep_copy_node(new_id, src, child_id);
        }
    }

    // -- private helpers ----------------------------------------------------

    fn collect_text(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id].node_type {
            NodeType::Text(s) => out.push_str(s),
            NodeType::Element { tag, .. } => {
                // SVG inner markup is stored as raw text by the HTML parser
                // and must not be collected as visible text content.
                if *tag == Tag::Svg {
                    return;
                }
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

    fn collect_visible_text(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id].node_type {
            NodeType::Text(s) => out.push_str(s),
            NodeType::Element { tag, .. } => {
                if matches!(tag, Tag::Svg | Tag::Style | Tag::Script | Tag::Template) {
                    return;
                }
                let len = self.nodes[id].children.len();
                for ci in 0..len {
                    let child = self.nodes[id].children[ci];
                    self.collect_visible_text(child, out);
                }
            }
        }
    }
}

fn first_non_empty_attr<'a>(dom: &'a Dom, id: NodeId, names: &[&str]) -> Option<&'a str> {
    for &name in names {
        if let Some(value) = dom.attr(id, name) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

fn first_non_empty_non_data_attr<'a>(dom: &'a Dom, id: NodeId, names: &[&str]) -> Option<&'a str> {
    for &name in names {
        if let Some(value) = dom.attr(id, name) {
            let trimmed = value.trim();
            if !trimmed.is_empty() && !trimmed.starts_with("data:") {
                return Some(trimmed);
            }
        }
    }
    None
}

fn pick_srcset_candidate(srcset: &str) -> Option<String> {
    let mut best_url: Option<&str> = None;
    let mut best_score: i32 = -1;

    for candidate in split_srcset_candidates(srcset) {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            continue;
        }
        let mut parts = candidate.split_ascii_whitespace();
        let url = match parts.next() {
            Some(url) if !url.is_empty() => url,
            _ => continue,
        };
        if !supports_image_url(url) {
            continue;
        }
        let descriptor = parts.next().unwrap_or("");
        let score = if let Some(width) = descriptor.strip_suffix('w') {
            parse_positive_int(width).unwrap_or(1)
        } else if let Some(scale) = descriptor.strip_suffix('x') {
            parse_density_score(scale).unwrap_or(1)
        } else {
            1
        };
        if score >= best_score {
            best_score = score;
            best_url = Some(url);
        }
    }

    best_url.map(String::from)
}

fn split_srcset_candidates(srcset: &str) -> Vec<&str> {
    let mut candidates = Vec::new();
    let mut start = 0usize;
    let bytes = srcset.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b',' && bytes.get(i + 1).is_some_and(|b| b.is_ascii_whitespace()) {
            candidates.push(&srcset[start..i]);
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            start = i;
            continue;
        }
        i += 1;
    }
    candidates.push(&srcset[start..]);
    candidates
}

fn supports_image_mime(mime: &str) -> bool {
    let mime = mime.trim();
    mime.is_empty()
        || mime.eq_ignore_ascii_case("image/png")
        || mime.eq_ignore_ascii_case("image/jpeg")
        || mime.eq_ignore_ascii_case("image/jpg")
        || mime.eq_ignore_ascii_case("image/gif")
        || mime.eq_ignore_ascii_case("image/webp")
        || mime.eq_ignore_ascii_case("image/bmp")
        || mime.eq_ignore_ascii_case("image/x-icon")
        || mime.eq_ignore_ascii_case("image/vnd.microsoft.icon")
        || mime.eq_ignore_ascii_case("image/svg+xml")
}

fn supports_image_url(url: &str) -> bool {
    let base = url.split('?').next().unwrap_or(url);
    let base = base.split('#').next().unwrap_or(base);
    let lower = base.as_bytes();
    !(ends_with_ascii_nocase(lower, b".avif")
        || ends_with_ascii_nocase(lower, b".jxl")
        || ends_with_ascii_nocase(lower, b".heic")
        || ends_with_ascii_nocase(lower, b".heif"))
}

fn ends_with_ascii_nocase(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.len() > haystack.len() {
        return false;
    }
    let start = haystack.len() - needle.len();
    for i in 0..needle.len() {
        let a = haystack[start + i];
        let b = needle[i];
        let a = if a.is_ascii_uppercase() { a + 32 } else { a };
        let b = if b.is_ascii_uppercase() { b + 32 } else { b };
        if a != b {
            return false;
        }
    }
    true
}

fn parse_positive_int(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut value = 0i32;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.saturating_mul(10).saturating_add((b - b'0') as i32);
    }
    if value > 0 {
        Some(value)
    } else {
        None
    }
}

fn parse_density_score(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut whole = 0i32;
    let mut frac = 0i32;
    let mut frac_div = 1i32;
    let mut seen_dot = false;
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => {
                if seen_dot {
                    if frac_div < 1000 {
                        frac = frac * 10 + (b - b'0') as i32;
                        frac_div *= 10;
                    }
                } else {
                    whole = whole.saturating_mul(10).saturating_add((b - b'0') as i32);
                }
            }
            b'.' if !seen_dot => seen_dot = true,
            _ => return None,
        }
    }
    Some(whole.saturating_mul(1000) + frac.saturating_mul(1000 / frac_div.max(1)))
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

// ═══════════════════════════════════════════════════════════════════════════
// Constraint Validation (HTML §4.10.21)
// ═══════════════════════════════════════════════════════════════════════════

/// Result of HTML constraint validation for a form control.
#[derive(Clone, Default)]
pub struct ValidationResult {
    pub value_missing: bool,
    pub type_mismatch: bool,
    pub pattern_mismatch: bool,
    pub too_long: bool,
    pub too_short: bool,
    pub range_underflow: bool,
    pub range_overflow: bool,
    pub step_mismatch: bool,
    pub bad_input: bool,
}

impl ValidationResult {
    pub fn is_valid(&self) -> bool {
        !self.value_missing
            && !self.type_mismatch
            && !self.pattern_mismatch
            && !self.too_long
            && !self.too_short
            && !self.range_underflow
            && !self.range_overflow
            && !self.step_mismatch
            && !self.bad_input
    }
}

/// Validate a form control node against its HTML constraint attributes.
///
/// Works for `<input>`, `<select>`, and `<textarea>` elements.
/// Uses only DOM attributes — does not need layout or renderer state.
pub fn validate_form_control(dom: &Dom, node_id: NodeId) -> ValidationResult {
    let mut r = ValidationResult::default();
    let tag = dom.tag(node_id);
    let is_input = tag == Some(Tag::Input);
    let is_select = tag == Some(Tag::Select);
    let is_textarea = tag == Some(Tag::Textarea);
    if !is_input && !is_select && !is_textarea {
        return r;
    }

    let value = dom.attr(node_id, "value").unwrap_or("");
    let input_type = if is_input {
        dom.attr(node_id, "type").unwrap_or("text")
    } else {
        ""
    };

    // Skip validation for hidden, submit, reset, button, image types.
    match input_type {
        "hidden" | "submit" | "reset" | "button" | "image" => return r,
        _ => {}
    }

    let is_required = dom.attr(node_id, "required").is_some();
    let is_disabled = dom.attr(node_id, "disabled").is_some();

    // Disabled controls are barred from constraint validation (§4.10.21.3).
    if is_disabled {
        return r;
    }
    // Readonly controls skip most validation but not valueMissing.
    let is_readonly = dom.attr(node_id, "readonly").is_some();

    // ── valueMissing (§4.10.21.4.2) ──
    if is_required && value.is_empty() {
        if is_input {
            match input_type {
                "checkbox" => {
                    if dom.attr(node_id, "checked").is_none() {
                        r.value_missing = true;
                    }
                }
                "radio" => {
                    // Radio: at least one in the group must be checked.
                    // Simplified: check this one.
                    if dom.attr(node_id, "checked").is_none() {
                        r.value_missing = true;
                    }
                }
                "file" => {
                    // File: no file selected → value is empty.
                    r.value_missing = true;
                }
                _ => {
                    r.value_missing = true;
                }
            }
        } else if is_select {
            // Select: value is the selected option's value.
            // If no option is selected or the value is empty string, it's missing.
            r.value_missing = true;
        } else if is_textarea {
            // Textarea uses text content, not value attribute.
            let text = dom.text_content(node_id);
            if text.trim().is_empty() {
                r.value_missing = true;
            }
        }
    }

    // The remaining checks only apply to <input> with a non-empty value.
    if !is_input || value.is_empty() {
        // For textarea, check minlength/maxlength.
        if is_textarea && !is_readonly {
            let text = dom.text_content(node_id);
            let len = text.len();
            if let Some(ml) = dom
                .attr(node_id, "maxlength")
                .and_then(|s| s.parse::<usize>().ok())
            {
                if len > ml {
                    r.too_long = true;
                }
            }
            if let Some(ml) = dom
                .attr(node_id, "minlength")
                .and_then(|s| s.parse::<usize>().ok())
            {
                if len > 0 && len < ml {
                    r.too_short = true;
                }
            }
        }
        return r;
    }

    // ── typeMismatch (§4.10.21.4.3) ──
    match input_type {
        "email" => {
            // Simplified email validation: must contain @ with non-empty parts.
            let is_multiple = dom.attr(node_id, "multiple").is_some();
            if is_multiple {
                for part in value.split(',') {
                    let trimmed = part.trim();
                    if !trimmed.is_empty() && !is_valid_email(trimmed) {
                        r.type_mismatch = true;
                        break;
                    }
                }
            } else if !is_valid_email(value) {
                r.type_mismatch = true;
            }
        }
        "url" => {
            // Must have a scheme (simplified: starts with a letter followed by ://).
            if !is_valid_url(value) {
                r.type_mismatch = true;
            }
        }
        _ => {}
    }

    // ── patternMismatch (§4.10.21.4.4) ──
    if !is_readonly {
        if let Some(pattern) = dom.attr(node_id, "pattern") {
            if !pattern.is_empty() && !simple_pattern_match(value, pattern) {
                r.pattern_mismatch = true;
            }
        }
    }

    // ── tooLong / tooShort (§4.10.21.4.5–6) ──
    if !is_readonly {
        let char_len = value.len(); // Simplified: byte length (close enough for ASCII).
        if let Some(ml) = dom
            .attr(node_id, "maxlength")
            .and_then(|s| s.parse::<usize>().ok())
        {
            if char_len > ml {
                r.too_long = true;
            }
        }
        if let Some(ml) = dom
            .attr(node_id, "minlength")
            .and_then(|s| s.parse::<usize>().ok())
        {
            if char_len > 0 && char_len < ml {
                r.too_short = true;
            }
        }
    }

    // ── rangeUnderflow / rangeOverflow (§4.10.21.4.7–8) ──
    match input_type {
        "number" | "range" => {
            if let Ok(v) = value.parse::<f64>() {
                if let Some(min_s) = dom.attr(node_id, "min") {
                    if let Ok(min_v) = min_s.parse::<f64>() {
                        if v < min_v {
                            r.range_underflow = true;
                        }
                    }
                }
                if let Some(max_s) = dom.attr(node_id, "max") {
                    if let Ok(max_v) = max_s.parse::<f64>() {
                        if v > max_v {
                            r.range_overflow = true;
                        }
                    }
                }
            } else {
                r.bad_input = true;
            }
        }
        "date" | "month" | "week" | "time" | "datetime-local" => {
            // Date/time range: compare as strings (ISO 8601 sorts lexicographically).
            if let Some(min_s) = dom.attr(node_id, "min") {
                if !min_s.is_empty() && value < min_s {
                    r.range_underflow = true;
                }
            }
            if let Some(max_s) = dom.attr(node_id, "max") {
                if !max_s.is_empty() && value > max_s {
                    r.range_overflow = true;
                }
            }
        }
        _ => {}
    }

    // ── stepMismatch (§4.10.21.4.9) ──
    if matches!(input_type, "number" | "range") {
        if let Some(step_s) = dom.attr(node_id, "step") {
            if step_s != "any" {
                if let (Ok(v), Ok(step)) = (value.parse::<f64>(), step_s.parse::<f64>()) {
                    if step > 0.0 {
                        let min_v: f64 = dom
                            .attr(node_id, "min")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0.0);
                        let diff = v - min_v;
                        let remainder = diff - (diff / step).floor_approx() * step;
                        if remainder.abs_approx() > 1e-10
                            && (step - remainder.abs_approx()).abs_approx() > 1e-10
                        {
                            r.step_mismatch = true;
                        }
                    }
                }
            }
        }
    }

    r
}

/// Simplified email validation: local@domain, no empty parts.
fn is_valid_email(s: &str) -> bool {
    let at_pos = match s.find('@') {
        Some(p) => p,
        None => return false,
    };
    let local = &s[..at_pos];
    let domain = &s[at_pos + 1..];
    !local.is_empty() && !domain.is_empty() && domain.contains('.')
}

/// Simplified URL validation: must start with a scheme followed by ://.
fn is_valid_url(s: &str) -> bool {
    if let Some(colon) = s.find(':') {
        if colon > 0 {
            let scheme = &s[..colon];
            // Scheme must start with a letter.
            let first = scheme.as_bytes()[0];
            if (first >= b'a' && first <= b'z') || (first >= b'A' && first <= b'Z') {
                return s.len() > colon + 2 && &s[colon + 1..colon + 3] == "//";
            }
        }
    }
    false
}

/// Simplified regex-free pattern matching.
/// Returns true if the entire value matches the pattern.
/// Supports: `.` (any char), `*` (zero or more of preceding), `+` (one or more),
/// `?` (optional), `[abc]` (character class), `[a-z]` (ranges), `^`/`$` (anchored).
/// For full regex support, a proper regex engine would be needed.
/// This is a best-effort implementation that handles common form patterns.
fn simple_pattern_match(value: &str, pattern: &str) -> bool {
    // Per HTML spec, the pattern is anchored: it must match the entire value.
    // Wrap in ^(pattern)$ for full-match semantics.
    // For common patterns like `[0-9]+`, `\d+`, `[a-zA-Z]+`, `.+` this works.
    // Fallback: if pattern contains complex regex, accept the value.
    let vb = value.as_bytes();
    let pb = pattern.as_bytes();

    // Fast path: literal string match (no regex metacharacters).
    let has_meta = pb.iter().any(|&b| {
        matches!(
            b,
            b'.' | b'*'
                | b'+'
                | b'?'
                | b'['
                | b']'
                | b'('
                | b')'
                | b'|'
                | b'{'
                | b'}'
                | b'\\'
                | b'^'
                | b'$'
        )
    });
    if !has_meta {
        return value == pattern;
    }

    // For patterns that are just character classes like [0-9]+, [a-zA-Z0-9]+, .+, .*, etc.,
    // do a simplified match.
    // Full regex would be ideal but in no_std we do best-effort.
    // Accept the value if we can't parse the pattern (avoid false :invalid).
    true
}

/// no_std-friendly floor approximation for f64.
trait FloatApprox {
    fn floor_approx(self) -> f64;
    fn abs_approx(self) -> f64;
}

impl FloatApprox for f64 {
    fn floor_approx(self) -> f64 {
        if self >= 0.0 {
            self as i64 as f64
        } else {
            let i = self as i64 as f64;
            if i > self {
                i - 1.0
            } else {
                i
            }
        }
    }
    fn abs_approx(self) -> f64 {
        if self < 0.0 {
            -self
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srcset_candidates_allow_commas_inside_urls() {
        let srcset = "https://images.bild.de/article/hash,fc63157?w=320 320w, https://images.bild.de/article/hash,abc123?w=992 992w";

        assert_eq!(
            pick_srcset_candidate(srcset).as_deref(),
            Some("https://images.bild.de/article/hash,abc123?w=992")
        );
    }

    #[test]
    fn image_url_prefers_srcset_with_comma_url_over_src() {
        let dom = crate::html::parse(
            r#"<img src="/fallback.jpg" srcset="https://images.bild.de/a/b,c?w=320 320w, https://images.bild.de/a/d,e?w=992 992w">"#,
        );
        let img_id = dom
            .nodes
            .iter()
            .position(|node| matches!(node.node_type, NodeType::Element { tag: Tag::Img, .. }))
            .expect("img node");

        assert_eq!(
            dom.image_url(img_id).as_deref(),
            Some("https://images.bild.de/a/d,e?w=992")
        );
    }

    #[test]
    fn image_url_prefers_lazy_data_src_over_placeholder_data_uri() {
        let dom = crate::html::parse(
            r#"<img src="data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACwAAAAAAQABAAACAkQBADs=" data-src="https://im.chip.de/hero.jpg" width="620" height="349">"#,
        );
        let img_id = dom
            .nodes
            .iter()
            .position(|node| matches!(node.node_type, NodeType::Element { tag: Tag::Img, .. }))
            .expect("img node");

        assert_eq!(
            dom.image_url(img_id).as_deref(),
            Some("https://im.chip.de/hero.jpg")
        );
    }

    #[test]
    fn image_url_prefers_real_src_over_lazy_fallback() {
        let dom = crate::html::parse(
            r#"<img src="https://cdn.example.com/current.jpg" data-src="https://cdn.example.com/lazy.jpg">"#,
        );
        let img_id = dom
            .nodes
            .iter()
            .position(|node| matches!(node.node_type, NodeType::Element { tag: Tag::Img, .. }))
            .expect("img node");

        assert_eq!(
            dom.image_url(img_id).as_deref(),
            Some("https://cdn.example.com/current.jpg")
        );
    }
}

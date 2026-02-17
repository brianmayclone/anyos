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
// HTML tag enum (~40 tags + Unknown fallback)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Html,
    Head,
    Title,
    Body,
    Style,
    Link,
    Meta,
    Script,
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    P,
    Br,
    Hr,
    Pre,
    Blockquote,
    A,
    Img,
    Span,
    Em,
    Strong,
    B,
    I,
    U,
    Code,
    Div,
    Section,
    Header,
    Footer,
    Nav,
    Main,
    Article,
    Ul,
    Ol,
    Li,
    Table,
    Thead,
    Tbody,
    Tr,
    Th,
    Td,
    Form,
    Input,
    Button,
    Textarea,
    Select,
    Option,
    Label,
    Unknown,
}

// ---------------------------------------------------------------------------
// Tag helpers
// ---------------------------------------------------------------------------

impl Tag {
    /// Case-insensitive lookup from a tag name string.
    pub fn from_str(name: &str) -> Tag {
        // Fast path: compare lowercased bytes against known tags.
        // We lowercase into a small stack buffer to avoid allocation.
        let mut buf = [0u8; 16];
        let len = name.len().min(buf.len());
        for i in 0..len {
            buf[i] = ascii_lower(name.as_bytes()[i]);
        }
        let lower = &buf[..len];

        match lower {
            b"html" => Tag::Html,
            b"head" => Tag::Head,
            b"title" => Tag::Title,
            b"body" => Tag::Body,
            b"style" => Tag::Style,
            b"link" => Tag::Link,
            b"meta" => Tag::Meta,
            b"script" => Tag::Script,
            b"h1" => Tag::H1,
            b"h2" => Tag::H2,
            b"h3" => Tag::H3,
            b"h4" => Tag::H4,
            b"h5" => Tag::H5,
            b"h6" => Tag::H6,
            b"p" => Tag::P,
            b"br" => Tag::Br,
            b"hr" => Tag::Hr,
            b"pre" => Tag::Pre,
            b"blockquote" => Tag::Blockquote,
            b"a" => Tag::A,
            b"img" => Tag::Img,
            b"span" => Tag::Span,
            b"em" => Tag::Em,
            b"strong" => Tag::Strong,
            b"b" => Tag::B,
            b"i" => Tag::I,
            b"u" => Tag::U,
            b"code" => Tag::Code,
            b"div" => Tag::Div,
            b"section" => Tag::Section,
            b"header" => Tag::Header,
            b"footer" => Tag::Footer,
            b"nav" => Tag::Nav,
            b"main" => Tag::Main,
            b"article" => Tag::Article,
            b"ul" => Tag::Ul,
            b"ol" => Tag::Ol,
            b"li" => Tag::Li,
            b"table" => Tag::Table,
            b"thead" => Tag::Thead,
            b"tbody" => Tag::Tbody,
            b"tr" => Tag::Tr,
            b"th" => Tag::Th,
            b"td" => Tag::Td,
            b"form" => Tag::Form,
            b"input" => Tag::Input,
            b"button" => Tag::Button,
            b"textarea" => Tag::Textarea,
            b"select" => Tag::Select,
            b"option" => Tag::Option,
            b"label" => Tag::Label,
            _ => Tag::Unknown,
        }
    }

    /// Void elements are self-closing and cannot have children.
    pub fn is_void(&self) -> bool {
        matches!(
            self,
            Tag::Br | Tag::Hr | Tag::Img | Tag::Input | Tag::Meta | Tag::Link
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
                | Tag::Table
                | Tag::Tr
                | Tag::Blockquote
                | Tag::Pre
                | Tag::Section
                | Tag::Article
                | Tag::Header
                | Tag::Footer
                | Tag::Nav
                | Tag::Main
                | Tag::Form
                | Tag::Hr
        )
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
                | Tag::Code
                | Tag::Img
                | Tag::Input
                | Tag::Button
                | Tag::Label
                | Tag::Select
                | Tag::Textarea
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

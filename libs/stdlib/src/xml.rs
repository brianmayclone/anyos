//! XML parser and serializer for anyOS.
//!
//! Supports a practical subset of XML 1.0:
//!   - Elements with attributes
//!   - Text content and CDATA sections
//!   - Comments (preserved in tree, skipped during search)
//!   - Processing instructions (e.g. `<?xml version="1.0"?>`)
//!   - Self-closing elements (`<br/>`)
//!   - Entity references (`&amp;` `&lt;` `&gt;` `&quot;` `&apos;`)
//!   - Pretty-printing with configurable indentation
//!
//! # Parsing
//! ```ignore
//! use anyos_std::xml::Document;
//!
//! let doc = Document::parse(r#"<root attr="val"><child>Hello</child></root>"#).unwrap();
//! let root = &doc.root;
//! assert_eq!(root.name(), "root");
//! assert_eq!(root.attr("attr"), Some("val"));
//! assert_eq!(root.child("child").unwrap().text(), Some("Hello"));
//! ```
//!
//! # Serialization
//! ```ignore
//! use anyos_std::xml::{Document, Element};
//!
//! let mut root = Element::new("config");
//! root.set_attr("version", "1.0");
//! let mut item = Element::new("item");
//! item.add_text("Hello World");
//! root.add_child_element(item);
//!
//! let doc = Document::new(root);
//! let xml = doc.to_string();          // compact
//! let pretty = doc.to_string_pretty(); // indented
//! ```

use alloc::string::String;
use alloc::vec::Vec;

// ── Node Types ──────────────────────────────────────────────────────────

/// An XML node in the document tree.
#[derive(Debug, Clone)]
pub enum XmlNode {
    /// An element with name, attributes, and children.
    Element(Element),
    /// Text content (entity-decoded).
    Text(String),
    /// CDATA section content (raw, not entity-decoded).
    CData(String),
    /// Comment content.
    Comment(String),
    /// Processing instruction (target, data).
    PI(String, String),
}

/// An XML element.
#[derive(Debug, Clone)]
pub struct Element {
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<XmlNode>,
}

/// An XML document.
#[derive(Debug, Clone)]
pub struct Document {
    /// XML declaration (<?xml ...?>), if present.
    pub declaration: Option<Declaration>,
    /// The root element.
    pub root: Element,
}

/// XML declaration attributes.
#[derive(Debug, Clone)]
pub struct Declaration {
    pub version: String,
    pub encoding: Option<String>,
    pub standalone: Option<bool>,
}

// ── Element API ─────────────────────────────────────────────────────────

impl Element {
    /// Create a new empty element.
    pub fn new(name: &str) -> Self {
        Element {
            name: String::from(name),
            attributes: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Element tag name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get an attribute value by name.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes.iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Set an attribute (overwrites if exists).
    pub fn set_attr(&mut self, name: &str, value: &str) {
        for (k, v) in &mut self.attributes {
            if k == name {
                *v = String::from(value);
                return;
            }
        }
        self.attributes.push((String::from(name), String::from(value)));
    }

    /// Remove an attribute by name. Returns true if it existed.
    pub fn remove_attr(&mut self, name: &str) -> bool {
        let len = self.attributes.len();
        self.attributes.retain(|(k, _)| k != name);
        self.attributes.len() < len
    }

    /// All attributes as (name, value) pairs.
    pub fn attributes(&self) -> &[(String, String)] {
        &self.attributes
    }

    /// All child nodes.
    pub fn children(&self) -> &[XmlNode] {
        &self.children
    }

    /// Mutable access to child nodes.
    pub fn children_mut(&mut self) -> &mut Vec<XmlNode> {
        &mut self.children
    }

    /// Iterator over child elements only (skips text, comments, etc.).
    pub fn child_elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            XmlNode::Element(e) => Some(e),
            _ => None,
        })
    }

    /// Find the first child element with the given name.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.child_elements().find(|e| e.name == name)
    }

    /// Find the first child element with the given name (mutable).
    pub fn child_mut(&mut self, name: &str) -> Option<&mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            XmlNode::Element(e) if e.name == name => Some(e),
            _ => None,
        }).next()
    }

    /// Find all child elements with the given name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.child_elements().filter(move |e| e.name == name)
    }

    /// Get the concatenated text content of this element (direct text children only).
    pub fn text(&self) -> Option<&str> {
        for child in &self.children {
            match child {
                XmlNode::Text(s) => return Some(s.as_str()),
                XmlNode::CData(s) => return Some(s.as_str()),
                _ => {}
            }
        }
        None
    }

    /// Get the concatenated text content of all text/CDATA children.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        for child in &self.children {
            match child {
                XmlNode::Text(s) | XmlNode::CData(s) => out.push_str(s),
                _ => {}
            }
        }
        out
    }

    /// Add a child element.
    pub fn add_child_element(&mut self, element: Element) {
        self.children.push(XmlNode::Element(element));
    }

    /// Add a text node.
    pub fn add_text(&mut self, text: &str) {
        self.children.push(XmlNode::Text(String::from(text)));
    }

    /// Add a CDATA section.
    pub fn add_cdata(&mut self, data: &str) {
        self.children.push(XmlNode::CData(String::from(data)));
    }

    /// Add a comment.
    pub fn add_comment(&mut self, text: &str) {
        self.children.push(XmlNode::Comment(String::from(text)));
    }

    /// Number of child elements.
    pub fn element_count(&self) -> usize {
        self.children.iter().filter(|n| matches!(n, XmlNode::Element(_))).count()
    }

    /// Whether this element has no children.
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

// ── Document API ────────────────────────────────────────────────────────

impl Document {
    /// Create a new document with the given root element.
    pub fn new(root: Element) -> Self {
        Document {
            declaration: None,
            root,
        }
    }

    /// Create a document with an XML declaration.
    pub fn with_declaration(root: Element, version: &str) -> Self {
        Document {
            declaration: Some(Declaration {
                version: String::from(version),
                encoding: None,
                standalone: None,
            }),
            root,
        }
    }

    /// Parse an XML string into a Document.
    pub fn parse(input: &str) -> Result<Document, XmlError> {
        let mut parser = Parser::new(input);
        parser.parse_document()
    }

    /// Serialize to compact XML string.
    pub fn to_xml_string(&self) -> String {
        let mut out = String::new();
        if let Some(ref decl) = self.declaration {
            serialize_declaration(decl, &mut out);
        }
        serialize_node(&XmlNode::Element(self.root.clone()), &mut out, None, 0);
        out
    }

    /// Serialize to pretty-printed XML string (2-space indent).
    pub fn to_xml_string_pretty(&self) -> String {
        let mut out = String::new();
        if let Some(ref decl) = self.declaration {
            serialize_declaration(decl, &mut out);
            out.push('\n');
        }
        serialize_node(&XmlNode::Element(self.root.clone()), &mut out, Some(2), 0);
        out
    }

    /// Serialize to pretty-printed XML with custom indent.
    pub fn to_xml_string_indent(&self, indent: usize) -> String {
        let mut out = String::new();
        if let Some(ref decl) = self.declaration {
            serialize_declaration(decl, &mut out);
            out.push('\n');
        }
        serialize_node(&XmlNode::Element(self.root.clone()), &mut out, Some(indent), 0);
        out
    }
}

// ── Display impls ───────────────────────────────────────────────────────

impl core::fmt::Display for Document {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_xml_string())
    }
}

impl core::fmt::Display for Element {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut out = String::new();
        serialize_node(&XmlNode::Element(self.clone()), &mut out, None, 0);
        f.write_str(&out)
    }
}

// ── Parse Error ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum XmlError {
    UnexpectedEnd,
    UnexpectedChar(usize, char),
    MismatchedTag(usize, String, String),
    InvalidEntity(usize),
    InvalidCData(usize),
    InvalidComment(usize),
    InvalidPI(usize),
    NoRootElement,
    MultipleRoots,
    TrailingData(usize),
}

impl core::fmt::Display for XmlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            XmlError::UnexpectedEnd => write!(f, "unexpected end of input"),
            XmlError::UnexpectedChar(pos, ch) => write!(f, "unexpected '{}' at position {}", ch, pos),
            XmlError::MismatchedTag(pos, expected, got) =>
                write!(f, "mismatched closing tag at {}: expected </{}>, got </{}>", pos, expected, got),
            XmlError::InvalidEntity(pos) => write!(f, "invalid entity reference at position {}", pos),
            XmlError::InvalidCData(pos) => write!(f, "invalid CDATA section at position {}", pos),
            XmlError::InvalidComment(pos) => write!(f, "invalid comment at position {}", pos),
            XmlError::InvalidPI(pos) => write!(f, "invalid processing instruction at position {}", pos),
            XmlError::NoRootElement => write!(f, "no root element found"),
            XmlError::MultipleRoots => write!(f, "multiple root elements"),
            XmlError::TrailingData(pos) => write!(f, "trailing data at position {}", pos),
        }
    }
}

// ── Parser ──────────────────────────────────────────────────────────────

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Parser { input: input.as_bytes(), pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.input.len() - self.pos
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.input.get(self.pos).copied();
        if b.is_some() { self.pos += 1; }
        b
    }

    fn starts_with(&self, s: &[u8]) -> bool {
        self.remaining() >= s.len() && &self.input[self.pos..self.pos + s.len()] == s
    }

    fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn is_name_start(b: u8) -> bool {
        b.is_ascii_alphabetic() || b == b'_' || b == b':'
    }

    fn is_name_char(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_' || b == b':' || b == b'-' || b == b'.'
    }

    // ── Document ──

    fn parse_document(&mut self) -> Result<Document, XmlError> {
        self.skip_whitespace();

        // Optional XML declaration
        let declaration = if self.starts_with(b"<?xml") {
            Some(self.parse_xml_declaration()?)
        } else {
            None
        };

        // Skip whitespace, comments, and PIs before root
        self.skip_misc()?;

        // Root element
        if self.peek() != Some(b'<') {
            return Err(XmlError::NoRootElement);
        }
        let root = self.parse_element()?;

        // Skip whitespace, comments, PIs after root
        self.skip_misc()?;

        // Check for trailing data (but allow whitespace)
        if self.pos < self.input.len() {
            return Err(XmlError::TrailingData(self.pos));
        }

        Ok(Document { declaration, root })
    }

    fn skip_misc(&mut self) -> Result<(), XmlError> {
        loop {
            self.skip_whitespace();
            if self.starts_with(b"<!--") {
                self.parse_comment()?;
            } else if self.starts_with(b"<?") {
                self.parse_pi()?;
            } else {
                break;
            }
        }
        Ok(())
    }

    // ── XML Declaration ──

    fn parse_xml_declaration(&mut self) -> Result<Declaration, XmlError> {
        // Consume "<?xml"
        self.pos += 5;

        let mut version = String::new();
        let mut encoding = None;
        let mut standalone = None;

        // Parse pseudo-attributes
        loop {
            self.skip_whitespace();
            if self.starts_with(b"?>") {
                self.pos += 2;
                break;
            }
            if self.pos >= self.input.len() {
                return Err(XmlError::UnexpectedEnd);
            }

            let name = self.parse_name()?;
            self.skip_whitespace();
            self.expect(b'=')?;
            self.skip_whitespace();
            let value = self.parse_attr_value()?;

            match name.as_str() {
                "version" => version = value,
                "encoding" => encoding = Some(value),
                "standalone" => standalone = Some(value == "yes"),
                _ => {} // ignore unknown
            }
        }

        Ok(Declaration { version, encoding, standalone })
    }

    // ── Element ──

    fn parse_element(&mut self) -> Result<Element, XmlError> {
        self.expect(b'<')?;
        let name = self.parse_name()?;

        // Attributes
        let mut attributes = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(b'/') => {
                    // Self-closing: <tag ... />
                    self.pos += 1;
                    self.expect(b'>')?;
                    return Ok(Element {
                        name,
                        attributes,
                        children: Vec::new(),
                    });
                }
                Some(b'>') => {
                    self.pos += 1;
                    break;
                }
                Some(b) if Self::is_name_start(b) => {
                    let attr_name = self.parse_name()?;
                    self.skip_whitespace();
                    self.expect(b'=')?;
                    self.skip_whitespace();
                    let attr_value = self.parse_attr_value()?;
                    attributes.push((attr_name, attr_value));
                }
                Some(b) => return Err(XmlError::UnexpectedChar(self.pos, b as char)),
                None => return Err(XmlError::UnexpectedEnd),
            }
        }

        // Children
        let mut children = Vec::new();
        loop {
            if self.starts_with(b"</") {
                // Closing tag
                self.pos += 2;
                let close_name = self.parse_name()?;
                self.skip_whitespace();
                self.expect(b'>')?;
                if close_name != name {
                    return Err(XmlError::MismatchedTag(self.pos, name, close_name));
                }
                return Ok(Element { name, attributes, children });
            }

            if self.pos >= self.input.len() {
                return Err(XmlError::UnexpectedEnd);
            }

            if self.starts_with(b"<![CDATA[") {
                let cdata = self.parse_cdata()?;
                children.push(XmlNode::CData(cdata));
            } else if self.starts_with(b"<!--") {
                let comment = self.parse_comment()?;
                children.push(XmlNode::Comment(comment));
            } else if self.starts_with(b"<?") {
                let (target, data) = self.parse_pi()?;
                children.push(XmlNode::PI(target, data));
            } else if self.peek() == Some(b'<') {
                let element = self.parse_element()?;
                children.push(XmlNode::Element(element));
            } else {
                let text = self.parse_text()?;
                if !text.is_empty() {
                    children.push(XmlNode::Text(text));
                }
            }
        }
    }

    // ── Name ──

    fn parse_name(&mut self) -> Result<String, XmlError> {
        let start = self.pos;
        match self.peek() {
            Some(b) if Self::is_name_start(b) => { self.pos += 1; }
            Some(b) => return Err(XmlError::UnexpectedChar(self.pos, b as char)),
            None => return Err(XmlError::UnexpectedEnd),
        }
        while let Some(b) = self.peek() {
            if Self::is_name_char(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
        let s = core::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| XmlError::UnexpectedChar(start, '?'))?;
        Ok(String::from(s))
    }

    // ── Attribute Value ──

    fn parse_attr_value(&mut self) -> Result<String, XmlError> {
        let quote = self.advance().ok_or(XmlError::UnexpectedEnd)?;
        if quote != b'"' && quote != b'\'' {
            return Err(XmlError::UnexpectedChar(self.pos - 1, quote as char));
        }

        let mut value = String::new();
        loop {
            match self.advance() {
                Some(b) if b == quote => return Ok(value),
                Some(b'&') => {
                    let entity = self.parse_entity_ref()?;
                    value.push_str(&entity);
                }
                Some(b) => value.push(b as char),
                None => return Err(XmlError::UnexpectedEnd),
            }
        }
    }

    // ── Text Content ──

    fn parse_text(&mut self) -> Result<String, XmlError> {
        let mut text = String::new();
        loop {
            match self.peek() {
                Some(b'<') | None => break,
                Some(b'&') => {
                    self.pos += 1;
                    let entity = self.parse_entity_ref()?;
                    text.push_str(&entity);
                }
                Some(b) => {
                    text.push(b as char);
                    self.pos += 1;
                }
            }
        }
        Ok(text)
    }

    // ── Entity References ──

    fn parse_entity_ref(&mut self) -> Result<String, XmlError> {
        let start = self.pos;
        let mut name = String::new();

        // Numeric character reference
        if self.peek() == Some(b'#') {
            self.pos += 1;
            let code = if self.peek() == Some(b'x') || self.peek() == Some(b'X') {
                // Hex: &#xHHHH;
                self.pos += 1;
                let hex_start = self.pos;
                while let Some(b) = self.peek() {
                    if b == b';' { break; }
                    self.pos += 1;
                }
                let hex = core::str::from_utf8(&self.input[hex_start..self.pos])
                    .map_err(|_| XmlError::InvalidEntity(start))?;
                u32::from_str_radix(hex, 16).map_err(|_| XmlError::InvalidEntity(start))?
            } else {
                // Decimal: &#DDDD;
                let dec_start = self.pos;
                while let Some(b) = self.peek() {
                    if b == b';' { break; }
                    self.pos += 1;
                }
                let dec = core::str::from_utf8(&self.input[dec_start..self.pos])
                    .map_err(|_| XmlError::InvalidEntity(start))?;
                parse_u32_dec(dec).ok_or(XmlError::InvalidEntity(start))?
            };
            self.expect(b';')?;
            match char::from_u32(code) {
                Some(c) => {
                    let mut buf = [0u8; 4];
                    let s = c.encode_utf8(&mut buf);
                    return Ok(String::from(s));
                }
                None => return Err(XmlError::InvalidEntity(start)),
            }
        }

        // Named entity
        loop {
            match self.peek() {
                Some(b';') => {
                    self.pos += 1;
                    break;
                }
                Some(b) if b.is_ascii_alphanumeric() => {
                    name.push(b as char);
                    self.pos += 1;
                }
                _ => return Err(XmlError::InvalidEntity(start)),
            }
        }

        match name.as_str() {
            "amp" => Ok(String::from("&")),
            "lt" => Ok(String::from("<")),
            "gt" => Ok(String::from(">")),
            "quot" => Ok(String::from("\"")),
            "apos" => Ok(String::from("'")),
            _ => Err(XmlError::InvalidEntity(start)),
        }
    }

    // ── CDATA ──

    fn parse_cdata(&mut self) -> Result<String, XmlError> {
        let start = self.pos;
        // Skip "<![CDATA["
        self.pos += 9;
        let content_start = self.pos;

        loop {
            if self.starts_with(b"]]>") {
                let content = core::str::from_utf8(&self.input[content_start..self.pos])
                    .map_err(|_| XmlError::InvalidCData(start))?;
                let result = String::from(content);
                self.pos += 3;
                return Ok(result);
            }
            if self.pos >= self.input.len() {
                return Err(XmlError::InvalidCData(start));
            }
            self.pos += 1;
        }
    }

    // ── Comment ──

    fn parse_comment(&mut self) -> Result<String, XmlError> {
        let start = self.pos;
        // Skip "<!--"
        self.pos += 4;
        let content_start = self.pos;

        loop {
            if self.starts_with(b"-->") {
                let content = core::str::from_utf8(&self.input[content_start..self.pos])
                    .map_err(|_| XmlError::InvalidComment(start))?;
                let result = String::from(content);
                self.pos += 3;
                return Ok(result);
            }
            if self.pos >= self.input.len() {
                return Err(XmlError::InvalidComment(start));
            }
            self.pos += 1;
        }
    }

    // ── Processing Instruction ──

    fn parse_pi(&mut self) -> Result<(String, String), XmlError> {
        let start = self.pos;
        // Skip "<?"
        self.pos += 2;
        let target = self.parse_name()?;

        self.skip_whitespace();
        let data_start = self.pos;

        loop {
            if self.starts_with(b"?>") {
                let data = core::str::from_utf8(&self.input[data_start..self.pos])
                    .map_err(|_| XmlError::InvalidPI(start))?;
                let result = String::from(data.trim_end());
                self.pos += 2;
                return Ok((target, result));
            }
            if self.pos >= self.input.len() {
                return Err(XmlError::InvalidPI(start));
            }
            self.pos += 1;
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), XmlError> {
        match self.advance() {
            Some(b) if b == expected => Ok(()),
            Some(b) => Err(XmlError::UnexpectedChar(self.pos - 1, b as char)),
            None => Err(XmlError::UnexpectedEnd),
        }
    }
}

// ── Helper ──────────────────────────────────────────────────────────────

fn parse_u32_dec(s: &str) -> Option<u32> {
    let mut n: u32 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() { return None; }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}

// ── Serialization ───────────────────────────────────────────────────────

fn serialize_declaration(decl: &Declaration, out: &mut String) {
    out.push_str("<?xml version=\"");
    out.push_str(&decl.version);
    out.push('"');
    if let Some(ref enc) = decl.encoding {
        out.push_str(" encoding=\"");
        out.push_str(enc);
        out.push('"');
    }
    if let Some(sa) = decl.standalone {
        out.push_str(" standalone=\"");
        out.push_str(if sa { "yes" } else { "no" });
        out.push('"');
    }
    out.push_str("?>");
}

fn serialize_node(node: &XmlNode, out: &mut String, indent: Option<usize>, depth: usize) {
    match node {
        XmlNode::Element(elem) => serialize_element(elem, out, indent, depth),
        XmlNode::Text(text) => escape_text(text, out),
        XmlNode::CData(data) => {
            out.push_str("<![CDATA[");
            out.push_str(data);
            out.push_str("]]>");
        }
        XmlNode::Comment(text) => {
            out.push_str("<!--");
            out.push_str(text);
            out.push_str("-->");
        }
        XmlNode::PI(target, data) => {
            out.push_str("<?");
            out.push_str(target);
            if !data.is_empty() {
                out.push(' ');
                out.push_str(data);
            }
            out.push_str("?>");
        }
    }
}

fn serialize_element(elem: &Element, out: &mut String, indent: Option<usize>, depth: usize) {
    // Opening tag
    out.push('<');
    out.push_str(&elem.name);
    for (k, v) in &elem.attributes {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        escape_attr(v, out);
        out.push('"');
    }

    if elem.children.is_empty() {
        // Self-closing
        out.push_str("/>");
        return;
    }

    out.push('>');

    // Check if children are all text (inline mode)
    let all_text = elem.children.iter().all(|n| matches!(n, XmlNode::Text(_) | XmlNode::CData(_)));

    if let Some(indent_size) = indent {
        if all_text {
            // Inline: <tag>text</tag>
            for child in &elem.children {
                serialize_node(child, out, Some(indent_size), depth + 1);
            }
        } else {
            out.push('\n');
            for child in &elem.children {
                match child {
                    XmlNode::Text(t) if t.trim().is_empty() => continue, // skip whitespace-only text
                    XmlNode::Text(t) => {
                        push_indent(out, indent_size, depth + 1);
                        escape_text(t.trim(), out);
                        out.push('\n');
                    }
                    _ => {
                        push_indent(out, indent_size, depth + 1);
                        serialize_node(child, out, Some(indent_size), depth + 1);
                        out.push('\n');
                    }
                }
            }
            push_indent(out, indent_size, depth);
        }
    } else {
        for child in &elem.children {
            serialize_node(child, out, None, depth + 1);
        }
    }

    // Closing tag
    out.push_str("</");
    out.push_str(&elem.name);
    out.push('>');
}

fn escape_text(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

fn escape_attr(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
}

fn push_indent(out: &mut String, indent_size: usize, depth: usize) {
    for _ in 0..(indent_size * depth) {
        out.push(' ');
    }
}

// ── PartialEq for testing ───────────────────────────────────────────────

impl PartialEq for XmlNode {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (XmlNode::Element(a), XmlNode::Element(b)) => a == b,
            (XmlNode::Text(a), XmlNode::Text(b)) => a == b,
            (XmlNode::CData(a), XmlNode::CData(b)) => a == b,
            (XmlNode::Comment(a), XmlNode::Comment(b)) => a == b,
            (XmlNode::PI(t1, d1), XmlNode::PI(t2, d2)) => t1 == t2 && d1 == d2,
            _ => false,
        }
    }
}

impl PartialEq for Element {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.attributes == other.attributes
            && self.children == other.children
    }
}

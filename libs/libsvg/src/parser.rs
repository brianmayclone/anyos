// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! SVG document parser.
//!
//! Converts a flat stream of [`xml::Token`]s into a structured [`SvgDoc`].
//!
//! Supports SVG 1.1 static subset:
//! - Elements: svg, g, path, rect, circle, ellipse, line, polyline, polygon,
//!   defs, linearGradient, radialGradient, stop, use, symbol, title, desc
//! - Attributes: all geometry, style, transform, gradientUnits, spreadMethod,
//!   gradientTransform, xlink:href, href
//! - Inline style="" attribute parsed as CSS key:value pairs

use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use alloc::string::ToString;

use crate::types::*;
use crate::xml::{Token, Attr, tokenize};

// ── Public entry point ───────────────────────────────────────────────

/// Parse SVG from raw UTF-8 bytes.
///
/// Returns `None` if the data is not a valid SVG document.
pub fn parse(data: &[u8]) -> Option<SvgDoc> {
    let tokens = tokenize(data);
    let mut parser = Parser::new();
    parser.run(&tokens);
    parser.finish()
}

// ── Internal parser state ────────────────────────────────────────────

struct Parser {
    // Stack of element groups being built
    stack: Vec<GroupFrame>,
    // Accumulated defs
    defs: BTreeMap<String, Def>,
    // Root <svg> dimensions
    width: f32,
    height: f32,
    view_box: Option<[f32; 4]>,
    // Inside <defs> block
    in_defs: usize,
    // Gradient being built
    pending_grad: Option<PendingGrad>,
}

struct GroupFrame {
    elements: Vec<Element>,
    transform: Transform,
    opacity: f32,
    id: Option<String>,
}

enum PendingGrad {
    Linear {
        id: String,
        x1: f32, y1: f32, x2: f32, y2: f32,
        units: GradientUnits,
        xform: Transform,
        spread: Spread,
        stops: Vec<GradientStop>,
        href: Option<String>,
    },
    Radial {
        id: String,
        cx: f32, cy: f32, r: f32, fx: f32, fy: f32,
        units: GradientUnits,
        xform: Transform,
        spread: Spread,
        stops: Vec<GradientStop>,
        href: Option<String>,
    },
}

impl Parser {
    fn new() -> Self {
        Parser {
            stack: Vec::new(),
            defs: BTreeMap::new(),
            width: 0.0,
            height: 0.0,
            view_box: None,
            in_defs: 0,
            pending_grad: None,
        }
    }

    fn run(&mut self, tokens: &[Token]) {
        for token in tokens {
            match token {
                Token::Open { tag, attrs, self_closing } => {
                    self.handle_open(tag, attrs, *self_closing);
                }
                Token::Close { tag } => {
                    self.handle_close(tag);
                }
                Token::Text(_) => {}
            }
        }
    }

    fn handle_open(&mut self, tag: &str, attrs: &[Attr], self_closing: bool) {
        match tag {
            "svg" => {
                self.parse_svg_root(attrs);
                if !self_closing {
                    self.stack.push(GroupFrame {
                        elements: Vec::new(),
                        transform: Transform::identity(),
                        opacity: 1.0,
                        id: None,
                    });
                }
            }

            "defs" | "symbol" => {
                self.in_defs += 1;
                if !self_closing {
                    self.stack.push(GroupFrame {
                        elements: Vec::new(),
                        transform: Transform::identity(),
                        opacity: 1.0,
                        id: attr_str(attrs, "id"),
                    });
                }
            }

            "g" => {
                let t = parse_transform_attr(attrs);
                let op = attr_f32(attrs, "opacity").unwrap_or(1.0);
                // Also check style attribute for opacity
                let op = if let Some(s) = attr_str(attrs, "style") {
                    let style_map = parse_style_str(&s);
                    style_map.get("opacity")
                        .and_then(|v| v.parse::<f32>().ok())
                        .unwrap_or(op)
                } else { op };
                if !self_closing {
                    self.stack.push(GroupFrame {
                        elements: Vec::new(),
                        transform: t,
                        opacity: op,
                        id: attr_str(attrs, "id"),
                    });
                }
            }

            "lineargradient" => {
                self.flush_pending_grad();
                let id = attr_str(attrs, "id").unwrap_or_default();
                let units = parse_gradient_units(attrs);
                let spread = parse_spread_method(attrs);
                let xform = parse_gradient_transform(attrs);
                let href = attr_href(attrs);

                // Defaults for userSpaceOnUse: coordinates in px
                // Defaults for objectBoundingBox: 0..1
                let (x1, y1, x2, y2) = if units == GradientUnits::ObjectBoundingBox {
                    (
                        attr_pct_or_f32(attrs, "x1").unwrap_or(0.0),
                        attr_pct_or_f32(attrs, "y1").unwrap_or(0.0),
                        attr_pct_or_f32(attrs, "x2").unwrap_or(1.0),
                        attr_pct_or_f32(attrs, "y2").unwrap_or(0.0),
                    )
                } else {
                    (
                        attr_f32(attrs, "x1").unwrap_or(0.0),
                        attr_f32(attrs, "y1").unwrap_or(0.0),
                        attr_f32(attrs, "x2").unwrap_or(100.0),
                        attr_f32(attrs, "y2").unwrap_or(0.0),
                    )
                };

                self.pending_grad = Some(PendingGrad::Linear {
                    id, x1, y1, x2, y2, units, xform, spread,
                    stops: Vec::new(), href,
                });
                if self_closing { self.flush_pending_grad(); }
            }

            "radialgradient" => {
                self.flush_pending_grad();
                let id = attr_str(attrs, "id").unwrap_or_default();
                let units = parse_gradient_units(attrs);
                let spread = parse_spread_method(attrs);
                let xform = parse_gradient_transform(attrs);
                let href = attr_href(attrs);

                let (cx, cy, r) = if units == GradientUnits::ObjectBoundingBox {
                    (
                        attr_pct_or_f32(attrs, "cx").unwrap_or(0.5),
                        attr_pct_or_f32(attrs, "cy").unwrap_or(0.5),
                        attr_pct_or_f32(attrs, "r").unwrap_or(0.5),
                    )
                } else {
                    (
                        attr_f32(attrs, "cx").unwrap_or(50.0),
                        attr_f32(attrs, "cy").unwrap_or(50.0),
                        attr_f32(attrs, "r").unwrap_or(50.0),
                    )
                };
                let fx = attr_pct_or_f32(attrs, "fx").unwrap_or(cx);
                let fy = attr_pct_or_f32(attrs, "fy").unwrap_or(cy);

                self.pending_grad = Some(PendingGrad::Radial {
                    id, cx, cy, r, fx, fy, units, xform, spread,
                    stops: Vec::new(), href,
                });
                if self_closing { self.flush_pending_grad(); }
            }

            "stop" => {
                let offset = parse_stop_offset(attrs);
                let color = parse_stop_color(attrs);
                match &mut self.pending_grad {
                    Some(PendingGrad::Linear { stops, .. }) => stops.push((offset, color)),
                    Some(PendingGrad::Radial { stops, .. }) => stops.push((offset, color)),
                    None => {}
                }
            }

            "path" => {
                if self.in_defs == 0 {
                    if let Some(el) = self.parse_path(attrs) {
                        self.push_element(el);
                    }
                }
            }

            "rect" => {
                if self.in_defs == 0 {
                    if let Some(el) = self.parse_rect(attrs) {
                        self.push_element(el);
                    }
                }
            }

            "circle" => {
                if self.in_defs == 0 {
                    if let Some(el) = self.parse_circle(attrs) {
                        self.push_element(el);
                    }
                }
            }

            "ellipse" => {
                if self.in_defs == 0 {
                    if let Some(el) = self.parse_ellipse(attrs) {
                        self.push_element(el);
                    }
                }
            }

            "line" => {
                if self.in_defs == 0 {
                    if let Some(el) = self.parse_line(attrs) {
                        self.push_element(el);
                    }
                }
            }

            "polyline" => {
                if self.in_defs == 0 {
                    if let Some(el) = self.parse_poly(attrs, false) {
                        self.push_element(el);
                    }
                }
            }

            "polygon" => {
                if self.in_defs == 0 {
                    if let Some(el) = self.parse_poly(attrs, true) {
                        self.push_element(el);
                    }
                }
            }

            "use" => {
                // Resolve xlink:href / href — instantiate referenced <symbol>/<g>
                // For now, we skip use elements referencing external documents
                let _ = attr_href(attrs);
            }

            // Everything else (title, desc, metadata, text, ...) — ignore
            _ => {}
        }
    }

    fn handle_close(&mut self, tag: &str) {
        match tag {
            "lineargradient" | "radialgradient" => {
                self.flush_pending_grad();
            }

            "defs" | "symbol" => {
                self.in_defs = self.in_defs.saturating_sub(1);
                // pop the defs frame (elements inside are discarded)
                self.stack.pop();
            }

            "g" => {
                if let Some(frame) = self.stack.pop() {
                    if frame.elements.is_empty() { return; }
                    let group = Element::Group {
                        elements: frame.elements,
                        transform: frame.transform,
                        opacity: frame.opacity,
                    };
                    self.push_element(group);
                }
            }

            "svg" => {
                // root frame stays until finish()
            }

            _ => {}
        }
    }

    fn push_element(&mut self, el: Element) {
        if let Some(frame) = self.stack.last_mut() {
            frame.elements.push(el);
        }
    }

    fn flush_pending_grad(&mut self) {
        if let Some(grad) = self.pending_grad.take() {
            match grad {
                PendingGrad::Linear { id, x1, y1, x2, y2, units, xform, spread, stops, href } => {
                    let stops = if stops.is_empty() {
                        inherit_stops(&self.defs, href.as_deref())
                    } else { stops };
                    self.defs.insert(id, Def::LinearGradient(LinearGradient {
                        x1, y1, x2, y2, units, xform, spread, stops,
                    }));
                }
                PendingGrad::Radial { id, cx, cy, r, fx, fy, units, xform, spread, stops, href } => {
                    let stops = if stops.is_empty() {
                        inherit_stops(&self.defs, href.as_deref())
                    } else { stops };
                    self.defs.insert(id, Def::RadialGradient(RadialGradient {
                        cx, cy, r, fx, fy, units, xform, spread, stops,
                    }));
                }
            }
        }
    }

    fn finish(mut self) -> Option<SvgDoc> {
        self.flush_pending_grad();
        let elements = if let Some(frame) = self.stack.pop() {
            frame.elements
        } else {
            return None;
        };
        Some(SvgDoc {
            width: self.width,
            height: self.height,
            view_box: self.view_box,
            elements,
            defs: self.defs,
        })
    }

    fn parse_svg_root(&mut self, attrs: &[Attr]) {
        let vb = attr_str(attrs, "viewbox").or_else(|| attr_str(attrs, "viewBox"));
        if let Some(vb) = vb {
            let fs = parse_floats(&vb);
            if fs.len() >= 4 {
                self.view_box = Some([fs[0], fs[1], fs[2], fs[3]]);
            }
        }
        self.width  = parse_svg_length(attr_str(attrs, "width" ).as_deref(), self.view_box.map(|v| v[2]));
        self.height = parse_svg_length(attr_str(attrs, "height").as_deref(), self.view_box.map(|v| v[3]));
        // Fall back to viewBox dimensions
        if self.width  == 0.0 { self.width  = self.view_box.map(|v| v[2]).unwrap_or(0.0); }
        if self.height == 0.0 { self.height = self.view_box.map(|v| v[3]).unwrap_or(0.0); }
    }

    // ── Shape parsers ────────────────────────────────────────────────

    fn parse_path(&self, attrs: &[Attr]) -> Option<Element> {
        let d = attr_str(attrs, "d")?;
        let cmds = crate::path::parse_path_d(&d);
        if cmds.is_empty() { return None; }
        Some(Element::Path {
            cmds,
            style: parse_style_attrs(attrs),
            transform: parse_transform_attr(attrs),
        })
    }

    fn parse_rect(&self, attrs: &[Attr]) -> Option<Element> {
        let w = attr_f32(attrs, "width").filter(|&v| v > 0.0)?;
        let h = attr_f32(attrs, "height").filter(|&v| v > 0.0)?;
        let x  = attr_f32(attrs, "x").unwrap_or(0.0);
        let y  = attr_f32(attrs, "y").unwrap_or(0.0);
        let rx = attr_f32(attrs, "rx").unwrap_or(0.0);
        let ry = attr_f32(attrs, "ry").unwrap_or(rx);
        Some(Element::Rect {
            x, y, w, h, rx, ry,
            style: parse_style_attrs(attrs),
            transform: parse_transform_attr(attrs),
        })
    }

    fn parse_circle(&self, attrs: &[Attr]) -> Option<Element> {
        let r = attr_f32(attrs, "r").filter(|&v| v > 0.0)?;
        Some(Element::Circle {
            cx: attr_f32(attrs, "cx").unwrap_or(0.0),
            cy: attr_f32(attrs, "cy").unwrap_or(0.0),
            r,
            style: parse_style_attrs(attrs),
            transform: parse_transform_attr(attrs),
        })
    }

    fn parse_ellipse(&self, attrs: &[Attr]) -> Option<Element> {
        let rx = attr_f32(attrs, "rx").filter(|&v| v > 0.0)?;
        let ry = attr_f32(attrs, "ry").filter(|&v| v > 0.0)?;
        Some(Element::Ellipse {
            cx: attr_f32(attrs, "cx").unwrap_or(0.0),
            cy: attr_f32(attrs, "cy").unwrap_or(0.0),
            rx, ry,
            style: parse_style_attrs(attrs),
            transform: parse_transform_attr(attrs),
        })
    }

    fn parse_line(&self, attrs: &[Attr]) -> Option<Element> {
        Some(Element::Line {
            x1: attr_f32(attrs, "x1").unwrap_or(0.0),
            y1: attr_f32(attrs, "y1").unwrap_or(0.0),
            x2: attr_f32(attrs, "x2").unwrap_or(0.0),
            y2: attr_f32(attrs, "y2").unwrap_or(0.0),
            style: parse_style_attrs(attrs),
            transform: parse_transform_attr(attrs),
        })
    }

    fn parse_poly(&self, attrs: &[Attr], closed: bool) -> Option<Element> {
        let pts_str = attr_str(attrs, "points")?;
        let nums = parse_floats(&pts_str);
        let mut pts = Vec::new();
        let mut i = 0;
        while i + 1 < nums.len() {
            pts.push((nums[i], nums[i + 1]));
            i += 2;
        }
        if pts.is_empty() { return None; }
        let style = parse_style_attrs(attrs);
        let transform = parse_transform_attr(attrs);
        if closed {
            Some(Element::Polygon { pts, style, transform })
        } else {
            Some(Element::Polyline { pts, style, transform })
        }
    }
}

// ── Attribute helpers ────────────────────────────────────────────────

fn attr_by_name<'a>(attrs: &'a [Attr], key: &str) -> Option<&'a str> {
    attrs.iter().find(|a| a.key == key || a.key.ends_with(&[':'])
        .then(|| false).unwrap_or(false) || a.key == key)
        .map(|a| a.value.as_str())
        // Also handle namespace-prefixed keys like xlink:href
        .or_else(|| attrs.iter()
            .find(|a| {
                let k = &a.key;
                k.ends_with(key) && k.len() > key.len() && k.as_bytes()[k.len()-key.len()-1] == b':'
            })
            .map(|a| a.value.as_str()))
}

fn attr_str(attrs: &[Attr], key: &str) -> Option<String> {
    // Check exact match and namespace-prefixed variants
    for a in attrs {
        if a.key == key {
            return Some(a.value.clone());
        }
        // e.g. "xlink:href" when looking for "href"
        if let Some(local) = a.key.rfind(':').map(|i| &a.key[i+1..]) {
            if local == key {
                return Some(a.value.clone());
            }
        }
    }
    None
}

fn attr_href(attrs: &[Attr]) -> Option<String> {
    attr_str(attrs, "href").map(|s| {
        s.trim_start_matches('#').to_string()
    })
}

fn attr_f32(attrs: &[Attr], key: &str) -> Option<f32> {
    attr_str(attrs, key)
        .and_then(|s| parse_length_px(&s))
}

fn attr_pct_or_f32(attrs: &[Attr], key: &str) -> Option<f32> {
    attr_str(attrs, key).and_then(|s| {
        if s.ends_with('%') {
            s[..s.len()-1].parse::<f32>().ok().map(|v| v / 100.0)
        } else {
            parse_length_px(&s)
        }
    })
}

// ── Length / dimension parsing ───────────────────────────────────────

/// Convert a CSS length string to pixels (96 dpi).
pub fn parse_length_px(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.is_empty() { return None; }
    let (num_str, unit) = split_number_unit(s);
    let v: f32 = num_str.parse().ok()?;
    let px = match unit {
        "px" | ""   => v,
        "pt"        => v * 96.0 / 72.0,
        "pc"        => v * 16.0,
        "mm"        => v * 3.7795275,
        "cm"        => v * 37.795275,
        "in"        => v * 96.0,
        "em" | "ex" => v * 16.0,  // approximate
        "%"         => v,  // percentage — caller resolves
        _           => v,
    };
    Some(px)
}

fn split_number_unit(s: &str) -> (&str, &str) {
    let end = s.find(|c: char| c.is_alphabetic() || c == '%')
        .unwrap_or(s.len());
    (&s[..end], &s[end..])
}

fn parse_svg_length(s: Option<&str>, fallback: Option<f32>) -> f32 {
    s.and_then(|v| {
        if v.ends_with('%') {
            // percentage of viewport — not resolvable here, return fallback
            fallback
        } else {
            parse_length_px(v)
        }
    })
    .or(fallback)
    .unwrap_or(0.0)
}

// ── Style parsing ────────────────────────────────────────────────────

/// Parse element presentation attributes + `style=""` into a [`Style`].
pub fn parse_style_attrs(attrs: &[Attr]) -> Style {
    let mut s = Style::default();

    // First apply presentation attributes
    apply_presentation_attrs(attrs, &mut s);

    // Then override with inline style (higher specificity)
    if let Some(style_str) = attr_str(attrs, "style") {
        let props = parse_style_str(&style_str);
        apply_style_props(&props, &mut s);
    }

    s
}

fn apply_presentation_attrs(attrs: &[Attr], s: &mut Style) {
    for a in attrs {
        apply_one_prop(a.key.as_str(), a.value.as_str(), s);
    }
}

fn apply_style_props(props: &BTreeMap<String, String>, s: &mut Style) {
    for (k, v) in props {
        apply_one_prop(k.as_str(), v.as_str(), s);
    }
}

fn apply_one_prop(key: &str, val: &str, s: &mut Style) {
    let val = val.trim();
    match key.trim() {
        "fill" => { s.fill = parse_paint(val); }
        "fill-opacity" => { s.fill_opacity = clamp01(val.parse().unwrap_or(s.fill_opacity)); }
        "fill-rule" => {
            s.fill_rule = if val == "evenodd" { FillRule::EvenOdd } else { FillRule::NonZero };
        }
        "stroke" => { s.stroke = parse_paint(val); }
        "stroke-width" => { s.stroke_width = parse_length_px(val).unwrap_or(s.stroke_width); }
        "stroke-opacity" => { s.stroke_opacity = clamp01(val.parse().unwrap_or(s.stroke_opacity)); }
        "stroke-linecap" => {
            s.stroke_linecap = match val {
                "round"  => LineCap::Round,
                "square" => LineCap::Square,
                _        => LineCap::Butt,
            };
        }
        "stroke-linejoin" => {
            s.stroke_linejoin = match val {
                "round" => LineJoin::Round,
                "bevel" => LineJoin::Bevel,
                _       => LineJoin::Miter,
            };
        }
        "opacity" => { s.opacity = clamp01(val.parse().unwrap_or(s.opacity)); }
        "display"    => { s.display = val != "none"; }
        "visibility" => { s.display = val != "hidden" && val != "collapse"; }
        _ => {}
    }
}

/// Parse `"key: val; key2: val2"` into a map.
pub fn parse_style_str(style: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for part in style.split(';') {
        if let Some(colon) = part.find(':') {
            let key = part[..colon].trim().to_ascii_lowercase();
            let val = part[colon+1..].trim().to_string();
            if !key.is_empty() {
                map.insert(key, val);
            }
        }
    }
    map
}

/// Parse a paint value: `none`, `#rrggbb`, `rgb(...)`, named colour, `url(#id)`.
pub fn parse_paint(s: &str) -> Paint {
    let s = s.trim();
    if s == "none" || s.is_empty() { return Paint::None; }
    if let Some(id) = s.strip_prefix("url(#")
        .and_then(|r| r.strip_suffix(')'))
    {
        return Paint::Url(String::from(id));
    }
    if let Some(color) = parse_color(s) {
        return Paint::Color(color);
    }
    Paint::None
}

/// Parse a CSS colour to ARGB8888.
pub fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    if let Some(inner) = s.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        return parse_rgb_color(inner, false);
    }
    if let Some(inner) = s.strip_prefix("rgba(").and_then(|r| r.strip_suffix(')')) {
        return parse_rgb_color(inner, true);
    }
    // Named colours (CSS4 extended subset)
    named_color(s)
}

fn parse_hex_color(hex: &str) -> Option<u32> {
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            Some(0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32)
        }
        4 => {
            let a = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let r = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[3..4], 16).ok()? * 17;
            Some(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32)
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32)
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32)
        }
        _ => None,
    }
}

fn parse_rgb_color(inner: &str, has_alpha: bool) -> Option<u32> {
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() < 3 { return None; }
    let parse_ch = |s: &str| -> u8 {
        let s = s.trim();
        if s.ends_with('%') {
            let pct: f32 = s[..s.len()-1].parse().unwrap_or(0.0);
            (pct * 2.55) as u8
        } else {
            s.parse::<f32>().unwrap_or(0.0) as u8
        }
    };
    let r = parse_ch(parts[0]);
    let g = parse_ch(parts[1]);
    let b = parse_ch(parts[2]);
    let a = if has_alpha && parts.len() >= 4 {
        (clamp01(parts[3].trim().parse::<f32>().unwrap_or(1.0)) * 255.0) as u8
    } else { 255 };
    Some(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32)
}

fn named_color(name: &str) -> Option<u32> {
    // Most common SVG named colours
    Some(match name {
        "black"   => 0xFF000000, "white"   => 0xFFFFFFFF,
        "red"     => 0xFFFF0000, "green"   => 0xFF008000,
        "blue"    => 0xFF0000FF, "yellow"  => 0xFFFFFF00,
        "cyan"    | "aqua"    => 0xFF00FFFF,
        "magenta" | "fuchsia" => 0xFFFF00FF,
        "orange"  => 0xFFFFA500, "purple"  => 0xFF800080,
        "pink"    => 0xFFFFC0CB, "brown"   => 0xFFA52A2A,
        "gray"    | "grey"    => 0xFF808080,
        "darkgray"| "darkgrey"=> 0xFFA9A9A9,
        "lightgray"|"lightgrey"=>0xFFD3D3D3,
        "silver"  => 0xFFC0C0C0, "maroon"  => 0xFF800000,
        "navy"    => 0xFF000080, "teal"    => 0xFF008080,
        "olive"   => 0xFF808000, "lime"    => 0xFF00FF00,
        "darkred" => 0xFF8B0000, "darkblue"=> 0xFF00008B,
        "darkgreen"=>0xFF006400, "darkgoldenrod"=>0xFFB8860B,
        "skyblue" => 0xFF87CEEB, "steelblue"=>0xFF4682B4,
        "coral"   => 0xFFFF7F50, "salmon"  => 0xFFFA8072,
        "gold"    => 0xFFFFD700, "khaki"   => 0xFFF0E68C,
        "lavender"=> 0xFFE6E6FA, "beige"   => 0xFFF5F5DC,
        "ivory"   => 0xFFFFFFF0, "azure"   => 0xFFF0FFFF,
        "wheat"   => 0xFFF5DEB3, "tan"     => 0xFFD2B48C,
        "transparent" => 0x00000000,
        _ => return None,
    })
}

// ── Transform parsing ────────────────────────────────────────────────

fn parse_transform_attr(attrs: &[Attr]) -> Transform {
    attr_str(attrs, "transform")
        .map(|s| Transform::from_str(&s))
        .unwrap_or_else(Transform::identity)
}

fn parse_gradient_transform(attrs: &[Attr]) -> Transform {
    attr_str(attrs, "gradienttransform")
        .or_else(|| attr_str(attrs, "gradientTransform"))
        .map(|s| Transform::from_str(&s))
        .unwrap_or_else(Transform::identity)
}

fn parse_gradient_units(attrs: &[Attr]) -> GradientUnits {
    match attr_str(attrs, "gradientunits")
        .or_else(|| attr_str(attrs, "gradientUnits"))
        .as_deref()
    {
        Some("userSpaceOnUse") => GradientUnits::UserSpace,
        _ => GradientUnits::ObjectBoundingBox,
    }
}

fn parse_spread_method(attrs: &[Attr]) -> Spread {
    match attr_str(attrs, "spreadmethod")
        .or_else(|| attr_str(attrs, "spreadMethod"))
        .as_deref()
    {
        Some("reflect") => Spread::Reflect,
        Some("repeat")  => Spread::Repeat,
        _ => Spread::Pad,
    }
}

fn parse_stop_offset(attrs: &[Attr]) -> f32 {
    let raw = attr_str(attrs, "offset")
        .and_then(|s| {
            if s.ends_with('%') {
                s[..s.len()-1].parse::<f32>().ok().map(|v| v / 100.0)
            } else {
                s.parse::<f32>().ok()
            }
        })
        .unwrap_or(0.0f32);
    clamp01(raw)
}

fn parse_stop_color(attrs: &[Attr]) -> u32 {
    // stop-color can be in style="" or as a presentation attribute
    let color_str = attr_str(attrs, "stop-color")
        .or_else(|| {
            attr_str(attrs, "style").and_then(|s| {
                let props = parse_style_str(&s);
                props.get("stop-color").cloned()
            })
        });
    let opacity_str = attr_str(attrs, "stop-opacity")
        .or_else(|| {
            attr_str(attrs, "style").and_then(|s| {
                let props = parse_style_str(&s);
                props.get("stop-opacity").cloned()
            })
        });

    let color = color_str.as_deref()
        .and_then(parse_color)
        .unwrap_or(0xFF000000);
    let opacity: f32 = {
        let v: f32 = opacity_str.as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0f32);
        if v < 0.0 { 0.0 } else if v > 1.0 { 1.0 } else { v }
    };

    let alpha = (((color >> 24) as f32) * opacity) as u32;
    (alpha << 24) | (color & 0x00FFFFFF)
}

fn inherit_stops(defs: &BTreeMap<String, Def>, href: Option<&str>) -> Vec<GradientStop> {
    href.and_then(|id| defs.get(id)).map(|def| match def {
        Def::LinearGradient(g) => g.stops.clone(),
        Def::RadialGradient(g) => g.stops.clone(),
    }).unwrap_or_default()
}

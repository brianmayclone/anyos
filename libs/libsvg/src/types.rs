// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Core SVG document types.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;

// ── Affine Transform ────────────────────────────────────────────────

/// 6-element column-major affine transform matrix.
///
/// `[a, b, c, d, e, f]` applies as:
/// - `x' = a·x + c·y + e`
/// - `y' = b·x + d·y + f`
#[derive(Clone, Copy, Debug)]
pub struct Transform(pub [f32; 6]);

impl Transform {
    /// Identity transform.
    pub const fn identity() -> Self {
        Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
    }

    /// Apply this transform to a 2-D point.
    #[inline]
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }

    /// Concatenate (multiply): `self` is applied after `parent`.
    pub fn concat(&self, parent: &Transform) -> Transform {
        let [a, b, c, d, e, f] = self.0;
        let [p, q, r, s, t, u] = parent.0;
        Transform([
            a * p + c * q,
            b * p + d * q,
            a * r + c * s,
            b * r + d * s,
            a * t + c * u + e,
            b * t + d * u + f,
        ])
    }

    /// Parse a transform attribute string into a `Transform`.
    ///
    /// Supports: `matrix(a b c d e f)`, `translate(x[,y])`, `scale(x[,y])`,
    /// `rotate(angle[,cx,cy])`, `skewX(angle)`, `skewY(angle)`.
    pub fn from_str(s: &str) -> Transform {
        let mut result = Transform::identity();
        let s = s.trim();
        let mut pos = 0;
        while pos < s.len() {
            let rest = s[pos..].trim_start();
            if rest.is_empty() {
                break;
            }
            if let Some(t) = parse_one_transform(rest) {
                result = t.concat(&result);
                let consumed = rest.len() - rest[find_transform_end(rest)..].len();
                pos += (s.len() - rest.len()) + consumed;
                // skip optional comma/space separator
                let after = s[pos..].trim_start_matches([' ', ',', '\t', '\n', '\r']);
                pos = s.len() - after.len();
            } else {
                break;
            }
        }
        result
    }
}

fn find_transform_end(s: &str) -> usize {
    // Find the closing ')' and step past it
    s.find(')').map(|i| i + 1).unwrap_or(s.len())
}

fn parse_one_transform(s: &str) -> Option<Transform> {
    let paren = s.find('(')?;
    let close = s.find(')')?;
    let name = s[..paren].trim();
    let args_str = &s[paren + 1..close];
    let args = parse_floats(args_str);

    match name {
        "matrix" if args.len() >= 6 => Some(Transform([
            args[0], args[1], args[2], args[3], args[4], args[5],
        ])),
        "translate" if !args.is_empty() => {
            let tx = args[0];
            let ty = if args.len() >= 2 { args[1] } else { 0.0 };
            Some(Transform([1.0, 0.0, 0.0, 1.0, tx, ty]))
        }
        "scale" if !args.is_empty() => {
            let sx = args[0];
            let sy = if args.len() >= 2 { args[1] } else { sx };
            Some(Transform([sx, 0.0, 0.0, sy, 0.0, 0.0]))
        }
        "rotate" if !args.is_empty() => {
            let angle = args[0].to_radians();
            let (sin, cos) = (libm_sin(angle), libm_cos(angle));
            if args.len() >= 3 {
                let cx = args[1];
                let cy = args[2];
                // rotate around (cx,cy)
                Some(Transform([
                    cos, sin, -sin, cos,
                    cx - cos * cx + sin * cy,
                    cy - sin * cx - cos * cy,
                ]))
            } else {
                Some(Transform([cos, sin, -sin, cos, 0.0, 0.0]))
            }
        }
        "skewX" if !args.is_empty() => {
            let t = libm_tan(args[0].to_radians());
            Some(Transform([1.0, 0.0, t, 1.0, 0.0, 0.0]))
        }
        "skewY" if !args.is_empty() => {
            let t = libm_tan(args[0].to_radians());
            Some(Transform([1.0, t, 0.0, 1.0, 0.0, 0.0]))
        }
        _ => None,
    }
}

// ── Paint / Style ────────────────────────────────────────────────────

/// How a shape surface is painted.
#[derive(Clone)]
pub enum Paint {
    /// Transparent / not painted.
    None,
    /// Solid ARGB8888 colour.
    Color(u32),
    /// Reference to a gradient or pattern defined in `<defs>`.
    Url(String),
}

/// CSS fill-rule.
#[derive(Clone, Copy, PartialEq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// SVG stroke-linecap.
#[derive(Clone, Copy)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

/// SVG stroke-linejoin.
#[derive(Clone, Copy)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

/// Resolved style for a single SVG element.
#[derive(Clone)]
pub struct Style {
    pub fill: Paint,
    pub fill_opacity: f32,
    pub fill_rule: FillRule,
    pub stroke: Paint,
    pub stroke_width: f32,
    pub stroke_opacity: f32,
    pub stroke_linecap: LineCap,
    pub stroke_linejoin: LineJoin,
    pub opacity: f32,
    pub display: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fill: Paint::Color(0xFF000000),
            fill_opacity: 1.0,
            fill_rule: FillRule::NonZero,
            stroke: Paint::None,
            stroke_width: 1.0,
            stroke_opacity: 1.0,
            stroke_linecap: LineCap::Butt,
            stroke_linejoin: LineJoin::Miter,
            opacity: 1.0,
            display: true,
        }
    }
}

// ── Path commands ────────────────────────────────────────────────────

/// A single SVG path drawing command.
#[derive(Clone)]
pub enum PathCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),   // cp1x,cp1y, cp2x,cp2y, x,y
    QuadTo(f32, f32, f32, f32),               // cpx,cpy, x,y
    ArcTo {
        rx: f32, ry: f32, x_rot: f32,
        large: bool, sweep: bool,
        x: f32, y: f32,
    },
    ClosePath,
}

// ── Gradient types ───────────────────────────────────────────────────

/// Gradient stop: (offset in [0,1], ARGB colour).
pub type GradientStop = (f32, u32);

/// Gradient coordinate space.
#[derive(Clone, Copy, PartialEq)]
pub enum GradientUnits {
    /// Coordinates relative to the object bounding box (0..1).
    ObjectBoundingBox,
    /// Coordinates in user (SVG canvas) space.
    UserSpace,
}

/// How gradient colours extend beyond their defined range.
#[derive(Clone, Copy)]
pub enum Spread {
    Pad,
    Reflect,
    Repeat,
}

/// SVG linear gradient definition.
#[derive(Clone)]
pub struct LinearGradient {
    pub x1: f32, pub y1: f32,
    pub x2: f32, pub y2: f32,
    pub units: GradientUnits,
    pub stops: Vec<GradientStop>,
    pub xform: Transform,
    pub spread: Spread,
}

/// SVG radial gradient definition.
#[derive(Clone)]
pub struct RadialGradient {
    pub cx: f32, pub cy: f32, pub r: f32,
    pub fx: f32, pub fy: f32,
    pub units: GradientUnits,
    pub stops: Vec<GradientStop>,
    pub xform: Transform,
    pub spread: Spread,
}

/// A definition stored in `<defs>`.
#[derive(Clone)]
pub enum Def {
    LinearGradient(LinearGradient),
    RadialGradient(RadialGradient),
}

// ── SVG Document elements ────────────────────────────────────────────

/// A renderable SVG element.
#[derive(Clone)]
pub enum Element {
    Path {
        cmds: Vec<PathCmd>,
        style: Style,
        transform: Transform,
    },
    Rect {
        x: f32, y: f32, w: f32, h: f32,
        rx: f32, ry: f32,
        style: Style,
        transform: Transform,
    },
    Circle {
        cx: f32, cy: f32, r: f32,
        style: Style,
        transform: Transform,
    },
    Ellipse {
        cx: f32, cy: f32, rx: f32, ry: f32,
        style: Style,
        transform: Transform,
    },
    Line {
        x1: f32, y1: f32, x2: f32, y2: f32,
        style: Style,
        transform: Transform,
    },
    Polyline {
        pts: Vec<(f32, f32)>,
        style: Style,
        transform: Transform,
    },
    Polygon {
        pts: Vec<(f32, f32)>,
        style: Style,
        transform: Transform,
    },
    Group {
        elements: Vec<Element>,
        transform: Transform,
        opacity: f32,
    },
}

/// Top-level parsed SVG document.
pub struct SvgDoc {
    /// Declared width in pixels (after unit conversion).
    pub width: f32,
    /// Declared height in pixels.
    pub height: f32,
    /// Optional `viewBox="x y w h"`.
    pub view_box: Option<[f32; 4]>,
    /// Root-level elements.
    pub elements: Vec<Element>,
    /// Named definitions (gradients, etc.) from `<defs>`.
    pub defs: BTreeMap<String, Def>,
}

// ── Utility: parse a whitespace/comma-separated list of floats ───────

/// Parse a whitespace/comma-separated list of f32 values.
pub fn parse_floats(s: &str) -> Vec<f32> {
    let mut out = Vec::new();
    let mut pos = 0;
    let bytes = s.as_bytes();
    while pos < bytes.len() {
        // skip separators
        while pos < bytes.len()
            && matches!(bytes[pos], b' ' | b'\t' | b'\n' | b'\r' | b',')
        {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        // try to parse a number
        let start = pos;
        // optional sign
        if pos < bytes.len() && matches!(bytes[pos], b'+' | b'-') {
            pos += 1;
        }
        while pos < bytes.len() && (bytes[pos].is_ascii_digit() || bytes[pos] == b'.') {
            pos += 1;
        }
        // optional exponent
        if pos < bytes.len() && matches!(bytes[pos], b'e' | b'E') {
            pos += 1;
            if pos < bytes.len() && matches!(bytes[pos], b'+' | b'-') {
                pos += 1;
            }
            while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
        }
        if pos > start {
            if let Ok(f) = s[start..pos].parse::<f32>() {
                out.push(f);
            }
        }
    }
    out
}

// ── Minimal libm shims (no_std compatible) ───────────────────────────

/// sin(x) via Taylor series — sufficient for transforms.
pub fn libm_sin(x: f32) -> f32 {
    // Reduce to [-pi, pi]
    let pi = core::f32::consts::PI;
    let mut x = x % (2.0 * pi);
    if x > pi { x -= 2.0 * pi; }
    if x < -pi { x += 2.0 * pi; }
    // Taylor: x - x³/6 + x⁵/120 - x⁷/5040
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0)))
}

/// cos(x) = sin(x + π/2).
pub fn libm_cos(x: f32) -> f32 {
    libm_sin(x + core::f32::consts::FRAC_PI_2)
}

/// tan(x) = sin(x)/cos(x).
pub fn libm_tan(x: f32) -> f32 {
    let s = libm_sin(x);
    let c = libm_cos(x);
    if c.abs() < 1e-10 { 1e10 } else { s / c }
}

/// sqrt(x) via Newton-Raphson.
pub fn libm_sqrt(x: f32) -> f32 {
    if x <= 0.0 { return 0.0; }
    let mut g = x;
    for _ in 0..8 {
        g = 0.5 * (g + x / g);
    }
    g
}

/// Clamp `x` to `[0.0, 1.0]` (no_std replacement for `f32::clamp`).
#[inline(always)]
pub fn clamp01(x: f32) -> f32 {
    if x < 0.0 { 0.0 } else if x > 1.0 { 1.0 } else { x }
}

/// floor(x) — largest integer ≤ x.
pub fn libm_floor(x: f32) -> f32 {
    let i = x as i32;
    let f = i as f32;
    if x < f { f - 1.0 } else { f }
}

/// ceil(x) — smallest integer ≥ x.
pub fn libm_ceil(x: f32) -> f32 {
    let i = x as i32;
    let f = i as f32;
    if x > f { f + 1.0 } else { f }
}

/// acos(x) via acos(x) = atan2(sqrt(1−x²), x).
pub fn libm_acos(x: f32) -> f32 {
    // Clamp to valid domain.
    let x = if x < -1.0 { -1.0 } else if x > 1.0 { 1.0 } else { x };
    libm_atan2(libm_sqrt(1.0 - x * x), x)
}

/// atan2(y, x) using polynomial approximation.
pub fn libm_atan2(y: f32, x: f32) -> f32 {
    let pi = core::f32::consts::PI;
    if x == 0.0 {
        if y > 0.0 { return pi / 2.0; }
        if y < 0.0 { return -pi / 2.0; }
        return 0.0;
    }
    let r = y / x;
    let atan = r / (1.0 + 0.28125 * r * r);  // fast approx
    if x > 0.0 { atan }
    else if y >= 0.0 { atan + pi }
    else { atan - pi }
}

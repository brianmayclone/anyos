//! SVG path parser and rasterizer — renders SVG icon paths to ARGB8888 pixels.
//!
//! Supports the SVG path commands used by the Tabler icon set:
//! M, m, L, l, H, h, V, v, C, c, S, s, Q, q, T, t, A, a, Z, z
//!
//! Two rendering modes:
//! - **Filled**: scanline rasterizer with non-zero winding rule
//! - **Stroke**: distance-based coverage for each line segment
//!
//! The rasterizer uses fixed-point arithmetic (8 fractional bits) for
//! no_std compatibility, same approach as libfont's TTF rasterizer.

use alloc::vec;
use alloc::vec::Vec;

// ── Fixed-point helpers (8 fractional bits) ─────────────────────────

const FP: i32 = 8;
const FP_ONE: i32 = 1 << FP;

#[inline(always)]
fn fp(v: i32) -> i32 { v << FP }

#[inline(always)]
fn fp_from_f(v: i32, frac: i32) -> i32 { (v << FP) + frac }

#[inline(always)]
fn fp_floor(v: i32) -> i32 { v >> FP }

#[inline(always)]
fn fp_ceil(v: i32) -> i32 { (v + FP_ONE - 1) >> FP }

#[inline(always)]
fn fp_mul(a: i32, b: i32) -> i32 { ((a as i64 * b as i64) >> FP) as i32 }

// ── Parsed path command ─────────────────────────────────────────────

#[derive(Clone, Copy)]
enum Cmd {
    MoveTo(i32, i32),
    LineTo(i32, i32),
    Close,
}

// ── Edge for scanline rasterizer ────────────────────────────────────

#[derive(Clone)]
struct Edge {
    x0: i32, y0: i32,
    x1: i32, y1: i32,
    winding: i32,
}

// ── Public API ──────────────────────────────────────────────────────

/// Render a single SVG icon to ARGB8888 pixels.
///
/// - `path_data`: raw SVG path d="" string(s), multiple separated by \0
/// - `filled`: true = fill paths, false = stroke paths (width=2 in 24×24 viewbox)
/// - `size`: target pixel size (square)
/// - `color`: ARGB8888 color
/// - `out`: output pixel buffer, must be size*size elements
///
/// Returns 0 on success, negative on error.
pub fn render_icon(
    path_data: &[u8],
    filled: bool,
    size: u32,
    color: u32,
    out: &mut [u32],
) -> i32 {
    if size == 0 || size > 512 || out.len() < (size * size) as usize {
        return -1;
    }

    let sz = size as usize;
    let scale = fp(size as i32); // scale = size * 256 (fixed-point)

    // Parse all paths and collect edges
    let mut edges: Vec<Edge> = Vec::new();

    // Split on \0 for multiple paths
    let mut start = 0;
    loop {
        let end = path_data[start..].iter().position(|&b| b == 0)
            .map(|p| start + p)
            .unwrap_or(path_data.len());

        let path_str = &path_data[start..end];

        // Check for translate prefix "Tx y\n"
        let (tx, ty, actual_path) = parse_translate_prefix(path_str);

        let cmds = parse_svg_path(actual_path, scale, tx, ty);
        if filled {
            collect_fill_edges(&cmds, &mut edges);
        } else {
            // Stroke width: 2.0 in 24×24 viewbox → scaled
            let stroke_w = fp_mul(fp(2), scale) / 24;
            collect_stroke_edges(&cmds, stroke_w, &mut edges);
        }

        if end >= path_data.len() { break; }
        start = end + 1;
    }

    if edges.is_empty() {
        // Clear output
        for p in out[..sz * sz].iter_mut() { *p = 0; }
        return 0;
    }

    // Rasterize edges to coverage bitmap
    let coverage = rasterize_edges(&edges, size, size);

    // Apply color
    let ca = (color >> 24) & 0xFF;
    let cr = (color >> 16) & 0xFF;
    let cg = (color >> 8) & 0xFF;
    let cb = color & 0xFF;

    for i in 0..sz * sz {
        let cov = coverage[i] as u32;
        if cov == 0 {
            out[i] = 0;
        } else {
            let a = (cov * ca + 127) / 255;
            out[i] = (a << 24) | (cr << 16) | (cg << 8) | cb;
        }
    }

    0
}

// ── SVG Path Parser ─────────────────────────────────────────────────

/// Parse translate prefix "Tx y\n" if present.
fn parse_translate_prefix(data: &[u8]) -> (i32, i32, &[u8]) {
    if data.len() < 2 || data[0] != b'T' {
        return (0, 0, data);
    }
    // Find newline
    let nl = match data.iter().position(|&b| b == b'\n') {
        Some(p) => p,
        None => return (0, 0, data),
    };
    let prefix = &data[1..nl];
    // Parse "tx ty"
    let mut nums = [0i32; 2];
    let mut ni = 0;
    let mut i = 0;
    while i < prefix.len() && ni < 2 {
        // Skip spaces
        while i < prefix.len() && prefix[i] == b' ' { i += 1; }
        if i >= prefix.len() { break; }
        let (v, adv) = parse_number_at(prefix, i);
        nums[ni] = v;
        ni += 1;
        i += adv;
    }
    (nums[0], nums[1], &data[nl + 1..])
}

/// Parse SVG path data into a list of absolute MoveTo/LineTo/Close commands.
/// All curves are flattened to line segments. Coordinates are in fixed-point,
/// scaled from the 24×24 viewbox to the target size.
fn parse_svg_path(data: &[u8], scale: i32, tx: i32, ty: i32) -> Vec<Cmd> {
    let mut cmds: Vec<Cmd> = Vec::new();
    let mut cx: i32 = tx; // current point (fixed-point in viewbox * 256)
    let mut cy: i32 = ty;
    let mut sx: i32 = tx; // subpath start
    let mut sy: i32 = ty;
    let mut last_c2x: i32 = 0; // last cubic control point 2
    let mut last_c2y: i32 = 0;
    let mut last_qx: i32 = 0; // last quad control point
    let mut last_qy: i32 = 0;
    let mut last_cmd: u8 = 0;

    let mut i = 0;

    while i < data.len() {
        // Skip whitespace/commas
        while i < data.len() && (data[i] == b' ' || data[i] == b',' || data[i] == b'\n' || data[i] == b'\r') {
            i += 1;
        }
        if i >= data.len() { break; }

        let cmd_byte = data[i];
        let is_cmd = cmd_byte.is_ascii_alphabetic() && cmd_byte != b'e' && cmd_byte != b'E';

        let cmd = if is_cmd {
            i += 1;
            cmd_byte
        } else {
            // Implicit repeat
            match last_cmd {
                b'M' => b'L',
                b'm' => b'l',
                _ => last_cmd,
            }
        };

        match cmd {
            b'Z' | b'z' => {
                cmds.push(Cmd::Close);
                cx = sx; cy = sy;
                last_c2x = cx; last_c2y = cy;
                last_qx = cx; last_qy = cy;
                last_cmd = cmd;
                continue;
            }
            b'M' => {
                let (x, adv) = eat_num(data, &mut i);
                let (y, _) = eat_num(data, &mut i);
                cx = x; cy = y; sx = x; sy = y;
                let (px, py) = viewbox_to_px(cx, cy, scale);
                cmds.push(Cmd::MoveTo(px, py));
                last_c2x = cx; last_c2y = cy;
                last_qx = cx; last_qy = cy;
                last_cmd = b'M';
            }
            b'm' => {
                let (dx, _) = eat_num(data, &mut i);
                let (dy, _) = eat_num(data, &mut i);
                cx += dx; cy += dy; sx = cx; sy = cy;
                let (px, py) = viewbox_to_px(cx, cy, scale);
                cmds.push(Cmd::MoveTo(px, py));
                last_c2x = cx; last_c2y = cy;
                last_qx = cx; last_qy = cy;
                last_cmd = b'm';
            }
            b'L' => {
                let (x, _) = eat_num(data, &mut i);
                let (y, _) = eat_num(data, &mut i);
                cx = x; cy = y;
                let (px, py) = viewbox_to_px(cx, cy, scale);
                cmds.push(Cmd::LineTo(px, py));
                last_c2x = cx; last_c2y = cy;
                last_qx = cx; last_qy = cy;
                last_cmd = b'L';
            }
            b'l' => {
                let (dx, _) = eat_num(data, &mut i);
                let (dy, _) = eat_num(data, &mut i);
                cx += dx; cy += dy;
                let (px, py) = viewbox_to_px(cx, cy, scale);
                cmds.push(Cmd::LineTo(px, py));
                last_c2x = cx; last_c2y = cy;
                last_qx = cx; last_qy = cy;
                last_cmd = b'l';
            }
            b'H' => {
                let (x, _) = eat_num(data, &mut i);
                cx = x;
                let (px, py) = viewbox_to_px(cx, cy, scale);
                cmds.push(Cmd::LineTo(px, py));
                last_c2x = cx; last_c2y = cy;
                last_qx = cx; last_qy = cy;
                last_cmd = b'H';
            }
            b'h' => {
                let (dx, _) = eat_num(data, &mut i);
                cx += dx;
                let (px, py) = viewbox_to_px(cx, cy, scale);
                cmds.push(Cmd::LineTo(px, py));
                last_c2x = cx; last_c2y = cy;
                last_qx = cx; last_qy = cy;
                last_cmd = b'h';
            }
            b'V' => {
                let (y, _) = eat_num(data, &mut i);
                cy = y;
                let (px, py) = viewbox_to_px(cx, cy, scale);
                cmds.push(Cmd::LineTo(px, py));
                last_c2x = cx; last_c2y = cy;
                last_qx = cx; last_qy = cy;
                last_cmd = b'V';
            }
            b'v' => {
                let (dy, _) = eat_num(data, &mut i);
                cy += dy;
                let (px, py) = viewbox_to_px(cx, cy, scale);
                cmds.push(Cmd::LineTo(px, py));
                last_c2x = cx; last_c2y = cy;
                last_qx = cx; last_qy = cy;
                last_cmd = b'v';
            }
            b'C' => {
                let (c1x, _) = eat_num(data, &mut i);
                let (c1y, _) = eat_num(data, &mut i);
                let (c2x, _) = eat_num(data, &mut i);
                let (c2y, _) = eat_num(data, &mut i);
                let (ex, _) = eat_num(data, &mut i);
                let (ey, _) = eat_num(data, &mut i);
                flatten_cubic(cx, cy, c1x, c1y, c2x, c2y, ex, ey, scale, &mut cmds);
                last_c2x = c2x; last_c2y = c2y;
                cx = ex; cy = ey;
                last_qx = cx; last_qy = cy;
                last_cmd = b'C';
            }
            b'c' => {
                let (d1x, _) = eat_num(data, &mut i);
                let (d1y, _) = eat_num(data, &mut i);
                let (d2x, _) = eat_num(data, &mut i);
                let (d2y, _) = eat_num(data, &mut i);
                let (dx, _) = eat_num(data, &mut i);
                let (dy, _) = eat_num(data, &mut i);
                let c1x = cx + d1x; let c1y = cy + d1y;
                let c2x = cx + d2x; let c2y = cy + d2y;
                let ex = cx + dx; let ey = cy + dy;
                flatten_cubic(cx, cy, c1x, c1y, c2x, c2y, ex, ey, scale, &mut cmds);
                last_c2x = c2x; last_c2y = c2y;
                cx = ex; cy = ey;
                last_qx = cx; last_qy = cy;
                last_cmd = b'c';
            }
            b'S' => {
                let rcx = 2 * cx - last_c2x;
                let rcy = 2 * cy - last_c2y;
                let (c2x, _) = eat_num(data, &mut i);
                let (c2y, _) = eat_num(data, &mut i);
                let (ex, _) = eat_num(data, &mut i);
                let (ey, _) = eat_num(data, &mut i);
                flatten_cubic(cx, cy, rcx, rcy, c2x, c2y, ex, ey, scale, &mut cmds);
                last_c2x = c2x; last_c2y = c2y;
                cx = ex; cy = ey;
                last_qx = cx; last_qy = cy;
                last_cmd = b'S';
            }
            b's' => {
                let rcx = 2 * cx - last_c2x;
                let rcy = 2 * cy - last_c2y;
                let (d2x, _) = eat_num(data, &mut i);
                let (d2y, _) = eat_num(data, &mut i);
                let (dx, _) = eat_num(data, &mut i);
                let (dy, _) = eat_num(data, &mut i);
                let c2x = cx + d2x; let c2y = cy + d2y;
                let ex = cx + dx; let ey = cy + dy;
                flatten_cubic(cx, cy, rcx, rcy, c2x, c2y, ex, ey, scale, &mut cmds);
                last_c2x = c2x; last_c2y = c2y;
                cx = ex; cy = ey;
                last_qx = cx; last_qy = cy;
                last_cmd = b's';
            }
            b'Q' => {
                let (qx, _) = eat_num(data, &mut i);
                let (qy, _) = eat_num(data, &mut i);
                let (ex, _) = eat_num(data, &mut i);
                let (ey, _) = eat_num(data, &mut i);
                flatten_quad(cx, cy, qx, qy, ex, ey, scale, &mut cmds);
                last_qx = qx; last_qy = qy;
                cx = ex; cy = ey;
                last_c2x = cx; last_c2y = cy;
                last_cmd = b'Q';
            }
            b'q' => {
                let (dqx, _) = eat_num(data, &mut i);
                let (dqy, _) = eat_num(data, &mut i);
                let (dx, _) = eat_num(data, &mut i);
                let (dy, _) = eat_num(data, &mut i);
                let qx = cx + dqx; let qy = cy + dqy;
                let ex = cx + dx; let ey = cy + dy;
                flatten_quad(cx, cy, qx, qy, ex, ey, scale, &mut cmds);
                last_qx = qx; last_qy = qy;
                cx = ex; cy = ey;
                last_c2x = cx; last_c2y = cy;
                last_cmd = b'q';
            }
            b'T' => {
                let rqx = 2 * cx - last_qx;
                let rqy = 2 * cy - last_qy;
                let (ex, _) = eat_num(data, &mut i);
                let (ey, _) = eat_num(data, &mut i);
                flatten_quad(cx, cy, rqx, rqy, ex, ey, scale, &mut cmds);
                last_qx = rqx; last_qy = rqy;
                cx = ex; cy = ey;
                last_c2x = cx; last_c2y = cy;
                last_cmd = b'T';
            }
            b't' => {
                let rqx = 2 * cx - last_qx;
                let rqy = 2 * cy - last_qy;
                let (dx, _) = eat_num(data, &mut i);
                let (dy, _) = eat_num(data, &mut i);
                let ex = cx + dx; let ey = cy + dy;
                flatten_quad(cx, cy, rqx, rqy, ex, ey, scale, &mut cmds);
                last_qx = rqx; last_qy = rqy;
                cx = ex; cy = ey;
                last_c2x = cx; last_c2y = cy;
                last_cmd = b't';
            }
            b'A' => {
                let (rx, _) = eat_num(data, &mut i);
                let (ry, _) = eat_num(data, &mut i);
                let (rot, _) = eat_num(data, &mut i);
                let (la, _) = eat_flag(data, &mut i);
                let (sw, _) = eat_flag(data, &mut i);
                let (ex, _) = eat_num(data, &mut i);
                let (ey, _) = eat_num(data, &mut i);
                flatten_arc(cx, cy, rx, ry, rot, la != 0, sw != 0, ex, ey, scale, &mut cmds);
                cx = ex; cy = ey;
                last_c2x = cx; last_c2y = cy;
                last_qx = cx; last_qy = cy;
                last_cmd = b'A';
            }
            b'a' => {
                let (rx, _) = eat_num(data, &mut i);
                let (ry, _) = eat_num(data, &mut i);
                let (rot, _) = eat_num(data, &mut i);
                let (la, _) = eat_flag(data, &mut i);
                let (sw, _) = eat_flag(data, &mut i);
                let (dx, _) = eat_num(data, &mut i);
                let (dy, _) = eat_num(data, &mut i);
                let ex = cx + dx; let ey = cy + dy;
                flatten_arc(cx, cy, rx, ry, rot, la != 0, sw != 0, ex, ey, scale, &mut cmds);
                cx = ex; cy = ey;
                last_c2x = cx; last_c2y = cy;
                last_qx = cx; last_qy = cy;
                last_cmd = b'a';
            }
            _ => {
                // Unknown command, skip
                i += 1;
            }
        }
    }

    cmds
}

// ── Number parsing ──────────────────────────────────────────────────

/// Parse a fixed-point number from SVG path data at position i.
/// Numbers are in viewbox coordinates (0..24).
/// Returns (value_fp, bytes_consumed). Value is in fixed-point * 256.
/// We parse to millionths precision then convert.
fn eat_num(data: &[u8], i: &mut usize) -> (i32, usize) {
    // Skip whitespace/commas
    while *i < data.len() && (data[*i] == b' ' || data[*i] == b',' || data[*i] == b'\n' || data[*i] == b'\r') {
        *i += 1;
    }
    let start = *i;
    let (val, _) = parse_number_at(data, *i);
    // Advance past the number
    if *i < data.len() && (data[*i] == b'-' || data[*i] == b'+') { *i += 1; }
    while *i < data.len() && (data[*i].is_ascii_digit() || data[*i] == b'.') { *i += 1; }
    // Handle exponent
    if *i < data.len() && (data[*i] == b'e' || data[*i] == b'E') {
        *i += 1;
        if *i < data.len() && (data[*i] == b'-' || data[*i] == b'+') { *i += 1; }
        while *i < data.len() && data[*i].is_ascii_digit() { *i += 1; }
    }
    (val, *i - start)
}

/// Parse an arc flag (0 or 1) — may be concatenated with the next number.
fn eat_flag(data: &[u8], i: &mut usize) -> (i32, usize) {
    while *i < data.len() && (data[*i] == b' ' || data[*i] == b',' || data[*i] == b'\n') {
        *i += 1;
    }
    if *i < data.len() && (data[*i] == b'0' || data[*i] == b'1') {
        let v = (data[*i] - b'0') as i32;
        *i += 1;
        // Convert to fixed-point (flags are 0 or 1, so 0 or 256)
        (v * FP_ONE, 1)
    } else {
        eat_num(data, i)
    }
}

/// Parse a number at the given offset. Returns (fixed-point value, bytes consumed).
/// Parses integers and decimals, returns value * 256.
fn parse_number_at(data: &[u8], start: usize) -> (i32, usize) {
    let mut i = start;
    let mut neg = false;

    if i < data.len() && data[i] == b'-' {
        neg = true;
        i += 1;
    } else if i < data.len() && data[i] == b'+' {
        i += 1;
    }

    // Integer part
    let mut int_part: i64 = 0;
    while i < data.len() && data[i].is_ascii_digit() {
        int_part = int_part * 10 + (data[i] - b'0') as i64;
        i += 1;
    }

    // Fractional part
    let mut frac: i64 = 0;
    let mut frac_div: i64 = 1;
    if i < data.len() && data[i] == b'.' {
        i += 1;
        while i < data.len() && data[i].is_ascii_digit() {
            frac = frac * 10 + (data[i] - b'0') as i64;
            frac_div *= 10;
            i += 1;
        }
    }

    // Exponent (rare but possible)
    let mut exp: i32 = 0;
    let mut exp_neg = false;
    if i < data.len() && (data[i] == b'e' || data[i] == b'E') {
        i += 1;
        if i < data.len() && data[i] == b'-' { exp_neg = true; i += 1; }
        else if i < data.len() && data[i] == b'+' { i += 1; }
        while i < data.len() && data[i].is_ascii_digit() {
            exp = exp * 10 + (data[i] - b'0') as i32;
            i += 1;
        }
        if exp_neg { exp = -exp; }
    }

    // Compute fixed-point value: (int_part + frac/frac_div) * 256
    let mut val = int_part * 256 + (frac * 256 + frac_div / 2) / frac_div;

    // Apply exponent
    if exp > 0 {
        for _ in 0..exp.min(6) { val *= 10; }
    } else if exp < 0 {
        for _ in 0..(-exp).min(6) { val /= 10; }
    }

    if neg { val = -val; }

    (val as i32, i - start)
}

// ── Viewbox to pixel coordinate conversion ──────────────────────────

/// Convert viewbox coordinate (fixed-point * 256) to pixel coordinate.
/// viewbox is 24×24, target is `size` pixels.
#[inline]
fn viewbox_to_px(vx: i32, vy: i32, scale: i32) -> (i32, i32) {
    // vx is in viewbox units * 256
    // scale is target_size * 256
    // pixel = vx * scale / (24 * 256) = vx * scale / 6144
    // But we want fixed-point output for the rasterizer.
    // px_fp = vx * size / 24
    let px = ((vx as i64 * scale as i64) / (24 * 256)) as i32;
    let py = ((vy as i64 * scale as i64) / (24 * 256)) as i32;
    (px, py)
}

// ── Curve flattening ────────────────────────────────────────────────

/// Flatten a cubic Bezier curve to line segments.
/// All coordinates are in viewbox fixed-point (* 256).
fn flatten_cubic(
    p0x: i32, p0y: i32,
    p1x: i32, p1y: i32,
    p2x: i32, p2y: i32,
    p3x: i32, p3y: i32,
    scale: i32,
    cmds: &mut Vec<Cmd>,
) {
    flatten_cubic_recursive(p0x, p0y, p1x, p1y, p2x, p2y, p3x, p3y, scale, cmds, 0);
}

fn flatten_cubic_recursive(
    p0x: i32, p0y: i32,
    p1x: i32, p1y: i32,
    p2x: i32, p2y: i32,
    p3x: i32, p3y: i32,
    scale: i32,
    cmds: &mut Vec<Cmd>,
    depth: u32,
) {
    // Check flatness: distance of control points from the line p0→p3
    let dx = p3x - p0x;
    let dy = p3y - p0y;
    let d1 = ((p1x - p0x) as i64 * dy as i64 - (p1y - p0y) as i64 * dx as i64).abs();
    let d2 = ((p2x - p0x) as i64 * dy as i64 - (p2y - p0y) as i64 * dx as i64).abs();
    let len_sq = dx as i64 * dx as i64 + dy as i64 * dy as i64;

    // Threshold: ~0.25 pixel in viewbox coords, adjusted for scale
    let threshold = 64i64; // 0.25 * 256

    if depth >= 10 || ((d1 + d2) * (d1 + d2) <= threshold * threshold * len_sq.max(1)) {
        let (px, py) = viewbox_to_px(p3x, p3y, scale);
        cmds.push(Cmd::LineTo(px, py));
        return;
    }

    // De Casteljau subdivision at t=0.5
    let m01x = (p0x + p1x) / 2; let m01y = (p0y + p1y) / 2;
    let m12x = (p1x + p2x) / 2; let m12y = (p1y + p2y) / 2;
    let m23x = (p2x + p3x) / 2; let m23y = (p2y + p3y) / 2;
    let m012x = (m01x + m12x) / 2; let m012y = (m01y + m12y) / 2;
    let m123x = (m12x + m23x) / 2; let m123y = (m12y + m23y) / 2;
    let mx = (m012x + m123x) / 2; let my = (m012y + m123y) / 2;

    flatten_cubic_recursive(p0x, p0y, m01x, m01y, m012x, m012y, mx, my, scale, cmds, depth + 1);
    flatten_cubic_recursive(mx, my, m123x, m123y, m23x, m23y, p3x, p3y, scale, cmds, depth + 1);
}

/// Flatten a quadratic Bezier curve to line segments.
fn flatten_quad(
    p0x: i32, p0y: i32,
    p1x: i32, p1y: i32,
    p2x: i32, p2y: i32,
    scale: i32,
    cmds: &mut Vec<Cmd>,
) {
    flatten_quad_recursive(p0x, p0y, p1x, p1y, p2x, p2y, scale, cmds, 0);
}

fn flatten_quad_recursive(
    p0x: i32, p0y: i32,
    p1x: i32, p1y: i32,
    p2x: i32, p2y: i32,
    scale: i32,
    cmds: &mut Vec<Cmd>,
    depth: u32,
) {
    let mx = (p0x + p2x) / 2;
    let my = (p0y + p2y) / 2;
    let dx = (p1x - mx) as i64;
    let dy = (p1y - my) as i64;
    let dist_sq = dx * dx + dy * dy;
    let threshold = 64i64 * 64; // 0.25 * 256, squared

    if depth >= 8 || dist_sq <= threshold {
        let (px, py) = viewbox_to_px(p2x, p2y, scale);
        cmds.push(Cmd::LineTo(px, py));
        return;
    }

    let q0x = (p0x + p1x) / 2; let q0y = (p0y + p1y) / 2;
    let q1x = (p1x + p2x) / 2; let q1y = (p1y + p2y) / 2;
    let rx = (q0x + q1x) / 2; let ry = (q0y + q1y) / 2;

    flatten_quad_recursive(p0x, p0y, q0x, q0y, rx, ry, scale, cmds, depth + 1);
    flatten_quad_recursive(rx, ry, q1x, q1y, p2x, p2y, scale, cmds, depth + 1);
}

/// Flatten an SVG elliptical arc to line segments.
/// Approximates the arc with cubic Bezier segments, then flattens those.
fn flatten_arc(
    cx: i32, cy: i32,          // current point (viewbox fp)
    rx_: i32, ry_: i32,        // radii (viewbox fp)
    _rotation: i32,             // x-axis rotation (ignored for most icons)
    large_arc: bool,
    sweep: bool,
    ex: i32, ey: i32,          // endpoint (viewbox fp)
    scale: i32,
    cmds: &mut Vec<Cmd>,
) {
    // If radii are zero or start==end, just line to endpoint
    if (rx_ == 0 && ry_ == 0) || (cx == ex && cy == ey) {
        let (px, py) = viewbox_to_px(ex, ey, scale);
        cmds.push(Cmd::LineTo(px, py));
        return;
    }

    let mut rx = if rx_ < 0 { -rx_ } else { rx_ };
    let mut ry = if ry_ < 0 { -ry_ } else { ry_ };

    // Approximate arc with line segments using angular subdivision.
    // For simplicity (and since rotation is rarely used in these icons),
    // we parameterize the arc and emit points along it.

    // Midpoint between start and end
    let dx2 = (cx - ex) / 2;
    let dy2 = (cy - ey) / 2;

    // Ensure radii are large enough
    let d = ((dx2 as i64 * dx2 as i64) * 256 / (rx as i64 * rx as i64).max(1)
           + (dy2 as i64 * dy2 as i64) * 256 / (ry as i64 * ry as i64).max(1)) as i32;
    if d > 256 {
        // Scale up radii
        let sq = isqrt_fp(d); // sqrt(d/256) * 256
        rx = ((rx as i64 * sq as i64 + 128) / 256) as i32;
        ry = ((ry as i64 * sq as i64 + 128) / 256) as i32;
    }

    // Center computation (simplified, ignoring rotation)
    let rx2 = rx as i64 * rx as i64;
    let ry2 = ry as i64 * ry as i64;
    let dx2_64 = dx2 as i64;
    let dy2_64 = dy2 as i64;

    let num = (rx2 * ry2 - rx2 * dy2_64 * dy2_64 / 256 - ry2 * dx2_64 * dx2_64 / 256).max(0);
    let den = (rx2 * dy2_64 * dy2_64 / 256 + ry2 * dx2_64 * dx2_64 / 256).max(1);
    let sq = isqrt_fp(((num * 256) / den) as i32);
    let sign = if large_arc == sweep { -1i64 } else { 1i64 };

    let ccx = sign * sq as i64 * rx as i64 * dy2_64 / (ry as i64 * 256).max(1);
    let ccy = -sign * sq as i64 * ry as i64 * dx2_64 / (rx as i64 * 256).max(1);

    let center_x = (ccx / 256) as i32 + (cx + ex) / 2;
    let center_y = (ccy / 256) as i32 + (cy + ey) / 2;

    // Compute start and end angles using atan2 approximation
    let a1 = atan2_approx(
        ((cy - center_y) as i64 * 256 / ry as i64.max(1)) as i32,
        ((cx - center_x) as i64 * 256 / rx as i64.max(1)) as i32,
    );
    let a2 = atan2_approx(
        ((ey - center_y) as i64 * 256 / ry as i64.max(1)) as i32,
        ((ex - center_x) as i64 * 256 / rx as i64.max(1)) as i32,
    );

    // Determine sweep angle
    let mut da = a2 - a1;
    if sweep && da < 0 { da += 360 * 256; }
    if !sweep && da > 0 { da -= 360 * 256; }

    // Number of segments: more for larger arcs
    let abs_da = if da < 0 { -da } else { da };
    let n_segs = ((abs_da / (30 * 256)) + 1).max(1).min(24) as i32;

    let step = da / n_segs;
    for seg in 1..=n_segs {
        let angle = a1 + step * seg;
        let (sin_a, cos_a) = sin_cos_approx(angle);
        let px = center_x + ((rx as i64 * cos_a as i64) / 256) as i32;
        let py = center_y + ((ry as i64 * sin_a as i64) / 256) as i32;
        let (ppx, ppy) = viewbox_to_px(px, py, scale);
        cmds.push(Cmd::LineTo(ppx, ppy));
    }

    // Ensure we end exactly at the endpoint
    let (epx, epy) = viewbox_to_px(ex, ey, scale);
    if let Some(&Cmd::LineTo(lx, ly)) = cmds.last() {
        if lx != epx || ly != epy {
            cmds.push(Cmd::LineTo(epx, epy));
        }
    }
}

// ── Integer trig approximations ─────────────────────────────────────

/// Integer square root of fixed-point value (input and output * 256).
fn isqrt_fp(v: i32) -> i32 {
    if v <= 0 { return 0; }
    // sqrt(v/256) * 256
    let v64 = v as i64 * 256; // scale up for precision
    let mut x = 256i64;
    for _ in 0..16 {
        let nx = (x + v64 / x) / 2;
        if nx >= x { break; }
        x = nx;
    }
    x as i32
}

/// atan2 approximation. Returns angle in fixed-point degrees * 256.
fn atan2_approx(y: i32, x: i32) -> i32 {
    if x == 0 && y == 0 { return 0; }

    let ax = if x < 0 { -x } else { x } as i64;
    let ay = if y < 0 { -y } else { y } as i64;

    // Compute atan(min/max) using polynomial: atan(t) ≈ t * (45 - (t-1)*18) degrees
    let (min_v, max_v) = if ax > ay { (ay, ax) } else { (ax, ay) };
    let t = if max_v == 0 { 0 } else { (min_v * 256 / max_v) as i32 }; // 0..256

    // atan(t) ≈ 45*t/256 degrees (linear approx good enough for icons)
    let mut angle = (45i64 * t as i64) as i32; // degrees * 256

    if ax <= ay { angle = 90 * 256 - angle; }
    if x < 0 { angle = 180 * 256 - angle; }
    if y < 0 { angle = -angle; }

    angle
}

/// Sine/cosine approximation. Input: angle in degrees * 256.
/// Output: (sin, cos) each in fixed-point * 256.
fn sin_cos_approx(angle_deg256: i32) -> (i32, i32) {
    // Normalize to 0..360*256
    let mut a = angle_deg256 % (360 * 256);
    if a < 0 { a += 360 * 256; }

    // Convert to 0..1024 (quarter-turn units for table lookup)
    let idx = ((a as i64 * 1024) / (360 * 256)) as i32;

    // Use quadrant symmetry with a small sine table
    let quadrant = (idx / 256) & 3;
    let frac = idx & 255;

    // Linear interpolation in sine table (256 entries per quadrant)
    let sin_val = match quadrant {
        0 => sin_table(frac),
        1 => sin_table(256 - frac),
        2 => -sin_table(frac),
        _ => -sin_table(256 - frac),
    };
    let cos_val = match quadrant {
        0 => sin_table(256 - frac),
        1 => -sin_table(frac),
        2 => -sin_table(256 - frac),
        _ => sin_table(frac),
    };

    (sin_val, cos_val)
}

/// Simple sine lookup: input 0..256 (quarter turn), output 0..256 (amplitude).
fn sin_table(t: i32) -> i32 {
    // Quadratic approximation: sin(t) ≈ 4t(256-t) / 256^2 * 256
    // Peak at t=128 gives 256
    let t = t.max(0).min(256);
    ((4 * t as i64 * (256 - t) as i64) / 256) as i32
}

// ── Edge collection ─────────────────────────────────────────────────

/// Collect edges for filled path rendering.
fn collect_fill_edges(cmds: &[Cmd], edges: &mut Vec<Edge>) {
    let mut cx = 0i32;
    let mut cy = 0i32;

    for &cmd in cmds {
        match cmd {
            Cmd::MoveTo(x, y) => {
                cx = x; cy = y;
            }
            Cmd::LineTo(x, y) => {
                if cy != y { // skip horizontal edges
                    let (y0, y1, x0, x1, w) = if cy < y {
                        (cy, y, cx, x, 1)
                    } else {
                        (y, cy, x, cx, -1)
                    };
                    edges.push(Edge { x0, y0, x1, y1, winding: w });
                }
                cx = x; cy = y;
            }
            Cmd::Close => {
                // Close handled implicitly — subpath start tracked by MoveTo
            }
        }
    }
}

/// Collect edges for stroked path rendering.
/// Converts each line segment into a thick rectangle (parallelogram).
fn collect_stroke_edges(cmds: &[Cmd], half_width: i32, edges: &mut Vec<Edge>) {
    let mut cx = 0i32;
    let mut cy = 0i32;
    let mut sx = 0i32;
    let mut sy = 0i32;
    let hw = half_width;

    for &cmd in cmds {
        match cmd {
            Cmd::MoveTo(x, y) => {
                cx = x; cy = y; sx = x; sy = y;
            }
            Cmd::LineTo(x, y) => {
                if cx != x || cy != y {
                    stroke_segment(cx, cy, x, y, hw, edges);
                }
                cx = x; cy = y;
            }
            Cmd::Close => {
                if cx != sx || cy != sy {
                    stroke_segment(cx, cy, sx, sy, hw, edges);
                }
                cx = sx; cy = sy;
            }
        }
    }
}

/// Convert a line segment into edges for a thick stroke.
/// Creates a rectangle perpendicular to the segment direction.
fn stroke_segment(x0: i32, y0: i32, x1: i32, y1: i32, hw: i32, edges: &mut Vec<Edge>) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = isqrt_fp(((dx as i64 * dx as i64 + dy as i64 * dy as i64) / 256) as i32);
    if len == 0 { return; }

    // Normal vector (perpendicular), scaled to half_width
    let nx = ((-dy as i64 * hw as i64) / len as i64) as i32;
    let ny = ((dx as i64 * hw as i64) / len as i64) as i32;

    // Four corners of the stroke rectangle
    let ax = x0 + nx; let ay = y0 + ny;
    let bx = x0 - nx; let by = y0 - ny;
    let ccx = x1 - nx; let ccy = y1 - ny;
    let ddx = x1 + nx; let ddy = y1 + ny;

    // Add rectangle as 4 edges (a→d, d→c, c→b, b→a)
    add_fill_edge(ax, ay, ddx, ddy, edges);
    add_fill_edge(ddx, ddy, ccx, ccy, edges);
    add_fill_edge(ccx, ccy, bx, by, edges);
    add_fill_edge(bx, by, ax, ay, edges);

    // Round caps (approximated as circles at endpoints)
    add_round_cap(x0, y0, hw, edges);
    add_round_cap(x1, y1, hw, edges);
}

fn add_fill_edge(x0: i32, y0: i32, x1: i32, y1: i32, edges: &mut Vec<Edge>) {
    if y0 == y1 { return; }
    let (ey0, ey1, ex0, ex1, w) = if y0 < y1 {
        (y0, y1, x0, x1, 1)
    } else {
        (y1, y0, x1, x0, -1)
    };
    edges.push(Edge { x0: ex0, y0: ey0, x1: ex1, y1: ey1, winding: w });
}

/// Add a round cap (circle approximated as octagon) at the given point.
fn add_round_cap(cx: i32, cy: i32, r: i32, edges: &mut Vec<Edge>) {
    // Approximate circle with 8 segments
    let r7 = r * 181 / 256; // r * cos(45°) ≈ r * 0.707
    let pts: [(i32, i32); 8] = [
        (cx + r, cy),
        (cx + r7, cy + r7),
        (cx, cy + r),
        (cx - r7, cy + r7),
        (cx - r, cy),
        (cx - r7, cy - r7),
        (cx, cy - r),
        (cx + r7, cy - r7),
    ];
    for j in 0..8 {
        let (ax, ay) = pts[j];
        let (bx, by) = pts[(j + 1) % 8];
        add_fill_edge(ax, ay, bx, by, edges);
    }
}

// ── Scanline rasterizer ─────────────────────────────────────────────

/// Rasterize edges to a coverage bitmap using the non-zero winding rule.
/// Same algorithm as libfont's TTF rasterizer.
fn rasterize_edges(edges: &[Edge], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut accum: Vec<i32> = vec![0i32; w * h];

    for edge in edges {
        let ey0 = edge.y0;
        let ey1 = edge.y1;
        if ey0 >= ey1 { continue; }

        let row_start = fp_floor(ey0).max(0) as usize;
        let row_end = fp_ceil(ey1).min(height as i32) as usize;
        if row_start >= row_end { continue; }

        let dy = ey1 - ey0;
        let dx_total = (edge.x1 - edge.x0) as i64;

        for row in row_start..row_end {
            let y_top = (row as i32 * FP_ONE).max(ey0);
            let y_bot = ((row as i32 + 1) * FP_ONE).min(ey1);
            if y_top >= y_bot { continue; }

            let row_coverage = y_bot - y_top;
            let x_at_top = edge.x0 as i64 + dx_total * (y_top - ey0) as i64 / dy as i64;
            let x_at_bot = edge.x0 as i64 + dx_total * (y_bot - ey0) as i64 / dy as i64;
            let x_avg = ((x_at_top + x_at_bot) / 2) as i32;
            let col = fp_floor(x_avg);
            let frac_x = x_avg - (col << FP);
            let full = edge.winding * row_coverage;

            if col >= 0 && (col as usize) < w {
                let idx = row * w + col as usize;
                let c0 = fp_mul(full, FP_ONE - frac_x);
                accum[idx] += c0;
                if col as usize + 1 < w {
                    accum[idx + 1] += full - c0;
                }
            } else if col < 0 && w > 0 {
                accum[row * w] += full;
            }
        }
    }

    // Convert accumulator to coverage
    let mut coverage: Vec<u8> = vec![0u8; w * h];
    for row in 0..h {
        let base = row * w;
        let mut sum: i32 = 0;
        for col in 0..w {
            sum += accum[base + col];
            let v = if sum < 0 { -sum } else { sum };
            coverage[base + col] = v.min(255) as u8;
        }
    }

    coverage
}

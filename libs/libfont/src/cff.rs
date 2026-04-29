//! Compact Font Format (CFF Type 2) outline extraction.
//!
//! This is intentionally narrow: it decodes the pieces needed by modern
//! OpenType/CFF webfonts and feeds flattened outlines into the existing font
//! rasterizer. CID-keyed CFF and exotic hint machinery are ignored for now.

use alloc::vec::Vec;

use crate::ttf::{GlyphOutline, GlyphPoint};

#[derive(Clone, Copy)]
struct Index<'a> {
    data: &'a [u8],
    count: usize,
    off_size: usize,
    offsets_base: usize,
    objects_base: usize,
}

#[derive(Default, Clone, Copy)]
struct TopDict {
    charstrings: usize,
    private_size: usize,
    private_offset: usize,
}

#[derive(Default, Clone, Copy)]
struct PrivateDict {
    subrs_offset: usize,
}

#[derive(Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

struct Builder {
    contours: Vec<Vec<Point>>,
    current: Vec<Point>,
    x: i32,
    y: i32,
}

impl Builder {
    fn new() -> Self {
        Self { contours: Vec::new(), current: Vec::new(), x: 0, y: 0 }
    }

    fn move_to(&mut self, x: i32, y: i32) {
        self.close_contour();
        self.x = x;
        self.y = y;
        self.current.push(Point { x, y });
    }

    fn line_to(&mut self, x: i32, y: i32) {
        if self.current.is_empty() {
            self.current.push(Point { x: self.x, y: self.y });
        }
        self.x = x;
        self.y = y;
        self.current.push(Point { x, y });
    }

    fn curve_to(&mut self, c1: Point, c2: Point, p: Point) {
        if self.current.is_empty() {
            self.current.push(Point { x: self.x, y: self.y });
        }
        let p0 = Point { x: self.x, y: self.y };
        flatten_cubic(p0, c1, c2, p, &mut self.current, 0);
        self.x = p.x;
        self.y = p.y;
    }

    fn close_contour(&mut self) {
        if self.current.len() >= 2 {
            self.contours.push(core::mem::take(&mut self.current));
        } else {
            self.current.clear();
        }
    }

    fn finish(mut self) -> Option<GlyphOutline> {
        self.close_contour();
        if self.contours.is_empty() {
            return None;
        }

        let mut points = Vec::new();
        let mut contour_ends = Vec::new();
        let mut x_min = i32::MAX;
        let mut y_min = i32::MAX;
        let mut x_max = i32::MIN;
        let mut y_max = i32::MIN;

        for contour in &self.contours {
            for p in contour {
                x_min = x_min.min(p.x);
                y_min = y_min.min(p.y);
                x_max = x_max.max(p.x);
                y_max = y_max.max(p.y);
                points.push(GlyphPoint {
                    x: clamp_i16(p.x),
                    y: clamp_i16(p.y),
                    on_curve: true,
                });
            }
            contour_ends.push(points.len().saturating_sub(1) as u16);
        }

        Some(GlyphOutline {
            num_contours: contour_ends.len() as i16,
            x_min: clamp_i16(x_min),
            y_min: clamp_i16(y_min),
            x_max: clamp_i16(x_max),
            y_max: clamp_i16(y_max),
            contour_ends,
            points,
        })
    }
}

pub fn glyph_outline(data: &[u8], glyph_id: u16) -> Option<GlyphOutline> {
    if data.len() < 4 {
        return None;
    }
    let hdr_size = data[2] as usize;
    if hdr_size >= data.len() {
        return None;
    }

    let name_index = parse_index(data, hdr_size)?;
    let top_index = parse_index(data, name_index.end())?;
    let string_index = parse_index(data, top_index.end())?;
    let global_subrs = parse_index(data, string_index.end())?;
    let top_obj = top_index.get(0)?;
    let top = parse_top_dict(top_obj);
    if top.charstrings == 0 {
        return None;
    }

    let charstrings = parse_index(data, top.charstrings)?;
    let charstring = charstrings.get(glyph_id as usize)?;
    let local_subrs = if top.private_size > 0 && top.private_offset > 0 {
        let private_end = top.private_offset.checked_add(top.private_size)?;
        let private_data = data.get(top.private_offset..private_end)?;
        let private = parse_private_dict(private_data);
        if private.subrs_offset > 0 {
            parse_index(data, top.private_offset.checked_add(private.subrs_offset)?) 
        } else {
            None
        }
    } else {
        None
    };

    let mut builder = Builder::new();
    let mut stack = Vec::new();
    run_charstring(
        charstring,
        local_subrs.as_ref(),
        Some(&global_subrs),
        &mut builder,
        &mut stack,
        0,
    )?;
    builder.finish()
}

fn parse_index<'a>(data: &'a [u8], off: usize) -> Option<Index<'a>> {
    if off + 2 > data.len() {
        return None;
    }
    let count = read_u16(data, off) as usize;
    if count == 0 {
        return Some(Index { data, count: 0, off_size: 0, offsets_base: off + 2, objects_base: off + 2 });
    }
    let off_size = *data.get(off + 2)? as usize;
    if !(1..=4).contains(&off_size) {
        return None;
    }
    let offsets_base = off + 3;
    let objects_base = offsets_base.checked_add((count + 1).checked_mul(off_size)?)?;
    if objects_base > data.len() {
        return None;
    }
    let last = read_offset(data, offsets_base + count * off_size, off_size)?;
    if last == 0 {
        return None;
    }
    let end = objects_base.checked_add(last as usize - 1)?;
    if end > data.len() {
        return None;
    }
    Some(Index { data, count, off_size, offsets_base, objects_base })
}

impl<'a> Index<'a> {
    fn end(&self) -> usize {
        if self.count == 0 {
            return self.objects_base;
        }
        let last = read_offset(self.data, self.offsets_base + self.count * self.off_size, self.off_size).unwrap_or(1);
        self.objects_base + last as usize - 1
    }

    fn get(&self, idx: usize) -> Option<&'a [u8]> {
        if idx >= self.count {
            return None;
        }
        let start = read_offset(self.data, self.offsets_base + idx * self.off_size, self.off_size)? as usize;
        let end = read_offset(self.data, self.offsets_base + (idx + 1) * self.off_size, self.off_size)? as usize;
        if start == 0 || end < start {
            return None;
        }
        let a = self.objects_base + start - 1;
        let b = self.objects_base + end - 1;
        self.data.get(a..b)
    }
}

fn parse_top_dict(data: &[u8]) -> TopDict {
    let mut out = TopDict::default();
    let mut stack = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        if let Some(v) = read_dict_number(data, &mut i) {
            stack.push(v);
            continue;
        }
        let op = data[i];
        i += 1;
        let op2 = if op == 12 && i < data.len() {
            let b = data[i];
            i += 1;
            1200 + b as u16
        } else {
            op as u16
        };
        match op2 {
            17 => out.charstrings = stack.last().copied().unwrap_or(0).max(0) as usize,
            18 => {
                if stack.len() >= 2 {
                    out.private_size = stack[stack.len() - 2].max(0) as usize;
                    out.private_offset = stack[stack.len() - 1].max(0) as usize;
                }
            }
            _ => {}
        }
        stack.clear();
    }
    out
}

fn parse_private_dict(data: &[u8]) -> PrivateDict {
    let mut out = PrivateDict::default();
    let mut stack = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        if let Some(v) = read_dict_number(data, &mut i) {
            stack.push(v);
            continue;
        }
        let op = data[i];
        i += 1;
        let op2 = if op == 12 && i < data.len() {
            let b = data[i];
            i += 1;
            1200 + b as u16
        } else {
            op as u16
        };
        if op2 == 19 {
            out.subrs_offset = stack.last().copied().unwrap_or(0).max(0) as usize;
        }
        stack.clear();
    }
    out
}

fn run_charstring(
    data: &[u8],
    local_subrs: Option<&Index<'_>>,
    global_subrs: Option<&Index<'_>>,
    builder: &mut Builder,
    stack: &mut Vec<i32>,
    depth: u32,
) -> Option<()> {
    if depth > 16 {
        return None;
    }
    let mut i = 0usize;
    let mut stem_seen = false;
    let mut stem_count = 0usize;

    while i < data.len() {
        let b = data[i];
        if let Some(v) = read_char_number(data, &mut i) {
            stack.push(v);
            continue;
        }
        i += 1;
        match b {
            1 | 3 | 18 | 23 => {
                stem_count += stack.len() / 2;
                stem_seen = true;
                stack.clear();
            }
            4 => {
                drop_width_if_needed(stack, 1, stem_seen);
                let dy = pop_front(stack)?;
                builder.move_to(builder.x, builder.y + dy);
                stack.clear();
            }
            5 => {
                while stack.len() >= 2 {
                    let dx = pop_front(stack)?;
                    let dy = pop_front(stack)?;
                    builder.line_to(builder.x + dx, builder.y + dy);
                }
                stack.clear();
            }
            6 => {
                let mut horizontal = true;
                while !stack.is_empty() {
                    let d = pop_front(stack)?;
                    if horizontal {
                        builder.line_to(builder.x + d, builder.y);
                    } else {
                        builder.line_to(builder.x, builder.y + d);
                    }
                    horizontal = !horizontal;
                }
            }
            7 => {
                let mut vertical = true;
                while !stack.is_empty() {
                    let d = pop_front(stack)?;
                    if vertical {
                        builder.line_to(builder.x, builder.y + d);
                    } else {
                        builder.line_to(builder.x + d, builder.y);
                    }
                    vertical = !vertical;
                }
            }
            8 => {
                while stack.len() >= 6 {
                    let dx1 = pop_front(stack)?;
                    let dy1 = pop_front(stack)?;
                    let dx2 = pop_front(stack)?;
                    let dy2 = pop_front(stack)?;
                    let dx3 = pop_front(stack)?;
                    let dy3 = pop_front(stack)?;
                    let c1 = Point { x: builder.x + dx1, y: builder.y + dy1 };
                    let c2 = Point { x: c1.x + dx2, y: c1.y + dy2 };
                    let p = Point { x: c2.x + dx3, y: c2.y + dy3 };
                    builder.curve_to(c1, c2, p);
                }
                stack.clear();
            }
            10 => {
                let subr = pop_front(stack)?;
                let subrs = local_subrs?;
                let idx = biased_subr_index(subr, subrs.count)?;
                let bytes = subrs.get(idx)?;
                run_charstring(bytes, local_subrs, global_subrs, builder, stack, depth + 1)?;
            }
            11 => return Some(()),
            14 => return Some(()),
            19 | 20 => {
                if !stack.is_empty() {
                    stem_count += stack.len() / 2;
                }
                let mask_bytes = (stem_count + 7) / 8;
                i = i.checked_add(mask_bytes)?;
                if i > data.len() {
                    return None;
                }
                stem_seen = true;
                stack.clear();
            }
            21 => {
                drop_width_if_needed(stack, 2, stem_seen);
                let dx = pop_front(stack)?;
                let dy = pop_front(stack)?;
                builder.move_to(builder.x + dx, builder.y + dy);
                stack.clear();
            }
            22 => {
                drop_width_if_needed(stack, 1, stem_seen);
                let dx = pop_front(stack)?;
                builder.move_to(builder.x + dx, builder.y);
                stack.clear();
            }
            24 => {
                while stack.len() > 2 {
                    let dx1 = pop_front(stack)?;
                    let dy1 = pop_front(stack)?;
                    let dx2 = pop_front(stack)?;
                    let dy2 = pop_front(stack)?;
                    let dx3 = pop_front(stack)?;
                    let dy3 = pop_front(stack)?;
                    let c1 = Point { x: builder.x + dx1, y: builder.y + dy1 };
                    let c2 = Point { x: c1.x + dx2, y: c1.y + dy2 };
                    let p = Point { x: c2.x + dx3, y: c2.y + dy3 };
                    builder.curve_to(c1, c2, p);
                }
                if stack.len() == 2 {
                    let dx = pop_front(stack)?;
                    let dy = pop_front(stack)?;
                    builder.line_to(builder.x + dx, builder.y + dy);
                }
                stack.clear();
            }
            26 => {
                if stack.len() % 2 == 1 {
                    builder.line_to(builder.x + pop_front(stack)?, builder.y);
                }
                while stack.len() >= 4 {
                    let dy1 = pop_front(stack)?;
                    let dx2 = pop_front(stack)?;
                    let dy2 = pop_front(stack)?;
                    let dy3 = pop_front(stack)?;
                    let c1 = Point { x: builder.x, y: builder.y + dy1 };
                    let c2 = Point { x: c1.x + dx2, y: c1.y + dy2 };
                    let p = Point { x: c2.x, y: c2.y + dy3 };
                    builder.curve_to(c1, c2, p);
                }
                stack.clear();
            }
            27 => {
                if stack.len() % 2 == 1 {
                    builder.line_to(builder.x, builder.y + pop_front(stack)?);
                }
                while stack.len() >= 4 {
                    let dx1 = pop_front(stack)?;
                    let dx2 = pop_front(stack)?;
                    let dy2 = pop_front(stack)?;
                    let dx3 = pop_front(stack)?;
                    let c1 = Point { x: builder.x + dx1, y: builder.y };
                    let c2 = Point { x: c1.x + dx2, y: c1.y + dy2 };
                    let p = Point { x: c2.x + dx3, y: c2.y };
                    builder.curve_to(c1, c2, p);
                }
                stack.clear();
            }
            29 => {
                let subr = pop_front(stack)?;
                let subrs = global_subrs?;
                let idx = biased_subr_index(subr, subrs.count)?;
                let bytes = subrs.get(idx)?;
                run_charstring(bytes, local_subrs, global_subrs, builder, stack, depth + 1)?;
            }
            30 | 31 => {
                let mut horizontal_first = b == 31;
                while stack.len() >= 4 {
                    if horizontal_first {
                        let dx1 = pop_front(stack)?;
                        let dx2 = pop_front(stack)?;
                        let dy2 = pop_front(stack)?;
                        let mut dy3 = 0;
                        let dx3 = pop_front(stack)?;
                        if stack.len() == 1 {
                            dy3 = pop_front(stack)?;
                        }
                        let c1 = Point { x: builder.x + dx1, y: builder.y };
                        let c2 = Point { x: c1.x + dx2, y: c1.y + dy2 };
                        let p = Point { x: c2.x + dx3, y: c2.y + dy3 };
                        builder.curve_to(c1, c2, p);
                    } else {
                        let dy1 = pop_front(stack)?;
                        let dx2 = pop_front(stack)?;
                        let dy2 = pop_front(stack)?;
                        let mut dx3 = 0;
                        let dy3 = pop_front(stack)?;
                        if stack.len() == 1 {
                            dx3 = pop_front(stack)?;
                        }
                        let c1 = Point { x: builder.x, y: builder.y + dy1 };
                        let c2 = Point { x: c1.x + dx2, y: c1.y + dy2 };
                        let p = Point { x: c2.x + dx3, y: c2.y + dy3 };
                        builder.curve_to(c1, c2, p);
                    }
                    horizontal_first = !horizontal_first;
                }
                stack.clear();
            }
            12 => {
                if i >= data.len() {
                    return None;
                }
                let op = data[i];
                i += 1;
                if op == 34 || op == 35 || op == 36 || op == 37 {
                    run_flex(op, stack, builder)?;
                }
                stack.clear();
            }
            _ => stack.clear(),
        }
    }
    Some(())
}

fn run_flex(op: u8, stack: &mut Vec<i32>, builder: &mut Builder) -> Option<()> {
    match op {
        34 => {
            if stack.len() < 7 {
                return None;
            }
            let dx1 = pop_front(stack)?;
            let dx2 = pop_front(stack)?;
            let dy2 = pop_front(stack)?;
            let dx3 = pop_front(stack)?;
            let dx4 = pop_front(stack)?;
            let dx5 = pop_front(stack)?;
            let dx6 = pop_front(stack)?;
            let c1 = Point { x: builder.x + dx1, y: builder.y };
            let c2 = Point { x: c1.x + dx2, y: c1.y + dy2 };
            let p1 = Point { x: c2.x + dx3, y: c2.y };
            builder.curve_to(c1, c2, p1);
            let c3 = Point { x: builder.x + dx4, y: builder.y };
            let c4 = Point { x: c3.x + dx5, y: c3.y };
            let p2 = Point { x: c4.x + dx6, y: c4.y };
            builder.curve_to(c3, c4, p2);
        }
        35 => {
            if stack.len() < 13 {
                return None;
            }
            let vals = take_front(stack, 13)?;
            flex_pair(builder, &vals[..12]);
        }
        36 => {
            if stack.len() < 9 {
                return None;
            }
            let dx1 = pop_front(stack)?;
            let dy1 = pop_front(stack)?;
            let dx2 = pop_front(stack)?;
            let dy2 = pop_front(stack)?;
            let dx3 = pop_front(stack)?;
            let dx4 = pop_front(stack)?;
            let dx5 = pop_front(stack)?;
            let dy5 = pop_front(stack)?;
            let dx6 = pop_front(stack)?;
            let c1 = Point { x: builder.x + dx1, y: builder.y + dy1 };
            let c2 = Point { x: c1.x + dx2, y: c1.y + dy2 };
            let p1 = Point { x: c2.x + dx3, y: c2.y };
            builder.curve_to(c1, c2, p1);
            let c3 = Point { x: builder.x + dx4, y: builder.y };
            let c4 = Point { x: c3.x + dx5, y: c3.y + dy5 };
            let p2 = Point { x: c4.x + dx6, y: c4.y };
            builder.curve_to(c3, c4, p2);
        }
        37 => {
            if stack.len() < 11 {
                return None;
            }
            let vals = take_front(stack, 11)?;
            flex_pair(builder, &vals[..]);
        }
        _ => {}
    }
    Some(())
}

fn flex_pair(builder: &mut Builder, vals: &[i32]) {
    if vals.len() < 11 {
        return;
    }
    let c1 = Point { x: builder.x + vals[0], y: builder.y + vals[1] };
    let c2 = Point { x: c1.x + vals[2], y: c1.y + vals[3] };
    let p1 = Point { x: c2.x + vals[4], y: c2.y + vals[5] };
    builder.curve_to(c1, c2, p1);
    let c3 = Point { x: builder.x + vals[6], y: builder.y + vals[7] };
    let c4 = Point { x: c3.x + vals[8], y: c3.y + vals[9] };
    let p2 = Point { x: c4.x + vals[10], y: c4.y };
    builder.curve_to(c3, c4, p2);
}

fn flatten_cubic(p0: Point, p1: Point, p2: Point, p3: Point, out: &mut Vec<Point>, depth: u32) {
    let ux = 3 * p1.x - 2 * p0.x - p3.x;
    let uy = 3 * p1.y - 2 * p0.y - p3.y;
    let vx = 3 * p2.x - 2 * p3.x - p0.x;
    let vy = 3 * p2.y - 2 * p3.y - p0.y;
    let flat = ux.abs().max(uy.abs()).max(vx.abs()).max(vy.abs());
    if flat <= 8 || depth >= 8 {
        out.push(p3);
        return;
    }
    let p01 = mid(p0, p1);
    let p12 = mid(p1, p2);
    let p23 = mid(p2, p3);
    let p012 = mid(p01, p12);
    let p123 = mid(p12, p23);
    let p0123 = mid(p012, p123);
    flatten_cubic(p0, p01, p012, p0123, out, depth + 1);
    flatten_cubic(p0123, p123, p23, p3, out, depth + 1);
}

fn mid(a: Point, b: Point) -> Point {
    Point { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }
}

fn drop_width_if_needed(stack: &mut Vec<i32>, arg_count: usize, stem_seen: bool) {
    if !stem_seen && stack.len() > arg_count {
        stack.remove(0);
    }
}

fn biased_subr_index(v: i32, count: usize) -> Option<usize> {
    let bias = if count < 1240 { 107 } else if count < 33900 { 1131 } else { 32768 };
    let idx = v.checked_add(bias as i32)?;
    if idx < 0 || idx as usize >= count {
        return None;
    }
    Some(idx as usize)
}

fn pop_front(stack: &mut Vec<i32>) -> Option<i32> {
    if stack.is_empty() {
        None
    } else {
        Some(stack.remove(0))
    }
}

fn take_front(stack: &mut Vec<i32>, n: usize) -> Option<Vec<i32>> {
    if stack.len() < n {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(stack.remove(0));
    }
    Some(out)
}

fn read_dict_number(data: &[u8], i: &mut usize) -> Option<i32> {
    let b = *data.get(*i)?;
    match b {
        28 => {
            if *i + 2 >= data.len() {
                return None;
            }
            let v = i16::from_be_bytes([data[*i + 1], data[*i + 2]]) as i32;
            *i += 3;
            Some(v)
        }
        29 => {
            if *i + 4 >= data.len() {
                return None;
            }
            let v = i32::from_be_bytes([data[*i + 1], data[*i + 2], data[*i + 3], data[*i + 4]]);
            *i += 5;
            Some(v)
        }
        30 => {
            *i += 1;
            while *i < data.len() {
                let n = data[*i];
                *i += 1;
                if (n & 0x0F) == 0x0F || (n >> 4) == 0x0F {
                    break;
                }
            }
            Some(0)
        }
        32..=246 => {
            *i += 1;
            Some(b as i32 - 139)
        }
        247..=250 => {
            if *i + 1 >= data.len() {
                return None;
            }
            let v = ((b as i32 - 247) * 256) + data[*i + 1] as i32 + 108;
            *i += 2;
            Some(v)
        }
        251..=254 => {
            if *i + 1 >= data.len() {
                return None;
            }
            let v = -((b as i32 - 251) * 256) - data[*i + 1] as i32 - 108;
            *i += 2;
            Some(v)
        }
        _ => None,
    }
}

fn read_char_number(data: &[u8], i: &mut usize) -> Option<i32> {
    let b = *data.get(*i)?;
    match b {
        28 => {
            if *i + 2 >= data.len() {
                return None;
            }
            let v = i16::from_be_bytes([data[*i + 1], data[*i + 2]]) as i32;
            *i += 3;
            Some(v)
        }
        32..=246 => {
            *i += 1;
            Some(b as i32 - 139)
        }
        247..=250 => {
            if *i + 1 >= data.len() {
                return None;
            }
            let v = ((b as i32 - 247) * 256) + data[*i + 1] as i32 + 108;
            *i += 2;
            Some(v)
        }
        251..=254 => {
            if *i + 1 >= data.len() {
                return None;
            }
            let v = -((b as i32 - 251) * 256) - data[*i + 1] as i32 - 108;
            *i += 2;
            Some(v)
        }
        255 => {
            if *i + 4 >= data.len() {
                return None;
            }
            let raw = i32::from_be_bytes([data[*i + 1], data[*i + 2], data[*i + 3], data[*i + 4]]);
            *i += 5;
            Some(raw >> 16)
        }
        _ => None,
    }
}

fn read_offset(data: &[u8], off: usize, size: usize) -> Option<u32> {
    if off + size > data.len() {
        return None;
    }
    let mut v = 0u32;
    for b in &data[off..off + size] {
        v = (v << 8) | *b as u32;
    }
    Some(v)
}

fn read_u16(data: &[u8], off: usize) -> u16 {
    ((data[off] as u16) << 8) | data[off + 1] as u16
}

fn clamp_i16(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

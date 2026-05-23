use crate::format::fmt_u32;
use crate::types::{CpuHistory, CpuState, MetricHistory, ResourceView, MAX_CPUS};
use libanyui_client as ui;

pub const APP_BG: u32 = 0xFF242424;
pub const SIDEBAR_BG: u32 = 0xFF303030;
pub const SIDEBAR_SELECTED: u32 = 0xFF4A4A4A;
pub const CARD_BG: u32 = 0xFF3A3A3A;
pub const PLOT_BG: u32 = 0xFF34383B;
pub const BORDER: u32 = 0xFF4C4C4C;
pub const GRID: u32 = 0x554EA1FF;
pub const TEXT: u32 = 0xFFEDEDED;
pub const TEXT_DIM: u32 = 0xFFB6B6B6;
pub const TEXT_MUTED: u32 = 0xFF8E8E8E;
pub const CPU_COLOR: u32 = 0xFF4EA1FF;
pub const MEM_COLOR: u32 = 0xFFE044A7;
pub const DISK_COLOR: u32 = 0xFFFF8A00;
pub const NET_COLOR: u32 = 0xFF26A6B8;
pub const SYS_COLOR: u32 = 0xFF8BC34A;

const CARD_RADIUS: u32 = 8;
const PLOT_RADIUS: u32 = 7;
const MINI_RADIUS: u32 = 5;
const AA_SAMPLES: i32 = 4;
const AA_SCALE2: i32 = AA_SAMPLES * 2;

#[derive(Clone, Copy)]
struct CanvasTarget {
    pixels: *mut u32,
    w: i32,
    h: i32,
}

#[derive(Clone, Copy)]
struct RoundClip {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: i32,
}

pub fn fill_card(cv: &ui::Canvas, x: i32, y: i32, w: u32, h: u32) {
    fill_rounded_rect(cv, x, y, w, h, CARD_RADIUS, BORDER);
    if w > 2 && h > 2 {
        fill_rounded_rect(cv, x + 1, y + 1, w - 2, h - 2, CARD_RADIUS - 1, CARD_BG);
    }
}

fn canvas_target(cv: &ui::Canvas) -> Option<CanvasTarget> {
    let pixels = cv.get_buffer();
    let w = cv.get_stride() as i32;
    let h = cv.get_height() as i32;
    if pixels.is_null() || w <= 0 || h <= 0 {
        None
    } else {
        Some(CanvasTarget { pixels, w, h })
    }
}

fn color_alpha(color: u32) -> u32 {
    (color >> 24) & 0xFF
}

fn with_alpha(color: u32, alpha: u32) -> u32 {
    (color & 0x00FF_FFFF) | ((alpha.min(255)) << 24)
}

fn blend_argb(src: u32, alpha: u32, dst: u32) -> u32 {
    let a = alpha.min(255);
    if a == 0 {
        return dst;
    }
    if a >= 255 {
        return (0xFF << 24) | (src & 0x00FF_FFFF);
    }

    let inv = 255 - a;
    let sr = (src >> 16) & 0xFF;
    let sg = (src >> 8) & 0xFF;
    let sb = src & 0xFF;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let da = (dst >> 24) & 0xFF;

    let oa = a + (da * inv + 127) / 255;
    let r = (sr * a + dr * inv + 127) / 255;
    let g = (sg * a + dg * inv + 127) / 255;
    let b = (sb * a + db * inv + 127) / 255;
    (oa << 24) | (r << 16) | (g << 8) | b
}

fn blend_pixel(target: CanvasTarget, x: i32, y: i32, color: u32, coverage: u32) {
    if x < 0 || y < 0 || x >= target.w || y >= target.h {
        return;
    }
    let alpha = (color_alpha(color) * coverage.min(255) + 127) / 255;
    if alpha == 0 {
        return;
    }
    let idx = y as usize * target.w as usize + x as usize;
    unsafe {
        let dst = *target.pixels.add(idx);
        *target.pixels.add(idx) = blend_argb(color, alpha, dst);
    }
}

fn set_pixel(target: CanvasTarget, x: i32, y: i32, color: u32) {
    if x < 0 || y < 0 || x >= target.w || y >= target.h {
        return;
    }
    unsafe {
        *target
            .pixels
            .add(y as usize * target.w as usize + x as usize) = color;
    }
}

fn fill_rect_soft(cv: &ui::Canvas, x: i32, y: i32, w: u32, h: u32, color: u32) {
    if w == 0 || h == 0 {
        return;
    }
    if color_alpha(color) >= 255 {
        cv.fill_rect(x, y, w, h, color);
        return;
    }
    let Some(target) = canvas_target(cv) else {
        return;
    };
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w as i32).min(target.w);
    let y1 = (y + h as i32).min(target.h);
    for yy in y0..y1 {
        for xx in x0..x1 {
            blend_pixel(target, xx, yy, color, 255);
        }
    }
}

fn corner_coverage(px: i32, py: i32, cx: i32, cy: i32, r: i32) -> u32 {
    if r <= 0 {
        return 255;
    }
    let mut inside = 0u32;
    let radius = r * AA_SCALE2;
    let radius_sq = radius * radius;
    let center_x = cx * AA_SCALE2;
    let center_y = cy * AA_SCALE2;

    for sy in 0..AA_SAMPLES {
        for sx in 0..AA_SAMPLES {
            let sample_x = px * AA_SCALE2 + sx * 2 + 1;
            let sample_y = py * AA_SCALE2 + sy * 2 + 1;
            let dx = sample_x - center_x;
            let dy = sample_y - center_y;
            if dx * dx + dy * dy <= radius_sq {
                inside += 1;
            }
        }
    }

    (inside * 255 + (AA_SAMPLES * AA_SAMPLES / 2) as u32) / (AA_SAMPLES * AA_SAMPLES) as u32
}

fn fill_rounded_rect(cv: &ui::Canvas, x: i32, y: i32, w: u32, h: u32, radius: u32, color: u32) {
    if w == 0 || h == 0 {
        return;
    }
    let r = radius.min(w / 2).min(h / 2) as i32;
    if r <= 0 {
        fill_rect_soft(cv, x, y, w, h, color);
        return;
    }

    fill_rect_soft(cv, x + r, y, w.saturating_sub((r * 2) as u32), h, color);
    fill_rect_soft(cv, x, y + r, w, h.saturating_sub((r * 2) as u32), color);

    let Some(target) = canvas_target(cv) else {
        return;
    };
    let right = x + w as i32 - r;
    let bottom = y + h as i32 - r;

    for yy in 0..r {
        for xx in 0..r {
            let tl = corner_coverage(x + xx, y + yy, x + r, y + r, r);
            let tr = corner_coverage(right + xx, y + yy, right, y + r, r);
            let bl = corner_coverage(x + xx, bottom + yy, x + r, bottom, r);
            let br = corner_coverage(right + xx, bottom + yy, right, bottom, r);

            if tl >= 255 && color_alpha(color) >= 255 {
                set_pixel(target, x + xx, y + yy, color);
            } else {
                blend_pixel(target, x + xx, y + yy, color, tl);
            }
            if tr >= 255 && color_alpha(color) >= 255 {
                set_pixel(target, right + xx, y + yy, color);
            } else {
                blend_pixel(target, right + xx, y + yy, color, tr);
            }
            if bl >= 255 && color_alpha(color) >= 255 {
                set_pixel(target, x + xx, bottom + yy, color);
            } else {
                blend_pixel(target, x + xx, bottom + yy, color, bl);
            }
            if br >= 255 && color_alpha(color) >= 255 {
                set_pixel(target, right + xx, bottom + yy, color);
            } else {
                blend_pixel(target, right + xx, bottom + yy, color, br);
            }
        }
    }
}

fn fill_rounded_frame(
    cv: &ui::Canvas,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius: u32,
    border: u32,
    fill: u32,
) {
    if w < 2 || h < 2 {
        return;
    }
    fill_rounded_rect(cv, x, y, w, h, radius, border);
    fill_rounded_rect(
        cv,
        x + 1,
        y + 1,
        w - 2,
        h - 2,
        radius.saturating_sub(1),
        fill,
    );
}

fn round_clip(x: i32, y: i32, w: u32, h: u32, r: u32) -> RoundClip {
    RoundClip {
        x,
        y,
        w: w as i32,
        h: h as i32,
        r: r.min(w / 2).min(h / 2) as i32,
    }
}

fn inside_round_clip(px: i32, py: i32, clip: RoundClip) -> bool {
    if px < clip.x || py < clip.y || px >= clip.x + clip.w || py >= clip.y + clip.h {
        return false;
    }
    if clip.r <= 0 {
        return true;
    }
    if px >= clip.x + clip.r && px < clip.x + clip.w - clip.r
        || py >= clip.y + clip.r && py < clip.y + clip.h - clip.r
    {
        return true;
    }

    let cx = if px < clip.x + clip.r {
        clip.x + clip.r
    } else {
        clip.x + clip.w - clip.r
    };
    let cy = if py < clip.y + clip.r {
        clip.y + clip.r
    } else {
        clip.y + clip.h - clip.r
    };
    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy <= clip.r * clip.r
}

fn blend_pixel_clipped(
    target: CanvasTarget,
    x: i32,
    y: i32,
    color: u32,
    coverage: u32,
    clip: RoundClip,
) {
    if inside_round_clip(x, y, clip) {
        blend_pixel(target, x, y, color, coverage);
    }
}

fn draw_hline_clipped(cv: &ui::Canvas, x: i32, y: i32, w: u32, color: u32, clip: RoundClip) {
    let Some(target) = canvas_target(cv) else {
        return;
    };
    for xx in x..x + w as i32 {
        blend_pixel_clipped(target, xx, y, color, 255, clip);
    }
}

fn draw_vline_clipped(cv: &ui::Canvas, x: i32, y: i32, h: u32, color: u32, clip: RoundClip) {
    let Some(target) = canvas_target(cv) else {
        return;
    };
    for yy in y..y + h as i32 {
        blend_pixel_clipped(target, x, yy, color, 255, clip);
    }
}

fn fill_vline_clipped(target: CanvasTarget, x: i32, y0: i32, y1: i32, color: u32, clip: RoundClip) {
    if color_alpha(color) == 0 {
        return;
    }
    let top = y0.min(y1);
    let bottom = y0.max(y1);
    for yy in top..=bottom {
        blend_pixel_clipped(target, x, yy, color, 255, clip);
    }
}

fn draw_aa_line_clipped(
    target: CanvasTarget,
    mut x0: i32,
    mut y0: i32,
    mut x1: i32,
    mut y1: i32,
    color: u32,
    clip: RoundClip,
) {
    let steep = (y1 - y0).abs() > (x1 - x0).abs();
    if steep {
        core::mem::swap(&mut x0, &mut y0);
        core::mem::swap(&mut x1, &mut y1);
    }
    if x0 > x1 {
        core::mem::swap(&mut x0, &mut x1);
        core::mem::swap(&mut y0, &mut y1);
    }

    let dx = x1 - x0;
    if dx == 0 {
        if steep {
            core::mem::swap(&mut x0, &mut y0);
            core::mem::swap(&mut x1, &mut y1);
        }
        let top = y0.min(y1);
        let bottom = y0.max(y1);
        for yy in top..=bottom {
            blend_pixel_clipped(target, x0, yy, color, 255, clip);
        }
        return;
    }

    let dy = y1 - y0;
    let gradient = (dy << 16) / dx;
    let mut y_fixed = y0 << 16;

    for x in x0..=x1 {
        let yi = y_fixed >> 16;
        let frac = (y_fixed & 0xFFFF) as u32;
        let lower = frac >> 8;
        let upper = 255 - lower;

        if steep {
            blend_pixel_clipped(target, yi, x, color, upper, clip);
            blend_pixel_clipped(target, yi + 1, x, color, lower, clip);
        } else {
            blend_pixel_clipped(target, x, yi, color, upper, clip);
            blend_pixel_clipped(target, x, yi + 1, color, lower, clip);
        }
        y_fixed += gradient;
    }
}

pub fn draw_text(cv: &ui::Canvas, x: i32, y: i32, color: u32, size: u16, text: &str) {
    cv.draw_text(x, y, color, 0, size, text);
}

pub fn draw_kv(cv: &ui::Canvas, x: i32, y: i32, label: &str, value: &str) {
    draw_text(cv, x, y, TEXT_MUTED, 12, label);
    draw_text(cv, x, y + 18, TEXT, 15, value);
}

pub fn draw_history_chart(
    cv: &ui::Canvas,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    title: &str,
    value: &str,
    history: &MetricHistory,
    max_hint: u32,
    line: u32,
    fill: u32,
) {
    fill_card(cv, x, y, w, h);
    draw_text(cv, x + 14, y + 10, TEXT, 14, title);
    draw_text(cv, x + 14, y + 30, TEXT_DIM, 12, value);

    if w < 80 || h < 70 {
        return;
    }

    let px = x + 14;
    let py = y + 54;
    let pw = w.saturating_sub(28);
    let ph = h.saturating_sub(68);
    fill_rounded_frame(cv, px, py, pw, ph, PLOT_RADIUS, 0xFF525252, PLOT_BG);
    let ix = px + 1;
    let iy = py + 1;
    let iw = pw.saturating_sub(2);
    let ih = ph.saturating_sub(2);
    let clip = round_clip(ix, iy, iw, ih, PLOT_RADIUS.saturating_sub(1));

    for i in 1..4 {
        let gy = iy + (ih as i32 * i / 4);
        draw_hline_clipped(cv, ix, gy, iw, GRID, clip);
    }
    for i in 1..6 {
        let gx = ix + (iw as i32 * i / 6);
        draw_vline_clipped(cv, gx, iy, ih, GRID, clip);
    }

    let mut max_v = max_hint.max(history.max());
    if max_v == 0 {
        max_v = 1;
    }
    if max_hint == 100 {
        max_v = 100;
    }

    draw_series(cv, ix, iy, iw, ih, history, max_v, line, fill, clip);
}

pub fn draw_dual_history_chart(
    cv: &ui::Canvas,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    title: &str,
    value_a: &str,
    value_b: &str,
    a: &MetricHistory,
    b: &MetricHistory,
    color_a: u32,
    color_b: u32,
) {
    fill_card(cv, x, y, w, h);
    draw_text(cv, x + 14, y + 10, TEXT, 14, title);
    draw_text(cv, x + 14, y + 30, color_a, 12, value_a);
    draw_text(cv, x + 190, y + 30, color_b, 12, value_b);

    if w < 80 || h < 70 {
        return;
    }

    let px = x + 14;
    let py = y + 54;
    let pw = w.saturating_sub(28);
    let ph = h.saturating_sub(68);
    fill_rounded_frame(cv, px, py, pw, ph, PLOT_RADIUS, 0xFF525252, PLOT_BG);
    let ix = px + 1;
    let iy = py + 1;
    let iw = pw.saturating_sub(2);
    let ih = ph.saturating_sub(2);
    let clip = round_clip(ix, iy, iw, ih, PLOT_RADIUS.saturating_sub(1));

    for i in 1..4 {
        let gy = iy + (ih as i32 * i / 4);
        draw_hline_clipped(cv, ix, gy, iw, 0x334EA1FF, clip);
    }
    for i in 1..6 {
        let gx = ix + (iw as i32 * i / 6);
        draw_vline_clipped(cv, gx, iy, ih, 0x334EA1FF, clip);
    }

    let max_v = a.max().max(b.max()).max(1);
    draw_series(cv, ix, iy, iw, ih, a, max_v, color_a, 0x00000000, clip);
    draw_series(cv, ix, iy, iw, ih, b, max_v, color_b, 0x00000000, clip);
}

pub fn draw_mini_chart(
    cv: &ui::Canvas,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    history: &MetricHistory,
    max_hint: u32,
    color: u32,
) {
    fill_rounded_frame(cv, x, y, w, h, MINI_RADIUS, color & 0xAAFFFFFF, PLOT_BG);
    let ix = x + 1;
    let iy = y + 1;
    let iw = w.saturating_sub(2);
    let ih = h.saturating_sub(2);
    let clip = round_clip(ix, iy, iw, ih, MINI_RADIUS.saturating_sub(1));
    for i in 1..4 {
        let gy = iy + (ih as i32 * i / 4);
        draw_hline_clipped(cv, ix, gy, iw, color & 0x55FFFFFF, clip);
    }
    for i in 1..5 {
        let gx = ix + (iw as i32 * i / 5);
        draw_vline_clipped(cv, gx, iy, ih, color & 0x55FFFFFF, clip);
    }
    let max_v = if max_hint == 100 {
        100
    } else {
        max_hint.max(history.max()).max(1)
    };
    draw_series(cv, ix, iy, iw, ih, history, max_v, color, 0x00000000, clip);
}

fn draw_series(
    cv: &ui::Canvas,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    history: &MetricHistory,
    max_value: u32,
    line: u32,
    fill: u32,
    clip: RoundClip,
) {
    if history.count < 2 || w < 2 || h < 2 {
        return;
    }
    let Some(target) = canvas_target(cv) else {
        return;
    };
    let count = history.count.min(w as usize);
    let mut prev_x = x;
    let mut prev_y = y + h as i32 - 1;

    for px in 0..w {
        let age = if w > 1 {
            ((w - 1 - px) as usize * (count - 1)) / (w as usize - 1)
        } else {
            0
        };
        let value = history.get(age).min(max_value);
        let val_h = ((value as u64 * h as u64) / max_value as u64) as i32;
        let cy = (y + h as i32 - val_h).min(y + h as i32 - 1).max(y);
        let cx = x + px as i32;

        if fill != 0 && val_h > 0 {
            fill_vline_clipped(target, cx, cy, y + h as i32 - 1, fill, clip);
        }
        if px > 0 {
            draw_aa_line_clipped(target, prev_x, prev_y, cx, cy, with_alpha(line, 88), clip);
            draw_aa_line_clipped(target, prev_x, prev_y, cx, cy, line, clip);
        } else {
            blend_pixel_clipped(target, cx, cy, line, 255, clip);
        }
        prev_x = cx;
        prev_y = cy;
    }
}

pub fn draw_sidebar_item(
    cv: &ui::Canvas,
    y: i32,
    w: u32,
    view: ResourceView,
    selected: bool,
    subtitle: &str,
    value: &str,
    history: &MetricHistory,
) {
    let h = 112u32;
    let item_bg = if selected {
        SIDEBAR_SELECTED
    } else {
        SIDEBAR_BG
    };
    fill_rounded_rect(cv, 8, y, w.saturating_sub(16), h, CARD_RADIUS, item_bg);
    if selected {
        fill_rounded_rect(cv, 8, y, 5, h, CARD_RADIUS, view.accent());
    }

    let title_x = 22;
    draw_text(cv, title_x, y + 10, TEXT, 15, view.title());
    draw_text(cv, title_x, y + 32, TEXT_DIM, 11, subtitle);
    draw_text(cv, title_x, y + 50, view.accent(), 11, value);

    let chart_w = w.saturating_sub(44);
    draw_mini_chart(
        cv,
        title_x,
        y + 70,
        chart_w,
        32,
        history,
        sidebar_max_hint(view),
        view.accent(),
    );
}

fn sidebar_max_hint(view: ResourceView) -> u32 {
    match view {
        ResourceView::Cpu | ResourceView::Memory => 100,
        ResourceView::Processes => 128,
        _ => 0,
    }
}

pub fn draw_cpu_core_grid(
    cv: &ui::Canvas,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    cpu: &CpuState,
    history: &CpuHistory,
) {
    fill_card(cv, x, y, w, h);
    draw_text(cv, x + 14, y + 10, TEXT, 14, "Logical processors");

    let ncpu = (cpu.num_cpus as usize).max(1).min(MAX_CPUS);
    let cols = if ncpu <= 4 {
        2
    } else if ncpu <= 8 {
        4
    } else {
        4
    };
    let rows = (ncpu + cols - 1) / cols;
    let gap = 8i32;
    let start_y = y + 38;
    let cell_w = ((w as i32 - 28) - gap * (cols as i32 - 1)) / cols as i32;
    let cell_h = ((h as i32 - 52) - gap * (rows as i32 - 1)) / rows as i32;

    if cell_w < 32 || cell_h < 28 {
        return;
    }

    for core in 0..ncpu {
        let col = core % cols;
        let row = core / cols;
        let cx = x + 14 + col as i32 * (cell_w + gap);
        let cy = start_y + row as i32 * (cell_h + gap);
        fill_rounded_frame(
            cv,
            cx,
            cy,
            cell_w as u32,
            cell_h as u32,
            MINI_RADIUS,
            0xFF555555,
            PLOT_BG,
        );

        let mut label = [0u8; 28];
        let mut p = 0usize;
        label[p] = b'C';
        p += 1;
        let mut num = [0u8; 12];
        let s = fmt_u32(&mut num, core as u32);
        label[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        label[p] = b' ';
        p += 1;
        let s = fmt_u32(&mut num, cpu.core_pct[core]);
        label[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        label[p] = b'%';
        p += 1;
        if let Ok(text) = core::str::from_utf8(&label[..p]) {
            draw_text(cv, cx + 5, cy + 4, TEXT_DIM, 10, text);
        }

        draw_core_series(
            cv,
            cx + 4,
            cy + 18,
            (cell_w - 8) as u32,
            (cell_h - 22) as u32,
            core,
            history,
            CPU_COLOR,
            round_clip(
                cx + 4,
                cy + 18,
                (cell_w - 8) as u32,
                (cell_h - 22) as u32,
                MINI_RADIUS.saturating_sub(1),
            ),
        );
    }
}

fn draw_core_series(
    cv: &ui::Canvas,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    core: usize,
    history: &CpuHistory,
    color: u32,
    clip: RoundClip,
) {
    if history.count < 2 || w < 2 || h < 2 {
        return;
    }
    let Some(target) = canvas_target(cv) else {
        return;
    };
    let count = history.count.min(w as usize);
    let mut prev_x = x;
    let mut prev_y = y + h as i32 - 1;
    for px in 0..w {
        let age = ((w - 1 - px) as usize * (count - 1)) / (w as usize - 1);
        let pct = history.get(core, age) as u32;
        let val_h = ((pct as u64 * h as u64) / 100) as i32;
        let cy = (y + h as i32 - val_h).min(y + h as i32 - 1).max(y);
        let cx = x + px as i32;
        if px > 0 {
            draw_aa_line_clipped(target, prev_x, prev_y, cx, cy, color, clip);
        }
        prev_x = cx;
        prev_y = cy;
    }
}

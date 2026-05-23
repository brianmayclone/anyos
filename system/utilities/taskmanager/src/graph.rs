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

pub fn fill_card(cv: &ui::Canvas, x: i32, y: i32, w: u32, h: u32) {
    cv.fill_rect(x, y, w, h, CARD_BG);
    stroke_rect(cv, x, y, w, h, BORDER);
}

pub fn stroke_rect(cv: &ui::Canvas, x: i32, y: i32, w: u32, h: u32, color: u32) {
    if w == 0 || h == 0 {
        return;
    }
    cv.fill_rect(x, y, w, 1, color);
    cv.fill_rect(x, y + h as i32 - 1, w, 1, color);
    cv.fill_rect(x, y, 1, h, color);
    cv.fill_rect(x + w as i32 - 1, y, 1, h, color);
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
    cv.fill_rect(px, py, pw, ph, PLOT_BG);

    for i in 1..4 {
        let gy = py + (ph as i32 * i / 4);
        cv.fill_rect(px, gy, pw, 1, GRID);
    }
    for i in 1..6 {
        let gx = px + (pw as i32 * i / 6);
        cv.fill_rect(gx, py, 1, ph, GRID);
    }

    let mut max_v = max_hint.max(history.max());
    if max_v == 0 {
        max_v = 1;
    }
    if max_hint == 100 {
        max_v = 100;
    }

    draw_series(cv, px, py, pw, ph, history, max_v, line, fill);
    stroke_rect(cv, px, py, pw, ph, 0xFF525252);
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
    cv.fill_rect(px, py, pw, ph, PLOT_BG);

    for i in 1..4 {
        let gy = py + (ph as i32 * i / 4);
        cv.fill_rect(px, gy, pw, 1, 0x334EA1FF);
    }
    for i in 1..6 {
        let gx = px + (pw as i32 * i / 6);
        cv.fill_rect(gx, py, 1, ph, 0x334EA1FF);
    }

    let max_v = a.max().max(b.max()).max(1);
    draw_series(cv, px, py, pw, ph, a, max_v, color_a, 0x00000000);
    draw_series(cv, px, py, pw, ph, b, max_v, color_b, 0x00000000);
    stroke_rect(cv, px, py, pw, ph, 0xFF525252);
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
    cv.fill_rect(x, y, w, h, PLOT_BG);
    for i in 1..4 {
        let gy = y + (h as i32 * i / 4);
        cv.fill_rect(x, gy, w, 1, color & 0x55FFFFFF);
    }
    for i in 1..5 {
        let gx = x + (w as i32 * i / 5);
        cv.fill_rect(gx, y, 1, h, color & 0x55FFFFFF);
    }
    let max_v = if max_hint == 100 {
        100
    } else {
        max_hint.max(history.max()).max(1)
    };
    draw_series(cv, x, y, w, h, history, max_v, color, 0x00000000);
    stroke_rect(cv, x, y, w, h, color & 0xAAFFFFFF);
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
) {
    if history.count < 2 || w < 2 || h < 2 {
        return;
    }
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
        let cy = y + h as i32 - val_h;
        let cx = x + px as i32;

        if fill != 0 && val_h > 0 {
            cv.fill_rect(cx, cy, 1, val_h as u32, fill);
        }
        if px > 0 {
            cv.draw_line(prev_x, prev_y, cx, cy, line);
        } else {
            cv.fill_rect(cx, cy, 1, 1, line);
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
    cv.fill_rect(8, y, w.saturating_sub(16), h, item_bg);
    if selected {
        cv.fill_rect(8, y, 4, h, view.accent());
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
        cv.fill_rect(cx, cy, cell_w as u32, cell_h as u32, PLOT_BG);
        stroke_rect(cv, cx, cy, cell_w as u32, cell_h as u32, 0xFF555555);

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
) {
    if history.count < 2 || w < 2 || h < 2 {
        return;
    }
    let count = history.count.min(w as usize);
    let mut prev_x = x;
    let mut prev_y = y + h as i32 - 1;
    for px in 0..w {
        let age = ((w - 1 - px) as usize * (count - 1)) / (w as usize - 1);
        let pct = history.get(core, age) as u32;
        let val_h = ((pct as u64 * h as u64) / 100) as i32;
        let cy = y + h as i32 - val_h;
        let cx = x + px as i32;
        if px > 0 {
            cv.draw_line(prev_x, prev_y, cx, cy, color);
        }
        prev_x = cx;
        prev_y = cy;
    }
}

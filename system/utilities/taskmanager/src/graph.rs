use crate::format::fmt_u32;
use crate::types::CpuHistory;
use libanyui_client as ui;

pub fn draw_cpu_graph(
    cv: &ui::Canvas,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    core: usize,
    current_pct: u32,
    current_mhz: u32,
    history: &CpuHistory,
) {
    let graph_bg = 0xFF1A1A2E;
    let grid_color = 0xFF2A2A3E;
    let line_color = 0xFF00C8FF;
    let fill_color = 0xFF0D2840;

    cv.fill_rect(x, y, w, h, graph_bg);

    let mut label = [0u8; 40];
    let mut p = 0usize;
    let mut num = [0u8; 12];
    label[p] = b'C';
    p += 1;
    let s = fmt_u32(&mut num, core as u32);
    label[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    label[p..p + 2].copy_from_slice(b": ");
    p += 2;
    let s = fmt_u32(&mut num, current_pct);
    label[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    label[p] = b'%';
    p += 1;
    if current_mhz > 0 {
        label[p] = b' ';
        p += 1;
        let s = fmt_u32(&mut num, current_mhz);
        label[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        label[p..p + 3].copy_from_slice(b"MHz");
        p += 3;
    }
    if let Ok(text) = core::str::from_utf8(&label[..p]) {
        cv.draw_text(x + 4, y + 2, 0xFFE8E8F0, 0, 11, text);
    }

    let label_h = 16u32;
    // Draw graph area below label
    let gy = y + label_h as i32;
    let gh = h.saturating_sub(label_h);
    if gh < 8 {
        return;
    }

    // Grid lines at 25%, 50%, 75%
    for pct in [25u32, 50, 75] {
        let ly = gy + (gh as i32 - (pct as i32 * gh as i32 / 100));
        cv.fill_rect(x, ly, w, 1, grid_color);
    }

    // Line graph
    let sample_count = history.count;
    if sample_count < 2 {
        return;
    }
    let num_pts = (w as usize).min(sample_count);

    let mut prev_vy: i32 = -1;
    for px in 0..w {
        let age = if w > 1 {
            ((w - 1 - px) as usize * (num_pts - 1)) / (w as usize - 1).max(1)
        } else {
            0
        };
        let pct = history.get(core, age) as i32;
        let val_h = pct * gh as i32 / 100;
        let vy = gy + gh as i32 - val_h;

        if val_h > 0 {
            cv.fill_rect(x + px as i32, vy, 1, val_h as u32, fill_color);
        }

        if prev_vy >= 0 {
            let y0 = prev_vy.min(vy);
            let y1 = prev_vy.max(vy);
            let seg_h = (y1 - y0 + 1).max(1);
            cv.fill_rect(x + px as i32, y0, 1, seg_h as u32, line_color);
        } else {
            cv.fill_rect(x + px as i32, vy, 1, 1, line_color);
        }
        prev_vy = vy;
    }

    // Border
    let border = 0xFF3A3A4E;
    cv.fill_rect(x, y, w, 1, border);
    cv.fill_rect(x, y + h as i32 - 1, w, 1, border);
    cv.fill_rect(x, y, 1, h, border);
    cv.fill_rect(x + w as i32 - 1, y, 1, h, border);
}

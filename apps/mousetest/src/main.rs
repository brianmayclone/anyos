//! Mouse Test — diagnostic tool for verifying mouse input on multi-monitor.
//!
//! Layout:
//!   ┌─────────────────────────────────────────────────────┐
//!   │ Mouse Test                                  _ □ ✕ │
//!   ├─────────────────────────────────────────────────────┤
//!   │ Window: (X, Y)  Size: WxH                           │
//!   │ Desktop: (DX, DY)  Monitor: M0  Local: (LX, LY)     │
//!   ├─────────────────────────────────────────────────────┤
//!   │                                                     │
//!   │   [coordinate grid + crosshair at cursor]           │
//!   │                                                     │
//!   ├─────────────────────────────────────────────────────┤
//!   │ Last button: Left (single click)  Wheel: 5 events  │
//!   └─────────────────────────────────────────────────────┘
//!
//! On every mouse event we also dump the raw coords to serial via
//! `anyos_std::println!`, and a 1 Hz timer prints a positional snapshot
//! so a parallel `tee /tmp/anyos.log` capture preserves the full trace.

#![no_std]
#![no_main]

use anyos_std::{format, String};
use libanyui_client as ui;
use ui::Widget;

anyos_std::entry!(main);

const WIN_W: u32 = 800;
const WIN_H: u32 = 600;
const WIN_INITIAL_X: i32 = 100;
const WIN_INITIAL_Y: i32 = 100;

const COL_BG: u32 = 0xFF1E1E1E;
const COL_PANEL: u32 = 0xFF2A2A2C;
const COL_TEXT: u32 = 0xFFE6E6E6;
const COL_TEXT_DIM: u32 = 0xFF9A9A9A;
const COL_GRID: u32 = 0xFF2E2E32;
const COL_GRID_MAJOR: u32 = 0xFF44444A;
const COL_CROSSHAIR: u32 = 0xFF0A84FF;
const COL_TRAIL: u32 = 0xFF80B0FF;

struct ClickInfo {
    button: u8, // 1=L, 2=R, 4=M, combined as bitmask
    last_time_ms: u32,
    consecutive: u32,
}

struct AppState {
    canvas: ui::Canvas,
    canvas_w: u32,
    canvas_h: u32,

    /// Last cursor position in canvas-local coords.
    cur_x: i32,
    cur_y: i32,

    /// Position trail for visualization (last N samples).
    trail: anyos_std::Vec<(i32, i32)>,

    /// Click detection.
    click: ClickInfo,

    /// Wheel event counter.
    wheel_events: u32,

    /// Status labels.
    lbl_win_pos: ui::Label,
    lbl_win_size: ui::Label,
    lbl_canvas_pos: ui::Label,
    lbl_desktop_pos: ui::Label,
    lbl_monitor: ui::Label,
    lbl_monitor_local: ui::Label,
    lbl_button: ui::Label,
    lbl_wheel: ui::Label,

    /// Cached window handle so we can query its position each frame
    /// (the user might drag the window mid-test).
    win_id: u32,
}

static mut APP: Option<AppState> = None;
fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().unwrap() }
}

const TRAIL_LEN: usize = 64;

fn current_time_ms() -> u32 {
    // anyos_std::sys::uptime_ms exists; fall back to a monotonic counter
    // if it doesn't on this branch. Approximate ms granularity is fine
    // for click-debounce.
    anyos_std::sys::uptime_ms() as u32
}

fn button_label(mask: u8) -> &'static str {
    match mask {
        0 => "(none)",
        1 => "Left",
        2 => "Right",
        4 => "Middle",
        3 => "Left+Right",
        5 => "Left+Middle",
        6 => "Right+Middle",
        7 => "Left+Right+Middle",
        _ => "(unknown)",
    }
}

fn click_count_word(n: u32) -> &'static str {
    match n {
        0 | 1 => "single click",
        2 => "double click",
        3 => "triple click",
        _ => "multi click",
    }
}

fn record_click(button: u8) {
    let now = current_time_ms();
    let s = app();
    // 500 ms debounce — same window most desktops use for double-click.
    let same_button = s.click.button == button;
    let close_in_time = now.saturating_sub(s.click.last_time_ms) <= 500;
    if same_button && close_in_time {
        s.click.consecutive = s.click.consecutive.saturating_add(1);
    } else {
        s.click.consecutive = 1;
    }
    s.click.button = button;
    s.click.last_time_ms = now;
}

fn render_grid() {
    let s = app();
    let canvas = &s.canvas;
    canvas.clear(COL_PANEL);

    // Grid: minor 25 px, major 100 px.
    let cw = s.canvas_w as i32;
    let ch = s.canvas_h as i32;
    let mut x = 0i32;
    while x <= cw {
        let col = if x % 100 == 0 { COL_GRID_MAJOR } else { COL_GRID };
        canvas.fill_rect(x, 0, 1, ch as u32, col);
        x += 25;
    }
    let mut y = 0i32;
    while y <= ch {
        let col = if y % 100 == 0 { COL_GRID_MAJOR } else { COL_GRID };
        canvas.fill_rect(0, y, cw as u32, 1, col);
        y += 25;
    }

    // Coordinate labels on major lines.
    let mut x = 0i32;
    while x <= cw {
        if x > 0 && x < cw {
            let label = format!("{}", x);
            canvas.draw_text(x + 2, 2, COL_TEXT_DIM, 0, 10, &label);
        }
        x += 100;
    }
    let mut y = 0i32;
    while y <= ch {
        if y > 0 && y < ch {
            let label = format!("{}", y);
            canvas.draw_text(2, y + 2, COL_TEXT_DIM, 0, 10, &label);
        }
        y += 100;
    }

    // Trail of recent positions (oldest-faded).
    for (i, &(tx, ty)) in s.trail.iter().enumerate() {
        let radius = 2i32 + (i as i32 / 16);
        canvas.fill_rect(
            (tx - radius).max(0),
            (ty - radius).max(0),
            (radius * 2) as u32,
            (radius * 2) as u32,
            COL_TRAIL,
        );
    }

    // Crosshair at current cursor.
    if s.cur_x >= 0 && s.cur_x < cw && s.cur_y >= 0 && s.cur_y < ch {
        canvas.fill_rect(0, s.cur_y, cw as u32, 1, COL_CROSSHAIR);
        canvas.fill_rect(s.cur_x, 0, 1, ch as u32, COL_CROSSHAIR);
        // Centre marker.
        let r = 6i32;
        canvas.draw_rect(
            (s.cur_x - r).max(0),
            (s.cur_y - r).max(0),
            (r * 2) as u32,
            (r * 2) as u32,
            COL_CROSSHAIR,
            2,
        );
    }
}

/// Map a canvas-local point to virtual-desktop coordinates by combining
/// the window's frame position with the canvas's window-relative
/// position (queried live from libanyui so layout changes / drags are
/// reflected each event).
fn canvas_to_desktop(s: &AppState, cx: i32, cy: i32) -> (i32, i32) {
    let win: ui::Window = unsafe { core::mem::transmute(s.win_id) };
    let (wx, wy) = win.get_position();
    let (cax, cay) = s.canvas.get_abs_position();
    (wx + cax + cx, wy + cay + cy)
}

fn refresh_position_labels() {
    let s = app();
    let win: ui::Window = unsafe { core::mem::transmute(s.win_id) };
    let (wx, wy) = win.get_position();
    let (ww, wh) = win.get_size();

    s.lbl_win_pos.set_text(&format!("Window pos: ({}, {})", wx, wy));
    s.lbl_win_size.set_text(&format!("Window size: {}x{}", ww, wh));
    s.lbl_canvas_pos
        .set_text(&format!("Canvas-local: ({}, {})", s.cur_x, s.cur_y));

    // Desktop coords via libanyui's get_abs_position — includes the
    // window's frame offset (titlebar + borders) plus the canvas's
    // position inside the window content area, queried live so a window
    // drag or resize is picked up automatically.
    let (dx, dy) = canvas_to_desktop(s, s.cur_x, s.cur_y);
    s.lbl_desktop_pos
        .set_text(&format!("Desktop: ({}, {})", dx, dy));

    // Note: ui::Screen::at falls back to the primary when no screen
    // contains the point — useless for the "is the cursor visible
    // somewhere?" question. Do strict containment ourselves.
    let screens = ui::Screen::list();
    let containing = screens.iter().find(|scr| {
        dx >= scr.virtual_x
            && dy >= scr.virtual_y
            && dx < scr.right()
            && dy < scr.bottom()
    });
    if let Some(scr) = containing {
        let lx = dx - scr.virtual_x;
        let ly = dy - scr.virtual_y;
        let primary = if scr.primary { " primary" } else { "" };
        s.lbl_monitor.set_text(&format!(
            "Monitor: M{} ({}x{}{})",
            scr.id, scr.width, scr.height, primary
        ));
        s.lbl_monitor_local
            .set_text(&format!("Monitor-local: ({}, {})", lx, ly));
    } else {
        // Cursor sits outside every output — typical when the window
        // has been dragged so the canvas extends past a monitor edge.
        // Surface this clearly instead of silently reporting the
        // primary monitor's coords (which would be misleading).
        s.lbl_monitor
            .set_text("Monitor: (outside any output — window past screen edge?)");
        s.lbl_monitor_local
            .set_text(&format!("Monitor-local: n/a (desktop=({}, {}))", dx, dy));
    }
}

fn refresh_button_labels() {
    let s = app();
    let btn = button_label(s.click.button);
    let count = click_count_word(s.click.consecutive);
    s.lbl_button
        .set_text(&format!("Last: {} ({}, x{})", btn, count, s.click.consecutive));
    s.lbl_wheel
        .set_text(&format!("Wheel events: {}", s.wheel_events));
}

fn handle_mouse_move(cx: i32, cy: i32) {
    let s = app();
    s.cur_x = cx;
    s.cur_y = cy;
    if s.trail.len() >= TRAIL_LEN {
        s.trail.remove(0);
    }
    s.trail.push((cx, cy));
    render_grid();
    refresh_position_labels();
}

fn handle_mouse_down(cx: i32, cy: i32, button_mask: u32) {
    // Filter out libanyui's wheel-as-mouse-down synthesised events
    // (button=2 -> wheel up, button=3 -> wheel down). The proper wheel
    // event is delivered via the on_wheel handler below; logging the
    // synthesised down here would double-count and inflate the
    // consecutive-click counter.
    if button_mask == 2 || button_mask == 3 {
        return;
    }
    let s = app();
    s.cur_x = cx;
    s.cur_y = cy;
    record_click(button_mask as u8);
    refresh_button_labels();
    refresh_position_labels();

    // Per-event serial dump for offline analysis.
    let (dx, dy) = canvas_to_desktop(s, cx, cy);
    anyos_std::println!(
        "[mousetest] DOWN btn=0x{:x} canvas=({},{}) desktop=({},{}) consecutive={}",
        button_mask,
        cx,
        cy,
        dx,
        dy,
        s.click.consecutive
    );
}

fn handle_mouse_up(cx: i32, cy: i32, button_mask: u32) {
    if button_mask == 2 || button_mask == 3 {
        return;
    }
    let s = app();
    let (dx, dy) = canvas_to_desktop(s, cx, cy);
    anyos_std::println!(
        "[mousetest]   UP btn=0x{:x} canvas=({},{}) desktop=({},{})",
        button_mask,
        cx,
        cy,
        dx,
        dy
    );
}

fn handle_wheel(dz: i32) {
    let s = app();
    s.wheel_events = s.wheel_events.saturating_add(1);
    refresh_button_labels();
    let dir = if dz > 0 { "up" } else { "down" };
    let (dx, dy) = canvas_to_desktop(s, s.cur_x, s.cur_y);
    anyos_std::println!(
        "[mousetest] WHEEL {} dz={} canvas=({},{}) desktop=({},{}) total={}",
        dir,
        dz,
        s.cur_x,
        s.cur_y,
        dx,
        dy,
        s.wheel_events
    );
}

fn one_second_tick() {
    let s = app();
    let (dx, dy) = canvas_to_desktop(s, s.cur_x, s.cur_y);
    // Strict containment — Screen::at falls back to primary which would
    // log "M0" for a cursor that's actually off-screen.
    let screens = ui::Screen::list();
    let monitor: String = match screens.iter().find(|scr| {
        dx >= scr.virtual_x
            && dy >= scr.virtual_y
            && dx < scr.right()
            && dy < scr.bottom()
    }) {
        Some(scr) => format!(
            "M{} local=({},{})",
            scr.id,
            dx - scr.virtual_x,
            dy - scr.virtual_y
        ),
        None => String::from("(off-screen)"),
    };
    anyos_std::println!(
        "[mousetest] tick canvas=({},{}) desktop=({},{}) {} btn={} clicks={} wheel={}",
        s.cur_x,
        s.cur_y,
        dx,
        dy,
        monitor,
        button_label(s.click.button),
        s.click.consecutive,
        s.wheel_events
    );
}

fn main() {
    if !ui::init() {
        return;
    }

    let win = ui::Window::new("Mouse Test", WIN_INITIAL_X, WIN_INITIAL_Y, WIN_W, WIN_H);
    win.set_color(COL_BG);

    // ── Top info card ─────────────────────────────────────────
    let info = ui::View::new();
    info.set_dock(ui::DOCK_TOP);
    info.set_size(WIN_W, 96);
    info.set_color(0xFF252526);
    info.set_padding(12, 8, 12, 8);

    let lbl_win_pos = ui::Label::new("Window pos: (?, ?)");
    lbl_win_pos.set_position(0, 0);
    lbl_win_pos.set_size(380, 18);
    lbl_win_pos.set_text_color(COL_TEXT);
    info.add(&lbl_win_pos);

    let lbl_win_size = ui::Label::new("Window size: ? x ?");
    lbl_win_size.set_position(390, 0);
    lbl_win_size.set_size(380, 18);
    lbl_win_size.set_text_color(COL_TEXT);
    info.add(&lbl_win_size);

    let lbl_canvas_pos = ui::Label::new("Canvas-local: (?, ?)");
    lbl_canvas_pos.set_position(0, 22);
    lbl_canvas_pos.set_size(380, 18);
    lbl_canvas_pos.set_text_color(COL_TEXT);
    info.add(&lbl_canvas_pos);

    let lbl_desktop_pos = ui::Label::new("Desktop: (?, ?)");
    lbl_desktop_pos.set_position(390, 22);
    lbl_desktop_pos.set_size(380, 18);
    lbl_desktop_pos.set_text_color(COL_TEXT);
    info.add(&lbl_desktop_pos);

    let lbl_monitor = ui::Label::new("Monitor: ?");
    lbl_monitor.set_position(0, 44);
    lbl_monitor.set_size(380, 18);
    lbl_monitor.set_text_color(COL_TEXT);
    info.add(&lbl_monitor);

    let lbl_monitor_local = ui::Label::new("Monitor-local: (?, ?)");
    lbl_monitor_local.set_position(390, 44);
    lbl_monitor_local.set_size(380, 18);
    lbl_monitor_local.set_text_color(COL_TEXT);
    info.add(&lbl_monitor_local);

    // Hint
    let hint = ui::Label::new(
        "Move / click / scroll over the grid below. Positions are logged to serial every second.",
    );
    hint.set_position(0, 70);
    hint.set_size(WIN_W - 24, 18);
    hint.set_text_color(COL_TEXT_DIM);
    hint.set_font_size(11);
    info.add(&hint);

    win.add(&info);

    // ── Bottom status card ───────────────────────────────────
    let status = ui::View::new();
    status.set_dock(ui::DOCK_BOTTOM);
    status.set_size(WIN_W, 36);
    status.set_color(0xFF252526);
    status.set_padding(12, 8, 12, 8);

    let lbl_button = ui::Label::new("Last: (no click)");
    lbl_button.set_position(0, 0);
    lbl_button.set_size(420, 20);
    lbl_button.set_text_color(COL_TEXT);
    status.add(&lbl_button);

    let lbl_wheel = ui::Label::new("Wheel events: 0");
    lbl_wheel.set_position(430, 0);
    lbl_wheel.set_size(280, 20);
    lbl_wheel.set_text_color(COL_TEXT);
    status.add(&lbl_wheel);

    win.add(&status);

    // ── Centre canvas ────────────────────────────────────────
    let canvas_w: u32 = WIN_W - 24;
    let canvas_h: u32 = WIN_H - 96 - 36 - 24;
    let canvas = ui::Canvas::new(canvas_w, canvas_h);
    canvas.set_dock(ui::DOCK_FILL);
    canvas.set_size(canvas_w, canvas_h);
    canvas.set_margin(12, 0, 12, 0);
    canvas.set_interactive(true);
    win.add(&canvas);

    // Initialise app state.
    unsafe {
        APP = Some(AppState {
            canvas: canvas.clone(),
            canvas_w,
            canvas_h,
            cur_x: -1,
            cur_y: -1,
            trail: anyos_std::Vec::with_capacity(TRAIL_LEN),
            click: ClickInfo {
                button: 0,
                last_time_ms: 0,
                consecutive: 0,
            },
            wheel_events: 0,
            lbl_win_pos,
            lbl_win_size,
            lbl_canvas_pos,
            lbl_desktop_pos,
            lbl_monitor,
            lbl_monitor_local,
            lbl_button,
            lbl_wheel,
            win_id: win.id(),
        });
    }

    render_grid();
    refresh_position_labels();
    refresh_button_labels();

    // ── Wiring ────────────────────────────────────────────────
    canvas.on_mouse_move(|cx, cy| handle_mouse_move(cx, cy));
    canvas.on_mouse_down(|cx, cy, btn| handle_mouse_down(cx, cy, btn));
    canvas.on_mouse_up(|cx, cy, btn| handle_mouse_up(cx, cy, btn));
    canvas.on_wheel(|dz| handle_wheel(dz));

    // 1 Hz tick logging snapshot to serial.
    let _timer = ui::set_timer(1000, || one_second_tick());

    win.on_close(|_| ui::quit());
    ui::run();
}

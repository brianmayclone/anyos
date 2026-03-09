#![no_std]
#![no_main]

use anyos_std::{String, format};
use anyos_std::sys;
use libanyui_client as ui;

anyos_std::entry!(main);

// ---- Colors ----
const BG: u32 = 0xFF1E1E1E;
const FACE_BG: u32 = 0xFF2A2A2C;
const FACE_RIM: u32 = 0xFF4A4A4C;
const TICK_MAJOR: u32 = 0xFFE0E0E0;
const TICK_MINOR: u32 = 0xFF606060;
const HAND_HOUR: u32 = 0xFFE0E0E0;
const HAND_MIN: u32 = 0xFFE0E0E0;
const HAND_SEC: u32 = 0xFFFF3B30;
const CENTER_DOT: u32 = 0xFFFF3B30;
const TEXT_PRIMARY: u32 = 0xFFE0E0E0;
const TEXT_DIM: u32 = 0xFF808080;
const TZ_BG: u32 = 0xFF2A2A2C;
const BTN_BG: u32 = 0xFF3A3A3C;
const BTN_BG_ACTIVE: u32 = 0xFF0A84FF;
const BTN_TEXT: u32 = 0xFFE0E0E0;
const TIMER_RUNNING: u32 = 0xFF30D158;
const TIMER_DONE: u32 = 0xFFFF3B30;

// ---- Layout ----
const WIN_W: u32 = 300;
const WIN_H: u32 = 700;
const CLOCK_CX: i32 = 150;
const CLOCK_CY: i32 = 140;
const CLOCK_R: i32 = 110;
const CLOCK_CANVAS_H: u32 = 310;

// Timer section Y positions (relative to window)
const TIMER_Y: i32 = 318;
const TIMER_DISPLAY_Y: i32 = TIMER_Y + 20;
const TIMER_INPUT_Y: i32 = TIMER_Y + 56;
const TIMER_PRESETS_Y: i32 = TIMER_Y + 90;
const TIMER_BTNS_Y: i32 = TIMER_Y + 118;
const TIMER_END_Y: i32 = TIMER_Y + 150;

// World clocks
const WC_LABEL_Y: i32 = TIMER_END_Y + 8;

// Preset buttons
const PRESET_BTN_W: u32 = 40;
const PRESET_BTN_H: u32 = 22;
const PRESET_BTN_GAP: i32 = 6;
const PRESET_BTN_X0: i32 = 14;

// Control buttons
const CTRL_BTN_W: u32 = 130;
const CTRL_BTN_H: u32 = 28;
const CTRL_BTN_X0: i32 = 14;
const CTRL_BTN_GAP: i32 = 12;

// ---- Sin/Cos lookup table (60 positions, scaled by 10000) ----
const SIN60: [i32; 60] = [
        0,  1045,  2079,  3090,  4067,  5000,  5878,  6691,  7431,  8090,
     8660,  9135,  9511,  9781,  9945, 10000,  9945,  9781,  9511,  9135,
     8660,  8090,  7431,  6691,  5878,  5000,  4067,  3090,  2079,  1045,
        0, -1045, -2079, -3090, -4067, -5000, -5878, -6691, -7431, -8090,
    -8660, -9135, -9511, -9781, -9945,-10000, -9945, -9781, -9511, -9135,
    -8660, -8090, -7431, -6691, -5878, -5000, -4067, -3090, -2079, -1045,
];
const COS60: [i32; 60] = [
    10000,  9945,  9781,  9511,  9135,  8660,  8090,  7431,  6691,  5878,
     5000,  4067,  3090,  2079,  1045,     0, -1045, -2079, -3090, -4067,
    -5000, -5878, -6691, -7431, -8090, -8660, -9135, -9511, -9781, -9945,
   -10000, -9945, -9781, -9511, -9135, -8660, -8090, -7431, -6691, -5878,
    -5000, -4067, -3090, -2079, -1045,     0,  1045,  2079,  3090,  4067,
     5000,  5878,  6691,  7431,  8090,  8660,  9135,  9511,  9781,  9945,
];

// ---- Timezones ----
struct Timezone {
    name: &'static str,
    city: &'static str,
    offset_h: i32,
}

const TIMEZONES: [Timezone; 8] = [
    Timezone { name: "UTC",     city: "London",       offset_h: 0 },
    Timezone { name: "CET",     city: "Berlin",       offset_h: 1 },
    Timezone { name: "EET",     city: "Helsinki",     offset_h: 2 },
    Timezone { name: "MSK",     city: "Moscow",       offset_h: 3 },
    Timezone { name: "IST",     city: "Mumbai",       offset_h: 5 },
    Timezone { name: "CST",     city: "Shanghai",     offset_h: 8 },
    Timezone { name: "JST",     city: "Tokyo",        offset_h: 9 },
    Timezone { name: "EST",     city: "New York",     offset_h: -5 },
];

// ---- Timer presets (seconds) ----
struct Preset {
    label: &'static str,
    secs: u32,
}

const PRESETS: [Preset; 6] = [
    Preset { label: "1m",  secs: 60 },
    Preset { label: "3m",  secs: 180 },
    Preset { label: "5m",  secs: 300 },
    Preset { label: "10m", secs: 600 },
    Preset { label: "15m", secs: 900 },
    Preset { label: "30m", secs: 1800 },
];

// ---- App state ----
struct AppState {
    clock_canvas: ui::Canvas,
    wc_canvas: ui::Canvas,
    timer_display: ui::Label,
    timer_status: ui::Label,
    timer_input: ui::TextField,
    start_btn: ui::Button,
    preset_btns: [ui::Button; 6],
    // Timer state
    timer_remaining: u32,
    timer_set: u32,
    timer_active: bool,
    timer_notified: bool,
}

static mut APP: Option<AppState> = None;
fn app() -> &'static mut AppState { unsafe { APP.as_mut().unwrap() } }

// ---- Time helpers ----
fn get_time() -> (u32, u32, u32, u32, u32, u32) {
    let mut buf = [0u8; 8];
    sys::time(&mut buf);
    let year = buf[0] as u32 | ((buf[1] as u32) << 8);
    (year, buf[2] as u32, buf[3] as u32, buf[4] as u32, buf[5] as u32, buf[6] as u32)
}

fn apply_offset(hour: u32, min: u32, offset_h: i32) -> (u32, u32) {
    let total_min = hour as i32 * 60 + min as i32 + offset_h * 60;
    let total_min = ((total_min % 1440) + 1440) % 1440;
    ((total_min / 60) as u32, (total_min % 60) as u32)
}

fn fmt_02(val: u32) -> [u8; 2] {
    [b'0' + (val / 10 % 10) as u8, b'0' + (val % 10) as u8]
}

// ---- Timer actions ----
fn start_timer(secs: u32) {
    let a = app();
    a.timer_set = secs;
    a.timer_remaining = secs;
    a.timer_active = true;
    a.timer_notified = false;
    update_timer_ui();
}

fn toggle_timer() {
    let a = app();
    if a.timer_remaining == 0 && !a.timer_active {
        return;
    }
    if a.timer_active {
        a.timer_active = false;
    } else if a.timer_remaining > 0 {
        a.timer_active = true;
        a.timer_notified = false;
    } else {
        a.timer_remaining = a.timer_set;
        a.timer_active = true;
        a.timer_notified = false;
    }
    update_timer_ui();
}

fn reset_timer() {
    let a = app();
    a.timer_active = false;
    a.timer_remaining = 0;
    a.timer_set = 0;
    a.timer_notified = false;
    update_timer_ui();
}

fn tick_timer() {
    let a = app();
    if a.timer_active && a.timer_remaining > 0 {
        a.timer_remaining -= 1;
        if a.timer_remaining == 0 {
            a.timer_active = false;
            if !a.timer_notified {
                a.timer_notified = true;
                ui::show_notification("Timer", "Timer abgelaufen!", None, 5000);
            }
        }
        update_timer_ui();
    }
}

fn update_timer_ui() {
    let a = app();
    let remaining = a.timer_remaining;
    let mins = remaining / 60;
    let secs = remaining % 60;
    let m2 = fmt_02(mins);
    let s2 = fmt_02(secs);
    let display = format!("{}{}:{}{}", m2[0] as char, m2[1] as char, s2[0] as char, s2[1] as char);

    let color = if a.timer_active {
        TIMER_RUNNING
    } else if remaining == 0 && a.timer_set > 0 && a.timer_notified {
        TIMER_DONE
    } else {
        TEXT_PRIMARY
    };
    a.timer_display.set_text(&display);
    a.timer_display.set_text_color(color);

    let status = if a.timer_active {
        "Laeuft..."
    } else if remaining == 0 && a.timer_set > 0 && a.timer_notified {
        "Abgelaufen!"
    } else if remaining > 0 {
        "Pausiert"
    } else {
        ""
    };
    a.timer_status.set_text(status);

    let btn_label = if a.timer_active {
        "Pause"
    } else if a.timer_remaining > 0 {
        "Weiter"
    } else {
        "Start"
    };
    a.start_btn.set_text(btn_label);

    // Highlight active preset
    for i in 0..6 {
        let bg = if a.timer_set == PRESETS[i].secs && (a.timer_active || a.timer_remaining > 0) {
            BTN_BG_ACTIVE
        } else {
            BTN_BG
        };
        a.preset_btns[i].set_color(bg);
    }
}

/// Parse user input: "mm:ss" format or plain number as minutes
fn parse_timer_input(input: &str) -> Option<u32> {
    let input = input.trim();
    if input.is_empty() { return None; }

    if let Some(colon_pos) = input.find(':') {
        let mins_str = &input[..colon_pos];
        let secs_str = &input[colon_pos + 1..];
        let mins = parse_u32(mins_str)?;
        let secs = parse_u32(secs_str)?;
        if secs >= 60 { return None; }
        let total = mins * 60 + secs;
        if total == 0 { return None; }
        return Some(total);
    }

    let val = parse_u32(input)?;
    if val == 0 { return None; }
    Some(val * 60)
}

fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() { return None; }
    let mut result: u32 = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' { return None; }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(result)
}

// ---- Clock canvas rendering ----
fn render_clock() {
    let a = app();
    let canvas = &a.clock_canvas;

    canvas.clear(BG);

    let (_year, _month, _day, utc_h, utc_m, utc_s) = get_time();
    let hour = utc_h;
    let min = utc_m;
    let sec = utc_s;

    // Clock face
    canvas.fill_circle(CLOCK_CX, CLOCK_CY, CLOCK_R, FACE_BG);
    canvas.draw_circle(CLOCK_CX, CLOCK_CY, CLOCK_R, FACE_RIM);
    canvas.draw_circle(CLOCK_CX, CLOCK_CY, CLOCK_R + 1, FACE_RIM);

    // Tick marks
    for i in 0..60 {
        let inner = if i % 5 == 0 { CLOCK_R - 14 } else { CLOCK_R - 6 };
        let outer = CLOCK_R - 3;
        let color = if i % 5 == 0 { TICK_MAJOR } else { TICK_MINOR };
        let x0 = CLOCK_CX + SIN60[i] * inner / 10000;
        let y0 = CLOCK_CY - COS60[i] * inner / 10000;
        let x1 = CLOCK_CX + SIN60[i] * outer / 10000;
        let y1 = CLOCK_CY - COS60[i] * outer / 10000;
        if i % 5 == 0 {
            canvas.draw_thick_line(x0, y0, x1, y1, TICK_MAJOR, 2);
        } else {
            canvas.draw_line(x0, y0, x1, y1, color);
        }
    }

    // Hour hand
    let hour_pos = ((hour % 12) * 5 + min / 12) as usize % 60;
    let hx = CLOCK_CX + SIN60[hour_pos] * 60 / 10000;
    let hy = CLOCK_CY - COS60[hour_pos] * 60 / 10000;
    canvas.draw_thick_line(CLOCK_CX, CLOCK_CY, hx, hy, HAND_HOUR, 4);

    // Minute hand
    let min_pos = min as usize % 60;
    let mx = CLOCK_CX + SIN60[min_pos] * 85 / 10000;
    let my = CLOCK_CY - COS60[min_pos] * 85 / 10000;
    canvas.draw_thick_line(CLOCK_CX, CLOCK_CY, mx, my, HAND_MIN, 3);

    // Second hand
    let sec_pos = sec as usize % 60;
    let sx = CLOCK_CX + SIN60[sec_pos] * 95 / 10000;
    let sy = CLOCK_CY - COS60[sec_pos] * 95 / 10000;
    let stx = CLOCK_CX - SIN60[sec_pos] * 20 / 10000;
    let sty = CLOCK_CY + COS60[sec_pos] * 20 / 10000;
    canvas.draw_line(stx, sty, sx, sy, HAND_SEC);

    // Center dot
    canvas.fill_circle(CLOCK_CX, CLOCK_CY, 4, CENTER_DOT);

    // Digital time
    let h1 = fmt_02(hour);
    let m1 = fmt_02(min);
    let s1 = fmt_02(sec);
    let time_str = format!("{}{}:{}{}:{}{}", h1[0] as char, h1[1] as char,
        m1[0] as char, m1[1] as char, s1[0] as char, s1[1] as char);
    let (tw, _) = ui::measure_text(&time_str, 0, 28);
    let tx = (WIN_W as i32 - tw as i32) / 2;
    canvas.draw_text(tx, 260, TEXT_PRIMARY, 0, 28, &time_str);

    // Date
    let (_year, month, day, _, _, _) = get_time();
    let month_name = match month {
        1 => "January", 2 => "February", 3 => "March", 4 => "April",
        5 => "May", 6 => "June", 7 => "July", 8 => "August",
        9 => "September", 10 => "October", 11 => "November", 12 => "December",
        _ => "?",
    };
    let date_str = format!("{} {}", month_name, day);
    let (tw, _) = ui::measure_text(&date_str, 0, 14);
    let tx = (WIN_W as i32 - tw as i32) / 2;
    canvas.draw_text(tx, 292, TEXT_DIM, 0, 14, &date_str);
}

// ---- World clocks rendering ----
fn render_world_clocks() {
    let a = app();
    let canvas = &a.wc_canvas;
    let w = WIN_W - 24;
    let (_year, _month, _day, utc_h, utc_m, _utc_s) = get_time();

    canvas.clear(BG);

    // "WORLD CLOCKS" header
    canvas.draw_text(4, 0, TEXT_DIM, 0, 11, "WORLD CLOCKS");

    let tz_h: i32 = 38;
    let gap: i32 = 4;
    let start_y: i32 = 18;

    for (i, tz) in TIMEZONES.iter().enumerate() {
        let ty = start_y + i as i32 * (tz_h + gap);

        // Background
        canvas.fill_rect(0, ty, w, tz_h as u32, TZ_BG);

        // City name
        canvas.draw_text(8, ty + 4, TEXT_PRIMARY, 0, 13, tz.city);
        // Timezone abbreviation
        canvas.draw_text(8, ty + 20, TEXT_DIM, 0, 11, tz.name);

        // Time
        let (th, tm) = apply_offset(utc_h, utc_m, tz.offset_h);
        let h2 = fmt_02(th);
        let m2 = fmt_02(tm);
        let tz_time = format!("{}{}:{}{}", h2[0] as char, h2[1] as char, m2[0] as char, m2[1] as char);
        let (tw, _) = ui::measure_text(&tz_time, 0, 20);
        let tx = w as i32 - 8 - tw as i32;
        canvas.draw_text(tx, ty + 8, TEXT_PRIMARY, 0, 20, &tz_time);
    }
}

fn main() {
    if !ui::init() { return; }

    let win = ui::Window::new_with_flags("Clock", -1, -1, WIN_W, WIN_H,
        ui::WIN_FLAG_NOT_RESIZABLE);

    // ---- Clock canvas (analog clock + digital time + date) ----
    let clock_canvas = ui::Canvas::new(WIN_W, CLOCK_CANVAS_H);
    clock_canvas.set_position(0, 0);
    win.add(&clock_canvas);

    // ---- Divider before timer ----
    let div1 = ui::Divider::new();
    div1.set_position(16, TIMER_Y - 4);
    div1.set_size(WIN_W - 32, 1);
    win.add(&div1);

    // ---- Timer section ----
    let timer_header = ui::Label::new("TIMER");
    timer_header.set_position(16, TIMER_Y);
    timer_header.set_size(100, 16);
    timer_header.set_text_color(TEXT_DIM);
    timer_header.set_font_size(11);
    win.add(&timer_header);

    let timer_display = ui::Label::new("00:00");
    timer_display.set_position(0, TIMER_DISPLAY_Y);
    timer_display.set_size(WIN_W, 32);
    timer_display.set_text_color(TEXT_PRIMARY);
    timer_display.set_font_size(28);
    timer_display.set_text_align(ui::TEXT_ALIGN_CENTER);
    win.add(&timer_display);

    let timer_status = ui::Label::new("");
    timer_status.set_position(0, TIMER_DISPLAY_Y + 32);
    timer_status.set_size(WIN_W, 16);
    timer_status.set_text_color(TEXT_DIM);
    timer_status.set_font_size(11);
    timer_status.set_text_align(ui::TEXT_ALIGN_CENTER);
    win.add(&timer_status);

    // ---- Manual timer input ----
    let timer_input = ui::TextField::new();
    timer_input.set_position(70, TIMER_INPUT_Y);
    timer_input.set_size(160, 26);
    timer_input.set_placeholder("mm:ss oder Minuten");
    win.add(&timer_input);

    timer_input.on_submit(|_e| {
        let a = app();
        let mut buf = [0u8; 64];
        let len = a.timer_input.get_text(&mut buf);
        if len > 0 {
            let text = unsafe { core::str::from_utf8_unchecked(&buf[..len as usize]) };
            if let Some(secs) = parse_timer_input(text) {
                start_timer(secs);
                a.timer_input.set_text("");
            }
        }
    });

    // ---- Preset buttons ----
    // Create a dummy button first to fill the array, then overwrite
    let dummy = ui::Button::new("");
    let mut preset_btns = [dummy; 6];
    for i in 0..6 {
        let bx = PRESET_BTN_X0 + i as i32 * (PRESET_BTN_W as i32 + PRESET_BTN_GAP);
        let btn = ui::Button::new(PRESETS[i].label);
        btn.set_position(bx, TIMER_PRESETS_Y);
        btn.set_size(PRESET_BTN_W, PRESET_BTN_H);
        btn.set_color(BTN_BG);
        btn.set_text_color(BTN_TEXT);
        btn.set_font_size(12);
        preset_btns[i] = btn;
        win.add(&btn);

        let secs = PRESETS[i].secs;
        btn.on_click(move |_| {
            start_timer(secs);
        });
    }

    // ---- Start/Pause button ----
    let start_btn = ui::Button::new("Start");
    start_btn.set_position(CTRL_BTN_X0, TIMER_BTNS_Y);
    start_btn.set_size(CTRL_BTN_W, CTRL_BTN_H);
    start_btn.set_color(BTN_BG_ACTIVE);
    start_btn.set_text_color(BTN_TEXT);
    start_btn.set_font_size(13);
    win.add(&start_btn);

    start_btn.on_click(|_| {
        let a = app();
        if a.timer_set == 0 {
            // Try to read from input field first
            let mut buf = [0u8; 64];
            let len = a.timer_input.get_text(&mut buf);
            if len > 0 {
                let text = unsafe { core::str::from_utf8_unchecked(&buf[..len as usize]) };
                if let Some(secs) = parse_timer_input(text) {
                    start_timer(secs);
                    app().timer_input.set_text("");
                    return;
                }
            }
            start_timer(300);
        } else {
            toggle_timer();
        }
    });

    // ---- Reset button ----
    let reset_btn = ui::Button::new("Reset");
    let reset_x = CTRL_BTN_X0 + CTRL_BTN_W as i32 + CTRL_BTN_GAP;
    reset_btn.set_position(reset_x, TIMER_BTNS_Y);
    reset_btn.set_size(CTRL_BTN_W, CTRL_BTN_H);
    reset_btn.set_color(BTN_BG);
    reset_btn.set_text_color(BTN_TEXT);
    reset_btn.set_font_size(13);
    win.add(&reset_btn);

    reset_btn.on_click(|_| {
        reset_timer();
    });

    // ---- Divider before world clocks ----
    let div2 = ui::Divider::new();
    div2.set_position(16, TIMER_END_Y);
    div2.set_size(WIN_W - 32, 1);
    win.add(&div2);

    // ---- World clocks canvas ----
    let wc_canvas_h = WIN_H - WC_LABEL_Y as u32 - 4;
    let wc_canvas = ui::Canvas::new(WIN_W - 24, wc_canvas_h);
    wc_canvas.set_position(12, WC_LABEL_Y);
    win.add(&wc_canvas);

    // ---- Initialize state ----
    unsafe {
        APP = Some(AppState {
            clock_canvas,
            wc_canvas,
            timer_display,
            timer_status,
            timer_input,
            start_btn,
            preset_btns,
            timer_remaining: 0,
            timer_set: 0,
            timer_active: false,
            timer_notified: false,
        });
    }

    // Initial render
    render_clock();
    render_world_clocks();

    // ---- 1-second timer for updates ----
    ui::set_timer(1000, || {
        tick_timer();
        render_clock();
        render_world_clocks();
    });

    // ---- Close handler ----
    win.on_close(|_| {
        ui::quit();
    });

    ui::run();
}

#![no_std]
#![no_main]

use anyos_std::ui::window;
use anyos_std::sys;
use anyos_std::String;
use anyos_std::format;

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
const DIVIDER: u32 = 0xFF3A3A3C;
const TZ_BG: u32 = 0xFF2A2A2C;
const BTN_BG: u32 = 0xFF3A3A3C;
const BTN_BG_HOVER: u32 = 0xFF4A4A4C;
const BTN_BG_ACTIVE: u32 = 0xFF0A84FF;
const BTN_TEXT: u32 = 0xFFE0E0E0;
const TIMER_RUNNING: u32 = 0xFF30D158;
const TIMER_DONE: u32 = 0xFFFF3B30;

// ---- Layout ----
const WIN_W: u16 = 300;
const WIN_H: u16 = 640;
const CLOCK_CX: i32 = 150;
const CLOCK_CY: i32 = 140;
const CLOCK_R: i32 = 110;

// Timer section layout
const TIMER_Y: i16 = 324;
const TIMER_LABEL_Y: i16 = TIMER_Y;
const TIMER_DISPLAY_Y: i16 = TIMER_Y + 18;
const TIMER_PRESETS_Y: i16 = TIMER_Y + 56;
const TIMER_BTNS_Y: i16 = TIMER_Y + 86;
const TIMER_SECTION_END: i16 = TIMER_Y + 120;

// World clocks start after timer section
const WC_DIVIDER_Y: i16 = TIMER_SECTION_END + 5;
const WC_LABEL_Y: i16 = WC_DIVIDER_Y + 9;
const WC_START_Y: i16 = WC_LABEL_Y + 18;

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

// Preset button geometry
const PRESET_BTN_W: i16 = 40;
const PRESET_BTN_H: i16 = 22;
const PRESET_BTN_GAP: i16 = 6;
const PRESET_BTN_X0: i16 = 14;

// Control button geometry (Start/Pause, Reset)
const CTRL_BTN_W: i16 = 130;
const CTRL_BTN_H: i16 = 26;
const CTRL_BTN_X0: i16 = 14;
const CTRL_BTN_GAP: i16 = 12;

// ---- Timer state ----
static mut TIMER_REMAINING: u32 = 0;
static mut TIMER_SET: u32 = 0;
static mut TIMER_ACTIVE: bool = false;
static mut TIMER_NOTIFIED: bool = false;

fn timer_remaining() -> u32 { unsafe { TIMER_REMAINING } }
fn timer_set() -> u32 { unsafe { TIMER_SET } }
fn timer_active() -> bool { unsafe { TIMER_ACTIVE } }
fn timer_notified() -> bool { unsafe { TIMER_NOTIFIED } }

fn set_timer_remaining(v: u32) { unsafe { TIMER_REMAINING = v; } }
fn set_timer_set(v: u32) { unsafe { TIMER_SET = v; } }
fn set_timer_active(v: bool) { unsafe { TIMER_ACTIVE = v; } }
fn set_timer_notified(v: bool) { unsafe { TIMER_NOTIFIED = v; } }

// ---- Menu IDs ----
const MENU_ABOUT: u32 = 100;
const MENU_QUIT: u32 = 199;
const MENU_TIMER_1M: u32 = 201;
const MENU_TIMER_3M: u32 = 202;
const MENU_TIMER_5M: u32 = 203;
const MENU_TIMER_10M: u32 = 204;
const MENU_TIMER_15M: u32 = 205;
const MENU_TIMER_30M: u32 = 206;
const MENU_TIMER_STOP: u32 = 210;

// ---- Pixel drawing helpers ----
fn set_pixel(pixels: &mut [u32], stride: u32, height: u32, x: i32, y: i32, color: u32) {
    if x >= 0 && y >= 0 && (x as u32) < stride && (y as u32) < height {
        pixels[y as usize * stride as usize + x as usize] = color;
    }
}

fn draw_line(pixels: &mut [u32], stride: u32, height: u32, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx: i32 = if x0 < x1 { 1 } else { -1 };
    let sy: i32 = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut x = x0;
    let mut y = y0;
    loop {
        set_pixel(pixels, stride, height, x, y, color);
        if x == x1 && y == y1 { break; }
        let e2 = 2 * err;
        if e2 >= dy { err += dy; x += sx; }
        if e2 <= dx { err += dx; y += sy; }
    }
}

fn draw_thick_line(pixels: &mut [u32], stride: u32, height: u32, x0: i32, y0: i32, x1: i32, y1: i32, thickness: i32, color: u32) {
    for t in -thickness / 2..=(thickness + 1) / 2 {
        let dx = (x1 - x0).abs();
        let dy = (y1 - y0).abs();
        if dx >= dy {
            draw_line(pixels, stride, height, x0, y0 + t, x1, y1 + t, color);
        } else {
            draw_line(pixels, stride, height, x0 + t, y0, x1 + t, y1, color);
        }
    }
}

fn fill_circle(pixels: &mut [u32], stride: u32, height: u32, cx: i32, cy: i32, r: i32, color: u32) {
    for dy in -r..=r {
        let dx = isqrt((r * r - dy * dy) as u32) as i32;
        for x in (cx - dx)..=(cx + dx) {
            set_pixel(pixels, stride, height, x, cy + dy, color);
        }
    }
}

fn draw_circle_outline(pixels: &mut [u32], stride: u32, height: u32, cx: i32, cy: i32, r: i32, color: u32) {
    let mut x = r;
    let mut y = 0i32;
    let mut d = 1 - r;
    while x >= y {
        set_pixel(pixels, stride, height, cx + x, cy + y, color);
        set_pixel(pixels, stride, height, cx - x, cy + y, color);
        set_pixel(pixels, stride, height, cx + x, cy - y, color);
        set_pixel(pixels, stride, height, cx - x, cy - y, color);
        set_pixel(pixels, stride, height, cx + y, cy + x, color);
        set_pixel(pixels, stride, height, cx - y, cy + x, color);
        set_pixel(pixels, stride, height, cx + y, cy - x, color);
        set_pixel(pixels, stride, height, cx - y, cy - x, color);
        y += 1;
        if d <= 0 {
            d += 2 * y + 1;
        } else {
            x -= 1;
            d += 2 * (y - x) + 1;
        }
    }
}

fn isqrt(n: u32) -> u32 {
    if n == 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ---- Time helpers ----
fn get_time() -> (u32, u32, u32, u32, u32, u32, u32) {
    let mut buf = [0u8; 8];
    sys::time(&mut buf);
    let year = buf[0] as u32 | ((buf[1] as u32) << 8);
    (year, buf[2] as u32, buf[3] as u32, buf[4] as u32, buf[5] as u32, buf[6] as u32, 0)
}

fn apply_offset(hour: u32, min: u32, offset_h: i32) -> (u32, u32) {
    let total_min = hour as i32 * 60 + min as i32 + offset_h * 60;
    let total_min = ((total_min % 1440) + 1440) % 1440;
    ((total_min / 60) as u32, (total_min % 60) as u32)
}

fn fmt_02(val: u32) -> [u8; 2] {
    [b'0' + (val / 10 % 10) as u8, b'0' + (val % 10) as u8]
}

// ---- Hit testing ----
fn hit_preset_button(mx: i16, my: i16) -> Option<usize> {
    if my < TIMER_PRESETS_Y || my >= TIMER_PRESETS_Y + PRESET_BTN_H {
        return None;
    }
    for i in 0..PRESETS.len() {
        let bx = PRESET_BTN_X0 + i as i16 * (PRESET_BTN_W + PRESET_BTN_GAP);
        if mx >= bx && mx < bx + PRESET_BTN_W {
            return Some(i);
        }
    }
    None
}

fn hit_start_pause(mx: i16, my: i16) -> bool {
    mx >= CTRL_BTN_X0
        && mx < CTRL_BTN_X0 + CTRL_BTN_W
        && my >= TIMER_BTNS_Y
        && my < TIMER_BTNS_Y + CTRL_BTN_H
}

fn hit_reset(mx: i16, my: i16) -> bool {
    let rx = CTRL_BTN_X0 + CTRL_BTN_W + CTRL_BTN_GAP;
    mx >= rx
        && mx < rx + CTRL_BTN_W
        && my >= TIMER_BTNS_Y
        && my < TIMER_BTNS_Y + CTRL_BTN_H
}

// ---- Timer actions ----
fn start_timer(secs: u32) {
    set_timer_set(secs);
    set_timer_remaining(secs);
    set_timer_active(true);
    set_timer_notified(false);
}

fn toggle_timer() {
    if timer_remaining() == 0 && !timer_active() {
        // Nothing set — ignore
        return;
    }
    if timer_active() {
        // Pause
        set_timer_active(false);
    } else if timer_remaining() > 0 {
        // Resume
        set_timer_active(true);
        set_timer_notified(false);
    } else {
        // Restart from set value
        set_timer_remaining(timer_set());
        set_timer_active(true);
        set_timer_notified(false);
    }
}

fn reset_timer() {
    set_timer_active(false);
    set_timer_remaining(0);
    set_timer_set(0);
    set_timer_notified(false);
}

fn tick_timer() {
    if timer_active() && timer_remaining() > 0 {
        set_timer_remaining(timer_remaining() - 1);
        if timer_remaining() == 0 {
            set_timer_active(false);
            if !timer_notified() {
                set_timer_notified(true);
                window::show_notification("Timer", "Timer abgelaufen!", 5000);
            }
        }
    }
}

// ---- Rendering ----
fn render(win: u32) {
    let (w, h) = match window::get_size(win) {
        Some(s) => s,
        None => return,
    };
    let (pixels, stride, sh) = match window::surface_info(win) {
        Some(info) => info,
        None => return,
    };
    let pixels = unsafe { core::slice::from_raw_parts_mut(pixels, (stride * sh) as usize) };

    // Clear
    for p in pixels.iter_mut() { *p = BG; }

    let (_year, _month, _day, utc_h, utc_m, utc_s, _) = get_time();
    let hour = utc_h;
    let min = utc_m;
    let sec = utc_s;

    // ---- Analog clock ----
    fill_circle(pixels, stride, sh, CLOCK_CX, CLOCK_CY, CLOCK_R, FACE_BG);
    for r in CLOCK_R..CLOCK_R + 2 {
        draw_circle_outline(pixels, stride, sh, CLOCK_CX, CLOCK_CY, r, FACE_RIM);
    }

    // Hour markers
    for i in 0..60 {
        let inner = if i % 5 == 0 { CLOCK_R - 14 } else { CLOCK_R - 6 };
        let outer = CLOCK_R - 3;
        let color = if i % 5 == 0 { TICK_MAJOR } else { TICK_MINOR };
        let x0 = CLOCK_CX + (SIN60[i] * inner / 10000) as i32;
        let y0 = CLOCK_CY - (COS60[i] * inner / 10000) as i32;
        let x1 = CLOCK_CX + (SIN60[i] * outer / 10000) as i32;
        let y1 = CLOCK_CY - (COS60[i] * outer / 10000) as i32;
        if i % 5 == 0 {
            draw_thick_line(pixels, stride, sh, x0, y0, x1, y1, 2, color);
        } else {
            draw_line(pixels, stride, sh, x0, y0, x1, y1, color);
        }
    }

    // Hour hand
    let hour_pos = ((hour % 12) * 5 + min / 12) as usize % 60;
    let hx = CLOCK_CX + (SIN60[hour_pos] * 60 / 10000) as i32;
    let hy = CLOCK_CY - (COS60[hour_pos] * 60 / 10000) as i32;
    draw_thick_line(pixels, stride, sh, CLOCK_CX, CLOCK_CY, hx, hy, 4, HAND_HOUR);

    // Minute hand
    let min_pos = min as usize % 60;
    let mx = CLOCK_CX + (SIN60[min_pos] * 85 / 10000) as i32;
    let my = CLOCK_CY - (COS60[min_pos] * 85 / 10000) as i32;
    draw_thick_line(pixels, stride, sh, CLOCK_CX, CLOCK_CY, mx, my, 3, HAND_MIN);

    // Second hand
    let sec_pos = sec as usize % 60;
    let sx = CLOCK_CX + (SIN60[sec_pos] * 95 / 10000) as i32;
    let sy = CLOCK_CY - (COS60[sec_pos] * 95 / 10000) as i32;
    let stx = CLOCK_CX - (SIN60[sec_pos] * 20 / 10000) as i32;
    let sty = CLOCK_CY + (COS60[sec_pos] * 20 / 10000) as i32;
    draw_line(pixels, stride, sh, stx, sty, sx, sy, HAND_SEC);

    // Center dot
    fill_circle(pixels, stride, sh, CLOCK_CX, CLOCK_CY, 4, CENTER_DOT);

    // ---- Digital time ----
    let h1 = fmt_02(hour);
    let m1 = fmt_02(min);
    let s1 = fmt_02(sec);
    let mut time_str = [0u8; 8];
    time_str[0] = h1[0]; time_str[1] = h1[1];
    time_str[2] = b':';
    time_str[3] = m1[0]; time_str[4] = m1[1];
    time_str[5] = b':';
    time_str[6] = s1[0]; time_str[7] = s1[1];
    let ts = unsafe { core::str::from_utf8_unchecked(&time_str) };

    let (tw, _) = window::font_measure(0, 28, ts);
    let tx = (w as i32 - tw as i32) / 2;
    window::draw_text_ex(win, tx as i16, 260, TEXT_PRIMARY, 0, 28, ts);

    // Date
    let (_year, month, day, _, _, _, _) = get_time();
    let month_name = match month {
        1 => "January", 2 => "February", 3 => "March", 4 => "April",
        5 => "May", 6 => "June", 7 => "July", 8 => "August",
        9 => "September", 10 => "October", 11 => "November", 12 => "December",
        _ => "?",
    };
    let date_str = format!("{} {}", month_name, day);
    let (tw, _) = window::font_measure(0, 14, &date_str);
    let tx = (w as i32 - tw as i32) / 2;
    window::draw_text_ex(win, tx as i16, 292, TEXT_DIM, 0, 14, &date_str);

    // Divider before timer
    window::fill_rect(win, 16, 315, (w - 32) as u16, 1, DIVIDER);

    // ---- Timer section ----
    render_timer(win, w as u16);

    // Divider before world clocks
    window::fill_rect(win, 16, WC_DIVIDER_Y, (w - 32) as u16, 1, DIVIDER);

    // ---- World Clocks ----
    window::draw_text_ex(win, 16, WC_LABEL_Y, TEXT_DIM, 0, 11, "WORLD CLOCKS");

    let tz_h = 38i16;
    for (i, tz) in TIMEZONES.iter().enumerate() {
        let ty = WC_START_Y + i as i16 * (tz_h + 4);
        if ty + tz_h < 0 || ty > h as i16 { continue; }

        window::fill_rounded_rect(win, 12, ty, (w - 24) as u16, tz_h as u16, 6, TZ_BG);

        window::draw_text_ex(win, 20, ty + 4, TEXT_PRIMARY, 0, 13, tz.city);
        window::draw_text_ex(win, 20, ty + 20, TEXT_DIM, 0, 11, tz.name);

        let (th, tm) = apply_offset(utc_h, utc_m, tz.offset_h);
        let h2 = fmt_02(th);
        let m2 = fmt_02(tm);
        let mut tz_time = [0u8; 5];
        tz_time[0] = h2[0]; tz_time[1] = h2[1];
        tz_time[2] = b':';
        tz_time[3] = m2[0]; tz_time[4] = m2[1];
        let tts = unsafe { core::str::from_utf8_unchecked(&tz_time) };

        let (tw, _) = window::font_measure(0, 20, tts);
        let tx = w as i16 - 20 - tw as i16;
        window::draw_text_ex(win, tx, ty + 8, TEXT_PRIMARY, 0, 20, tts);
    }
}

fn render_timer(win: u32, w: u16) {
    // Header
    window::draw_text_ex(win, 16, TIMER_LABEL_Y, TEXT_DIM, 0, 11, "TIMER");

    // Countdown display
    let remaining = timer_remaining();
    let mins = remaining / 60;
    let secs = remaining % 60;
    let m2 = fmt_02(mins);
    let s2 = fmt_02(secs);
    let mut timer_str = [0u8; 5];
    timer_str[0] = m2[0]; timer_str[1] = m2[1];
    timer_str[2] = b':';
    timer_str[3] = s2[0]; timer_str[4] = s2[1];
    let ts = unsafe { core::str::from_utf8_unchecked(&timer_str) };

    let color = if timer_active() {
        TIMER_RUNNING
    } else if remaining == 0 && timer_set() > 0 && timer_notified() {
        TIMER_DONE
    } else {
        TEXT_PRIMARY
    };

    let (tw, _) = window::font_measure(0, 28, ts);
    let tx = (w as i32 - tw as i32) / 2;
    window::draw_text_ex(win, tx as i16, TIMER_DISPLAY_Y, color, 0, 28, ts);

    // Status text
    let status = if timer_active() {
        "Laeuft..."
    } else if remaining == 0 && timer_set() > 0 && timer_notified() {
        "Abgelaufen!"
    } else if remaining > 0 {
        "Pausiert"
    } else {
        ""
    };
    if !status.is_empty() {
        let (tw, _) = window::font_measure(0, 11, status);
        let tx = (w as i32 - tw as i32) / 2;
        window::draw_text_ex(win, tx as i16, TIMER_DISPLAY_Y + 30, TEXT_DIM, 0, 11, status);
    }

    // Preset buttons
    for i in 0..PRESETS.len() {
        let bx = PRESET_BTN_X0 + i as i16 * (PRESET_BTN_W + PRESET_BTN_GAP);
        let bg = if timer_set() == PRESETS[i].secs && (timer_active() || timer_remaining() > 0) {
            BTN_BG_ACTIVE
        } else {
            BTN_BG
        };
        window::fill_rounded_rect(win, bx, TIMER_PRESETS_Y, PRESET_BTN_W as u16, PRESET_BTN_H as u16, 4, bg);
        let (tw, _) = window::font_measure(0, 12, PRESETS[i].label);
        let tx = bx + (PRESET_BTN_W - tw as i16) / 2;
        window::draw_text_ex(win, tx, TIMER_PRESETS_Y + 4, BTN_TEXT, 0, 12, PRESETS[i].label);
    }

    // Start/Pause button
    let start_label = if timer_active() { "Pause" } else if timer_remaining() > 0 { "Weiter" } else { "Start" };
    let start_bg = if timer_active() { BTN_BG_HOVER } else { BTN_BG_ACTIVE };
    window::fill_rounded_rect(win, CTRL_BTN_X0, TIMER_BTNS_Y, CTRL_BTN_W as u16, CTRL_BTN_H as u16, 6, start_bg);
    let (tw, _) = window::font_measure(0, 13, start_label);
    let tx = CTRL_BTN_X0 + (CTRL_BTN_W - tw as i16) / 2;
    window::draw_text_ex(win, tx, TIMER_BTNS_Y + 5, BTN_TEXT, 0, 13, start_label);

    // Reset button
    let reset_x = CTRL_BTN_X0 + CTRL_BTN_W + CTRL_BTN_GAP;
    window::fill_rounded_rect(win, reset_x, TIMER_BTNS_Y, CTRL_BTN_W as u16, CTRL_BTN_H as u16, 6, BTN_BG);
    let (tw, _) = window::font_measure(0, 13, "Reset");
    let tx = reset_x + (CTRL_BTN_W - tw as i16) / 2;
    window::draw_text_ex(win, tx, TIMER_BTNS_Y + 5, BTN_TEXT, 0, 13, "Reset");
}

fn main() {
    let win = window::create_ex(
        "Clock", 200, 50, WIN_W, WIN_H,
        window::WIN_FLAG_NOT_RESIZABLE,
    );
    if win == u32::MAX { return; }

    let mut mb = window::MenuBarBuilder::new()
        .menu("Clock")
            .item(MENU_ABOUT, "About Clock", 0)
            .separator()
            .item(MENU_QUIT, "Quit", 0)
        .end_menu()
        .menu("Timer")
            .item(MENU_TIMER_1M,  "1 Minute", 0)
            .item(MENU_TIMER_3M,  "3 Minuten", 0)
            .item(MENU_TIMER_5M,  "5 Minuten", 0)
            .item(MENU_TIMER_10M, "10 Minuten", 0)
            .item(MENU_TIMER_15M, "15 Minuten", 0)
            .item(MENU_TIMER_30M, "30 Minuten", 0)
            .separator()
            .item(MENU_TIMER_STOP, "Timer stoppen", 0)
        .end_menu();
    window::set_menu(win, mb.build());

    let mut event = [0u32; 5];
    let mut last_sec = u32::MAX;

    loop {
        let t0 = anyos_std::sys::uptime_ms();
        while window::get_event(win, &mut event) == 1 {
            match event[0] {
                window::EVENT_WINDOW_CLOSE => { window::destroy(win); return; }
                window::EVENT_MENU_ITEM => {
                    match event[1] {
                        x if x == MENU_QUIT || x == window::APP_MENU_QUIT => {
                            window::destroy(win); return;
                        }
                        x if x == MENU_TIMER_1M  => start_timer(60),
                        x if x == MENU_TIMER_3M  => start_timer(180),
                        x if x == MENU_TIMER_5M  => start_timer(300),
                        x if x == MENU_TIMER_10M => start_timer(600),
                        x if x == MENU_TIMER_15M => start_timer(900),
                        x if x == MENU_TIMER_30M => start_timer(1800),
                        x if x == MENU_TIMER_STOP => reset_timer(),
                        _ => {}
                    }
                }
                window::EVENT_MOUSE_DOWN => {
                    let mx = event[1] as i16;
                    let my = event[2] as i16;

                    if let Some(idx) = hit_preset_button(mx, my) {
                        start_timer(PRESETS[idx].secs);
                    } else if hit_start_pause(mx, my) {
                        if timer_set() == 0 {
                            // No timer set — default to 5 minutes
                            start_timer(300);
                        } else {
                            toggle_timer();
                        }
                    } else if hit_reset(mx, my) {
                        reset_timer();
                    }
                }
                _ => {}
            }
        }

        // Redraw every second
        let mut buf = [0u8; 8];
        sys::time(&mut buf);
        let sec = buf[6] as u32;
        if sec != last_sec {
            last_sec = sec;
            tick_timer();
            render(win);
            window::present(win);
        }

        let elapsed = anyos_std::sys::uptime_ms().wrapping_sub(t0);
        if elapsed < 16 { anyos_std::process::sleep(16 - elapsed); }
    }
}

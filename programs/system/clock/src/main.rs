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

// ---- Layout ----
const WIN_W: u16 = 300;
const WIN_H: u16 = 500;
const CLOCK_CX: i32 = 150;
const CLOCK_CY: i32 = 140;
const CLOCK_R: i32 = 110;

// ---- Sin/Cos lookup table (60 positions, scaled by 10000) ----
// Position 0 = 12 o'clock, 15 = 3 o'clock, 30 = 6, 45 = 9
// sin = x component, cos = y component (negated for screen coords)
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
    offset_h: i32, // offset from UTC in hours
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
    // (year, month, day, hour, min, sec, _)
}

fn apply_offset(hour: u32, min: u32, offset_h: i32) -> (u32, u32) {
    let total_min = hour as i32 * 60 + min as i32 + offset_h * 60;
    let total_min = ((total_min % 1440) + 1440) % 1440;
    ((total_min / 60) as u32, (total_min % 60) as u32)
}

fn fmt_02(val: u32) -> [u8; 2] {
    [b'0' + (val / 10 % 10) as u8, b'0' + (val % 10) as u8]
}

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
    // QEMU RTC is UTC — we assume UTC for the main clock
    let hour = utc_h;
    let min = utc_m;
    let sec = utc_s;

    // ---- Analog clock ----
    // Face background
    fill_circle(pixels, stride, sh, CLOCK_CX, CLOCK_CY, CLOCK_R, FACE_BG);
    // Rim
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
    // Tail
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

    // Divider
    window::fill_rect(win, 16, 315, (w - 32) as u16, 1, DIVIDER);

    // ---- World Clocks ----
    window::draw_text_ex(win, 16, 324, TEXT_DIM, 0, 11, "WORLD CLOCKS");

    let tz_y_start = 342i16;
    let tz_h = 38i16;
    for (i, tz) in TIMEZONES.iter().enumerate() {
        let ty = tz_y_start + i as i16 * (tz_h + 4);
        if ty + tz_h < 0 || ty > h as i16 { continue; }

        // Background
        window::fill_rounded_rect(win, 12, ty, (w - 24) as u16, tz_h as u16, 6, TZ_BG);

        // City name
        window::draw_text_ex(win, 20, ty + 4, TEXT_PRIMARY, 0, 13, tz.city);
        // Timezone abbreviation
        window::draw_text_ex(win, 20, ty + 20, TEXT_DIM, 0, 11, tz.name);

        // Time in this timezone
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

fn main() {
    let win = window::create_ex(
        "Clock", 200, 50, WIN_W, WIN_H,
        window::WIN_FLAG_NOT_RESIZABLE,
    );
    if win == u32::MAX { return; }

    let mut mb = window::MenuBarBuilder::new()
        .menu("Clock")
            .item(100, "About Clock", 0)
            .separator()
            .item(199, "Quit", 0)
        .end_menu();
    window::set_menu(win, mb.build());

    let mut event = [0u32; 5];
    let mut last_sec = u32::MAX;

    loop {
        while window::get_event(win, &mut event) == 1 {
            match event[0] {
                window::EVENT_WINDOW_CLOSE => { window::destroy(win); return; }
                window::EVENT_MENU_ITEM if event[1] == 199 || event[1] == window::APP_MENU_QUIT => {
                    window::destroy(win); return;
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
            render(win);
            window::present(win);
        }

        anyos_std::process::sleep(16);
    }
}

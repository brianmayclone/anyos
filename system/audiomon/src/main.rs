#![no_std]
#![no_main]

anyos_std::entry!(main);

use libcompositor_client::{TrayClient, WindowHandle, EVT_STATUS_ICON_CLICK,
    EVT_MOUSE_DOWN, EVT_MOUSE_UP, EVT_MOUSE_MOVE, EVT_WINDOW_CLOSE, EVT_FOCUS_LOST};

use librender_client::Surface;

// ── Constants ────────────────────────────────────────────────────────────────

const ICON_ID: u32 = 1;
const POPUP_W: u32 = 260;
const POPUP_H_NORMAL: u32 = 130;
const POPUP_H_NO_DEVICE: u32 = 80;
const BORDERLESS: u32 = 0x01;

const PAD: i32 = 16;

// Theme colors
const COLOR_CARD_BG: u32 = 0xFF2C2C2C;
const COLOR_TEXT: u32 = 0xFFE6E6E6;
const COLOR_TEXT_DIM: u32 = 0xFF999999;
const COLOR_DIVIDER: u32 = 0xFF444444;
const COLOR_SLIDER_BG: u32 = 0xFF555555;
const COLOR_SLIDER_FG: u32 = 0xFF007AFF;
const COLOR_THUMB: u32 = 0xFFFFFFFF;
const COLOR_BTN_BG: u32 = 0xFF3C3C3C;
const COLOR_BTN_TEXT: u32 = 0xFFE6E6E6;
const COLOR_GREEN: u32 = 0xFF34C759;

const FONT_LARGE: u16 = 16;
const FONT_NORMAL: u16 = 13;

// ── Icon drawing ─────────────────────────────────────────────────────────────

const WHITE: u32 = 0xFFE6E6E6;
const DIM: u32 = 0xFF888888;

fn draw_speaker_icon(pixels: &mut [u32; 256], volume: u8, muted: bool, available: bool) {
    for p in pixels.iter_mut() { *p = 0; }
    let color = if !available { DIM } else if muted { DIM } else { WHITE };

    // Speaker body
    for y in 5..11 { for x in 3..7 { pixels[y * 16 + x] = color; } }
    // Speaker cone
    for y in 3..13 {
        let half = if y < 8 { 8 - y } else { y - 7 };
        let x_end = (7 + half).min(11);
        for x in 7..x_end { pixels[y * 16 + x] = color; }
    }

    if !available {
        for i in 0..6 {
            let x = 10 + i;
            let y1 = 3 + i;
            let y2 = 12 - i;
            if x < 16 && y1 < 16 { pixels[y1 * 16 + x] = 0xFFFF3B30; }
            if x < 16 && y2 < 16 { pixels[y2 * 16 + x] = 0xFFFF3B30; }
        }
    } else if muted {
        for i in 0..4 {
            let x = 11 + i;
            let y1 = 5 + i;
            let y2 = 10 - i;
            if x < 16 { pixels[y1 * 16 + x] = color; }
            if x < 16 { pixels[y2 * 16 + x] = color; }
        }
    } else {
        if volume > 0 { for y in 6..10 { pixels[y * 16 + 12] = color; } }
        if volume > 33 { for y in 4..12 { pixels[y * 16 + 13] = color; } }
        if volume > 66 { for y in 3..13 { pixels[y * 16 + 14] = color; } }
    }
}

// ── Popup state ──────────────────────────────────────────────────────────────

struct PopupState {
    // Slider
    slider_x: i32,
    slider_y: i32,
    slider_w: u32,
    slider_value: u32,  // 0..100
    slider_dragging: bool,
    // Mute button
    btn_x: i32,
    btn_y: i32,
    btn_w: u32,
    btn_h: u32,
    // State
    muted: bool,
    saved_volume: u8,
}

impl PopupState {
    fn new(volume: u8, muted: bool) -> Self {
        let slider_y = PAD + 30 + 8;
        let btn_w = 60u32;
        let slider_w = POPUP_W - PAD as u32 * 2 - btn_w - 8;
        PopupState {
            slider_x: PAD,
            slider_y,
            slider_w,
            slider_value: if muted { 0 } else { volume as u32 },
            slider_dragging: false,
            btn_x: PAD + slider_w as i32 + 8,
            btn_y: slider_y,
            btn_w,
            btn_h: 28,
            muted,
            saved_volume: volume,
        }
    }

    fn hit_slider(&self, mx: i32, my: i32) -> bool {
        mx >= self.slider_x && mx < self.slider_x + self.slider_w as i32
            && my >= self.slider_y - 4 && my < self.slider_y + 24
    }

    fn hit_button(&self, mx: i32, my: i32) -> bool {
        mx >= self.btn_x && mx < self.btn_x + self.btn_w as i32
            && my >= self.btn_y && my < self.btn_y + self.btn_h as i32
    }

    fn slider_value_from_x(&self, mx: i32) -> u32 {
        let rel = (mx - self.slider_x).max(0).min(self.slider_w as i32) as u32;
        (rel * 100 / self.slider_w).min(100)
    }
}

// ── Volume formatting ────────────────────────────────────────────────────────

fn fmt_volume<'a>(buf: &'a mut [u8; 8], vol: u32) -> &'a str {
    let mut pos = 0;
    if vol >= 100 {
        buf[pos] = b'1'; pos += 1; buf[pos] = b'0'; pos += 1; buf[pos] = b'0'; pos += 1;
    } else if vol >= 10 {
        buf[pos] = b'0' + (vol / 10) as u8; pos += 1;
        buf[pos] = b'0' + (vol % 10) as u8; pos += 1;
    } else {
        buf[pos] = b'0' + vol as u8; pos += 1;
    }
    buf[pos] = b'%'; pos += 1;
    core::str::from_utf8(&buf[..pos]).unwrap_or("?")
}

// ── Popup drawing ────────────────────────────────────────────────────────────

fn draw_popup(win: &WindowHandle, state: &mut PopupState, available: bool) {
    let popup_h = if available { POPUP_H_NORMAL } else { POPUP_H_NO_DEVICE };
    let buf = win.surface();
    let mut surface = unsafe { Surface::from_raw(buf, POPUP_W, popup_h) };

    surface.fill(0x00000000);
    surface.fill_rounded_rect_aa(0, 0, POPUP_W, popup_h, 8, COLOR_CARD_BG);

    if !available {
        libfont_client::draw_string_buf(buf, POPUP_W, popup_h, PAD, PAD + 4, COLOR_TEXT, 0, FONT_LARGE, "Sound");
        surface.fill_rect(PAD, PAD + 30, POPUP_W - PAD as u32 * 2, 1, COLOR_DIVIDER);
        libfont_client::draw_string_buf(buf, POPUP_W, popup_h, PAD, PAD + 38, COLOR_TEXT_DIM, 0, FONT_NORMAL, "No Audio Device Found");
        return;
    }

    // Title row: "Sound" + volume percentage
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, PAD, PAD + 4, COLOR_TEXT, 0, FONT_LARGE, "Sound");
    let vol_display = if state.muted { 0 } else { state.slider_value };
    let mut vbuf = [0u8; 8];
    let vol_str = fmt_volume(&mut vbuf, vol_display);
    let (tw, _) = libfont_client::measure(0, FONT_NORMAL, vol_str);
    libfont_client::draw_string_buf(
        buf, POPUP_W, popup_h,
        POPUP_W as i32 - PAD - tw as i32,
        PAD + 6, COLOR_TEXT_DIM, 0, FONT_NORMAL, vol_str,
    );

    // Divider
    surface.fill_rect(PAD, PAD + 30, POPUP_W - PAD as u32 * 2, 1, COLOR_DIVIDER);

    // Slider track
    let sy = state.slider_y;
    let sx = state.slider_x;
    let sw = state.slider_w;
    let track_y = sy + 10;
    surface.fill_rounded_rect_aa(sx, track_y, sw, 4, 2, COLOR_SLIDER_BG);

    // Filled portion
    let fill_w = (state.slider_value * sw / 100).max(1);
    surface.fill_rounded_rect_aa(sx, track_y, fill_w, 4, 2, COLOR_SLIDER_FG);

    // Thumb
    let thumb_x = sx + fill_w as i32;
    surface.fill_circle_aa(thumb_x, track_y + 2, 8, COLOR_THUMB);

    // Mute button
    surface.fill_rounded_rect_aa(state.btn_x, state.btn_y, state.btn_w, state.btn_h, 4, COLOR_BTN_BG);
    let mute_label = if state.muted { "Unmute" } else { "Mute" };
    let (tw, _) = libfont_client::measure(0, FONT_NORMAL, mute_label);
    let text_x = state.btn_x + (state.btn_w as i32 - tw as i32) / 2;
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, text_x, state.btn_y + 8, COLOR_BTN_TEXT, 0, FONT_NORMAL, mute_label);

    // Playing indicator
    let playing = anyos_std::audio::audio_is_playing();
    if playing {
        let status_y = sy + 36;
        surface.fill_circle_aa(PAD + 5, status_y + 6, 4, COLOR_GREEN);
        libfont_client::draw_string_buf(buf, POPUP_W, popup_h, PAD + 16, status_y + 2, COLOR_TEXT, 0, FONT_NORMAL, "Playing");
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    anyos_std::process::sleep(500);

    if !libfont_client::init() {
        anyos_std::println!("audiomon: libfont init failed");
        return;
    }

    let client = match TrayClient::init() {
        Some(c) => c,
        None => {
            anyos_std::println!("audiomon: compositor not available");
            return;
        }
    };

    let available = anyos_std::audio::audio_is_available();
    let mut volume = if available { anyos_std::audio::audio_get_volume() } else { 0 };
    let mut muted = volume == 0;

    let mut icon_pixels = [0u32; 256];
    draw_speaker_icon(&mut icon_pixels, volume, muted, available);
    client.set_icon(ICON_ID, &icon_pixels);

    let mut popup: Option<WindowHandle> = None;
    let mut popup_state: Option<PopupState> = None;
    let mut tick_count: u32 = 0;

    loop {
        while let Some(event) = client.poll_event() {
            match event.event_type {
                EVT_STATUS_ICON_CLICK if event.arg1 == ICON_ID => {
                    if popup.is_some() {
                        if let Some(ref p) = popup { client.destroy_window(p); }
                        popup = None;
                        popup_state = None;
                    } else {
                        if available {
                            volume = anyos_std::audio::audio_get_volume();
                            muted = volume == 0;
                        }
                        open_popup(&client, &mut popup, &mut popup_state, volume, muted, available);
                    }
                }
                EVT_MOUSE_DOWN => {
                    if let Some(ref win) = popup {
                        if event.window_id == win.id {
                            if let Some(ref mut state) = popup_state {
                                if !available { continue; }
                                let mx = event.arg1 as i32;
                                let my = event.arg2 as i32;

                                if state.hit_slider(mx, my) {
                                    state.slider_dragging = true;
                                    let new_val = state.slider_value_from_x(mx);
                                    state.slider_value = new_val;
                                    if new_val > 0 {
                                        state.muted = false;
                                        volume = new_val as u8;
                                    } else {
                                        state.muted = true;
                                    }
                                    anyos_std::audio::audio_set_volume(new_val as u8);
                                    draw_popup(win, state, available);
                                    client.present(win);
                                    update_icon(&client, volume, state.muted, available, &mut icon_pixels);
                                } else if state.hit_button(mx, my) {
                                    if state.muted {
                                        state.muted = false;
                                        volume = if state.saved_volume > 0 { state.saved_volume } else { 50 };
                                        state.slider_value = volume as u32;
                                        anyos_std::audio::audio_set_volume(volume);
                                    } else {
                                        state.saved_volume = volume;
                                        state.muted = true;
                                        state.slider_value = 0;
                                        anyos_std::audio::audio_set_volume(0);
                                    }
                                    draw_popup(win, state, available);
                                    client.present(win);
                                    update_icon(&client, volume, state.muted, available, &mut icon_pixels);
                                }
                            }
                        }
                    }
                }
                EVT_MOUSE_MOVE => {
                    if let Some(ref win) = popup {
                        if event.window_id == win.id {
                            if let Some(ref mut state) = popup_state {
                                if !available || !state.slider_dragging { continue; }
                                let mx = event.arg1 as i32;
                                let new_val = state.slider_value_from_x(mx);
                                state.slider_value = new_val;
                                if new_val > 0 {
                                    state.muted = false;
                                    volume = new_val as u8;
                                } else {
                                    state.muted = true;
                                }
                                anyos_std::audio::audio_set_volume(new_val as u8);
                                draw_popup(win, state, available);
                                client.present(win);
                                update_icon(&client, volume, state.muted, available, &mut icon_pixels);
                            }
                        }
                    }
                }
                EVT_MOUSE_UP => {
                    if let Some(ref mut _win) = popup {
                        if let Some(ref mut state) = popup_state {
                            state.slider_dragging = false;
                        }
                    }
                }
                EVT_FOCUS_LOST | EVT_WINDOW_CLOSE => {
                    if let Some(ref win) = popup {
                        if event.window_id == win.id {
                            client.destroy_window(win);
                            popup = None;
                            popup_state = None;
                        }
                    }
                }
                _ => {}
            }
        }

        // Periodic icon update
        tick_count += 1;
        if tick_count >= 20 {
            tick_count = 0;
            if available {
                let new_vol = anyos_std::audio::audio_get_volume();
                let new_muted = new_vol == 0;
                if new_vol != volume || new_muted != muted {
                    volume = new_vol;
                    muted = new_muted;
                    update_icon(&client, volume, muted, available, &mut icon_pixels);
                }
            }
        }

        anyos_std::process::sleep(100);
    }
}

fn open_popup(
    client: &TrayClient,
    popup: &mut Option<WindowHandle>,
    popup_state: &mut Option<PopupState>,
    volume: u8,
    muted: bool,
    available: bool,
) {
    let (sw, _sh) = client.screen_size();
    let popup_h = if available { POPUP_H_NORMAL } else { POPUP_H_NO_DEVICE };
    let x = sw as i32 - POPUP_W as i32 - 8;
    let y = 26;
    let win = match client.create_window(x, y, POPUP_W, popup_h, BORDERLESS) {
        Some(w) => w,
        None => return,
    };
    let mut state = PopupState::new(volume, muted);
    draw_popup(&win, &mut state, available);
    client.present(&win);
    *popup = Some(win);
    *popup_state = Some(state);
}

fn update_icon(client: &TrayClient, volume: u8, muted: bool, available: bool, pixels: &mut [u32; 256]) {
    draw_speaker_icon(pixels, volume, muted, available);
    client.set_icon(ICON_ID, pixels);
}

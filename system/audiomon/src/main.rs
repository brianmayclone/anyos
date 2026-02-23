#![no_std]
#![no_main]

anyos_std::entry!(main);

use libcompositor_client::{TrayClient, WindowHandle, EVT_STATUS_ICON_CLICK,
    EVT_MOUSE_DOWN, EVT_MOUSE_UP, EVT_MOUSE_MOVE, EVT_WINDOW_CLOSE, EVT_FOCUS_LOST};

use uisys_client::{self, ButtonStyle, ButtonState, FontSize, TextAlign, UiSlider, UiButton, UiEvent};
use uisys_client::colors;

// ── Constants ────────────────────────────────────────────────────────────────

const ICON_ID: u32 = 1;
const POPUP_W: u32 = 260;
const POPUP_H_NORMAL: u32 = 130;
const POPUP_H_NO_DEVICE: u32 = 80;
const BORDERLESS: u32 = 0x01;

// Layout
const PAD: i32 = 16;

// ── WinSurface for uisys DLL surface-mode rendering ──────────────────────────

#[repr(C)]
struct WinSurface {
    pixels: *mut u32,
    width: u32,
    height: u32,
}

// ── Icon drawing (16x16 ARGB) ────────────────────────────────────────────────

const WHITE: u32 = 0xFFE6E6E6;
const DIM: u32 = 0xFF888888;

fn draw_speaker_icon(pixels: &mut [u32; 256], volume: u8, muted: bool, available: bool) {
    // Clear to transparent
    for p in pixels.iter_mut() { *p = 0; }

    let color = if !available { DIM } else if muted { DIM } else { WHITE };

    // Speaker body (rectangle 3..7 x 5..11)
    for y in 5..11 {
        for x in 3..7 {
            pixels[y * 16 + x] = color;
        }
    }
    // Speaker cone (triangle 7..11)
    for y in 3..13 {
        let half = if y < 8 { 8 - y } else { y - 7 };
        let x_start = 7;
        let x_end = (7 + half).min(11);
        for x in x_start..x_end {
            pixels[y * 16 + x] = color;
        }
    }

    if !available {
        // Draw X for no device
        for i in 0..6 {
            let x = 10 + i;
            let y1 = 3 + i;
            let y2 = 12 - i;
            if x < 16 && y1 < 16 { pixels[y1 * 16 + x] = 0xFFFF3B30; }
            if x < 16 && y2 < 16 { pixels[y2 * 16 + x] = 0xFFFF3B30; }
        }
    } else if muted {
        // Draw X for mute
        for i in 0..4 {
            let x = 11 + i;
            let y1 = 5 + i;
            let y2 = 10 - i;
            if x < 16 { pixels[y1 * 16 + x] = color; }
            if x < 16 { pixels[y2 * 16 + x] = color; }
        }
    } else {
        // Sound waves based on volume
        if volume > 0 {
            // Small wave
            for y in 6..10 {
                pixels[y * 16 + 12] = color;
            }
        }
        if volume > 33 {
            // Medium wave
            for y in 4..12 {
                pixels[y * 16 + 13] = color;
            }
        }
        if volume > 66 {
            // Large wave
            for y in 3..13 {
                pixels[y * 16 + 14] = color;
            }
        }
    }
}

// ── Popup state ──────────────────────────────────────────────────────────────

struct PopupState {
    slider: UiSlider,
    btn_mute: UiButton,
    muted: bool,
    saved_volume: u8, // volume before mute
}

impl PopupState {
    fn new(volume: u8, muted: bool) -> Self {
        let slider_y = PAD + 30 + 8; // after title row + spacing
        let btn_w = 60u32;
        let slider_w = POPUP_W as u32 - PAD as u32 * 2 - btn_w - 8;
        PopupState {
            slider: UiSlider::new(PAD, slider_y, slider_w, 0, 100, if muted { 0 } else { volume as u32 }),
            btn_mute: UiButton::new(
                PAD + slider_w as i32 + 8,
                slider_y,
                btn_w,
                28,
                ButtonStyle::Default,
            ),
            muted,
            saved_volume: volume,
        }
    }
}

// ── Format helpers ───────────────────────────────────────────────────────────

fn fmt_volume<'a>(buf: &'a mut [u8; 8], vol: u32) -> &'a str {
    let mut pos = 0;
    if vol >= 100 {
        buf[pos] = b'1'; pos += 1;
        buf[pos] = b'0'; pos += 1;
        buf[pos] = b'0'; pos += 1;
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
    let surface = WinSurface {
        pixels: win.surface(),
        width: POPUP_W,
        height: popup_h,
    };
    let w = &surface as *const WinSurface as u32;

    // Background card
    uisys_client::card(w, 0, 0, POPUP_W, popup_h);

    if !available {
        // No device message
        uisys_client::label(w, PAD, PAD + 4, "Sound", colors::TEXT(), FontSize::Large, TextAlign::Left);
        uisys_client::divider_h(w, PAD, PAD + 30, POPUP_W as u32 - PAD as u32 * 2);
        uisys_client::label(
            w, PAD, PAD + 38,
            "No Audio Device Found",
            colors::TEXT_SECONDARY(),
            FontSize::Normal,
            TextAlign::Left,
        );
        return;
    }

    // Title row: "Sound" + volume percentage
    uisys_client::label(w, PAD, PAD + 4, "Sound", colors::TEXT(), FontSize::Large, TextAlign::Left);
    let vol_display = if state.muted { 0 } else { state.slider.value };
    let mut vbuf = [0u8; 8];
    let vol_str = fmt_volume(&mut vbuf, vol_display);
    uisys_client::label(
        w,
        POPUP_W as i32 - PAD - 40,
        PAD + 6,
        vol_str,
        colors::TEXT_SECONDARY(),
        FontSize::Normal,
        TextAlign::Right,
    );

    // Divider
    uisys_client::divider_h(w, PAD, PAD + 30, POPUP_W as u32 - PAD as u32 * 2);

    // Slider + Mute button row
    state.slider.render(w);
    let mute_label = if state.muted { "Unmute" } else { "Mute" };
    state.btn_mute.render(w, mute_label);

    // Playing indicator
    let playing = anyos_std::audio::audio_is_playing();
    let status_y = state.slider.y + 36;
    if playing {
        uisys_client::status_indicator(
            w, PAD, status_y,
            uisys_client::StatusKind::Online, "Playing",
        );
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    // Wait for compositor to be ready
    anyos_std::process::sleep(500);

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

    // Register tray icon
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
                        if let Some(ref p) = popup {
                            client.destroy_window(p);
                        }
                        popup = None;
                        popup_state = None;
                    } else {
                        // Re-read volume before opening popup
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
                                let ui_evt = UiEvent {
                                    event_type: uisys_client::EVENT_MOUSE_DOWN,
                                    p1: event.arg1,
                                    p2: event.arg2,
                                    p3: 0,
                                    p4: 0,
                                };

                                if let Some(new_val) = state.slider.handle_event(&ui_evt) {
                                    // Slider changed
                                    if new_val > 0 {
                                        state.muted = false;
                                        volume = new_val as u8;
                                        anyos_std::audio::audio_set_volume(volume);
                                    } else {
                                        state.muted = true;
                                        anyos_std::audio::audio_set_volume(0);
                                    }
                                    draw_popup(win, state, available);
                                    client.present(win);
                                    update_icon(&client, volume, state.muted, available, &mut icon_pixels);
                                } else if state.btn_mute.handle_event(&ui_evt) {
                                    // Mute toggle
                                    if state.muted {
                                        // Unmute: restore saved volume
                                        state.muted = false;
                                        volume = if state.saved_volume > 0 { state.saved_volume } else { 50 };
                                        state.slider.value = volume as u32;
                                        anyos_std::audio::audio_set_volume(volume);
                                    } else {
                                        // Mute: save current volume
                                        state.saved_volume = volume;
                                        state.muted = true;
                                        state.slider.value = 0;
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
                                if !available { continue; }
                                let ui_evt = UiEvent {
                                    event_type: uisys_client::EVENT_MOUSE_MOVE,
                                    p1: event.arg1,
                                    p2: event.arg2,
                                    p3: 0,
                                    p4: 0,
                                };
                                if let Some(new_val) = state.slider.handle_event(&ui_evt) {
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
                }
                EVT_MOUSE_UP => {
                    if let Some(ref win) = popup {
                        if event.window_id == win.id {
                            if let Some(ref mut state) = popup_state {
                                let ui_evt = UiEvent {
                                    event_type: uisys_client::EVENT_MOUSE_UP,
                                    p1: event.arg1,
                                    p2: event.arg2,
                                    p3: 0,
                                    p4: 0,
                                };
                                state.slider.handle_event(&ui_evt);
                                state.btn_mute.handle_event(&ui_evt);
                            }
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

        // Periodic icon update (every ~2 seconds)
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
    // Position near top-right (below menubar)
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

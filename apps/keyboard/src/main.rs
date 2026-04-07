// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! On-Screen Keyboard — visual keyboard viewer and input tool.
//!
//! Shows the current keyboard layout with labeled keys. Pressed keys
//! are highlighted in real-time. Layout can be switched via a selector.
//! Inspired by Ubuntu Onboard.

#![no_std]
#![no_main]

use anyos_std::{format, String, Vec};
use anyos_std::kbd;
use libanyui_client as ui;
use ui::{Widget, DOCK_TOP, DOCK_BOTTOM, DOCK_FILL};

anyos_std::entry!(main);

fn char_from_u32(v: u32) -> char {
    if let Some(c) = core::char::from_u32(v) { c } else { '.' }
}

/// Map a compositor KEY_* code back to a PS/2 scancode for the on-screen keyboard.
/// The compositor's encode_scancode() translates raw PS/2 scancodes into KEY_* constants
/// (e.g. 0x01→0x103 for Escape). This function reverses that mapping.
fn key_code_to_scancode(kc: u32) -> u32 {
    match kc {
        0x100 => 0x1C, // KEY_ENTER
        0x101 => 0x0E, // KEY_BACKSPACE
        0x102 => 0x0F, // KEY_TAB
        0x103 => 0x01, // KEY_ESCAPE
        0x104 => 0x39, // KEY_SPACE
        0x105 => 0x48, // KEY_UP
        0x106 => 0x50, // KEY_DOWN
        0x107 => 0x4B, // KEY_LEFT
        0x108 => 0x4D, // KEY_RIGHT
        0x120 => 0x53, // KEY_DELETE
        0x121 => 0x47, // KEY_HOME
        0x122 => 0x4F, // KEY_END
        0x123 => 0x49, // KEY_PAGE_UP
        0x124 => 0x51, // KEY_PAGE_DOWN
        0x140 => 0x3B, // KEY_F1
        0x141 => 0x3C, // KEY_F2
        0x142 => 0x3D, // KEY_F3
        0x143 => 0x3E, // KEY_F4
        0x144 => 0x3F, // KEY_F5
        0x145 => 0x40, // KEY_F6
        0x146 => 0x41, // KEY_F7
        0x147 => 0x42, // KEY_F8
        0x148 => 0x43, // KEY_F9
        0x149 => 0x44, // KEY_F10
        0x14A => 0x57, // KEY_F11
        0x14B => 0x58, // KEY_F12
        other => other, // Already a raw PS/2 scancode
    }
}

// ── Layout ──────────────────────────────────────────────────────────────────

const WIN_W: u32 = 780;
const WIN_H: u32 = 340;
const KEY_GAP: i32 = 3;
const KEY_H: i32 = 42;
const BOARD_X: i32 = 8;
const BOARD_Y: i32 = 8;

// Colors
const KEY_BG: u32       = 0xFF3A3A4A;
const KEY_BG_PRESS: u32 = 0xFF007AFF; // Blue highlight when pressed
const KEY_BG_MOD: u32   = 0xFF4A4A5A; // Modifier keys
const KEY_BG_MOD_ON: u32 = 0xFF5856D6; // Active modifier (shift/caps)
const KEY_TEXT: u32      = 0xFFEEEEEE;
const KEY_TEXT_DIM: u32  = 0xFF999999;

// ── Key definition ──────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct KeyDef {
    scancode: u8,     // PS/2 scancode (0 = spacer/special)
    width: i32,       // width in units (1 unit = 48px)
    is_mod: bool,     // modifier key (different color)
}

impl KeyDef {
    const fn key(sc: u8, w: i32) -> Self { KeyDef { scancode: sc, width: w, is_mod: false } }
    const fn modkey(sc: u8, w: i32) -> Self { KeyDef { scancode: sc, width: w, is_mod: true } }
}

// Physical key layout (ISO 105-key, 5 rows)
// Width units: 48px per unit. Half-units for wider keys.

// Row 0: Number row (Esc, 1-0, -, =, Backspace)
const ROW0: &[KeyDef] = &[
    KeyDef::key(0x01, 48),     // Esc
    KeyDef::key(0x02, 48), KeyDef::key(0x03, 48), KeyDef::key(0x04, 48), KeyDef::key(0x05, 48),
    KeyDef::key(0x06, 48), KeyDef::key(0x07, 48), KeyDef::key(0x08, 48), KeyDef::key(0x09, 48),
    KeyDef::key(0x0A, 48), KeyDef::key(0x0B, 48), KeyDef::key(0x0C, 48), KeyDef::key(0x0D, 48),
    KeyDef::modkey(0x0E, 90),  // Backspace
];

// Row 1: QWERTY row (Tab, Q-], \)
const ROW1: &[KeyDef] = &[
    KeyDef::modkey(0x0F, 66),  // Tab
    KeyDef::key(0x10, 48), KeyDef::key(0x11, 48), KeyDef::key(0x12, 48), KeyDef::key(0x13, 48),
    KeyDef::key(0x14, 48), KeyDef::key(0x15, 48), KeyDef::key(0x16, 48), KeyDef::key(0x17, 48),
    KeyDef::key(0x18, 48), KeyDef::key(0x19, 48), KeyDef::key(0x1A, 48), KeyDef::key(0x1B, 48),
    KeyDef::key(0x2B, 72),     // Backslash
];

// Row 2: Home row (CapsLock, A-', Enter)
const ROW2: &[KeyDef] = &[
    KeyDef::modkey(0x3A, 78),  // CapsLock
    KeyDef::key(0x1E, 48), KeyDef::key(0x1F, 48), KeyDef::key(0x20, 48), KeyDef::key(0x21, 48),
    KeyDef::key(0x22, 48), KeyDef::key(0x23, 48), KeyDef::key(0x24, 48), KeyDef::key(0x25, 48),
    KeyDef::key(0x26, 48), KeyDef::key(0x27, 48), KeyDef::key(0x28, 48),
    KeyDef::modkey(0x1C, 102), // Enter
];

// Row 3: Bottom alpha (LShift, <, Z-/, RShift)
const ROW3: &[KeyDef] = &[
    KeyDef::modkey(0x2A, 62),  // Left Shift
    KeyDef::key(0x56, 48),     // ISO key (< > |)
    KeyDef::key(0x2C, 48), KeyDef::key(0x2D, 48), KeyDef::key(0x2E, 48), KeyDef::key(0x2F, 48),
    KeyDef::key(0x30, 48), KeyDef::key(0x31, 48), KeyDef::key(0x32, 48), KeyDef::key(0x33, 48),
    KeyDef::key(0x34, 48), KeyDef::key(0x35, 48),
    KeyDef::modkey(0x36, 120), // Right Shift
];

// Row 4: Space row (Ctrl, Super, Alt, Space, AltGr, Fn, Ctrl)
const ROW4: &[KeyDef] = &[
    KeyDef::modkey(0x1D, 62),  // Left Ctrl
    KeyDef::modkey(0x00, 48),  // Super (no scancode)
    KeyDef::modkey(0x38, 62),  // Left Alt
    KeyDef::key(0x39, 288),    // Space
    KeyDef::modkey(0xB8, 62),  // AltGr (E0 0x38, we use 0xB8 as marker)
    KeyDef::modkey(0x00, 48),  // Fn (no scancode)
    KeyDef::modkey(0x9D, 62),  // Right Ctrl (E0 0x1D)
];

const ROWS: &[&[KeyDef]] = &[ROW0, ROW1, ROW2, ROW3, ROW4];

// ── Layout label tables ─────────────────────────────────────────────────────
// For each scancode, the character to display (normal layer).
// Loaded from kernel layout data at runtime.

// Fixed labels for special keys (not affected by layout)
fn special_label(scancode: u8) -> Option<&'static str> {
    match scancode {
        0x00 => Some(""),        // No scancode (Super, Fn)
        0x01 => Some("Esc"),
        0x0E => Some("\u{2190}"), // ← Backspace
        0x0F => Some("Tab"),
        0x1C => Some("\u{21B5}"), // ↵ Enter
        0x1D => Some("Ctrl"),
        0x2A => Some("\u{21E7}"), // ⇧ Shift
        0x36 => Some("\u{21E7}"), // ⇧ Shift
        0x38 => Some("Alt"),
        0x39 => Some(""),        // Space
        0x3A => Some("Caps"),
        0x9D => Some("Ctrl"),    // Right Ctrl
        0xB8 => Some("AltGr"),   // Right Alt / AltGr
        _ => None,
    }
}

// ── Layout character tables (loaded at runtime) ─────────────────────────────

// Normal and shift character for each scancode (0-127)
static mut NORMAL_CHARS: [char; 128] = ['\0'; 128];
static mut SHIFT_CHARS: [char; 128] = ['\0'; 128];

fn load_layout_chars(layout_id: u32) {
    // Read layout tables from kernel via a translation syscall.
    // We call translate for each scancode to get the characters.
    // This is a bit indirect but works without exposing raw layout arrays.

    // We build the tables by calling the kernel's translate function
    // for each scancode with shift=false and shift=true.
    // Since there's no direct "get layout table" syscall, we use
    // the known PS/2 scancode→character mapping structure.

    // For now, hard-code the 5 layout tables (same data as kernel).
    // The layouts are small and this avoids adding a new syscall.

    let (normal, shift) = match layout_id {
        0 => (US_NORMAL, US_SHIFT),
        1 => (DE_NORMAL, DE_SHIFT),
        2 => (CH_NORMAL, CH_SHIFT),
        3 => (FR_NORMAL, FR_SHIFT),
        4 => (PL_NORMAL, PL_SHIFT),
        _ => (US_NORMAL, US_SHIFT),
    };
    unsafe {
        NORMAL_CHARS = normal;
        SHIFT_CHARS = shift;
    }
}

fn key_label(scancode: u8, shift: bool) -> String {
    if let Some(s) = special_label(scancode) {
        return String::from(s);
    }
    let idx = scancode as usize;
    if idx >= 128 { return String::new(); }
    let ch = if shift {
        unsafe { SHIFT_CHARS[idx] }
    } else {
        unsafe { NORMAL_CHARS[idx] }
    };
    if ch == '\0' { return String::new(); }
    let mut s = String::new();
    s.push(ch);
    s
}

// ── Main ────────────────────────────────────────────────────────────────────

struct KeyboardApp {
    win: ui::Window,
    canvas: ui::Canvas,
    layout_seg: ui::SegmentedControl,
    status_label: ui::Label,
    current_layout: u32,
    pressed: [bool; 256],  // scancode → pressed state
    shift_active: bool,
    caps_active: bool,
}

static mut APP: Option<KeyboardApp> = None;
fn app() -> &'static mut KeyboardApp { unsafe { APP.as_mut().unwrap() } }

fn main() {
    if !ui::init() { return; }
    let tc = ui::theme::colors();

    let win = ui::Window::new("Keyboard", -1, -1, WIN_W, WIN_H);

    // ── Toolbar with layout selector ──
    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(DOCK_TOP);
    toolbar.set_size(WIN_W, 38);
    toolbar.set_color(ui::theme::darken(tc.window_bg, 10));
    toolbar.set_padding(8, 4, 8, 4);
    win.add(&toolbar);

    let lbl = toolbar.add_label("Layout:");
    lbl.set_size(50, 28);

    // Build layout selector from available layouts
    let mut layouts_buf = [kbd::LayoutInfo { id: 0, code: [0; 8], label: [0; 4] }; 8];
    let layout_count = kbd::list_layouts(&mut layouts_buf);
    let current_layout = kbd::get_layout();

    let mut seg_labels = String::new();
    let mut initial_seg: u32 = 0;
    for i in 0..layout_count as usize {
        if i > 0 { seg_labels.push('|'); }
        let label = kbd::label_str(&layouts_buf[i].label);
        seg_labels.push_str(label);
        if layouts_buf[i].id == current_layout {
            initial_seg = i as u32;
        }
    }

    let layout_seg = ui::SegmentedControl::new(&seg_labels);
    layout_seg.set_position(62, 5);
    layout_seg.set_size(250, 28);
    layout_seg.set_state(initial_seg);
    toolbar.add(&layout_seg);

    // ── Status label ──
    let status = ui::View::new();
    status.set_dock(DOCK_BOTTOM);
    status.set_size(WIN_W, 24);
    status.set_color(ui::theme::darken(tc.window_bg, 6));
    win.add(&status);

    let status_label = ui::Label::new("Press a key to see raw scancode bytes");
    status_label.set_position(10, 4);
    status_label.set_size(WIN_W - 20, 16);
    status_label.set_font_size(11);
    status_label.set_color(ui::theme::darken(tc.window_bg, 6));
    status_label.set_text_color(0xFF888888);
    status.add(&status_label);

    // ── Keyboard canvas ──
    let canvas = ui::Canvas::new(WIN_W - 16, WIN_H - 38 - 24 - 16);
    canvas.set_dock(DOCK_FILL);
    canvas.set_margin(8, 8, 8, 8);
    win.add(&canvas);

    // Load current layout characters
    load_layout_chars(current_layout);

    unsafe {
        APP = Some(KeyboardApp {
            win,
            canvas,
            layout_seg,
            status_label,
            current_layout,
            pressed: [false; 256],
            shift_active: false,
            caps_active: false,
        });
    }

    // Draw initial keyboard
    draw_keyboard();

    // ── Layout change handler ──
    app().layout_seg.on_active_changed(move |e| {
        let idx = e.index as usize;
        if idx < layout_count as usize {
            let new_id = layouts_buf[idx].id;
            kbd::set_layout(new_id);
            let a = app();
            a.current_layout = new_id;
            load_layout_chars(new_id);
            draw_keyboard();
        }
    });

    // ── Key event handler (on window) ──
    app().win.on_key_down(|e| {
        let sc = key_code_to_scancode(e.keycode) as usize;
        if sc < 256 {
            let a = app();
            a.pressed[sc] = true;
            if sc == 0x2A || sc == 0x36 { a.shift_active = true; }
            if sc == 0x3A { a.caps_active = !a.caps_active; }
            // Show raw bytes in status bar
            let ch = e.char_code;
            let mods = e.modifiers;
            let info = format!(
                "DOWN  sc=0x{:02X} ({})  char=0x{:04X} ('{}')  mods=0x{:02X}{}{}{}",
                sc, sc, ch,
                if ch >= 0x20 { char_from_u32(ch) } else { '.' },
                mods,
                if mods & 1 != 0 { " Shift" } else { "" },
                if mods & 2 != 0 { " Ctrl" } else { "" },
                if mods & 4 != 0 { " Alt" } else { "" },
            );
            a.status_label.set_text(&info);
            draw_keyboard();
        }
    });

    app().win.on_key_up(|e| {
        let sc = key_code_to_scancode(e.keycode) as usize;
        if sc < 256 {
            let a = app();
            a.pressed[sc] = false;
            if sc == 0x2A || sc == 0x36 { a.shift_active = false; }
            let ch = e.char_code;
            let mods = e.modifiers;
            let info = format!(
                "UP    sc=0x{:02X} ({})  char=0x{:04X} ('{}')  mods=0x{:02X}{}{}{}",
                sc, sc, ch,
                if ch >= 0x20 { char_from_u32(ch) } else { '.' },
                mods,
                if mods & 1 != 0 { " Shift" } else { "" },
                if mods & 2 != 0 { " Ctrl" } else { "" },
                if mods & 4 != 0 { " Alt" } else { "" },
            );
            a.status_label.set_text(&info);
            draw_keyboard();
        }
    });

    app().win.on_close(|_| { ui::quit(); });

    ui::run();
}

// ── Draw keyboard ───────────────────────────────────────────────────────────

fn draw_keyboard() {
    let a = app();
    let tc = ui::theme::colors();
    a.canvas.clear(tc.window_bg);

    let shift = a.shift_active ^ a.caps_active;
    let mut y = BOARD_Y;

    for &row in ROWS {
        let mut x = BOARD_X;
        for kd in row {
            let w = kd.width;
            let sc = kd.scancode as usize;

            // Determine key color
            let is_pressed = sc < 256 && a.pressed[sc];
            let is_mod_active = kd.is_mod && match kd.scancode {
                0x2A | 0x36 => a.shift_active,
                0x3A => a.caps_active,
                _ => false,
            };

            let bg = if is_pressed {
                KEY_BG_PRESS
            } else if is_mod_active {
                KEY_BG_MOD_ON
            } else if kd.is_mod {
                KEY_BG_MOD
            } else {
                KEY_BG
            };

            // Draw key background
            a.canvas.fill_rect(x, y, w as u32, KEY_H as u32, bg);

            // Draw key label
            let label = key_label(kd.scancode, shift);
            if !label.is_empty() {
                let text_color = if is_pressed { 0xFFFFFFFF } else { KEY_TEXT };
                // Center text in key
                let text_x = x + w / 2 - (label.len() as i32 * 4);
                let text_y = y + KEY_H / 2 - 6;
                a.canvas.draw_text(text_x, text_y, text_color, 0, 13, &label);
            }

            // Show shift character in corner for alpha keys
            if !kd.is_mod && kd.scancode != 0x39 {
                let alt_label = key_label(kd.scancode, !shift);
                if !alt_label.is_empty() && alt_label != label {
                    a.canvas.draw_text(x + 4, y + 3, KEY_TEXT_DIM, 0, 10, &alt_label);
                }
            }

            x += w + KEY_GAP;
        }
        y += KEY_H + KEY_GAP;
    }
}

// ── Layout character data ───────────────────────────────────────────────────
// Mirrored from kernel layout files. Only the printable key scancodes.

const fn layout_table(pairs: &[(usize, char)]) -> [char; 128] {
    let mut m = ['\0'; 128];
    let mut i = 0;
    while i < pairs.len() {
        m[pairs[i].0] = pairs[i].1;
        i += 1;
    }
    m
}

// US QWERTY
const US_NORMAL: [char; 128] = layout_table(&[
    (0x02,'1'),(0x03,'2'),(0x04,'3'),(0x05,'4'),(0x06,'5'),(0x07,'6'),
    (0x08,'7'),(0x09,'8'),(0x0A,'9'),(0x0B,'0'),(0x0C,'-'),(0x0D,'='),
    (0x10,'q'),(0x11,'w'),(0x12,'e'),(0x13,'r'),(0x14,'t'),(0x15,'y'),
    (0x16,'u'),(0x17,'i'),(0x18,'o'),(0x19,'p'),(0x1A,'['),(0x1B,']'),
    (0x1E,'a'),(0x1F,'s'),(0x20,'d'),(0x21,'f'),(0x22,'g'),(0x23,'h'),
    (0x24,'j'),(0x25,'k'),(0x26,'l'),(0x27,';'),(0x28,'\''),(0x29,'`'),(0x2B,'\\'),
    (0x2C,'z'),(0x2D,'x'),(0x2E,'c'),(0x2F,'v'),(0x30,'b'),(0x31,'n'),
    (0x32,'m'),(0x33,','),(0x34,'.'),(0x35,'/'),
]);
const US_SHIFT: [char; 128] = layout_table(&[
    (0x02,'!'),(0x03,'@'),(0x04,'#'),(0x05,'$'),(0x06,'%'),(0x07,'^'),
    (0x08,'&'),(0x09,'*'),(0x0A,'('),(0x0B,')'),(0x0C,'_'),(0x0D,'+'),
    (0x10,'Q'),(0x11,'W'),(0x12,'E'),(0x13,'R'),(0x14,'T'),(0x15,'Y'),
    (0x16,'U'),(0x17,'I'),(0x18,'O'),(0x19,'P'),(0x1A,'{'),(0x1B,'}'),
    (0x1E,'A'),(0x1F,'S'),(0x20,'D'),(0x21,'F'),(0x22,'G'),(0x23,'H'),
    (0x24,'J'),(0x25,'K'),(0x26,'L'),(0x27,':'),(0x28,'"'),(0x29,'~'),(0x2B,'|'),
    (0x2C,'Z'),(0x2D,'X'),(0x2E,'C'),(0x2F,'V'),(0x30,'B'),(0x31,'N'),
    (0x32,'M'),(0x33,'<'),(0x34,'>'),(0x35,'?'),
]);

// German QWERTZ
const DE_NORMAL: [char; 128] = layout_table(&[
    (0x02,'1'),(0x03,'2'),(0x04,'3'),(0x05,'4'),(0x06,'5'),(0x07,'6'),
    (0x08,'7'),(0x09,'8'),(0x0A,'9'),(0x0B,'0'),(0x0C,'\u{00DF}'),(0x0D,'\u{00B4}'),
    (0x10,'q'),(0x11,'w'),(0x12,'e'),(0x13,'r'),(0x14,'t'),(0x15,'z'),
    (0x16,'u'),(0x17,'i'),(0x18,'o'),(0x19,'p'),(0x1A,'\u{00FC}'),(0x1B,'+'),
    (0x1E,'a'),(0x1F,'s'),(0x20,'d'),(0x21,'f'),(0x22,'g'),(0x23,'h'),
    (0x24,'j'),(0x25,'k'),(0x26,'l'),(0x27,'\u{00F6}'),(0x28,'\u{00E4}'),(0x29,'^'),(0x2B,'#'),
    (0x2C,'y'),(0x2D,'x'),(0x2E,'c'),(0x2F,'v'),(0x30,'b'),(0x31,'n'),
    (0x32,'m'),(0x33,','),(0x34,'.'),(0x35,'-'),(0x56,'<'),
]);
const DE_SHIFT: [char; 128] = layout_table(&[
    (0x02,'!'),(0x03,'"'),(0x04,'\u{00A7}'),(0x05,'$'),(0x06,'%'),(0x07,'&'),
    (0x08,'/'),(0x09,'('),(0x0A,')'),(0x0B,'='),(0x0C,'?'),(0x0D,'`'),
    (0x10,'Q'),(0x11,'W'),(0x12,'E'),(0x13,'R'),(0x14,'T'),(0x15,'Z'),
    (0x16,'U'),(0x17,'I'),(0x18,'O'),(0x19,'P'),(0x1A,'\u{00DC}'),(0x1B,'*'),
    (0x1E,'A'),(0x1F,'S'),(0x20,'D'),(0x21,'F'),(0x22,'G'),(0x23,'H'),
    (0x24,'J'),(0x25,'K'),(0x26,'L'),(0x27,'\u{00D6}'),(0x28,'\u{00C4}'),(0x29,'\u{00B0}'),(0x2B,'\''),
    (0x2C,'Y'),(0x2D,'X'),(0x2E,'C'),(0x2F,'V'),(0x30,'B'),(0x31,'N'),
    (0x32,'M'),(0x33,';'),(0x34,':'),(0x35,'_'),(0x56,'>'),
]);

// Swiss German QWERTZ
const CH_NORMAL: [char; 128] = layout_table(&[
    (0x02,'1'),(0x03,'2'),(0x04,'3'),(0x05,'4'),(0x06,'5'),(0x07,'6'),
    (0x08,'7'),(0x09,'8'),(0x0A,'9'),(0x0B,'0'),(0x0C,'\''),(0x0D,'^'),
    (0x10,'q'),(0x11,'w'),(0x12,'e'),(0x13,'r'),(0x14,'t'),(0x15,'z'),
    (0x16,'u'),(0x17,'i'),(0x18,'o'),(0x19,'p'),(0x1A,'\u{00FC}'),(0x1B,'\u{00A8}'),
    (0x1E,'a'),(0x1F,'s'),(0x20,'d'),(0x21,'f'),(0x22,'g'),(0x23,'h'),
    (0x24,'j'),(0x25,'k'),(0x26,'l'),(0x27,'\u{00F6}'),(0x28,'\u{00E4}'),(0x29,'\u{00A7}'),(0x2B,'$'),
    (0x2C,'y'),(0x2D,'x'),(0x2E,'c'),(0x2F,'v'),(0x30,'b'),(0x31,'n'),
    (0x32,'m'),(0x33,','),(0x34,'.'),(0x35,'-'),(0x56,'<'),
]);
const CH_SHIFT: [char; 128] = layout_table(&[
    (0x02,'+'),(0x03,'"'),(0x04,'*'),(0x05,'\u{00E7}'),(0x06,'%'),(0x07,'&'),
    (0x08,'/'),(0x09,'('),(0x0A,')'),(0x0B,'='),(0x0C,'?'),(0x0D,'`'),
    (0x10,'Q'),(0x11,'W'),(0x12,'E'),(0x13,'R'),(0x14,'T'),(0x15,'Z'),
    (0x16,'U'),(0x17,'I'),(0x18,'O'),(0x19,'P'),(0x1A,'\u{00E8}'),(0x1B,'!'),
    (0x1E,'A'),(0x1F,'S'),(0x20,'D'),(0x21,'F'),(0x22,'G'),(0x23,'H'),
    (0x24,'J'),(0x25,'K'),(0x26,'L'),(0x27,'\u{00E9}'),(0x28,'\u{00E0}'),(0x29,'\u{00B0}'),(0x2B,'\u{00A3}'),
    (0x2C,'Y'),(0x2D,'X'),(0x2E,'C'),(0x2F,'V'),(0x30,'B'),(0x31,'N'),
    (0x32,'M'),(0x33,';'),(0x34,':'),(0x35,'_'),(0x56,'>'),
]);

// French AZERTY
const FR_NORMAL: [char; 128] = layout_table(&[
    (0x02,'&'),(0x03,'\u{00E9}'),(0x04,'"'),(0x05,'\''),(0x06,'('),(0x07,'-'),
    (0x08,'\u{00E8}'),(0x09,'_'),(0x0A,'\u{00E7}'),(0x0B,'\u{00E0}'),(0x0C,')'),(0x0D,'='),
    (0x10,'a'),(0x11,'z'),(0x12,'e'),(0x13,'r'),(0x14,'t'),(0x15,'y'),
    (0x16,'u'),(0x17,'i'),(0x18,'o'),(0x19,'p'),(0x1A,'^'),(0x1B,'$'),
    (0x1E,'q'),(0x1F,'s'),(0x20,'d'),(0x21,'f'),(0x22,'g'),(0x23,'h'),
    (0x24,'j'),(0x25,'k'),(0x26,'l'),(0x27,'m'),(0x28,'\u{00F9}'),(0x29,'\u{00B2}'),(0x2B,'*'),
    (0x2C,'w'),(0x2D,'x'),(0x2E,'c'),(0x2F,'v'),(0x30,'b'),(0x31,'n'),
    (0x32,','),(0x33,';'),(0x34,':'),(0x35,'!'),(0x56,'<'),
]);
const FR_SHIFT: [char; 128] = layout_table(&[
    (0x02,'1'),(0x03,'2'),(0x04,'3'),(0x05,'4'),(0x06,'5'),(0x07,'6'),
    (0x08,'7'),(0x09,'8'),(0x0A,'9'),(0x0B,'0'),(0x0C,'\u{00B0}'),(0x0D,'+'),
    (0x10,'A'),(0x11,'Z'),(0x12,'E'),(0x13,'R'),(0x14,'T'),(0x15,'Y'),
    (0x16,'U'),(0x17,'I'),(0x18,'O'),(0x19,'P'),(0x1A,'\u{00A8}'),(0x1B,'\u{00A3}'),
    (0x1E,'Q'),(0x1F,'S'),(0x20,'D'),(0x21,'F'),(0x22,'G'),(0x23,'H'),
    (0x24,'J'),(0x25,'K'),(0x26,'L'),(0x27,'M'),(0x28,'%'),(0x29,'\0'),(0x2B,'\u{00B5}'),
    (0x2C,'W'),(0x2D,'X'),(0x2E,'C'),(0x2F,'V'),(0x30,'B'),(0x31,'N'),
    (0x32,'?'),(0x33,'.'),(0x34,'/'),(0x35,'\u{00A7}'),(0x56,'>'),
]);

// Polish Programmer (same base as US, AltGr has Polish chars)
const PL_NORMAL: [char; 128] = US_NORMAL;
const PL_SHIFT: [char; 128] = US_SHIFT;

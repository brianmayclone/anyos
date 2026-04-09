//! DateTimePicker — inline date/time editing control.
//!
//! Displays a date and/or time in segmented fields (DD.MM.YYYY HH:MM).
//! Click on a segment to select it, then use Up/Down to change values
//! and Left/Right or Tab to navigate between segments.
//!
//! ## Mode (stored in `text_base.text`)
//!
//! - `"date"` — date only (DD.MM.YYYY)
//! - `"time"` — time only (HH:MM)
//! - `""` or `"datetime"` — full date+time (DD.MM.YYYY HH:MM)
//!
//! ## Value encoding (`base.state`, u32)
//!
//! | Bits  | Field  | Range   |
//! |-------|--------|---------|
//! | 0–5   | minute | 0–59    |
//! | 6–10  | hour   | 0–23    |
//! | 11–15 | day    | 1–31    |
//! | 16–19 | month  | 1–12    |
//! | 20–31 | year   | 0–4095  |

use crate::control::{
    prepare_render, Control, ControlBase, ControlKind, EventResponse, TextControlBase,
};
use crate::control::{KEY_DOWN, KEY_ENTER, KEY_ESCAPE, KEY_LEFT, KEY_RIGHT, KEY_TAB, KEY_UP};

/// Segments the user can navigate between.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Segment {
    Day = 0,
    Month = 1,
    Year = 2,
    Hour = 3,
    Minute = 4,
}

pub struct DateTimePicker {
    pub(crate) text_base: TextControlBase,
    /// Which segment is currently active for editing.
    active_segment: u8,
}

// ── Value packing helpers ──────────────────────────────────────────

fn pack(year: u32, month: u32, day: u32, hour: u32, minute: u32) -> u32 {
    (minute & 0x3F)
        | ((hour & 0x1F) << 6)
        | ((day & 0x1F) << 11)
        | ((month & 0x0F) << 16)
        | ((year & 0xFFF) << 20)
}

fn unpack(v: u32) -> (u32, u32, u32, u32, u32) {
    let minute = v & 0x3F;
    let hour = (v >> 6) & 0x1F;
    let day = (v >> 11) & 0x1F;
    let month = (v >> 16) & 0x0F;
    let year = (v >> 20) & 0xFFF;
    (year, month, day, hour, minute)
}

fn days_in_month(month: u32, year: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 31,
    }
}

// ── Formatting helpers (no alloc) ──────────────────────────────────

fn fmt_2(v: u32, buf: &mut [u8; 2]) {
    buf[0] = b'0' + ((v / 10) % 10) as u8;
    buf[1] = b'0' + (v % 10) as u8;
}

fn fmt_4(v: u32, buf: &mut [u8; 4]) {
    buf[0] = b'0' + ((v / 1000) % 10) as u8;
    buf[1] = b'0' + ((v / 100) % 10) as u8;
    buf[2] = b'0' + ((v / 10) % 10) as u8;
    buf[3] = b'0' + (v % 10) as u8;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Date,
    Time,
    DateTime,
}

impl DateTimePicker {
    pub fn new(text_base: TextControlBase) -> Self {
        Self {
            text_base,
            active_segment: 0,
        }
    }

    fn mode(&self) -> Mode {
        let t = &self.text_base.text;
        if t == b"time" {
            Mode::Time
        } else if t == b"date" {
            Mode::Date
        } else {
            Mode::DateTime
        }
    }

    /// Number of navigable segments for the current mode.
    fn segment_count(&self) -> u8 {
        match self.mode() {
            Mode::Date => 3,     // day, month, year
            Mode::Time => 2,     // hour, minute
            Mode::DateTime => 5, // day, month, year, hour, minute
        }
    }

    /// Map segment index to the logical Segment, accounting for mode.
    fn segment_at(&self, idx: u8) -> Segment {
        match self.mode() {
            Mode::Date => match idx {
                0 => Segment::Day,
                1 => Segment::Month,
                _ => Segment::Year,
            },
            Mode::Time => match idx {
                0 => Segment::Hour,
                _ => Segment::Minute,
            },
            Mode::DateTime => match idx {
                0 => Segment::Day,
                1 => Segment::Month,
                2 => Segment::Year,
                3 => Segment::Hour,
                _ => Segment::Minute,
            },
        }
    }

    /// Increment the active segment value.
    fn adjust(&mut self, delta: i32) {
        let (mut year, mut month, mut day, mut hour, mut minute) =
            unpack(self.text_base.base.state);
        if month == 0 { month = 1; }
        if day == 0 { day = 1; }

        let seg = self.segment_at(self.active_segment);
        match seg {
            Segment::Day => {
                let max = days_in_month(month, year);
                day = wrap_add(day, delta, 1, max);
            }
            Segment::Month => {
                month = wrap_add(month, delta, 1, 12);
                let max = days_in_month(month, year);
                if day > max { day = max; }
            }
            Segment::Year => {
                let ny = year as i32 + delta;
                year = ny.clamp(0, 4095) as u32;
                let max = days_in_month(month, year);
                if day > max { day = max; }
            }
            Segment::Hour => {
                hour = wrap_add(hour, delta, 0, 23);
            }
            Segment::Minute => {
                minute = wrap_add(minute, delta, 0, 59);
            }
        }
        self.text_base.base.state = pack(year, month, day, hour, minute);
    }

    /// Find segment at local x coordinate by measuring rendered text widths.
    fn segment_at_x(&self, lx: i32, font_size: u16) -> u8 {
        let pad = crate::theme::scale_i32(10);
        let (year, month, day, hour, minute) = unpack(self.text_base.base.state);

        // Build the text to measure positions of each segment
        let mode = self.mode();
        let mut positions: [(i32, i32); 5] = [(0, 0); 5]; // (start_x, end_x) per segment index
        let mut x = pad;

        let measure = |text: &[u8]| -> i32 {
            let (w, _) = crate::draw::text_size_at(text, font_size);
            w as i32
        };

        let mut seg_idx: u8 = 0;
        if mode == Mode::Date || mode == Mode::DateTime {
            // Day
            let mut buf2 = [0u8; 2];
            fmt_2(day, &mut buf2);
            let w = measure(&buf2);
            positions[seg_idx as usize] = (x, x + w);
            x += w;
            // dot
            x += measure(b".");
            seg_idx += 1;

            // Month
            fmt_2(month, &mut buf2);
            let w = measure(&buf2);
            positions[seg_idx as usize] = (x, x + w);
            x += w;
            // dot
            x += measure(b".");
            seg_idx += 1;

            // Year
            let mut buf4 = [0u8; 4];
            fmt_4(year, &mut buf4);
            let w = measure(&buf4);
            positions[seg_idx as usize] = (x, x + w);
            x += w;
            seg_idx += 1;

            if mode == Mode::DateTime {
                // space
                x += measure(b"  ");
            }
        }

        if mode == Mode::Time || mode == Mode::DateTime {
            // Hour
            let mut buf2 = [0u8; 2];
            fmt_2(hour, &mut buf2);
            let w = measure(&buf2);
            positions[seg_idx as usize] = (x, x + w);
            x += w;
            // colon
            x += measure(b":");
            seg_idx += 1;

            // Minute
            fmt_2(minute, &mut buf2);
            let w = measure(&buf2);
            positions[seg_idx as usize] = (x, x + w);
            seg_idx += 1;
        }

        // Find the closest segment
        let count = seg_idx;
        for i in 0..count {
            let (sx, ex) = positions[i as usize];
            if lx >= sx && lx <= ex {
                return i;
            }
        }
        // If past all segments, select the last one
        if count > 0 && lx > positions[(count - 1) as usize].1 {
            return count - 1;
        }
        0
    }
}

fn wrap_add(val: u32, delta: i32, min: u32, max: u32) -> u32 {
    let range = (max - min + 1) as i32;
    let v = (val as i32 - min as i32 + delta).rem_euclid(range);
    (v as u32) + min
}

impl Control for DateTimePicker {
    fn base(&self) -> &ControlBase {
        &self.text_base.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.text_base.base
    }
    fn text_base(&self) -> Option<&TextControlBase> {
        Some(&self.text_base)
    }
    fn text_base_mut(&mut self) -> Option<&mut TextControlBase> {
        Some(&mut self.text_base)
    }
    fn kind(&self) -> ControlKind {
        ControlKind::DateTimePicker
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let ctx = prepare_render(b, ax, ay);
        let (x, y, w, h) = (ctx.x, ctx.y, ctx.w, ctx.h);
        let tc = crate::theme::colors();
        let corner = crate::theme::scale(6);

        // Background — field style like TextField/DropDown
        let palette =
            crate::controls::chrome::flat_field_palette(0, ctx.hovered, ctx.focused, ctx.disabled);
        if ctx.focused && !ctx.disabled {
            crate::controls::chrome::draw_focus(surface, x, y, w, h, corner, palette);
        }
        crate::controls::chrome::draw_surface(surface, x, y, w, h, corner, palette);

        // ── Text rendering ─────────────────────────────────────────
        let logical_fs = if self.text_base.text_style.font_size > 0 {
            self.text_base.text_style.font_size
        } else {
            13
        };
        let font_size = crate::draw::scale_font(logical_fs);
        let text_color = if ctx.disabled { tc.text_disabled } else { tc.text };
        let sep_color = if ctx.disabled { tc.text_disabled } else { tc.text_secondary };
        let sel_bg = if ctx.focused && !ctx.disabled {
            tc.accent
        } else {
            crate::theme::darken(tc.control_bg, 12)
        };
        let sel_fg = if ctx.focused && !ctx.disabled { 0xFFFFFFFF } else { text_color };

        let (year, month, day, hour, minute) = unpack(b.state);
        let mode = self.mode();
        let pad = crate::theme::scale_i32(10);
        let ty = y + (h as i32 - font_size as i32) / 2;
        let mut cx = x + pad;

        let mut seg_idx: u8 = 0;

        let draw_seg = |surface: &crate::draw::Surface,
                        cx: &mut i32,
                        text: &[u8],
                        idx: u8,
                        active: u8,
                        ty: i32,
                        font_size: u16,
                        text_color: u32,
                        sel_bg: u32,
                        sel_fg: u32| {
            let (tw, th) = crate::draw::text_size_at(text, font_size);
            if idx == active {
                let rx = *cx - crate::theme::scale_i32(2);
                let ry = ty - crate::theme::scale_i32(1);
                let rw = tw + crate::theme::scale(4);
                let rh = th + crate::theme::scale(2);
                crate::draw::fill_rounded_rect(
                    surface,
                    rx,
                    ry,
                    rw,
                    rh,
                    crate::theme::scale(3),
                    sel_bg,
                );
                crate::draw::draw_text_sized(surface, *cx, ty, sel_fg, text, font_size);
            } else {
                crate::draw::draw_text_sized(surface, *cx, ty, text_color, text, font_size);
            }
            *cx += tw as i32;
        };

        let draw_sep =
            |surface: &crate::draw::Surface, cx: &mut i32, text: &[u8], ty: i32, font_size: u16, color: u32| {
                let (tw, _) = crate::draw::text_size_at(text, font_size);
                crate::draw::draw_text_sized(surface, *cx, ty, color, text, font_size);
                *cx += tw as i32;
            };

        if mode == Mode::Date || mode == Mode::DateTime {
            let mut buf2 = [0u8; 2];
            // Day
            fmt_2(day, &mut buf2);
            draw_seg(surface, &mut cx, &buf2, seg_idx, self.active_segment, ty, font_size, text_color, sel_bg, sel_fg);
            draw_sep(surface, &mut cx, b".", ty, font_size, sep_color);
            seg_idx += 1;

            // Month
            fmt_2(month, &mut buf2);
            draw_seg(surface, &mut cx, &buf2, seg_idx, self.active_segment, ty, font_size, text_color, sel_bg, sel_fg);
            draw_sep(surface, &mut cx, b".", ty, font_size, sep_color);
            seg_idx += 1;

            // Year
            let mut buf4 = [0u8; 4];
            fmt_4(year, &mut buf4);
            draw_seg(surface, &mut cx, &buf4, seg_idx, self.active_segment, ty, font_size, text_color, sel_bg, sel_fg);
            seg_idx += 1;

            if mode == Mode::DateTime {
                draw_sep(surface, &mut cx, b"  ", ty, font_size, sep_color);
            }
        }

        if mode == Mode::Time || mode == Mode::DateTime {
            let mut buf2 = [0u8; 2];
            // Hour
            fmt_2(hour, &mut buf2);
            draw_seg(surface, &mut cx, &buf2, seg_idx, self.active_segment, ty, font_size, text_color, sel_bg, sel_fg);
            draw_sep(surface, &mut cx, b":", ty, font_size, sep_color);
            seg_idx += 1;

            // Minute
            fmt_2(minute, &mut buf2);
            draw_seg(surface, &mut cx, &buf2, seg_idx, self.active_segment, ty, font_size, text_color, sel_bg, sel_fg);
            let _ = seg_idx;
        }

        // ── Up/Down chevrons on the right ──────────────────────────
        if ctx.focused && !ctx.disabled {
            let chev_color = tc.text_secondary;
            let chev_x = x + w as i32 - crate::theme::scale_i32(18);
            let chev_size = crate::theme::scale_i32(3);

            // Up chevron (▲)
            let uy = y + crate::theme::scale_i32(5);
            for row in 0..chev_size {
                let half = chev_size - 1 - row;
                let lx = chev_x + row;
                let lw = 1 + half * 2;
                crate::draw::fill_rect(surface, lx, uy + row, lw as u32, 1, chev_color);
            }

            // Down chevron (▼)
            let dy = y + h as i32 - crate::theme::scale_i32(8);
            for row in 0..chev_size {
                let half = chev_size - 1 - row;
                let lx = chev_x + (chev_size - 1 - half);
                let lw = 1 + half * 2;
                crate::draw::fill_rect(surface, lx, dy + row, lw as u32, 1, chev_color);
            }
        }
    }

    fn is_interactive(&self) -> bool {
        !self.text_base.base.disabled
    }

    fn handle_click(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        let logical_fs = if self.text_base.text_style.font_size > 0 {
            self.text_base.text_style.font_size
        } else {
            13
        };
        let font_size = crate::draw::scale_font(logical_fs);
        self.active_segment = self.segment_at_x(lx, font_size);
        self.text_base.base.mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, _char_code: u32, modifiers: u32) -> EventResponse {
        let count = self.segment_count();
        match keycode {
            KEY_UP => {
                self.adjust(1);
                self.text_base.base.mark_dirty();
                EventResponse::CHANGED
            }
            KEY_DOWN => {
                self.adjust(-1);
                self.text_base.base.mark_dirty();
                EventResponse::CHANGED
            }
            KEY_LEFT => {
                if self.active_segment > 0 {
                    self.active_segment -= 1;
                }
                self.text_base.base.mark_dirty();
                EventResponse::CONSUMED
            }
            KEY_RIGHT | KEY_TAB => {
                if self.active_segment < count - 1 {
                    self.active_segment += 1;
                    self.text_base.base.mark_dirty();
                    EventResponse::CONSUMED
                } else if keycode == KEY_TAB {
                    EventResponse::IGNORED // let focus move
                } else {
                    EventResponse::CONSUMED
                }
            }
            KEY_ENTER => EventResponse::SUBMIT,
            _ => {
                // Digit input — replace active segment value
                let ch = _char_code;
                if ch >= b'0' as u32 && ch <= b'9' as u32 {
                    let digit = ch - b'0' as u32;
                    self.type_digit(digit);
                    self.text_base.base.mark_dirty();
                    EventResponse::CHANGED
                } else {
                    EventResponse::IGNORED
                }
            }
        }
    }

    fn handle_blur(&mut self) {
        self.text_base.base.focused = false;
        self.text_base.base.mark_dirty();
    }
}

impl DateTimePicker {
    /// Handle digit typing into the active segment.
    /// Uses a simple scheme: first digit sets tens, on the next digit it shifts
    /// and sets the result then advances to the next segment.
    fn type_digit(&mut self, digit: u32) {
        let (mut year, mut month, mut day, mut hour, mut minute) =
            unpack(self.text_base.base.state);
        if month == 0 { month = 1; }
        if day == 0 { day = 1; }

        let seg = self.segment_at(self.active_segment);
        match seg {
            Segment::Day => {
                let new = (day % 10) * 10 + digit;
                let max = days_in_month(month, year);
                day = if new >= 1 && new <= max { new } else if digit >= 1 && digit <= 9 { digit } else { day };
            }
            Segment::Month => {
                let new = (month % 10) * 10 + digit;
                month = if new >= 1 && new <= 12 { new } else if digit >= 1 && digit <= 9 { digit } else { month };
            }
            Segment::Year => {
                year = (year % 1000) * 10 + digit;
            }
            Segment::Hour => {
                let new = (hour % 10) * 10 + digit;
                hour = if new <= 23 { new } else { digit };
            }
            Segment::Minute => {
                let new = (minute % 10) * 10 + digit;
                minute = if new <= 59 { new } else { digit };
            }
        }
        self.text_base.base.state = pack(year, month, day, hour, minute);
    }
}

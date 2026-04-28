use alloc::string::String;

use crate::{
    theme, Button, Control, Label, PlainButton, TextField, View, Widget, Window, DOCK_BOTTOM,
    DOCK_FILL, DOCK_TOP,
};

const DLG_W: u32 = 430;
const DLG_H: u32 = 250;

pub struct ColorPickerDialog;

impl ColorPickerDialog {
    /// Show a reusable color picker for anyOS ARGB colors.
    ///
    /// The dialog accepts and returns colors in `#AARRGGBB` format. Returning
    /// `true` from `on_accept` closes the dialog; returning `false` keeps it
    /// open so clients can surface validation errors elsewhere.
    pub fn show(initial: &str, mut on_accept: impl FnMut(String) -> bool + 'static) {
        let tc = theme::colors();
        let win = Window::new("Color", -1, -1, DLG_W, DLG_H);
        let win_id = win.id();

        let header = View::new();
        header.set_dock(DOCK_TOP);
        header.set_size(DLG_W, 58);
        header.set_color(tc.sidebar_bg);
        win.add(&header);

        let title = Label::new("Color");
        title.set_position(22, 14);
        title.set_size(220, 22);
        title.set_font_size(17);
        title.set_text_color(tc.text);
        header.add(&title);

        let subtitle = Label::new("anyOS colors use #AARRGGBB.");
        subtitle.set_position(22, 38);
        subtitle.set_size(360, 18);
        subtitle.set_font_size(10);
        subtitle.set_text_color(tc.text_secondary);
        header.add(&subtitle);

        let content = View::new();
        content.set_dock(DOCK_FILL);
        content.set_color(tc.editor_bg);
        win.add(&content);

        let preview = View::new();
        preview.set_position(24, 24);
        preview.set_size(64, 64);
        preview.set_color(parse_argb(initial).unwrap_or(tc.control_bg));
        content.add(&preview);

        let label = Label::new("Value");
        label.set_position(112, 24);
        label.set_size(90, 18);
        label.set_text_color(tc.text);
        content.add(&label);

        let field = TextField::new();
        field.set_position(112, 46);
        field.set_size(280, 30);
        field.set_color(tc.control_bg);
        field.set_text_color(tc.text);
        field.set_text(if is_argb(initial) {
            initial
        } else {
            "#FFFFFFFF"
        });
        field.set_max_length(9);
        field.select_all();
        content.add(&field);

        let hint = Label::new("Example: #FF007ACC");
        hint.set_position(112, 82);
        hint.set_size(280, 18);
        hint.set_font_size(10);
        hint.set_text_color(tc.text_secondary);
        content.add(&hint);

        let presets = [
            0xFFFFFFFF, 0xFF000000, 0xFF007ACC, 0xFF22C55E, 0xFFF59E0B, 0xFFEF4444, 0xFF8B5CF6,
            0xFF1E1E1E,
        ];
        for (i, color) in presets.iter().enumerate() {
            let swatch = PlainButton::new("");
            swatch.set_position(112 + (i as i32 * 34), 110);
            swatch.set_size(26, 26);
            swatch.set_color(*color);
            swatch.set_tooltip("Preset color");
            content.add(&swatch);
            let field_id = field.id();
            let preview_id = preview.id();
            let color = *color;
            swatch.on_click(move |_| {
                let text = argb_to_string(color);
                Control::from_id(field_id).set_text(&text);
                Control::from_id(preview_id).set_color(color);
            });
        }

        let field_id = field.id();
        let preview_id = preview.id();
        field.on_text_changed(move |_| {
            let value = read_string(field_id);
            if let Some(color) = parse_argb(&value) {
                Control::from_id(preview_id).set_color(color);
            }
        });

        let footer = View::new();
        footer.set_dock(DOCK_BOTTOM);
        footer.set_size(DLG_W, 54);
        footer.set_color(tc.sidebar_bg);
        win.add(&footer);

        let btn_ok = Button::new("OK");
        btn_ok.set_size(88, 30);
        btn_ok.set_position((DLG_W as i32) - 196, 12);
        btn_ok.set_color(tc.success);
        footer.add(&btn_ok);

        let btn_cancel = Button::new("Cancel");
        btn_cancel.set_size(88, 30);
        btn_cancel.set_position((DLG_W as i32) - 100, 12);
        btn_cancel.set_color(tc.control_bg);
        footer.add(&btn_cancel);

        btn_ok.on_click(move |_| {
            let value = read_string(field_id);
            if is_argb(&value) && on_accept(value) {
                Window::from_id(win_id).destroy();
            }
        });

        btn_cancel.on_click(move |_| {
            Window::from_id(win_id).destroy();
        });
    }
}

fn read_string(id: u32) -> String {
    let mut buf = [0u8; 64];
    let len = Control::from_id(id).get_text(&mut buf);
    core::str::from_utf8(&buf[..len as usize])
        .unwrap_or("")
        .trim()
        .into()
}

fn is_argb(value: &str) -> bool {
    parse_argb(value).is_some()
}

fn parse_argb(value: &str) -> Option<u32> {
    let bytes = value.trim().as_bytes();
    if bytes.len() != 9 || bytes[0] != b'#' {
        return None;
    }
    let mut out = 0u32;
    for &byte in &bytes[1..] {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        out = (out << 4) | digit as u32;
    }
    Some(out)
}

fn argb_to_string(color: u32) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::from("#");
    for shift in [28u32, 24, 20, 16, 12, 8, 4, 0] {
        out.push(HEX[((color >> shift) & 0xF) as usize] as char);
    }
    out
}

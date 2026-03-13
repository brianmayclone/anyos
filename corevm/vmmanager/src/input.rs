use eframe::egui;

/// Map egui Key to PS/2 scancode set 1 (default set used by the PS/2 controller).
/// Returns (scancode, is_extended) where is_extended means E0 prefix needed.
pub fn scancode_for_key(key: egui::Key) -> Option<(u8, bool)> {
    match key {
        egui::Key::A => Some((0x1E, false)),
        egui::Key::B => Some((0x30, false)),
        egui::Key::C => Some((0x2E, false)),
        egui::Key::D => Some((0x20, false)),
        egui::Key::E => Some((0x12, false)),
        egui::Key::F => Some((0x21, false)),
        egui::Key::G => Some((0x22, false)),
        egui::Key::H => Some((0x23, false)),
        egui::Key::I => Some((0x17, false)),
        egui::Key::J => Some((0x24, false)),
        egui::Key::K => Some((0x25, false)),
        egui::Key::L => Some((0x26, false)),
        egui::Key::M => Some((0x32, false)),
        egui::Key::N => Some((0x31, false)),
        egui::Key::O => Some((0x18, false)),
        egui::Key::P => Some((0x19, false)),
        egui::Key::Q => Some((0x10, false)),
        egui::Key::R => Some((0x13, false)),
        egui::Key::S => Some((0x1F, false)),
        egui::Key::T => Some((0x14, false)),
        egui::Key::U => Some((0x16, false)),
        egui::Key::V => Some((0x2F, false)),
        egui::Key::W => Some((0x11, false)),
        egui::Key::X => Some((0x2D, false)),
        egui::Key::Y => Some((0x15, false)),
        egui::Key::Z => Some((0x2C, false)),
        egui::Key::Num0 => Some((0x0B, false)),
        egui::Key::Num1 => Some((0x02, false)),
        egui::Key::Num2 => Some((0x03, false)),
        egui::Key::Num3 => Some((0x04, false)),
        egui::Key::Num4 => Some((0x05, false)),
        egui::Key::Num5 => Some((0x06, false)),
        egui::Key::Num6 => Some((0x07, false)),
        egui::Key::Num7 => Some((0x08, false)),
        egui::Key::Num8 => Some((0x09, false)),
        egui::Key::Num9 => Some((0x0A, false)),
        egui::Key::Enter => Some((0x1C, false)),
        egui::Key::Escape => Some((0x01, false)),
        egui::Key::Backspace => Some((0x0E, false)),
        egui::Key::Tab => Some((0x0F, false)),
        egui::Key::Space => Some((0x39, false)),
        egui::Key::ArrowLeft => Some((0x4B, true)),
        egui::Key::ArrowRight => Some((0x4D, true)),
        egui::Key::ArrowUp => Some((0x48, true)),
        egui::Key::ArrowDown => Some((0x50, true)),
        egui::Key::F1 => Some((0x3B, false)),
        egui::Key::F2 => Some((0x3C, false)),
        egui::Key::F3 => Some((0x3D, false)),
        egui::Key::F4 => Some((0x3E, false)),
        egui::Key::F5 => Some((0x3F, false)),
        egui::Key::F6 => Some((0x40, false)),
        egui::Key::F7 => Some((0x41, false)),
        egui::Key::F8 => Some((0x42, false)),
        egui::Key::F9 => Some((0x43, false)),
        egui::Key::F10 => Some((0x44, false)),
        egui::Key::F11 => Some((0x57, false)),
        egui::Key::F12 => Some((0x58, false)),
        egui::Key::Delete => Some((0x53, true)),
        egui::Key::Home => Some((0x47, true)),
        egui::Key::End => Some((0x4F, true)),
        egui::Key::PageUp => Some((0x49, true)),
        egui::Key::PageDown => Some((0x51, true)),
        egui::Key::Insert => Some((0x52, true)),
        // Punctuation / symbols (Set 1)
        egui::Key::Minus => Some((0x0C, false)),
        egui::Key::Equals => Some((0x0D, false)),
        egui::Key::OpenBracket => Some((0x1A, false)),
        egui::Key::CloseBracket => Some((0x1B, false)),
        egui::Key::Backslash => Some((0x2B, false)),
        egui::Key::Semicolon => Some((0x27, false)),
        egui::Key::Quote => Some((0x28, false)),
        egui::Key::Backtick => Some((0x29, false)),
        egui::Key::Comma => Some((0x33, false)),
        egui::Key::Period => Some((0x34, false)),
        egui::Key::Slash => Some((0x35, false)),
        _ => None,
    }
}

/// Handle keyboard events from egui and send to VM via libcorevm.
/// vm_handle: the corevm VM handle (u64)
/// display_focused: whether the display area has focus (captures all keys)
/// Returns a label for the last key pressed (if any) for status bar display.
pub fn handle_keyboard_events(ctx: &egui::Context, vm_handle: u64, display_focused: bool) -> Option<String> {
    if !display_focused {
        return None;
    }

    // Use input_mut to both process AND remove key events in one pass.
    // This must be called BEFORE any egui widgets are drawn, so egui
    // never sees Enter/Tab/etc. for its own navigation.
    let mut last_key: Option<String> = None;
    ctx.input_mut(|i| {
        for event in &i.events {
            match event {
                egui::Event::Key { key, pressed, repeat, .. } => {
                    // Ignore key-repeat events — we only want actual press/release
                    if *repeat {
                        continue;
                    }
                    if let Some((scancode, _extended)) = scancode_for_key(*key) {
                        if *pressed {
                            libcorevm::ffi::corevm_ps2_key_press(vm_handle, scancode);
                            last_key = Some(format!("{:?} (0x{:02X})", key, scancode));
                        } else {
                            libcorevm::ffi::corevm_ps2_key_release(vm_handle, scancode);
                        }
                    }
                }
                // Ignore Text events entirely — we handle everything via Key events.
                // Text events would cause duplicate input for keys already handled above.
                _ => {}
            }
        }
        i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
    });
    last_key
}

/// Map ASCII characters to PS/2 scancode set 1 (for characters not covered by egui::Key)
fn scancode_for_char(ch: char) -> Option<u8> {
    match ch {
        '-' | '_' => Some(0x0C),
        '=' | '+' => Some(0x0D),
        '[' | '{' => Some(0x1A),
        ']' | '}' => Some(0x1B),
        '\\' | '|' => Some(0x2B),
        ';' | ':' => Some(0x27),
        '\'' | '"' => Some(0x28),
        '`' | '~' => Some(0x29),
        ',' | '<' => Some(0x33),
        '.' | '>' => Some(0x34),
        '/' | '?' => Some(0x35),
        _ => None, // Letters, digits, space handled via Key events
    }
}


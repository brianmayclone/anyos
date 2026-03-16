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

/// Track modifier key state to detect press/release transitions.
static mut PREV_SHIFT: bool = false;
static mut PREV_CTRL: bool = false;
static mut PREV_ALT: bool = false;

/// Handle keyboard events from egui and send to VM via libcorevm.
/// vm_handle: the corevm VM handle (u64)
/// display_focused: whether the display area has focus (captures all keys)
/// Returns a label for the last key pressed (if any) for status bar display.
pub fn handle_keyboard_events(ctx: &egui::Context, vm_handle: u64, display_focused: bool) -> Option<String> {
    if !display_focused {
        // Release all modifiers when focus is lost
        unsafe {
            if PREV_SHIFT { libcorevm::ffi::corevm_ps2_key_release(vm_handle, 0x2A); PREV_SHIFT = false; }
            if PREV_CTRL  { libcorevm::ffi::corevm_ps2_key_release(vm_handle, 0x1D); PREV_CTRL = false; }
            if PREV_ALT   { libcorevm::ffi::corevm_ps2_key_release(vm_handle, 0x38); PREV_ALT = false; }
        }
        return None;
    }

    // Track modifier state from egui and send press/release scancodes
    let modifiers = ctx.input(|i| i.modifiers);
    unsafe {
        if modifiers.shift && !PREV_SHIFT {
            libcorevm::ffi::corevm_ps2_key_press(vm_handle, 0x2A); // Left Shift
            PREV_SHIFT = true;
        } else if !modifiers.shift && PREV_SHIFT {
            libcorevm::ffi::corevm_ps2_key_release(vm_handle, 0x2A);
            PREV_SHIFT = false;
        }
        if modifiers.ctrl && !PREV_CTRL {
            libcorevm::ffi::corevm_ps2_key_press(vm_handle, 0x1D); // Left Ctrl
            PREV_CTRL = true;
        } else if !modifiers.ctrl && PREV_CTRL {
            libcorevm::ffi::corevm_ps2_key_release(vm_handle, 0x1D);
            PREV_CTRL = false;
        }
        if modifiers.alt && !PREV_ALT {
            libcorevm::ffi::corevm_ps2_key_press(vm_handle, 0x38); // Left Alt
            PREV_ALT = true;
        } else if !modifiers.alt && PREV_ALT {
            libcorevm::ffi::corevm_ps2_key_release(vm_handle, 0x38);
            PREV_ALT = false;
        }
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

const SC_LSHIFT: u8 = 0x2A;

/// Map a character to its PS/2 scancode and whether Shift is required.
/// Returns (scancode, needs_shift).
fn char_to_scancode(ch: char) -> Option<(u8, bool)> {
    match ch {
        'a'..='z' => {
            let idx = (ch as u8) - b'a';
            let scancodes: [u8; 26] = [
                0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24,
                0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10, 0x13, 0x1F, 0x14,
                0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
            ];
            Some((scancodes[idx as usize], false))
        }
        'A'..='Z' => {
            let idx = (ch as u8) - b'A';
            let scancodes: [u8; 26] = [
                0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24,
                0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10, 0x13, 0x1F, 0x14,
                0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
            ];
            Some((scancodes[idx as usize], true))
        }
        '0' => Some((0x0B, false)),
        '1' => Some((0x02, false)),
        '2' => Some((0x03, false)),
        '3' => Some((0x04, false)),
        '4' => Some((0x05, false)),
        '5' => Some((0x06, false)),
        '6' => Some((0x07, false)),
        '7' => Some((0x08, false)),
        '8' => Some((0x09, false)),
        '9' => Some((0x0A, false)),
        ' ' => Some((0x39, false)),
        '\n' => Some((0x1C, false)),
        '\t' => Some((0x0F, false)),
        // Unshifted symbols
        '-' => Some((0x0C, false)),
        '=' => Some((0x0D, false)),
        '[' => Some((0x1A, false)),
        ']' => Some((0x1B, false)),
        '\\' => Some((0x2B, false)),
        ';' => Some((0x27, false)),
        '\'' => Some((0x28, false)),
        '`' => Some((0x29, false)),
        ',' => Some((0x33, false)),
        '.' => Some((0x34, false)),
        '/' => Some((0x35, false)),
        // Shifted symbols
        '!' => Some((0x02, true)),  // Shift+1
        '@' => Some((0x03, true)),  // Shift+2
        '#' => Some((0x04, true)),  // Shift+3
        '$' => Some((0x05, true)),  // Shift+4
        '%' => Some((0x06, true)),  // Shift+5
        '^' => Some((0x07, true)),  // Shift+6
        '&' => Some((0x08, true)),  // Shift+7
        '*' => Some((0x09, true)),  // Shift+8
        '(' => Some((0x0A, true)),  // Shift+9
        ')' => Some((0x0B, true)),  // Shift+0
        '_' => Some((0x0C, true)),  // Shift+-
        '+' => Some((0x0D, true)),  // Shift+=
        '{' => Some((0x1A, true)),  // Shift+[
        '}' => Some((0x1B, true)),  // Shift+]
        '|' => Some((0x2B, true)),  // Shift+\
        ':' => Some((0x27, true)),  // Shift+;
        '"' => Some((0x28, true)),  // Shift+'
        '~' => Some((0x29, true)),  // Shift+`
        '<' => Some((0x33, true)),  // Shift+,
        '>' => Some((0x34, true)),  // Shift+.
        '?' => Some((0x35, true)),  // Shift+/
        _ => None,
    }
}

/// Inject a string as PS/2 keystrokes into the VM.
/// Each character is mapped to its scancode, with Shift pressed as needed.
/// Returns the number of characters successfully typed.
pub fn type_string_to_vm(vm_handle: u64, text: &str) -> usize {
    let mut count = 0;
    for ch in text.chars() {
        if let Some((scancode, needs_shift)) = char_to_scancode(ch) {
            if needs_shift {
                libcorevm::ffi::corevm_ps2_key_press(vm_handle, SC_LSHIFT);
            }
            libcorevm::ffi::corevm_ps2_key_press(vm_handle, scancode);
            libcorevm::ffi::corevm_ps2_key_release(vm_handle, scancode);
            if needs_shift {
                libcorevm::ffi::corevm_ps2_key_release(vm_handle, SC_LSHIFT);
            }
            count += 1;
        }
        // Skip characters we can't map (e.g. Unicode beyond ASCII)
    }
    count
}


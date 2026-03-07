use eframe::egui;

/// Map egui Key to PS/2 scancode set 2.
/// Returns (scancode, is_extended) where is_extended means E0 prefix needed.
pub fn scancode_for_key(key: egui::Key) -> Option<(u8, bool)> {
    match key {
        egui::Key::A => Some((0x1C, false)),
        egui::Key::B => Some((0x32, false)),
        egui::Key::C => Some((0x21, false)),
        egui::Key::D => Some((0x23, false)),
        egui::Key::E => Some((0x24, false)),
        egui::Key::F => Some((0x2B, false)),
        egui::Key::G => Some((0x34, false)),
        egui::Key::H => Some((0x33, false)),
        egui::Key::I => Some((0x43, false)),
        egui::Key::J => Some((0x3B, false)),
        egui::Key::K => Some((0x42, false)),
        egui::Key::L => Some((0x4B, false)),
        egui::Key::M => Some((0x3A, false)),
        egui::Key::N => Some((0x31, false)),
        egui::Key::O => Some((0x44, false)),
        egui::Key::P => Some((0x4D, false)),
        egui::Key::Q => Some((0x15, false)),
        egui::Key::R => Some((0x2D, false)),
        egui::Key::S => Some((0x1B, false)),
        egui::Key::T => Some((0x2C, false)),
        egui::Key::U => Some((0x3C, false)),
        egui::Key::V => Some((0x2A, false)),
        egui::Key::W => Some((0x1D, false)),
        egui::Key::X => Some((0x22, false)),
        egui::Key::Y => Some((0x35, false)),
        egui::Key::Z => Some((0x1A, false)),
        egui::Key::Num0 => Some((0x45, false)),
        egui::Key::Num1 => Some((0x16, false)),
        egui::Key::Num2 => Some((0x1E, false)),
        egui::Key::Num3 => Some((0x26, false)),
        egui::Key::Num4 => Some((0x25, false)),
        egui::Key::Num5 => Some((0x2E, false)),
        egui::Key::Num6 => Some((0x36, false)),
        egui::Key::Num7 => Some((0x3D, false)),
        egui::Key::Num8 => Some((0x3E, false)),
        egui::Key::Num9 => Some((0x46, false)),
        egui::Key::Enter => Some((0x5A, false)),
        egui::Key::Escape => Some((0x76, false)),
        egui::Key::Backspace => Some((0x66, false)),
        egui::Key::Tab => Some((0x0D, false)),
        egui::Key::Space => Some((0x29, false)),
        egui::Key::ArrowLeft => Some((0x6B, true)),
        egui::Key::ArrowRight => Some((0x74, true)),
        egui::Key::ArrowUp => Some((0x75, true)),
        egui::Key::ArrowDown => Some((0x72, true)),
        egui::Key::F1 => Some((0x05, false)),
        egui::Key::F2 => Some((0x06, false)),
        egui::Key::F3 => Some((0x04, false)),
        egui::Key::F4 => Some((0x0C, false)),
        egui::Key::F5 => Some((0x03, false)),
        egui::Key::F6 => Some((0x0B, false)),
        egui::Key::F7 => Some((0x83, false)),
        egui::Key::F8 => Some((0x0A, false)),
        egui::Key::F9 => Some((0x01, false)),
        egui::Key::F10 => Some((0x09, false)),
        egui::Key::F11 => Some((0x78, false)),
        egui::Key::F12 => Some((0x07, false)),
        egui::Key::Delete => Some((0x71, true)),
        egui::Key::Home => Some((0x6C, true)),
        egui::Key::End => Some((0x69, true)),
        egui::Key::PageUp => Some((0x7D, true)),
        egui::Key::PageDown => Some((0x7A, true)),
        egui::Key::Insert => Some((0x70, true)),
        _ => None,
    }
}

/// Handle keyboard events from egui and send to VM via libcorevm.
/// vm_handle: the corevm VM handle (u64)
pub fn handle_keyboard_events(ctx: &egui::Context, vm_handle: u64) {
    ctx.input(|i| {
        for event in &i.events {
            if let egui::Event::Key { key, pressed, .. } = event {
                if let Some((scancode, _extended)) = scancode_for_key(*key) {
                    if *pressed {
                        libcorevm::corevm_ps2_key_press(vm_handle, scancode);
                    } else {
                        libcorevm::corevm_ps2_key_release(vm_handle, scancode);
                    }
                }
            }
        }
    });
}

/// Mouse capture state
pub struct MouseCapture {
    pub captured: bool,
    pub last_pos: Option<egui::Pos2>,
}

impl Default for MouseCapture {
    fn default() -> Self {
        Self {
            captured: false,
            last_pos: None,
        }
    }
}

impl MouseCapture {
    /// Handle mouse input. Call this when the display widget has focus.
    /// Returns true if mouse is captured.
    pub fn handle_mouse(
        &mut self,
        ui: &egui::Ui,
        display_rect: egui::Rect,
        vm_handle: u64,
    ) -> bool {
        let response = ui.interact(
            display_rect,
            egui::Id::new("vm_display_mouse"),
            egui::Sense::click_and_drag(),
        );

        // Click to capture
        if response.clicked() {
            self.captured = true;
            self.last_pos = None;
        }

        // Ctrl+Alt to release
        ui.input(|i| {
            if i.modifiers.ctrl && i.modifiers.alt {
                self.captured = false;
                self.last_pos = None;
            }
        });

        if self.captured {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                if let Some(last) = self.last_pos {
                    let dx = (pos.x - last.x) as i16;
                    let dy = (pos.y - last.y) as i16;
                    if dx != 0 || dy != 0 {
                        let buttons = ui.input(|i| {
                            let mut b = 0u8;
                            if i.pointer.button_down(egui::PointerButton::Primary) {
                                b |= 1;
                            }
                            if i.pointer.button_down(egui::PointerButton::Secondary) {
                                b |= 2;
                            }
                            if i.pointer.button_down(egui::PointerButton::Middle) {
                                b |= 4;
                            }
                            b
                        });
                        libcorevm::corevm_ps2_mouse_move(vm_handle, dx, dy, buttons);
                    }
                }
                self.last_pos = Some(pos);
            }
        }

        self.captured
    }
}

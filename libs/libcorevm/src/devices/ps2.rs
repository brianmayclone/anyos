//! PS/2 controller (keyboard + mouse) emulation.
//!
//! Emulates an Intel 8042-compatible PS/2 controller that manages a
//! keyboard on the first port and an optional mouse on the second port.
//!
//! # I/O Ports
//!
//! | Port | Direction | Description |
//! |------|-----------|-------------|
//! | 0x60 | Read      | Output buffer (data from device to guest) |
//! | 0x60 | Write     | Input buffer (data/commands to device) |
//! | 0x64 | Read      | Status register |
//! | 0x64 | Write     | Controller command |
//!
//! # Status Register Bits
//!
//! | Bit | Name | Description |
//! |-----|------|-------------|
//! | 0   | OBF  | Output buffer full (data available for guest to read) |
//! | 1   | IBF  | Input buffer full (controller processing a command) |
//! | 5   | MOBF | Mouse output buffer full (data is from mouse, not keyboard) |

use alloc::collections::VecDeque;
use crate::error::Result;
use crate::io::IoHandler;

/// Intel 8042-compatible PS/2 controller.
#[derive(Debug)]
pub struct Ps2Controller {
    /// Data ready to be read by the guest via port 0x60.
    /// Each entry is `(byte, is_mouse)` so STATUS_MOUSE_DATA tracks per-byte.
    output_buffer: VecDeque<(u8, bool)>,
    /// Controller status register.
    pub status: u8,
    /// Controller configuration byte (read/written via commands 0x20/0x60).
    pub command_byte: u8,
    /// When `Some(cmd)`, the next byte written to port 0x60 is a data
    /// argument for the specified controller command.
    pub expecting_data: Option<u8>,
    /// Whether the mouse port is enabled.
    pub mouse_enabled: bool,
    /// Whether the keyboard port is enabled.
    pub keyboard_enabled: bool,
    /// Active scancode set (1, 2, or 3). Defaults to scancode set 1.
    pub scancode_set: u8,
    /// Buffered mouse data packets.
    pub mouse_buffer: VecDeque<u8>,
    /// Buffered keyboard scancodes.
    pub keyboard_buffer: VecDeque<u8>,
    /// Whether the next device write (port 0x60) should go to the mouse
    /// (set by controller command 0xD4).
    write_to_mouse: bool,
    /// Whether the keyboard is expecting a parameter byte for a
    /// multi-byte device command (e.g., 0xED set LEDs, 0xF0 scancode set).
    kbd_expecting_param: Option<u8>,
    /// Set when the guest sends the system reset command (0xFE to port 0x64).
    pub reset_requested: bool,
    /// Last byte read from port 0x60 (latched; real hardware holds last value).
    last_read: u8,
    /// Set when new data enters the output buffer; cleared by the IRQ raiser.
    pub irq_needed: bool,
    /// Alternation flag for fair scheduling between keyboard and mouse buffers.
    mouse_priority: bool,
    /// Remaining bytes in the current 3-byte mouse movement packet.
    /// When > 0, update_output_buffer will ALWAYS serve from mouse_buffer.
    mouse_packet_remaining: u8,
}

/// Status register bit masks.
const STATUS_OUTPUT_FULL: u8 = 0x01;
const STATUS_INPUT_FULL: u8 = 0x02;
const STATUS_MOUSE_DATA: u8 = 0x20;

impl Ps2Controller {
    /// Create a new PS/2 controller with keyboard enabled and mouse disabled.
    pub fn new() -> Self {
        Ps2Controller {
            output_buffer: VecDeque::new(),
            status: 0,
            command_byte: 0x47, // keyboard interrupt enabled, translation on
            expecting_data: None,
            mouse_enabled: false,
            keyboard_enabled: true,
            scancode_set: 1,
            mouse_buffer: VecDeque::new(),
            keyboard_buffer: VecDeque::new(),
            write_to_mouse: false,
            kbd_expecting_param: None,
            reset_requested: false,
            last_read: 0,
            irq_needed: false,
            mouse_priority: false,
            mouse_packet_remaining: 0,
        }
    }

    /// Enqueue a keyboard make (press) scancode.
    pub fn key_press(&mut self, scancode: u8) {
        if self.keyboard_enabled {
            self.keyboard_buffer.push_back(scancode);
            self.update_output_buffer();
        }
    }

    /// Enqueue a keyboard break (release) scancode.
    pub fn key_release(&mut self, scancode: u8) {
        if self.keyboard_enabled {
            self.keyboard_buffer.push_back(scancode | 0x80);
            self.update_output_buffer();
        }
    }

    /// Enqueue mouse movement as one or more 3-byte PS/2 packets.
    ///
    /// Large deltas are split into multiple packets clamped to the 9-bit
    /// signed range (-255..=255) to avoid overflow bits, which cause Linux's
    /// psmouse driver to discard the entire packet.
    pub fn mouse_move(&mut self, dx: i16, dy: i16, buttons: u8) {
        if !self.mouse_enabled {
            return;
        }

        let mut rem_dx = dx as i32;
        let mut rem_dy = dy as i32;

        loop {
            // Clamp each chunk to the PS/2 9-bit signed range.
            let chunk_dx = rem_dx.clamp(-255, 255) as i16;
            let chunk_dy = rem_dy.clamp(-255, 255) as i16;

            let mut status_byte: u8 = buttons & 0x07;
            status_byte |= 0x08; // bit 3 always set
            if chunk_dx < 0 { status_byte |= 0x10; }
            if chunk_dy < 0 { status_byte |= 0x20; }
            // No overflow bits — we clamped to valid range.

            self.mouse_buffer.push_back(status_byte);
            self.mouse_buffer.push_back(chunk_dx as u8);
            self.mouse_buffer.push_back(chunk_dy as u8);

            if self.mouse_packet_remaining == 0 {
                self.mouse_packet_remaining = 3;
            }

            rem_dx -= chunk_dx as i32;
            rem_dy -= chunk_dy as i32;

            if rem_dx == 0 && rem_dy == 0 {
                break;
            }
        }

        self.update_output_buffer();
    }

    /// Transfer one byte from device buffers into the output buffer.
    ///
    /// Mouse movement packets are served atomically (all 3 bytes before
    /// any keyboard data) to prevent packet misalignment. Init responses
    /// and keyboard data alternate to prevent starvation.
    fn update_output_buffer(&mut self) {
        if self.status & STATUS_OUTPUT_FULL != 0 {
            return;
        }

        let byte_and_source = if self.mouse_packet_remaining > 0 && !self.mouse_buffer.is_empty() {
            self.mouse_packet_remaining -= 1;
            self.mouse_buffer.pop_front().map(|b| (b, true))
        } else {
            if self.mouse_packet_remaining > 0 && self.mouse_buffer.is_empty() {
                self.mouse_packet_remaining = 0;
            }
            let serve_mouse_first = self.mouse_priority
                && !self.mouse_buffer.is_empty()
                && !self.keyboard_buffer.is_empty();
            if serve_mouse_first {
                self.mouse_buffer.pop_front().map(|b| (b, true))
            } else if let Some(byte) = self.keyboard_buffer.pop_front() {
                Some((byte, false))
            } else {
                self.mouse_buffer.pop_front().map(|b| (b, true))
            }
        };

        if let Some((byte, is_mouse)) = byte_and_source {
            self.output_buffer.push_back((byte, is_mouse));
            self.status |= STATUS_OUTPUT_FULL;
            if is_mouse {
                self.status |= STATUS_MOUSE_DATA;
            } else {
                self.status &= !STATUS_MOUSE_DATA;
            }
            self.irq_needed = true;
            self.mouse_priority = !self.mouse_priority;
        }
    }

    /// Try to fill the output buffer if it's empty and device buffers have data.
    pub fn try_fill_output(&mut self) {
        if self.status & STATUS_OUTPUT_FULL == 0 {
            self.update_output_buffer();
        }
    }

    /// Handle a device command written to port 0x60 targeting the keyboard.
    fn handle_keyboard_data(&mut self, byte: u8) {
        if let Some(cmd) = self.kbd_expecting_param {
            self.kbd_expecting_param = None;
            match cmd {
                0xED => {
                    self.keyboard_buffer.push_back(0xFA);
                }
                0xF0 => {
                    if byte == 0 {
                        self.keyboard_buffer.push_back(0xFA);
                        self.keyboard_buffer.push_back(self.scancode_set);
                    } else if byte >= 1 && byte <= 3 {
                        self.scancode_set = byte;
                        self.keyboard_buffer.push_back(0xFA);
                    } else {
                        self.keyboard_buffer.push_back(0xFE);
                    }
                }
                0xF3 => {
                    self.keyboard_buffer.push_back(0xFA);
                }
                _ => {
                    self.keyboard_buffer.push_back(0xFA);
                }
            }
            self.update_output_buffer();
            return;
        }

        match byte {
            0xED => {
                self.keyboard_buffer.push_back(0xFA);
                self.kbd_expecting_param = Some(0xED);
            }
            0xF0 => {
                self.keyboard_buffer.push_back(0xFA);
                self.kbd_expecting_param = Some(0xF0);
            }
            0xF3 => {
                self.keyboard_buffer.push_back(0xFA);
                self.kbd_expecting_param = Some(0xF3);
            }
            0xF4 => {
                self.keyboard_enabled = true;
                self.keyboard_buffer.push_back(0xFA);
            }
            0xF5 => {
                self.keyboard_enabled = false;
                self.keyboard_buffer.push_back(0xFA);
            }
            0xF6 => {
                self.scancode_set = 1;
                self.keyboard_buffer.push_back(0xFA);
            }
            0xFF => {
                self.keyboard_buffer.push_back(0xFA);
                self.keyboard_buffer.push_back(0xAA);
                self.scancode_set = 1;
            }
            _ => {
                self.keyboard_buffer.push_back(0xFA);
            }
        }
        self.update_output_buffer();
    }

    /// Push a mouse init response directly into the output buffer.
    /// These are NOT movement packets and need their own is_mouse tag.
    fn push_mouse_response(&mut self, byte: u8) {
        self.output_buffer.push_back((byte, true));
        self.status |= STATUS_OUTPUT_FULL | STATUS_MOUSE_DATA;
        self.irq_needed = true;
    }

    /// Handle a device command written to port 0x60 targeting the mouse.
    fn handle_mouse_data(&mut self, byte: u8) {
        match byte {
            0xE6 => { self.push_mouse_response(0xFA); }
            0xE7 => { self.push_mouse_response(0xFA); }
            0xE8 => { self.push_mouse_response(0xFA); }
            0xE9 => {
                self.push_mouse_response(0xFA);
                let status = if self.mouse_enabled { 0x20 } else { 0x00 };
                self.push_mouse_response(status);
                self.push_mouse_response(0x02);
                self.push_mouse_response(100);
            }
            0xEA => { self.push_mouse_response(0xFA); }
            0xF0 => { self.push_mouse_response(0xFA); }
            0xF2 => {
                self.push_mouse_response(0xFA);
                self.push_mouse_response(0x00);
            }
            0xF3 => { self.push_mouse_response(0xFA); }
            0xF4 => {
                self.mouse_enabled = true;
                self.push_mouse_response(0xFA);
            }
            0xF5 => {
                self.mouse_enabled = false;
                self.push_mouse_response(0xFA);
            }
            0xFF => {
                self.push_mouse_response(0xFA);
                self.push_mouse_response(0xAA);
                self.push_mouse_response(0x00);
                self.mouse_enabled = false;
            }
            _ => {
                self.push_mouse_response(0xFA);
            }
        }
    }
}

impl IoHandler for Ps2Controller {
    fn read(&mut self, port: u16, _size: u8) -> Result<u32> {
        let val = match port {
            0x60 => {
                let (byte, is_mouse) = if let Some(entry) = self.output_buffer.pop_front() {
                    entry
                } else {
                    // Output buffer empty — return latched last value.
                    // Use the CURRENT STATUS_MOUSE_DATA since we have no per-byte info.
                    (self.last_read, self.status & STATUS_MOUSE_DATA != 0)
                };
                self.last_read = byte;
                // Clear output buffer full flag.
                self.status &= !STATUS_OUTPUT_FULL;
                self.status &= !STATUS_MOUSE_DATA;

                // Set STATUS_MOUSE_DATA for the NEXT byte in the buffer,
                // so port 0x64 reads reflect what the NEXT port 0x60 read
                // will return. This is critical for the i8042 handler which
                // checks status BEFORE reading data.
                if let Some(&(_, next_is_mouse)) = self.output_buffer.front() {
                    self.status |= STATUS_OUTPUT_FULL;
                    if next_is_mouse {
                        self.status |= STATUS_MOUSE_DATA;
                    }
                } else {
                    // Try to fill from device buffers.
                    self.update_output_buffer();
                }

                byte
            }
            0x64 => self.status,
            _ => 0xFF,
        };
        Ok(val as u32)
    }

    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        let byte = val as u8;
        match port {
            0x60 => {
                if let Some(cmd) = self.expecting_data.take() {
                    match cmd {
                        0x60 => {
                            self.command_byte = byte;
                        }
                        0xD1 => {
                            // Write Controller Output Port.
                            // Bit 0 = system reset (0 = reset), bit 1 = A20 gate.
                            // We don't emulate A20/reset here — just consume the byte.
                            // PS/2 output port write (A20 gate, system reset)
                        }
                        0xD3 => {
                            // Write to AUX output buffer (loopback test).
                            // Echo the byte back with AUXDATA set.
                            self.output_buffer.push_back((byte, true));
                            self.status |= STATUS_OUTPUT_FULL | STATUS_MOUSE_DATA;
                            self.irq_needed = true;
                        }
                        0xD4 => {
                            self.handle_mouse_data(byte);
                        }
                        _ => {}
                    }
                } else if self.write_to_mouse {
                    self.write_to_mouse = false;
                    self.handle_mouse_data(byte);
                } else {
                    self.handle_keyboard_data(byte);
                }
            }
            0x64 => {
                match byte {
                    0x20 => {
                        self.output_buffer.push_back((self.command_byte, false));
                        self.status |= STATUS_OUTPUT_FULL;
                        self.status &= !STATUS_MOUSE_DATA;
                    }
                    0x60 => {
                        self.expecting_data = Some(0x60);
                    }
                    0xA7 => {
                        // Disable AUX (mouse) port — set bit 5 of command byte.
                        self.mouse_enabled = false;
                        self.command_byte |= 0x20;
                    }
                    0xA8 => {
                        // Enable AUX (mouse) port — clear bit 5 of command byte.
                        self.mouse_enabled = true;
                        self.command_byte &= !0x20;
                    }
                    0xAA => {
                        // Controller self-test — return 0x55 (test passed).
                        self.output_buffer.push_back((0x55, false));
                        self.status |= STATUS_OUTPUT_FULL;
                        self.status &= !STATUS_MOUSE_DATA;
                    }
                    0xA9 => {
                        // AUX (mouse) interface test — return 0x00 (no error).
                        self.output_buffer.push_back((0x00, false));
                        self.status |= STATUS_OUTPUT_FULL;
                        self.status &= !STATUS_MOUSE_DATA;
                    }
                    0xAB => {
                        // Keyboard interface test — return 0x00 (no error).
                        self.output_buffer.push_back((0x00, false));
                        self.status |= STATUS_OUTPUT_FULL;
                        self.status &= !STATUS_MOUSE_DATA;
                    }
                    0xAD => {
                        // Disable keyboard — set bit 4 of command byte.
                        self.keyboard_enabled = false;
                        self.command_byte |= 0x10;
                    }
                    0xAE => {
                        // Enable keyboard — clear bit 4 of command byte.
                        self.keyboard_enabled = true;
                        self.command_byte &= !0x10;
                    }
                    0xD0 => {
                        // Read Controller Output Port.
                        // Bits: 0=system reset (1=normal), 1=A20 gate,
                        //   4=IRQ1 output, 5=IRQ12 output, 6=kbd clock, 7=kbd data
                        // Return 0xCF: system running, A20 enabled, IRQ outputs active,
                        //   kbd clock+data high.
                        let output_port: u8 = 0xCF
                            | if self.status & STATUS_OUTPUT_FULL != 0 { 0x10 } else { 0 };
                        self.output_buffer.push_back((output_port, false));
                        self.status |= STATUS_OUTPUT_FULL;
                        self.status &= !STATUS_MOUSE_DATA;
                    }
                    0xD1 => {
                        // Write Controller Output Port — next byte to port 0x60
                        // sets output port bits (A20 gate, system reset).
                        self.expecting_data = Some(0xD1);
                    }
                    0xD3 => {
                        // Write next byte to AUX output buffer.
                        // The byte written to port 0x60 appears in the output
                        // buffer with AUXDATA set, as if it came from the mouse.
                        // Linux uses this as a loopback test to detect the AUX port.
                        self.expecting_data = Some(0xD3);
                    }
                    0xD4 => {
                        // Next byte written to port 0x60 goes to the mouse device.
                        self.expecting_data = Some(0xD4);
                    }
                    0xFE => {
                        self.reset_requested = true;
                    }
                    0xFF => {
                        // Pulse all output port lines — no-op.
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(())
    }
}

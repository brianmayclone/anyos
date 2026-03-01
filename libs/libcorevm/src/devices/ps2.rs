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
    pub output_buffer: VecDeque<u8>,
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
    /// Active scancode set (1, 2, or 3). Defaults to scancode set 2.
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
            scancode_set: 2,
            mouse_buffer: VecDeque::new(),
            keyboard_buffer: VecDeque::new(),
            write_to_mouse: false,
            kbd_expecting_param: None,
        }
    }

    /// Enqueue a keyboard make (press) scancode.
    ///
    /// The scancode is pushed into the keyboard buffer and will be
    /// delivered to the guest on the next read from port 0x60.
    pub fn key_press(&mut self, scancode: u8) {
        if self.keyboard_enabled {
            self.keyboard_buffer.push_back(scancode);
            self.update_output_buffer();
        }
    }

    /// Enqueue a keyboard break (release) scancode.
    ///
    /// For scancode set 2, the break code is the two-byte sequence
    /// `0xF0, scancode`. For set 1, the break code is `scancode | 0x80`.
    pub fn key_release(&mut self, scancode: u8) {
        if self.keyboard_enabled {
            if self.scancode_set == 1 {
                self.keyboard_buffer.push_back(scancode | 0x80);
            } else {
                // Scancode set 2 (and 3): break prefix + make code.
                self.keyboard_buffer.push_back(0xF0);
                self.keyboard_buffer.push_back(scancode);
            }
            self.update_output_buffer();
        }
    }

    /// Enqueue a 3-byte mouse movement packet.
    ///
    /// # Arguments
    /// - `dx`: horizontal displacement (-256..255)
    /// - `dy`: vertical displacement (-256..255)
    /// - `buttons`: button state (bit 0=left, bit 1=right, bit 2=middle)
    pub fn mouse_move(&mut self, dx: i16, dy: i16, buttons: u8) {
        if !self.mouse_enabled {
            return;
        }

        // Byte 0: status byte — buttons, sign bits, overflow bits.
        let mut status_byte: u8 = buttons & 0x07;
        // Bit 3 is always set in standard PS/2 mouse protocol.
        status_byte |= 0x08;
        if dx < 0 {
            status_byte |= 0x10; // X sign bit
        }
        if dy < 0 {
            status_byte |= 0x20; // Y sign bit
        }
        // Overflow bits (bits 6, 7) — set if magnitude exceeds 255.
        if dx > 255 || dx < -256 {
            status_byte |= 0x40;
        }
        if dy > 255 || dy < -256 {
            status_byte |= 0x80;
        }

        self.mouse_buffer.push_back(status_byte);
        self.mouse_buffer.push_back(dx as u8); // low 8 bits of dx
        self.mouse_buffer.push_back(dy as u8); // low 8 bits of dy

        self.update_output_buffer();
    }

    /// Transfer buffered device data into the output buffer for guest reading.
    ///
    /// Keyboard data takes priority over mouse data. The status register
    /// is updated to reflect whether data is available and its source.
    fn update_output_buffer(&mut self) {
        if self.status & STATUS_OUTPUT_FULL != 0 {
            // Output buffer already has data; do not overwrite.
            return;
        }

        if let Some(byte) = self.keyboard_buffer.pop_front() {
            self.output_buffer.push_back(byte);
            self.status |= STATUS_OUTPUT_FULL;
            self.status &= !STATUS_MOUSE_DATA;
        } else if let Some(byte) = self.mouse_buffer.pop_front() {
            self.output_buffer.push_back(byte);
            self.status |= STATUS_OUTPUT_FULL;
            self.status |= STATUS_MOUSE_DATA;
        }
    }

    /// Handle a device command written to port 0x60 targeting the keyboard.
    fn handle_keyboard_data(&mut self, byte: u8) {
        if let Some(cmd) = self.kbd_expecting_param {
            self.kbd_expecting_param = None;
            match cmd {
                0xED => {
                    // Set LEDs — acknowledge and ignore the LED state.
                    self.keyboard_buffer.push_back(0xFA);
                }
                0xF0 => {
                    // Set scancode set.
                    if byte == 0 {
                        // Query: respond with current set.
                        self.keyboard_buffer.push_back(0xFA);
                        self.keyboard_buffer.push_back(self.scancode_set);
                    } else if byte >= 1 && byte <= 3 {
                        self.scancode_set = byte;
                        self.keyboard_buffer.push_back(0xFA);
                    } else {
                        self.keyboard_buffer.push_back(0xFE); // resend
                    }
                }
                0xF3 => {
                    // Set typematic rate/delay — acknowledge and ignore.
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
                // Set LEDs (next byte is the LED state).
                self.keyboard_buffer.push_back(0xFA);
                self.kbd_expecting_param = Some(0xED);
            }
            0xF0 => {
                // Get/set scancode set (next byte is the set number).
                self.keyboard_buffer.push_back(0xFA);
                self.kbd_expecting_param = Some(0xF0);
            }
            0xF3 => {
                // Set typematic rate/delay (next byte is the rate).
                self.keyboard_buffer.push_back(0xFA);
                self.kbd_expecting_param = Some(0xF3);
            }
            0xF4 => {
                // Enable scanning.
                self.keyboard_enabled = true;
                self.keyboard_buffer.push_back(0xFA);
            }
            0xF5 => {
                // Disable scanning.
                self.keyboard_enabled = false;
                self.keyboard_buffer.push_back(0xFA);
            }
            0xF6 => {
                // Set default parameters.
                self.scancode_set = 2;
                self.keyboard_buffer.push_back(0xFA);
            }
            0xFF => {
                // Reset keyboard.
                self.keyboard_buffer.push_back(0xFA); // ACK
                self.keyboard_buffer.push_back(0xAA); // self-test passed
                self.scancode_set = 2;
            }
            _ => {
                // Unknown command — ACK anyway (many guests expect this).
                self.keyboard_buffer.push_back(0xFA);
            }
        }
        self.update_output_buffer();
    }

    /// Handle a device command written to port 0x60 targeting the mouse.
    fn handle_mouse_data(&mut self, byte: u8) {
        match byte {
            0xF4 => {
                // Enable data reporting.
                self.mouse_enabled = true;
                self.mouse_buffer.push_back(0xFA);
            }
            0xF5 => {
                // Disable data reporting.
                self.mouse_enabled = false;
                self.mouse_buffer.push_back(0xFA);
            }
            0xFF => {
                // Reset mouse.
                self.mouse_buffer.push_back(0xFA); // ACK
                self.mouse_buffer.push_back(0xAA); // self-test passed
                self.mouse_buffer.push_back(0x00); // mouse ID
                self.mouse_enabled = false;
            }
            _ => {
                // ACK unknown commands.
                self.mouse_buffer.push_back(0xFA);
            }
        }
        self.update_output_buffer();
    }
}

impl IoHandler for Ps2Controller {
    /// Read from PS/2 controller ports.
    ///
    /// - Port 0x60: reads the next byte from the output buffer and updates
    ///   status flags.
    /// - Port 0x64: reads the status register.
    fn read(&mut self, port: u16, _size: u8) -> Result<u32> {
        let val = match port {
            0x60 => {
                let byte = self.output_buffer.pop_front().unwrap_or(0);
                // Clear output buffer full flag.
                self.status &= !STATUS_OUTPUT_FULL;
                self.status &= !STATUS_MOUSE_DATA;
                // Immediately try to fill with next available data.
                self.update_output_buffer();
                byte
            }
            0x64 => self.status,
            _ => 0xFF,
        };
        Ok(val as u32)
    }

    /// Write to PS/2 controller ports.
    ///
    /// - Port 0x60: data written here goes either to the controller (if
    ///   it is expecting a data byte for a command) or to the keyboard/mouse
    ///   device.
    /// - Port 0x64: controller commands.
    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        let byte = val as u8;
        match port {
            0x60 => {
                if let Some(cmd) = self.expecting_data.take() {
                    // Data byte for a controller command.
                    match cmd {
                        0x60 => {
                            // Write controller configuration byte.
                            self.command_byte = byte;
                        }
                        0xD4 => {
                            // Write to mouse.
                            self.handle_mouse_data(byte);
                        }
                        _ => {}
                    }
                } else if self.write_to_mouse {
                    self.write_to_mouse = false;
                    self.handle_mouse_data(byte);
                } else {
                    // Device command to keyboard.
                    self.handle_keyboard_data(byte);
                }
            }
            0x64 => {
                // Controller commands.
                match byte {
                    0x20 => {
                        // Read controller configuration byte.
                        self.output_buffer.push_back(self.command_byte);
                        self.status |= STATUS_OUTPUT_FULL;
                        self.status &= !STATUS_MOUSE_DATA;
                    }
                    0x60 => {
                        // Write controller configuration byte (next data byte).
                        self.expecting_data = Some(0x60);
                    }
                    0xA7 => {
                        // Disable mouse port.
                        self.mouse_enabled = false;
                    }
                    0xA8 => {
                        // Enable mouse port.
                        self.mouse_enabled = true;
                    }
                    0xAA => {
                        // Controller self-test — return 0x55 (test passed).
                        self.output_buffer.push_back(0x55);
                        self.status |= STATUS_OUTPUT_FULL;
                        self.status &= !STATUS_MOUSE_DATA;
                    }
                    0xAB => {
                        // Keyboard interface test — return 0x00 (no error).
                        self.output_buffer.push_back(0x00);
                        self.status |= STATUS_OUTPUT_FULL;
                        self.status &= !STATUS_MOUSE_DATA;
                    }
                    0xAD => {
                        // Disable keyboard.
                        self.keyboard_enabled = false;
                    }
                    0xAE => {
                        // Enable keyboard.
                        self.keyboard_enabled = true;
                    }
                    0xD4 => {
                        // Next byte written to port 0x60 goes to the mouse.
                        self.expecting_data = Some(0xD4);
                    }
                    _ => {
                        // Unknown controller command — ignore.
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

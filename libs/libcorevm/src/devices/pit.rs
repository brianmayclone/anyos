//! Intel 8253/8254 Programmable Interval Timer (PIT) emulation.
//!
//! Emulates a 3-channel PIT as found in the IBM PC/AT:
//! - **Channel 0**: system timer, connected to IRQ 0
//! - **Channel 1**: DRAM refresh (typically unused by modern guests)
//! - **Channel 2**: PC speaker tone generation
//!
//! # I/O Ports
//!
//! | Port | Description |
//! |------|-------------|
//! | 0x40 | Channel 0 count register |
//! | 0x41 | Channel 1 count register |
//! | 0x42 | Channel 2 count register |
//! | 0x43 | Mode/command register |

use crate::error::Result;
use crate::io::IoHandler;

/// State of a single PIT counter channel.
#[derive(Debug)]
pub struct PitChannel {
    /// Reload value written by the guest. When the counter reaches zero
    /// (or the output triggers), it is reloaded from this value.
    pub count: u16,
    /// Current output pin state. Interpretation depends on the operating mode.
    pub output: bool,
    /// Operating mode (0-5). See Intel 8254 datasheet for mode descriptions.
    pub mode: u8,
    /// Access mode: 0=latch, 1=lobyte only, 2=hibyte only, 3=lobyte/hibyte.
    pub access_mode: u8,
    /// BCD counting mode. When `true`, the counter counts in BCD (0-9999).
    pub bcd: bool,
    /// Latched count value, captured by a latch command.
    pub latch: u16,
    /// Whether a latch command has been issued and not yet fully read.
    pub latched: bool,
    /// For lobyte/hibyte access mode: next read returns the high byte.
    pub read_hi: bool,
    /// For lobyte/hibyte access mode: next write goes to the high byte.
    pub write_hi: bool,
    /// Gate input pin. Always `true` for channel 0 (directly wired).
    pub gate: bool,
    /// Whether counting is enabled (set after a full reload value is written).
    pub enabled: bool,
    /// Internal running counter that decrements on each tick.
    current: u16,
}

impl PitChannel {
    /// Create a new PIT channel in its power-on default state.
    pub fn new() -> Self {
        PitChannel {
            count: 0,
            output: false,
            mode: 0,
            access_mode: 3, // lobyte/hibyte
            bcd: false,
            latch: 0,
            latched: false,
            read_hi: false,
            write_hi: false,
            gate: true,
            enabled: false,
            current: 0,
        }
    }

    /// Read the current count value, respecting access mode and latch state.
    ///
    /// For lobyte/hibyte access mode, successive reads return the low byte
    /// then the high byte. If a latch command was issued, the latched value
    /// is returned instead of the live counter.
    fn read_count(&mut self) -> u8 {
        let value = if self.latched { self.latch } else { self.current };

        match self.access_mode {
            1 => {
                // Lobyte only.
                self.latched = false;
                value as u8
            }
            2 => {
                // Hibyte only.
                self.latched = false;
                (value >> 8) as u8
            }
            3 => {
                // Lobyte/hibyte alternating.
                if self.read_hi {
                    self.read_hi = false;
                    self.latched = false;
                    (value >> 8) as u8
                } else {
                    self.read_hi = true;
                    value as u8
                }
            }
            _ => {
                self.latched = false;
                value as u8
            }
        }
    }

    /// Write to the count register, respecting access mode.
    ///
    /// For lobyte/hibyte access mode, the first write sets the low byte
    /// and the second write sets the high byte. Counting begins after
    /// the full reload value has been written.
    fn write_count(&mut self, val: u8) {
        match self.access_mode {
            1 => {
                // Lobyte only.
                self.count = val as u16;
                self.current = self.count;
                self.enabled = true;
            }
            2 => {
                // Hibyte only.
                self.count = (val as u16) << 8;
                self.current = self.count;
                self.enabled = true;
            }
            3 => {
                // Lobyte/hibyte: two successive writes.
                if self.write_hi {
                    self.count = (self.count & 0x00FF) | ((val as u16) << 8);
                    self.current = self.count;
                    self.write_hi = false;
                    self.enabled = true;
                } else {
                    self.count = (self.count & 0xFF00) | (val as u16);
                    self.write_hi = true;
                }
            }
            _ => {}
        }
    }

    /// Advance the channel by one tick.
    ///
    /// Returns `true` if the channel's output transitions from low to high,
    /// indicating an interrupt should be raised (relevant for channel 0).
    fn tick(&mut self) -> bool {
        if !self.enabled || !self.gate {
            return false;
        }

        let prev_output = self.output;

        match self.mode {
            0 => {
                // Mode 0: Interrupt on terminal count.
                // Output starts low, goes high when count reaches zero.
                if self.current == 0 {
                    self.output = true;
                } else {
                    self.current = self.current.wrapping_sub(1);
                    if self.current == 0 {
                        self.output = true;
                    }
                }
            }
            2 => {
                // Mode 2: Rate generator.
                // Output is normally high, goes low for one tick when count
                // reaches 1, then reloads.
                if self.current <= 1 {
                    self.current = self.count;
                    self.output = true;
                } else {
                    self.current = self.current.wrapping_sub(1);
                    if self.current == 1 {
                        self.output = false;
                    } else {
                        self.output = true;
                    }
                }
            }
            3 => {
                // Mode 3: Square wave generator.
                // Output toggles when the counter reaches zero, reloads
                // with half the reload value each time.
                if self.current <= 1 {
                    self.output = !self.output;
                    self.current = self.count;
                } else {
                    self.current = self.current.wrapping_sub(2);
                }
            }
            _ => {
                // Modes 1, 4, 5: simplified — just decrement.
                if self.current == 0 {
                    self.output = true;
                    self.current = self.count;
                } else {
                    self.current = self.current.wrapping_sub(1);
                }
            }
        }

        // Return true on a rising edge (low-to-high transition).
        !prev_output && self.output
    }
}

/// Three-channel Intel 8253/8254 PIT.
#[derive(Debug)]
pub struct Pit {
    /// The three counter channels (0, 1, 2).
    pub channels: [PitChannel; 3],
}

impl Pit {
    /// Create a new PIT with all channels in their power-on default state.
    pub fn new() -> Self {
        Pit {
            channels: [PitChannel::new(), PitChannel::new(), PitChannel::new()],
        }
    }

    /// Advance all channels by one tick.
    ///
    /// Returns `true` if channel 0's output fires (IRQ 0 should be raised).
    pub fn tick(&mut self) -> bool {
        let irq = self.channels[0].tick();
        self.channels[1].tick();
        self.channels[2].tick();
        irq
    }
}

impl IoHandler for Pit {
    /// Read from PIT ports.
    ///
    /// - 0x40-0x42: read channel 0-2 count register
    /// - 0x43: not readable (returns 0xFF)
    fn read(&mut self, port: u16, _size: u8) -> Result<u32> {
        let val = match port {
            0x40 => self.channels[0].read_count(),
            0x41 => self.channels[1].read_count(),
            0x42 => self.channels[2].read_count(),
            _ => 0xFF,
        };
        Ok(val as u32)
    }

    /// Write to PIT ports.
    ///
    /// - 0x40-0x42: write channel 0-2 count register
    /// - 0x43: mode/command word — selects channel, access mode, and
    ///   operating mode
    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        let byte = val as u8;
        match port {
            0x40 => self.channels[0].write_count(byte),
            0x41 => self.channels[1].write_count(byte),
            0x42 => self.channels[2].write_count(byte),
            0x43 => {
                // Mode/command register.
                let channel_idx = ((byte >> 6) & 0x03) as usize;
                if channel_idx == 3 {
                    // Read-back command (8254 only) — not implemented.
                    return Ok(());
                }

                let access = (byte >> 4) & 0x03;
                if access == 0 {
                    // Counter latch command: snapshot current count.
                    self.channels[channel_idx].latch = self.channels[channel_idx].count;
                    self.channels[channel_idx].latched = true;
                    self.channels[channel_idx].read_hi = false;
                } else {
                    // Set channel operating parameters.
                    let ch = &mut self.channels[channel_idx];
                    ch.access_mode = access;
                    ch.mode = (byte >> 1) & 0x07;
                    ch.bcd = byte & 0x01 != 0;
                    ch.output = false;
                    ch.enabled = false;
                    ch.write_hi = false;
                    ch.read_hi = false;
                    ch.latched = false;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

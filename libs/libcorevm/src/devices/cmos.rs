//! CMOS RTC and NVRAM emulation.
//!
//! Emulates the Motorola MC146818 (or compatible) CMOS real-time clock
//! and 128 bytes of battery-backed NVRAM found in the IBM PC/AT.
//!
//! # I/O Ports
//!
//! | Port | Description |
//! |------|-------------|
//! | 0x70 | Index register (write selects CMOS address; bit 7 controls NMI) |
//! | 0x71 | Data register (read/write the selected CMOS address) |
//!
//! # NVRAM Layout
//!
//! - `0x00-0x09`: RTC time fields
//! - `0x0A-0x0D`: Status registers A-D
//! - `0x0F`: Shutdown status
//! - `0x10`: Floppy drive types
//! - `0x14`: Equipment byte
//! - `0x15-0x16`: Base memory size (KB)
//! - `0x17-0x18`: Extended memory size above 1 MB (KB)
//! - `0x30-0x31`: Extended memory above 1 MB (KB, duplicate)
//! - `0x34-0x35`: Extended memory above 16 MB (64 KB units)

use crate::error::Result;
use crate::io::IoHandler;
#[cfg(feature = "host_test")]
use core::sync::atomic::{AtomicU32, Ordering};

#[cfg(feature = "host_test")]
static CMOS_LOG_BUDGET: AtomicU32 = AtomicU32::new(32);

#[cfg(feature = "host_test")]
fn cmos_log(args: core::fmt::Arguments<'_>) {
    if CMOS_LOG_BUDGET.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
        (n > 0).then_some(n - 1)
    }).is_ok() {
        eprintln!("[cmos] {args}");
    }
}

/// CMOS RTC and NVRAM controller.
#[derive(Debug)]
pub struct Cmos {
    /// Currently selected CMOS address (low 7 bits of port 0x70 write).
    pub index: u8,
    /// 128 bytes of NVRAM contents.
    pub data: [u8; 128],
    /// NMI disable flag (bit 7 of port 0x70).
    pub nmi_disabled: bool,
    /// Counter for simulating the RTC Update-In-Progress (UIP) cycle.
    /// Incremented on each read of status register A; UIP bit toggles
    /// to simulate the once-per-second update cycle.
    uip_counter: u32,
    /// Accumulated 32.768 kHz RTC base ticks toward the next periodic event.
    periodic_tick_credit: u64,
    /// Whether IRQF is currently asserted until status register C is read.
    irq_latched: bool,
}

impl Cmos {
    /// Create a new CMOS device pre-populated with memory size information.
    ///
    /// `ram_size_bytes` is the total guest RAM size. CMOS reports only the
    /// portion below the PCI hole (3.5 GB). RAM above that is relocated
    /// above 4 GB and reported via fw_cfg/e820 instead.
    pub fn new(ram_size_bytes: usize) -> Self {
        const PCI_HOLE_START: usize = 0xE000_0000; // 3.5 GB
        const PCI_HOLE_END: usize   = 0x1_0000_0000; // 4 GB

        // RAM below PCI hole (for below-4G CMOS fields).
        let ram_below = if ram_size_bytes > PCI_HOLE_START { PCI_HOLE_START } else { ram_size_bytes };
        // RAM above 4 GB (relocated from the PCI hole region).
        let ram_above_4g: usize = if ram_size_bytes > PCI_HOLE_START { ram_size_bytes - PCI_HOLE_START } else { 0 };

        let mut data = [0u8; 128];

        // Status Register A: divider = 010 (32.768 kHz), rate = 0110 (1024 Hz).
        data[0x0A] = 0x26;
        // Status Register B: 24-hour mode, BCD format, no interrupts.
        // BCD is the standard for PC-compatible RTC and is what SeaBIOS
        // and Linux expect by default.
        data[0x0B] = 0x02; // bit 1 = 24h, bit 2 = 0 → BCD mode
        // Status Register C: no interrupt flags pending.
        data[0x0C] = 0x00;
        // Status Register D: RTC valid (battery OK).
        data[0x0D] = 0x80;

        // Shutdown status: normal POST.
        data[0x0F] = 0x00;

        // Floppy drive types: none installed.
        data[0x10] = 0x00;

        // Equipment byte: bit 1 = math coprocessor present.
        data[0x14] = 0x02;

        // Base memory: 640 KB (0x0280).
        data[0x15] = 0x80; // low byte
        data[0x16] = 0x02; // high byte

        // Extended memory from 1 MB to 16 MB (in KB).
        // Registers 0x17/0x18 and 0x30/0x31 represent only the 1MB-16MB range,
        // capped at 15360 KB (= 15 MB = 16 MB - 1 MB). Memory above 16 MB is
        // reported separately in registers 0x34/0x35.
        let extended_kb = if ram_below > 1024 * 1024 {
            let ext = (ram_below - 1024 * 1024) / 1024;
            // Cap at 15360 KB (the 1MB-16MB range).
            if ext > 15360 { 15360u16 } else { ext as u16 }
        } else {
            0
        };
        data[0x17] = extended_kb as u8;
        data[0x18] = (extended_kb >> 8) as u8;
        data[0x30] = extended_kb as u8;
        data[0x31] = (extended_kb >> 8) as u8;

        // Extended memory above 16 MB (in 64 KB units), below PCI hole only.
        let above_16mb = if ram_below > 16 * 1024 * 1024 {
            let units = (ram_below - 16 * 1024 * 1024) / (64 * 1024);
            if units > 0xFFFF { 0xFFFF } else { units as u16 }
        } else {
            0
        };
        data[0x34] = above_16mb as u8;
        data[0x35] = (above_16mb >> 8) as u8;

        // Memory above 4 GB in 64KB units (CMOS registers 0x5B-0x5D, 3 bytes).
        // SeaBIOS reads these for RamSizeOver4G.
        if ram_above_4g > 0 {
            let units_64k = ram_above_4g / (64 * 1024);
            data[0x5B] = units_64k as u8;
            data[0x5C] = (units_64k >> 8) as u8;
            data[0x5D] = (units_64k >> 16) as u8;
        }

        // Initialize RTC time from host clock (UTC).
        // Register B bit 2 = binary mode, bit 1 = 24h mode.
        #[cfg(feature = "std")]
        {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            // BCD encoding helper
            let bin2bcd = |v: u8| -> u8 { ((v / 10) << 4) | (v % 10) };
            // Convert Unix timestamp to broken-down time (UTC).
            let days = secs / 86400;
            let day_secs = secs % 86400;
            let sec = (day_secs % 60) as u8;
            let min = ((day_secs / 60) % 60) as u8;
            let hour = (day_secs / 3600) as u8;
            data[0x00] = bin2bcd(sec);
            data[0x02] = bin2bcd(min);
            data[0x04] = bin2bcd(hour);
            // Day-of-week: 1970-01-01 was Thursday (day_of_week=4).
            data[0x06] = ((days + 4) % 7 + 1) as u8;    // 1=Sun..7=Sat (not BCD)
            // Year/month/day from days since 1970-01-01.
            let (y, m, d) = {
                let mut y = 1970u32;
                let mut rem = days;
                loop {
                    let ydays = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366u64 } else { 365 };
                    if rem < ydays { break; }
                    rem -= ydays;
                    y += 1;
                }
                let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
                let mdays: [u8; 12] = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
                let mut m = 0u32;
                for md in &mdays {
                    if rem < *md as u64 { break; }
                    rem -= *md as u64;
                    m += 1;
                }
                (y, m + 1, rem as u32 + 1)
            };
            data[0x07] = bin2bcd(d as u8);
            data[0x08] = bin2bcd(m as u8);
            data[0x09] = bin2bcd((y % 100) as u8);
            data[0x32] = bin2bcd((y / 100) as u8);       // century (BCD)
        }
        #[cfg(not(feature = "std"))]
        {
            // Century register: BCD 0x20 for year 20xx.
            data[0x32] = 0x20;
        }

        Cmos {
            index: 0,
            data,
            nmi_disabled: false,
            uip_counter: 0,
            periodic_tick_credit: 0,
            irq_latched: false,
        }
    }

    fn periodic_rate_select(&self) -> u8 {
        self.data[0x0A] & 0x0F
    }

    fn periodic_interval_ticks(&self) -> Option<u64> {
        let rate = self.periodic_rate_select();
        if !(3..=15).contains(&rate) {
            return None;
        }
        Some(1u64 << (rate - 1))
    }

    fn periodic_irq_enabled(&self) -> bool {
        (self.data[0x0B] & 0x40) != 0
    }

    /// Whether RTC periodic mode is currently configured and can raise IRQ8.
    pub fn periodic_irq_armed(&self) -> bool {
        self.periodic_irq_enabled() && self.periodic_interval_ticks().is_some()
    }

    /// Advance the RTC base clock by `ticks_32768` ticks.
    ///
    /// Returns `true` when a new IRQ8 edge should be raised.
    pub fn advance(&mut self, ticks_32768: u64) -> bool {
        let Some(interval) = self.periodic_interval_ticks() else {
            return false;
        };
        self.periodic_tick_credit = self.periodic_tick_credit.saturating_add(ticks_32768);
        if self.periodic_tick_credit < interval {
            return false;
        }

        // Consume all accumulated intervals (avoid IRQ starvation when
        // advance() is called infrequently relative to the periodic rate).
        self.periodic_tick_credit %= interval;
        if !self.periodic_irq_enabled() {
            return false;
        }

        self.data[0x0C] |= 0x40 | 0x80; // PF | IRQF
        if self.irq_latched {
            return false;
        }
        self.irq_latched = true;
        #[cfg(feature = "host_test")]
        cmos_log(format_args!(
            "periodic irq rate_sel={:X} statusC={:02X}",
            self.periodic_rate_select(),
            self.data[0x0C]
        ));
        true
    }
}

impl IoHandler for Cmos {
    /// Read from CMOS ports.
    ///
    /// - Port 0x70: returns last written index | NMI-disable bit.
    /// - Port 0x71: returns the NVRAM byte at the currently selected index.
    ///   Reading status register C (0x0C) clears all interrupt flags.
    fn read(&mut self, port: u16, _size: u8) -> Result<u32> {
        let val = match port {
            0x70 => {
                // Return the last written value (index + NMI bit).
                // Windows 10 bootmgr reads this to check NMI status.
                self.index | if self.nmi_disabled { 0x80 } else { 0x00 }
            }
            0x71 => {
                let idx = (self.index & 0x7F) as usize;
                let mut v = self.data[idx];
                if idx == 0x0A {
                    // Simulate UIP (Update In Progress) toggle in Status Register A.
                    // Real MC146818 sets bit 7 for ~244µs once per second during the
                    // time update. The Windows HAL polls this in a tight loop to
                    // calibrate timing. Toggle UIP every 512 reads to simulate the
                    // update cycle boundary.
                    let prev_phase = (self.uip_counter / 512) & 1;
                    self.uip_counter = self.uip_counter.wrapping_add(1);
                    let new_phase = (self.uip_counter / 512) & 1;
                    if new_phase != 0 {
                        v |= 0x80; // UIP set
                    } else {
                        v &= !0x80; // UIP clear
                    }
                    // When UIP transitions from set→clear, advance the seconds counter
                    // (simulating that the RTC update completed).
                    if prev_phase != 0 && new_phase == 0 {
                        let is_binary = (self.data[0x0B] & 0x04) != 0;
                        if is_binary {
                            self.data[0x00] = self.data[0x00].wrapping_add(1);
                            if self.data[0x00] >= 60 { self.data[0x00] = 0; }
                        } else {
                            // BCD increment
                            let mut sec = self.data[0x00];
                            let lo = sec & 0x0F;
                            let hi = sec >> 4;
                            let s = hi * 10 + lo + 1;
                            if s >= 60 {
                                sec = 0;
                            } else {
                                sec = ((s / 10) << 4) | (s % 10);
                            }
                            self.data[0x00] = sec;
                        }
                    }
                }
                // Reading status register C clears all interrupt flags.
                if idx == 0x0C {
                    self.data[0x0C] = 0x00;
                    self.irq_latched = false;
                }
                v
            }
            _ => 0xFF,
        };
        Ok(val as u32)
    }

    /// Write to CMOS ports.
    ///
    /// - Port 0x70: selects the CMOS address (bits 0-6) and NMI disable
    ///   flag (bit 7).
    /// - Port 0x71: writes a byte to the NVRAM at the currently selected
    ///   index. Status registers C and D are read-only and writes are
    ///   ignored.
    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        let byte = val as u8;
        match port {
            0x70 => {
                self.index = byte & 0x7F;
                self.nmi_disabled = byte & 0x80 != 0;
            }
            0x71 => {
                let idx = (self.index & 0x7F) as usize;
                // Status register C (0x0C) and D (0x0D) are read-only.
                if idx != 0x0C && idx != 0x0D {
                    #[cfg(feature = "host_test")]
                    if idx == 0x0A || idx == 0x0B {
                        cmos_log(format_args!("write reg {:02X} = {:02X}", idx, byte));
                    }
                    self.data[idx] = byte;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

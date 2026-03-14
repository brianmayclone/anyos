//! HPET (High Precision Event Timer) emulation.
//!
//! Emulates a minimal HPET at MMIO base 0xFED0_0000 (1KB region).
//! Provides a single timer (Timer 0) that can fire IRQ 0 or 2 via IOAPIC.
//!
//! Windows 7 ACPI HAL uses HPET as its primary timer source when LAPIC timer
//! calibration fails or when no HPET-less fallback is available.
//!
//! # Register Map (offsets from base 0xFED0_0000)
//!
//! | Offset | Size | Register                    |
//! |--------|------|-----------------------------|
//! | 0x000  |  8   | General Capabilities & ID   |
//! | 0x010  |  8   | General Configuration       |
//! | 0x020  |  8   | General Interrupt Status     |
//! | 0x0F0  |  8   | Main Counter Value           |
//! | 0x100  |  8   | Timer 0 Config & Capabilities|
//! | 0x108  |  8   | Timer 0 Comparator Value     |
//! | 0x110  |  8   | Timer 0 FSB Interrupt Route  |

use crate::error::Result;
use crate::memory::mmio::MmioHandler;

/// HPET counter frequency: ~14.318 MHz (period = ~69.841 ns = 69841279 femtoseconds).
/// This is the standard HPET frequency used by QEMU and real hardware.
const HPET_CLK_PERIOD_FS: u64 = 69_841_279; // femtoseconds per tick (~14.318 MHz)

/// HPET MMIO region base address.
pub const HPET_BASE: u64 = 0xFED0_0000;

/// HPET MMIO region size (1KB).
pub const HPET_SIZE: u64 = 0x400;

/// HPET device state.
#[derive(Debug)]
pub struct Hpet {
    /// General Configuration Register.
    /// Bit 0: ENABLE_CNF (overall enable).
    /// Bit 1: LEG_RT_CNF (legacy replacement routing).
    config: u64,

    /// Main counter value (64-bit free-running counter).
    counter: u64,

    /// General Interrupt Status Register.
    /// Bit N = Timer N interrupt active (write-1-to-clear).
    int_status: u64,

    /// Timer 0 configuration and capabilities.
    /// Bits [0]: reserved
    /// Bit 1: Tn_INT_TYPE_CNF (0=edge, 1=level)
    /// Bit 2: Tn_INT_ENB_CNF (interrupt enable)
    /// Bit 3: Tn_TYPE_CNF (0=one-shot, 1=periodic)
    /// Bit 4: Tn_PER_INT_CAP (periodic capable) — read-only, we set 1
    /// Bit 5: Tn_SIZE_CAP (64-bit capable) — read-only, we set 1
    /// Bit 6: Tn_VAL_SET_CNF (set accumulator for periodic)
    /// Bits [9:13]: Tn_INT_ROUTE_CNF (IOAPIC routing)
    /// Bits [32:63]: Tn_INT_ROUTE_CAP (allowed routes) — read-only
    timer0_config: u64,

    /// Timer 0 comparator value.
    timer0_comparator: u64,

    /// Timer 0 period (for periodic mode, stored when VAL_SET is written).
    timer0_period: u64,

    /// Timer 1 and Timer 2 config (stub, stored for read-back).
    timer1_config: u64,
    timer2_config: u64,

    /// Wall-clock epoch for real-time counter.
    #[cfg(feature = "std")]
    epoch: std::time::Instant,

    /// Whether Timer 0 IRQ is currently asserted (for level-triggered).
    pub irq_pending: bool,
}

impl Hpet {
    /// Create a new HPET device.
    pub fn new() -> Self {
        Hpet {
            config: 0,
            counter: 0,
            int_status: 0,
            // Capabilities: periodic capable (bit 4), 64-bit (bit 5)
            // INT_ROUTE_CAP: allow IRQ 0,2,8 (bits 32+0, 32+2, 32+8)
            timer0_config: (1 << 4) | (1 << 5) | ((1u64 | (1u64 << 2) | (1u64 << 8)) << 32),
            timer0_comparator: 0,
            timer0_period: 0,
            // Timer 1,2: 64-bit capable (bit 5), same route caps
            timer1_config: (1 << 5) | ((1u64 | (1u64 << 2) | (1u64 << 8)) << 32),
            timer2_config: (1 << 5) | ((1u64 | (1u64 << 2) | (1u64 << 8)) << 32),
            #[cfg(feature = "std")]
            epoch: std::time::Instant::now(),
            irq_pending: false,
        }
    }

    /// Get the current counter value. Uses wall-clock time on std platforms.
    fn counter_value(&self) -> u64 {
        if self.config & 1 == 0 {
            // Counter halted
            return self.counter;
        }
        #[cfg(feature = "std")]
        {
            let elapsed_ns = self.epoch.elapsed().as_nanos() as u64;
            // ticks = elapsed_ns * 1e6 / HPET_CLK_PERIOD_FS
            // = elapsed_ns * 1_000_000 / 69_841_279
            // Simplify: ≈ elapsed_ns * 14.318
            elapsed_ns.wrapping_mul(1_000_000) / HPET_CLK_PERIOD_FS
        }
        #[cfg(not(feature = "std"))]
        {
            self.counter
        }
    }

    /// Check if Timer 0 has fired (comparator match) and should assert IRQ.
    /// Called from the VM poll loop. In periodic mode, catches up to the
    /// current counter value by advancing the comparator by as many periods
    /// as needed, so timer interrupts are not lost.
    pub fn check_timer(&mut self) -> bool {
        // Must be enabled
        if self.config & 1 == 0 {
            return false;
        }
        // Timer 0 interrupt must be enabled
        if self.timer0_config & (1 << 2) == 0 {
            return false;
        }

        let counter = self.counter_value();
        let cmp = self.timer0_comparator;

        // Check if counter has passed comparator
        if cmp == 0 || counter < cmp {
            return false;
        }

        // Timer fired!
        let is_periodic = self.timer0_config & (1 << 3) != 0;
        if is_periodic && self.timer0_period > 0 {
            // Catch up: advance comparator past current counter
            let elapsed = counter - cmp;
            let periods = (elapsed / self.timer0_period) + 1;
            self.timer0_comparator = cmp.wrapping_add(self.timer0_period * periods);
        } else {
            // One-shot: clear comparator to prevent re-firing
            self.timer0_comparator = 0;
        }

        // Set interrupt status bit 0
        let is_level = self.timer0_config & (1 << 1) != 0;
        if is_level {
            self.int_status |= 1;
        }
        self.irq_pending = true;
        true
    }

    /// Get the IOAPIC IRQ number for Timer 0.
    /// In legacy replacement mode, Timer 0 replaces the 8254 PIT.
    /// On ACPI systems with IOAPIC, PIT IRQ 0 is typically routed to GSI 2.
    pub fn timer0_irq(&self) -> u32 {
        if self.config & 2 != 0 {
            // Legacy replacement mode: Timer 0 → IRQ 2 (PIT legacy routing via IOAPIC)
            2
        } else {
            // Normal routing from config bits [13:9]
            ((self.timer0_config >> 9) & 0x1F) as u32
        }
    }

    /// Build the General Capabilities & ID register value.
    fn capabilities(&self) -> u64 {
        // Bits [7:0]: REV_ID = 1
        // Bits [12:8]: NUM_TIM_CAP = 0 (1 timer, value is N-1)
        // Bit 13: COUNT_SIZE_CAP = 1 (64-bit counter)
        // Bit 15: LEG_RT_CAP = 1 (legacy replacement capable)
        // Bits [31:16]: VENDOR_ID = 0x8086 (Intel, for compatibility)
        // Bits [63:32]: CLK_PERIOD = HPET_CLK_PERIOD_FS (in femtoseconds)
        let lo: u32 = 0x01       // REV_ID = 1
            | (2 << 8)           // NUM_TIM_CAP = 2 (3 timers, matching QEMU)
            | (1 << 13)          // COUNT_SIZE_CAP = 1 (64-bit)
            | (1 << 15)          // LEG_RT_CAP = 1
            | (0x8086 << 16);    // VENDOR_ID
        (lo as u64) | ((HPET_CLK_PERIOD_FS as u64) << 32)
    }
}

impl MmioHandler for Hpet {
    fn read(&mut self, offset: u64, size: u8) -> Result<u64> {
        let val = match offset & !7 {
            // General Capabilities & ID
            0x000 => self.capabilities(),
            // General Configuration
            0x010 => self.config,
            // General Interrupt Status
            0x020 => self.int_status,
            // Main Counter Value
            0x0F0 => self.counter_value(),
            // Timer 0 Configuration & Capabilities
            0x100 => self.timer0_config,
            // Timer 0 Comparator Value
            0x108 => self.timer0_comparator,
            // Timer 0 FSB Interrupt Route
            0x110 => 0,
            // Timer 1 Configuration & Capabilities
            0x120 => self.timer1_config,
            // Timer 1 Comparator
            0x128 => 0,
            // Timer 2 Configuration & Capabilities
            0x140 => self.timer2_config,
            // Timer 2 Comparator
            0x148 => 0,
            _ => 0,
        };

        // Handle sub-register access (e.g., reading upper 32 bits)
        let shift = ((offset & 7) * 8) as u64;
        let shifted = val >> shift;
        let masked = match size {
            1 => shifted & 0xFF,
            2 => shifted & 0xFFFF,
            4 => shifted & 0xFFFF_FFFF,
            _ => shifted,
        };
        Ok(masked)
    }

    fn write(&mut self, offset: u64, size: u8, val: u64) -> Result<()> {
        // Build the full 64-bit value considering sub-register writes
        let shift = ((offset & 7) * 8) as u64;
        let aligned = offset & !7;

        match aligned {
            // General Configuration
            0x010 => {
                let mask = match size {
                    1 => 0xFF_u64,
                    2 => 0xFFFF,
                    4 => 0xFFFF_FFFF,
                    _ => u64::MAX,
                } << shift;
                let old = self.config;
                self.config = (old & !mask) | ((val << shift) & mask);

                // If just enabled, reset epoch for accurate timing
                #[cfg(feature = "std")]
                if old & 1 == 0 && self.config & 1 != 0 {
                    self.epoch = std::time::Instant::now();
                }
            }
            // General Interrupt Status (write-1-to-clear)
            0x020 => {
                let clear_bits = (val << shift) & 0xFFFF_FFFF;
                self.int_status &= !clear_bits;
                if self.int_status & 1 == 0 {
                    self.irq_pending = false;
                }
            }
            // Main Counter Value (writable when counter is halted)
            0x0F0 => {
                if self.config & 1 == 0 {
                    let mask = match size {
                        1 => 0xFF_u64,
                        2 => 0xFFFF,
                        4 => 0xFFFF_FFFF,
                        _ => u64::MAX,
                    } << shift;
                    self.counter = (self.counter & !mask) | ((val << shift) & mask);
                }
            }
            // Timer 0 Configuration
            0x100 => {
                // Read-only bits: 0,4,5,7,15,16-31,32-63
                // Writable: 1(INT_TYPE), 2(INT_ENB), 3(TYPE), 6(VAL_SET), 8(32MODE), 9-13(INT_ROUTE), 14(FSB_EN)
                let writable_mask: u64 = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 6)
                    | (1 << 8) | (0x1F << 9) | (1 << 14);
                let mask = match size {
                    4 => writable_mask & 0xFFFF_FFFF,
                    _ => writable_mask,
                };
                let shifted_mask = mask >> shift;
                let new_val = (val & shifted_mask) << shift;
                self.timer0_config = (self.timer0_config & !mask) | new_val;

                // VAL_SET_CNF (bit 6): if periodic mode, next comparator write sets period
                // Clear it after processing
                if self.timer0_config & (1 << 6) != 0 {
                    self.timer0_config &= !(1 << 6);
                }
            }
            // Timer 1 Configuration (stub — writable for read-back)
            0x120 => {
                let writable_mask: u64 = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 6)
                    | (1 << 8) | (0x1F << 9) | (1 << 14);
                let mask = match size {
                    4 => writable_mask & 0xFFFF_FFFF,
                    _ => writable_mask,
                };
                let shifted_mask = mask >> shift;
                let new_val = (val & shifted_mask) << shift;
                self.timer1_config = (self.timer1_config & !mask) | new_val;
            }
            // Timer 2 Configuration (stub — writable for read-back)
            0x140 => {
                let writable_mask: u64 = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 6)
                    | (1 << 8) | (0x1F << 9) | (1 << 14);
                let mask = match size {
                    4 => writable_mask & 0xFFFF_FFFF,
                    _ => writable_mask,
                };
                let shifted_mask = mask >> shift;
                let new_val = (val & shifted_mask) << shift;
                self.timer2_config = (self.timer2_config & !mask) | new_val;
            }
            // Timer 0 Comparator Value
            0x108 => {
                let mask = match size {
                    1 => 0xFF_u64,
                    2 => 0xFFFF,
                    4 => 0xFFFF_FFFF,
                    _ => u64::MAX,
                } << shift;
                let new_cmp = (self.timer0_comparator & !mask) | ((val << shift) & mask);
                self.timer0_comparator = new_cmp;

                // If periodic mode, store as period too
                if self.timer0_config & (1 << 3) != 0 {
                    self.timer0_period = new_cmp;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

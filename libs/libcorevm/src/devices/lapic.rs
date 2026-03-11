//! Local APIC (Local Advanced Programmable Interrupt Controller) emulation.
//!
//! The LAPIC is a per-CPU interrupt controller integrated into x86
//! processors. It handles inter-processor interrupts (IPIs), local
//! timer interrupts, and routing of external interrupts from the
//! IO-APIC to the CPU core.
//!
//! # MMIO Region
//!
//! The LAPIC is memory-mapped at physical address `0xFEE00000` (4 KB).
//! All registers are 32-bit aligned at 16-byte boundaries.
//!
//! # Key Registers
//!
//! | Offset | Register | Access |
//! |--------|----------|--------|
//! | 0x020 | LAPIC ID | R/W |
//! | 0x030 | Version | RO |
//! | 0x080 | Task Priority (TPR) | R/W |
//! | 0x0B0 | End of Interrupt (EOI) | WO |
//! | 0x0D0 | Logical Destination | R/W |
//! | 0x0E0 | Destination Format | R/W |
//! | 0x0F0 | Spurious Interrupt Vector | R/W |
//! | 0x300 | ICR Low | R/W |
//! | 0x310 | ICR High | R/W |
//! | 0x320 | LVT Timer | R/W |
//! | 0x350 | LVT LINT0 | R/W |
//! | 0x360 | LVT LINT1 | R/W |
//! | 0x370 | LVT Error | R/W |
//! | 0x380 | Timer Initial Count | R/W |
//! | 0x390 | Timer Current Count | RO |
//! | 0x3E0 | Timer Divide Config | R/W |

use crate::error::Result;
use crate::memory::mmio::MmioHandler;
#[cfg(feature = "host_test")]
use core::sync::atomic::{AtomicU32, Ordering};

/// LAPIC version: xAPIC, version 0x14 (20), max LVT entry 5.
const LAPIC_VERSION: u32 = (5 << 16) | 0x14;

#[cfg(feature = "host_test")]
static LAPIC_TRACE_BUDGET: AtomicU32 = AtomicU32::new(128);

#[cfg(feature = "host_test")]
fn lapic_trace(args: core::fmt::Arguments<'_>) {
    if LAPIC_TRACE_BUDGET
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| (n > 0).then_some(n - 1))
        .is_ok()
    {
        eprintln!("[lapic] {args}");
    }
}

/// Local APIC device emulation.
///
/// Provides the minimum LAPIC functionality needed for SeaBIOS and
/// other firmware to detect and configure the APIC. The timer is not
/// actively counting — `current_count` always returns 0.
#[derive(Debug)]
pub struct Lapic {
    /// LAPIC ID register (bits 31:24 = APIC ID).
    id: u32,
    /// Task Priority Register.
    tpr: u32,
    /// Logical Destination Register.
    ldr: u32,
    /// Destination Format Register.
    dfr: u32,
    /// Spurious Interrupt Vector Register.
    /// Bit 8 = APIC software enable; bits 7:0 = spurious vector.
    svr: u32,
    /// Interrupt Command Register — low 32 bits.
    icr_lo: u32,
    /// Interrupt Command Register — high 32 bits (destination field).
    icr_hi: u32,
    /// LVT Timer entry.
    lvt_timer: u32,
    /// LVT LINT0 entry.
    lvt_lint0: u32,
    /// LVT LINT1 entry.
    lvt_lint1: u32,
    /// LVT Error entry.
    lvt_error: u32,
    /// LVT Performance Monitor entry.
    lvt_perf: u32,
    /// LVT Thermal Sensor entry.
    lvt_thermal: u32,
    /// Timer Initial Count.
    timer_init_count: u32,
    /// Timer Current Count.
    timer_cur_count: u32,
    /// Fractional bus-tick credits toward the next timer decrement.
    timer_credit: u64,
    /// TSC value at the last wall-clock synchronization of the timer.
    timer_start_tsc: u64,
    /// Host TSC frequency in Hz (set once at init).
    host_tsc_freq: u64,
    /// Sticky pending state for the timer vector until VM glue drains it.
    timer_irq_pending: bool,
    /// Timer Divide Configuration.
    timer_divide: u32,
    /// Error Status Register.
    esr: u32,
    /// In-Service Register (8 × 32-bit = 256 bits).
    isr: [u32; 8],
    /// Trigger Mode Register (8 × 32-bit).
    tmr: [u32; 8],
    /// Interrupt Request Register (8 × 32-bit).
    irr: [u32; 8],
    /// FIFO of vectors completed via EOI, consumed by the VM glue so the
    /// IO-APIC can drop remote-IRR for level-triggered routes without losing
    /// multiple EOIs raised inside one host run slice.
    eoi_vectors: [u8; 16],
    eoi_head: u8,
    eoi_len: u8,
}

impl Lapic {
    /// Create a new LAPIC for the BSP (bootstrap processor, APIC ID 0).
    ///
    /// All LVT entries start masked. The APIC is software-disabled
    /// (SVR bit 8 = 0) until the guest enables it.
    pub fn new() -> Self {
        Lapic {
            id: 0,                 // BSP = APIC ID 0
            tpr: 0,
            ldr: 0,
            dfr: 0xFFFF_FFFF,     // Flat model default
            svr: 0xFF,            // APIC disabled, vector 0xFF
            icr_lo: 0,
            icr_hi: 0,
            lvt_timer: 1 << 16,   // masked
            lvt_lint0: 0x00000700, // ExtINT, unmasked (BIOS default)
            lvt_lint1: 0x00000400, // NMI, unmasked (BIOS default)
            lvt_error: 1 << 16,   // masked
            lvt_perf: 1 << 16,    // masked
            lvt_thermal: 1 << 16, // masked
            timer_init_count: 0,
            timer_cur_count: 0,
            timer_credit: 0,
            timer_start_tsc: 0,
            host_tsc_freq: 0,
            timer_irq_pending: false,
            timer_divide: 0,
            esr: 0,
            isr: [0; 8],
            tmr: [0; 8],
            irr: [0; 8],
            eoi_vectors: [0; 16],
            eoi_head: 0,
            eoi_len: 0,
        }
    }
}

impl Lapic {
    /// Set the host TSC frequency for real-time timer computation.
    pub fn set_host_tsc_freq(&mut self, freq: u64) {
        self.host_tsc_freq = freq;
    }

    #[inline]
    pub fn software_enabled(&self) -> bool {
        (self.svr & (1 << 8)) != 0
    }

    #[inline]
    fn priority_class(vector: u8) -> u8 {
        vector >> 4
    }

    fn highest_set_vector(bits: &[u32; 8]) -> Option<u8> {
        for (idx, &word) in bits.iter().enumerate().rev() {
            if word != 0 {
                let bit = 31 - word.leading_zeros() as u8;
                return Some((idx as u8) * 32 + bit);
            }
        }
        None
    }

    fn highest_isr_vector(&self) -> Option<u8> {
        Self::highest_set_vector(&self.isr)
    }

    fn highest_irr_vector(&self) -> Option<u8> {
        Self::highest_set_vector(&self.irr)
    }

    fn set_bit(bits: &mut [u32; 8], vector: u8) {
        let idx = (vector / 32) as usize;
        let bit = vector % 32;
        bits[idx] |= 1u32 << bit;
    }

    fn clear_bit(bits: &mut [u32; 8], vector: u8) {
        let idx = (vector / 32) as usize;
        let bit = vector % 32;
        bits[idx] &= !(1u32 << bit);
    }

    fn current_ppr(&self) -> u8 {
        let tpr_pri = (self.tpr & 0xF0) as u8;
        let isr_pri = self
            .highest_isr_vector()
            .map(Self::priority_class)
            .unwrap_or(0)
            << 4;
        tpr_pri.max(isr_pri)
    }

    pub fn timer_running(&self) -> bool {
        self.software_enabled()
            && self.timer_cur_count != 0
    }

    pub fn has_pending_vector(&self) -> bool {
        self.highest_irr_vector().is_some()
    }

    /// Whether the LAPIC accepts PIC interrupts via LINT0 (like QEMU's
    /// `apic_accept_pic_intr`). PIC interrupts are accepted when either
    /// the APIC is globally disabled OR LINT0 is unmasked.
    pub fn accepts_pic_intr(&self) -> bool {
        if !self.software_enabled() {
            return true; // APIC disabled → PIC goes directly to CPU
        }
        // LINT0 unmasked → PIC interrupts flow through
        (self.lvt_lint0 & (1 << 16)) == 0
    }

    pub fn raise_vector(&mut self, vector: u8, level_triggered: bool) {
        Self::set_bit(&mut self.irr, vector);
        if level_triggered {
            Self::set_bit(&mut self.tmr, vector);
        } else {
            Self::clear_bit(&mut self.tmr, vector);
        }
        #[cfg(feature = "host_test")]
        if vector == 0xFD {
            lapic_trace(format_args!(
                "raise vec={:02X} level={} tpr={:02X} ppr={:02X} isr6={:08X} isr7={:08X} irr6={:08X} irr7={:08X}",
                vector,
                level_triggered as u8,
                self.tpr,
                self.current_ppr(),
                self.isr[6],
                self.isr[7],
                self.irr[6],
                self.irr[7],
            ));
        }
    }

    pub fn next_deliverable_vector(&self) -> Option<u8> {
        if !self.software_enabled() {
            return None;
        }
        let ppr = self.current_ppr() >> 4;
        self.highest_irr_vector()
            .filter(|&vec| Self::priority_class(vec) > ppr)
    }

    pub fn accept_vector(&mut self, vector: u8) {
        Self::clear_bit(&mut self.irr, vector);
        Self::set_bit(&mut self.isr, vector);
    }

    pub fn eoi(&mut self) -> Option<u8> {
        let vec = self.highest_isr_vector()?;
        Self::clear_bit(&mut self.isr, vec);
        Self::clear_bit(&mut self.tmr, vec);
        #[cfg(feature = "host_test")]
        if vec == 0xFD {
            lapic_trace(format_args!(
                "eoi vec={:02X} tpr={:02X} ppr={:02X} isr6={:08X} isr7={:08X} irr6={:08X} irr7={:08X}",
                vec,
                self.tpr,
                self.current_ppr(),
                self.isr[6],
                self.isr[7],
                self.irr[6],
                self.irr[7],
            ));
        }
        Some(vec)
    }

    fn push_eoi_vector(&mut self, vector: u8) {
        let capacity = self.eoi_vectors.len() as u8;
        if self.eoi_len == capacity {
            // Keep the newest EOIs; dropping the oldest still preserves
            // forward progress for level-triggered lines that are being
            // actively serviced under interrupt load.
            self.eoi_head = (self.eoi_head + 1) % capacity;
            self.eoi_len -= 1;
        }
        let tail = (self.eoi_head + self.eoi_len) % capacity;
        self.eoi_vectors[tail as usize] = vector;
        self.eoi_len += 1;
    }

    pub fn take_eoi_vector(&mut self) -> Option<u8> {
        if self.eoi_len == 0 {
            return None;
        }
        let vector = self.eoi_vectors[self.eoi_head as usize];
        let capacity = self.eoi_vectors.len() as u8;
        self.eoi_head = (self.eoi_head + 1) % capacity;
        self.eoi_len -= 1;
        Some(vector)
    }

    pub fn diag_irr(&self, idx: usize) -> u32 {
        self.irr.get(idx).copied().unwrap_or(0)
    }

    pub fn diag_isr(&self, idx: usize) -> u32 {
        self.isr.get(idx).copied().unwrap_or(0)
    }

    pub fn diag_tpr(&self) -> u32 {
        self.tpr
    }
    pub fn diag_lint0(&self) -> u32 {
        self.lvt_lint0
    }
    pub fn diag_lint1(&self) -> u32 {
        self.lvt_lint1
    }
    pub fn diag_lvt_timer(&self) -> u32 {
        self.lvt_timer
    }
    pub fn diag_cur_count(&self) -> u32 {
        self.timer_cur_count
    }
    pub fn diag_init_count(&self) -> u32 {
        self.timer_init_count
    }

    pub fn diag_state(&self) -> (u32, u32, u32, u32, u32) {
        (
            self.svr,
            self.lvt_timer,
            self.timer_init_count,
            self.timer_cur_count,
            self.timer_divide,
        )
    }

    fn timer_divisor(&self) -> u64 {
        // APIC timer divide encoding:
        // 0b0000=2, 0001=4, 0010=8, 0011=16,
        // 1000=32, 1001=64, 1010=128, 1011=1.
        match self.timer_divide & 0xB {
            0x0 => 2,
            0x1 => 4,
            0x2 => 8,
            0x3 => 16,
            0x8 => 32,
            0x9 => 64,
            0xA => 128,
            0xB => 1,
            _ => 2,
        }
    }

    fn elapse_bus_ticks(&mut self, bus_ticks: u64) {
        if !self.software_enabled() || self.timer_cur_count == 0 {
            return;
        }

        let masked = (self.lvt_timer & (1 << 16)) != 0;
        let div = self.timer_divisor();
        self.timer_credit = self.timer_credit.saturating_add(bus_ticks);
        if self.timer_credit < div {
            return;
        }

        let dec = (self.timer_credit / div) as u32;
        self.timer_credit %= div;
        if dec < self.timer_cur_count {
            self.timer_cur_count -= dec;
            return;
        }

        #[cfg(feature = "host_test")]
        {
            static FIRE_LOG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            let cnt = FIRE_LOG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if cnt < 5 {
                eprintln!("[lapic] elapse: TIMER EXPIRED! dec={} cur_was={} init={} periodic={} masked={}",
                    dec, self.timer_cur_count, self.timer_init_count,
                    (self.lvt_timer & (1 << 17)) != 0, masked);
            }
        }

        let periodic = (self.lvt_timer & (1 << 17)) != 0;
        if periodic && self.timer_init_count != 0 {
            let overshoot = dec.saturating_sub(self.timer_cur_count);
            let remainder = overshoot % self.timer_init_count;
            self.timer_cur_count = if remainder == 0 {
                self.timer_init_count
            } else {
                self.timer_init_count - remainder
            };
        } else {
            self.timer_cur_count = 0;
        }

        if !masked {
            self.timer_irq_pending = true;
        }
    }

    /// Check and consume a pending timer IRQ flag.
    pub fn take_timer_irq(&mut self) -> bool {
        if self.timer_irq_pending {
            self.timer_irq_pending = false;
            true
        } else {
            false
        }
    }

    /// Return the vector configured in the LVT timer entry.
    pub fn timer_vector(&self) -> u8 {
        (self.lvt_timer & 0xFF) as u8
    }

    pub fn sync_timer_from_tsc(&mut self) {
        #[cfg(feature = "host_test")]
        {
            static SYNC_CALL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
            let n = SYNC_CALL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if n == 100_000 || n == 1_000_000 || n == 10_000_000 {
                eprintln!("[lapic] sync_timer_from_tsc called {}x: freq={} cur={} init={}",
                    n, self.host_tsc_freq, self.timer_cur_count, self.timer_init_count);
            }
        }
        if self.host_tsc_freq == 0 || self.timer_cur_count == 0 {
            return;
        }

        let now = {
            #[cfg(feature = "host_test")]
            { unsafe { core::arch::x86_64::_rdtsc() as u64 } }
            #[cfg(not(feature = "host_test"))]
            { unsafe { core::arch::x86_64::_rdtsc() as u64 } }
        };
        let elapsed_tsc = now.wrapping_sub(self.timer_start_tsc);
        if elapsed_tsc == 0 {
            return;
        }

        const APIC_BUS_FREQ: u128 = 100_000_000;
        let bus_ticks =
            (elapsed_tsc as u128 * APIC_BUS_FREQ / self.host_tsc_freq as u128) as u64;
        if bus_ticks == 0 {
            return;
        }

        let consumed_tsc =
            (bus_ticks as u128 * self.host_tsc_freq as u128 / APIC_BUS_FREQ) as u64;
        self.timer_start_tsc = self.timer_start_tsc.wrapping_add(consumed_tsc);

        #[cfg(feature = "host_test")]
        {
            static SYNC_DBG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            let cnt = SYNC_DBG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if cnt % 500_000 == 0 {
                eprintln!("[lapic] sync#{}: elapsed_tsc={} bus_ticks={} cur={} init={} sw_en={}",
                    cnt, elapsed_tsc, bus_ticks, self.timer_cur_count, self.timer_init_count,
                    self.software_enabled() as u8);
            }
        }

        self.elapse_bus_ticks(bus_ticks);
    }

    /// Advance the LAPIC timer by `bus_ticks` (at the APIC bus frequency).
    ///
    /// The caller is responsible for converting wall-clock or TSC time into
    /// bus ticks. A typical APIC bus frequency is 100 MHz.
    ///
    /// Returns the timer interrupt vector when the counter expires and the
    /// timer is unmasked. For periodic mode, the counter reloads.
    pub fn advance(&mut self, bus_ticks: u64) -> Option<u8> {
        #[cfg(feature = "host_test")]
        if (self.lvt_timer & 0xFF) == 0xFD {
            lapic_trace(format_args!(
                "advance-entry cur={} init={} div={} host_tsc_freq={} start_tsc={}",
                self.timer_cur_count,
                self.timer_init_count,
                self.timer_divisor(),
                self.host_tsc_freq,
                self.timer_start_tsc
            ));
        }
        let vec = (self.lvt_timer & 0xFF) as u8;
        let cur_before = self.timer_cur_count;
        self.elapse_bus_ticks(bus_ticks);
        #[cfg(feature = "host_test")]
        if vec == 0xFD && self.timer_irq_pending {
            lapic_trace(format_args!(
                "timer-expire vec={:02X} cur_before={} cur_after={} init={} credit={} div={}",
                vec,
                cur_before,
                self.timer_cur_count,
                self.timer_init_count,
                self.timer_credit,
                self.timer_divisor()
            ));
        }
        if self.timer_irq_pending {
            self.timer_irq_pending = false;
            Some(vec)
        } else {
            None
        }
    }

    /// Read the raw 32-bit value of a register by its 16-byte-aligned offset.
    fn read_register(&mut self, reg_base: u32) -> u32 {
        let val = self.read_register_inner(reg_base);
        #[cfg(feature = "host_test")]
        {
            static RLOG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            let cnt = RLOG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if cnt < 200 {
                eprintln!("[lapic] read  reg={:#05x} val={:#010x}", reg_base, val);
            }
        }
        val
    }

    fn read_register_inner(&mut self, reg_base: u32) -> u32 {
        match reg_base {
            0x020 => {
                #[cfg(feature = "host_test")]
                {
                    static RID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
                    let n = RID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    if n < 10 {
                        eprintln!("[lapic] read-0x020 #{}: id={:#010x}", n, self.id);
                    }
                }
                self.id
            }
            0x030 => LAPIC_VERSION,
            0x080 => self.tpr,
            0x090 => 0, // APR
            0x0A0 => self.current_ppr() as u32,
            0x0D0 => self.ldr,
            0x0E0 => self.dfr,
            0x0F0 => self.svr,
            // In-Service Register (ISR): 8 × 32-bit at 0x100-0x170.
            off @ 0x100..=0x170 => self.isr[((off - 0x100) >> 4) as usize],
            // Trigger Mode Register (TMR): 0x180-0x1F0.
            off @ 0x180..=0x1F0 => self.tmr[((off - 0x180) >> 4) as usize],
            // Interrupt Request Register (IRR): 0x200-0x270.
            off @ 0x200..=0x270 => self.irr[((off - 0x200) >> 4) as usize],
            0x280 => self.esr,
            0x300 => self.icr_lo,
            0x310 => self.icr_hi,
            0x320 => {
                #[cfg(feature = "host_test")]
                {
                    static RLVT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
                    let n = RLVT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    if n < 20 {
                        eprintln!("[lapic] read-0x320 #{}: lvt_timer={:#010x}", n, self.lvt_timer);
                    }
                }
                self.lvt_timer
            }
            0x330 => self.lvt_thermal,
            0x340 => self.lvt_perf,
            0x350 => self.lvt_lint0,
            0x360 => self.lvt_lint1,
            0x370 => self.lvt_error,
            0x380 => self.timer_init_count,
            0x390 => {
                self.sync_timer_from_tsc();
                #[cfg(feature = "host_test")]
                {
                    static RCNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
                    let n = RCNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    if n < 10 || (n < 100 && n % 10 == 0) || n % 1000 == 0 {
                        eprintln!("[lapic] read-0x390 #{}: cur_count={} init={} lvt={:#010x}",
                            n, self.timer_cur_count, self.timer_init_count, self.lvt_timer);
                    }
                }
                self.timer_cur_count
            }
            0x3E0 => self.timer_divide,
            _ => 0,
        }
    }

    /// Write a 32-bit value to a register by its 16-byte-aligned offset.
    fn write_register(&mut self, reg_base: u32, v: u32) {
        #[cfg(feature = "host_test")]
        {
            static WLOG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            let cnt = WLOG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            // Always log timer-related registers; budget other writes.
            if reg_base == 0x320 || reg_base == 0x380 || reg_base == 0x3E0 || reg_base == 0x020 {
                eprintln!("[lapic] write reg={:#05x} val={:#010x} (always-logged)", reg_base, v);
            } else if cnt < 200 {
                eprintln!("[lapic] write reg={:#05x} val={:#010x}", reg_base, v);
            }
        }
        match reg_base {
            // LAPIC ID: bits 31:24 are writable.
            0x020 => self.id = v & 0xFF00_0000,
            0x080 => self.tpr = v & 0xFF,
            // EOI: any write signals end-of-interrupt.
            0x0B0 => {
                if let Some(vector) = self.eoi() {
                    self.push_eoi_vector(vector);
                }
            }
            0x0D0 => self.ldr = v,
            0x0E0 => self.dfr = v,
            0x0F0 => {
                #[cfg(feature = "host_test")]
                {
                    static SVR_LOG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
                    let cnt = SVR_LOG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    if cnt < 10 {
                        eprintln!("[lapic] SVR write: {:#x} -> {:#x} (enabled={})", self.svr, v, (v >> 8) & 1);
                    }
                }
                self.svr = v;
            }
            0x280 => self.esr = 0, // Writing clears ESR
            0x300 => {
                self.icr_lo = v & !0x1000; // clear delivery status bit
                let vector = (self.icr_lo & 0xFF) as u8;
                let delivery_mode = ((self.icr_lo >> 8) & 0x7) as u8;
                let shorthand = ((self.icr_lo >> 18) & 0x3) as u8;
                let dest_apic = ((self.icr_hi >> 24) & 0xFF) as u8;
                let self_apic = ((self.id >> 24) & 0xFF) as u8;
                #[cfg(feature = "host_test")]
                {
                    static ICR_LOG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
                    let n = ICR_LOG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    if n < 50 {
                        eprintln!("[lapic] ICR #{}: lo={:#010x} hi={:#010x} dm={} vec={:#04x} dest={} short={}",
                            n, v, self.icr_hi, delivery_mode, vector, dest_apic, shorthand);
                    }
                }
                let hits_self = match shorthand {
                    0 => dest_apic == self_apic,
                    1 | 2 => true,  // self / all including self
                    3 => false,     // all excluding self
                    _ => false,
                };
                if hits_self && delivery_mode == 0 {
                    self.raise_vector(vector, false);
                }
            }
            0x310 => self.icr_hi = v,
            0x320 => {
                self.lvt_timer = v;
                #[cfg(feature = "host_test")]
                lapic_trace(format_args!("timer-lvt {:08X}", self.lvt_timer));
            }
            0x330 => self.lvt_thermal = v,
            0x340 => self.lvt_perf = v,
            0x350 => self.lvt_lint0 = v,
            0x360 => self.lvt_lint1 = v,
            0x370 => self.lvt_error = v,
            0x380 => {
                self.timer_init_count = v;
                self.timer_cur_count = v;
                self.timer_credit = 0;
                self.timer_irq_pending = false;
                #[cfg(feature = "host_test")]
                {
                    static TINIT_CNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
                    let cnt = TINIT_CNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    if cnt < 30 || cnt % 100 == 0 {
                        eprintln!("[lapic] timer-write-380 #{}: v={:#010x} lvt={:08X}",
                            cnt, v, self.lvt_timer);
                    }
                    // DIAGNOSTIC: force-unmask timer after second init count write
                    // (first is 0xFFFFFFFF calibration, second is operational)
                    if cnt == 1 && (self.lvt_timer & (1 << 16)) != 0 {
                        eprintln!("[lapic] DIAG: force-unmasking timer LVT {:#010x} -> {:#010x}",
                            self.lvt_timer, self.lvt_timer & !(1 << 16));
                        self.lvt_timer &= !(1 << 16);
                    }
                }
                // Record TSC at timer start for realtime current_count reads.
                if v != 0 {
                    #[cfg(feature = "host_test")]
                    { self.timer_start_tsc = unsafe { core::arch::x86_64::_rdtsc() as u64 }; }
                    #[cfg(not(feature = "host_test"))]
                    { self.timer_start_tsc = unsafe { core::arch::x86_64::_rdtsc() as u64 }; }
                }
            }
            0x3E0 => {
                self.timer_divide = v;
                #[cfg(feature = "host_test")]
                lapic_trace(format_args!("timer-div {:08X}", self.timer_divide));
            }
            _ => {}
        }
    }
}

impl MmioHandler for Lapic {
    /// Read from LAPIC MMIO register.
    ///
    /// LAPIC registers are 32-bit wide at 16-byte-aligned offsets.
    /// Bytes 0-3 of each slot hold the register; bytes 4-15 are reserved.
    /// Sub-dword accesses extract the correct byte(s) from the register.
    fn read(&mut self, offset: u64, size: u8) -> Result<u64> {
        let byte_in_slot = (offset & 0xF) as u32;
        // Bytes 4-15 within each 16-byte register slot are reserved.
        if byte_in_slot >= 4 {
            return Ok(0);
        }
        let reg_base = (offset & !0xF) as u32;
        let reg_val = self.read_register(reg_base);
        // Shift to the requested byte position and mask to access size.
        let shifted = (reg_val >> (byte_in_slot * 8)) as u64;
        let bits = (size as u32).min(4) * 8;
        let mask = if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
        Ok(shifted & mask)
    }

    /// Write to LAPIC MMIO register.
    ///
    /// Sub-dword writes perform a read-modify-write to merge partial bytes.
    fn write(&mut self, offset: u64, size: u8, val: u64) -> Result<()> {
        let byte_in_slot = (offset & 0xF) as u32;
        if byte_in_slot >= 4 {
            return Ok(());
        }
        let reg_base = (offset & !0xF) as u32;
        // Build the full 32-bit value to write.
        let v = if byte_in_slot == 0 && size >= 4 {
            // Standard 32-bit aligned write (the expected / common case).
            val as u32
        } else {
            // Sub-dword or byte-offset write: read-modify-write.
            let old = self.read_register(reg_base);
            let shift = byte_in_slot * 8;
            let bits = (size as u32).min(4) * 8;
            let mask = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
            (old & !(mask << shift)) | (((val as u32) & mask) << shift)
        };
        self.write_register(reg_base, v);
        Ok(())
    }
}

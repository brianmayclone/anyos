//! Intel AC'97 audio codec emulation (ICH).
//!
//! Emulates an Intel 82801AA AC'97 Audio Controller (PCI device 8086:2415).
//! Provides basic PCM output via DMA buffer descriptor lists.
//!
//! # I/O Layout
//!
//! The AC'97 controller uses two I/O port ranges:
//!
//! - **NAM** (Native Audio Mixer, BAR0): Codec mixer registers (256 bytes)
//!   Controls volume, mute, sample rate, codec identification.
//!
//! - **NABM** (Native Audio Bus Master, BAR1): DMA engine registers (64 bytes)
//!   Controls PCM In (PI), PCM Out (PO), and Mic In (MC) channels.
//!
//! ## NABM Channel Layout (each channel = 16 bytes)
//!
//! | Offset | Name | Size | Description |
//! |--------|------|------|-------------|
//! | 0x00 | BDBAR | 4 | Buffer Descriptor Base Address Register |
//! | 0x04 | CIV | 1 | Current Index Value (0-31) |
//! | 0x05 | LVI | 1 | Last Valid Index (0-31) |
//! | 0x06 | SR | 2 | Status Register (FIFO error, completion, etc.) |
//! | 0x08 | PICB | 2 | Position in Current Buffer (samples remaining) |
//! | 0x0A | PIV | 1 | Prefetch Index Value |
//! | 0x0B | CR | 1 | Control Register (run, reset, interrupt enable) |
//!
//! Channel base offsets: PI=0x00, PO=0x10, MC=0x20
//! Global registers: GLOB_CNT=0x2C, GLOB_STA=0x30

use alloc::vec;
use alloc::vec::Vec;
use crate::io::IoHandler;
use crate::error::Result;

// ── NAM Register Offsets (Mixer) ──

const NAM_RESET: u16         = 0x00;
const NAM_MASTER_VOL: u16    = 0x02;
const NAM_HEADPHONE_VOL: u16 = 0x04;
const NAM_MONO_VOL: u16      = 0x06;
const NAM_PCM_OUT_VOL: u16   = 0x18;
const NAM_REC_SELECT: u16    = 0x1A;
const NAM_REC_GAIN: u16      = 0x1C;
const NAM_POWERDOWN: u16     = 0x26;
const NAM_EXT_AUDIO_ID: u16  = 0x28;
const NAM_EXT_AUDIO_SC: u16  = 0x2A;
const NAM_PCM_FRONT_RATE: u16 = 0x2C;
const NAM_VENDOR_ID1: u16    = 0x7C;
const NAM_VENDOR_ID2: u16    = 0x7E;

// ── NABM Register Offsets (Bus Master) ──

const NABM_PO_BASE: u16     = 0x10; // PCM Out channel base
const NABM_GLOB_CNT: u16    = 0x2C; // Global Control
const NABM_GLOB_STA: u16    = 0x30; // Global Status

// Channel register offsets (relative to channel base)
const CH_BDBAR: u16 = 0x00; // Buffer Descriptor Base Address
const CH_CIV: u16   = 0x04; // Current Index Value
const CH_LVI: u16   = 0x05; // Last Valid Index
const CH_SR: u16    = 0x06; // Status Register
const CH_PICB: u16  = 0x08; // Position In Current Buffer
const CH_PIV: u16   = 0x0A; // Prefetch Index Value
const CH_CR: u16    = 0x0B; // Control Register

// Status Register bits
const SR_DCH: u16   = 0x01;  // DMA Controller Halted
const SR_CELV: u16  = 0x02;  // Current Equals Last Valid
const SR_LVBCI: u16 = 0x04;  // Last Valid Buffer Completion Interrupt
const SR_BCIS: u16  = 0x08;  // Buffer Completion Interrupt Status
const SR_FIFOE: u16 = 0x10;  // FIFO Error

// Control Register bits
const CR_RPBM: u8   = 0x01;  // Run/Pause Bus Master
const CR_RR: u8     = 0x02;  // Reset Registers
const CR_LVBIE: u8  = 0x04;  // Last Valid Buffer Interrupt Enable
const CR_FEIE: u8   = 0x08;  // FIFO Error Interrupt Enable
const CR_IOCE: u8   = 0x10;  // Interrupt On Completion Enable

// Global Status bits
const GS_MINT: u32  = 1 << 0;  // Modem interrupt
const GS_POINT: u32 = 1 << 1;  // PCM Out interrupt
const GS_PIINT: u32 = 1 << 2;  // PCM In interrupt
const GS_PRIMARY_READY: u32 = 1 << 8;  // Primary codec ready
const GS_S0CR: u32  = 1 << 20; // AC_SDIN0 codec ready

/// A single DMA channel (PI, PO, or MC).
#[derive(Debug, Clone)]
struct DmaChannel {
    /// Buffer Descriptor Base Address (physical).
    bdbar: u32,
    /// Current Index Value (0-31).
    civ: u8,
    /// Last Valid Index (0-31).
    lvi: u8,
    /// Status Register.
    sr: u16,
    /// Position In Current Buffer (samples remaining).
    picb: u16,
    /// Prefetch Index Value.
    piv: u8,
    /// Control Register.
    cr: u8,
}

impl DmaChannel {
    fn new() -> Self {
        Self {
            bdbar: 0,
            civ: 0,
            lvi: 0,
            sr: SR_DCH, // Halted by default
            picb: 0,
            piv: 0,
            cr: 0,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    /// Check if the channel is running.
    fn is_running(&self) -> bool {
        self.cr & CR_RPBM != 0
    }
}

/// Buffer Descriptor Entry (8 bytes in guest memory).
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
struct BufferDescriptor {
    /// Physical address of the audio data buffer.
    addr: u32,
    /// Number of samples in this buffer (bits 15:0).
    /// Bit 31 (BUP): Buffer Underrun Policy.
    /// Bit 30 (IOC): Interrupt On Completion.
    length_flags: u32,
}

impl BufferDescriptor {
    fn sample_count(&self) -> u16 {
        (self.length_flags & 0xFFFF) as u16
    }

    fn ioc(&self) -> bool {
        self.length_flags & (1 << 31) != 0
    }

    fn bup(&self) -> bool {
        self.length_flags & (1 << 30) != 0
    }
}

/// Intel AC'97 audio controller.
#[derive(Debug)]
pub struct Ac97 {
    // NAM (mixer) registers: 128 bytes (64 x 16-bit words)
    mixer: [u16; 64],

    // DMA channels
    po: DmaChannel, // PCM Out (the main audio output)
    pi: DmaChannel, // PCM In
    mc: DmaChannel, // Mic In

    // Global registers
    glob_cnt: u32,
    glob_sta: u32,

    /// Audio samples ready for host playback (interleaved 16-bit stereo PCM).
    /// The host drains this buffer periodically.
    pub audio_out: Vec<i16>,

    /// Guest RAM pointer for DMA reads (set by FFI layer).
    ram_ptr: *const u8,
    ram_size: usize,
}

unsafe impl Send for Ac97 {}

impl Ac97 {
    pub fn new() -> Self {
        let mut mixer = [0u16; 64];
        // Set default mixer values
        mixer[(NAM_MASTER_VOL / 2) as usize] = 0x0000; // Unmuted, max volume
        mixer[(NAM_PCM_OUT_VOL / 2) as usize] = 0x0808; // Mid volume
        mixer[(NAM_HEADPHONE_VOL / 2) as usize] = 0x0000;
        mixer[(NAM_MONO_VOL / 2) as usize] = 0x0000;
        mixer[(NAM_REC_SELECT / 2) as usize] = 0x0000;
        mixer[(NAM_REC_GAIN / 2) as usize] = 0x0000;
        // Extended Audio ID: VRA (Variable Rate Audio) supported
        mixer[(NAM_EXT_AUDIO_ID / 2) as usize] = 0x0001; // VRA bit
        mixer[(NAM_EXT_AUDIO_SC / 2) as usize] = 0x0000;
        // Default sample rate: 48000 Hz
        mixer[(NAM_PCM_FRONT_RATE / 2) as usize] = 48000;
        mixer[(NAM_POWERDOWN / 2) as usize] = 0x000F; // All sections powered
        // Vendor ID: Analog Devices AD1881A (commonly emulated)
        mixer[(NAM_VENDOR_ID1 / 2) as usize] = 0x4144; // "AD"
        mixer[(NAM_VENDOR_ID2 / 2) as usize] = 0x5370; // AD1881A

        Ac97 {
            mixer,
            po: DmaChannel::new(),
            pi: DmaChannel::new(),
            mc: DmaChannel::new(),
            glob_cnt: 0,
            glob_sta: GS_PRIMARY_READY | GS_S0CR, // Codec ready
            audio_out: Vec::new(),
            ram_ptr: core::ptr::null(),
            ram_size: 0,
        }
    }

    /// Set the guest RAM pointer for DMA buffer reads.
    pub fn set_ram(&mut self, ptr: *const u8, size: usize) {
        self.ram_ptr = ptr;
        self.ram_size = size;
    }

    /// Get the configured sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.mixer[(NAM_PCM_FRONT_RATE / 2) as usize] as u32
    }

    /// Read bytes from guest physical memory.
    fn read_guest(&self, gpa: u64, buf: &mut [u8]) -> bool {
        if self.ram_ptr.is_null() { return false; }
        let start = gpa as usize;
        let end = start + buf.len();
        if end > self.ram_size { return false; }
        unsafe {
            core::ptr::copy_nonoverlapping(self.ram_ptr.add(start), buf.as_mut_ptr(), buf.len());
        }
        true
    }

    /// Process the PCM Out DMA channel: read audio data from guest buffers.
    /// Call this periodically (e.g. every 10-20ms) to keep audio flowing.
    /// Returns true if an interrupt should be raised.
    pub fn process_po(&mut self) -> bool {
        if !self.po.is_running() { return false; }
        if self.po.bdbar == 0 { return false; }

        let mut need_irq = false;

        // Process up to 4 buffer descriptors per call to avoid starvation
        for _ in 0..4 {
            if self.po.civ == self.po.lvi && (self.po.sr & SR_CELV) != 0 {
                // Already at last valid — halted
                self.po.sr |= SR_DCH;
                break;
            }

            // Read current buffer descriptor from guest memory
            let bd_addr = self.po.bdbar as u64 + (self.po.civ as u64) * 8;
            let mut bd_bytes = [0u8; 8];
            if !self.read_guest(bd_addr, &mut bd_bytes) {
                break;
            }
            let bd = unsafe { core::ptr::read_unaligned(bd_bytes.as_ptr() as *const BufferDescriptor) };

            let sample_count = bd.sample_count() as usize;
            if sample_count == 0 {
                // Empty buffer, advance
                self.advance_po_civ();
                continue;
            }

            // Read audio samples from guest memory (16-bit samples)
            let byte_count = sample_count * 2; // 16-bit samples
            let data_addr = bd.addr as u64;
            let mut audio_data = vec![0u8; byte_count];
            if !self.read_guest(data_addr, &mut audio_data) {
                break;
            }

            // Convert to i16 samples and append to output buffer
            self.audio_out.reserve(sample_count);
            for chunk in audio_data.chunks_exact(2) {
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                self.audio_out.push(sample);
            }

            // Apply volume attenuation from mixer
            // (simplified: just check mute bit)
            let master = self.mixer[(NAM_MASTER_VOL / 2) as usize];
            let pcm_vol = self.mixer[(NAM_PCM_OUT_VOL / 2) as usize];
            let muted = (master & 0x8000) != 0 || (pcm_vol & 0x8000) != 0;
            if muted {
                let start = self.audio_out.len() - sample_count;
                for s in &mut self.audio_out[start..] {
                    *s = 0;
                }
            }

            // Set completion status
            self.po.sr |= SR_BCIS;
            if bd.ioc() {
                need_irq = true;
            }

            self.po.picb = 0;

            // Advance to next buffer descriptor
            self.advance_po_civ();

            // Check if we've reached the last valid
            if self.po.civ == self.po.lvi {
                self.po.sr |= SR_CELV | SR_LVBCI;
                if self.po.cr & CR_LVBIE != 0 {
                    need_irq = true;
                }
                break;
            }
        }

        if need_irq {
            self.glob_sta |= GS_POINT;
        }

        need_irq
    }

    fn advance_po_civ(&mut self) {
        self.po.civ = (self.po.civ + 1) % 32;
        self.po.piv = (self.po.civ + 1) % 32;
    }

    /// Take all buffered audio samples for host playback.
    /// Returns interleaved 16-bit PCM samples (stereo at configured sample rate).
    pub fn take_audio(&mut self) -> Vec<i16> {
        let mut out = Vec::new();
        core::mem::swap(&mut out, &mut self.audio_out);
        out
    }

    /// Check if there are pending audio samples.
    pub fn has_audio(&self) -> bool {
        !self.audio_out.is_empty()
    }

    // ── NAM (Mixer) Register Access ──

    fn nam_read(&self, offset: u16) -> u16 {
        let idx = (offset / 2) as usize;
        if idx < self.mixer.len() {
            self.mixer[idx]
        } else {
            0
        }
    }

    fn nam_write(&mut self, offset: u16, val: u16) {
        let idx = (offset / 2) as usize;
        match offset {
            NAM_RESET => {
                // Writing to reset register resets the codec
                self.mixer = [0u16; 64];
                self.mixer[(NAM_EXT_AUDIO_ID / 2) as usize] = 0x0001;
                self.mixer[(NAM_PCM_FRONT_RATE / 2) as usize] = 48000;
                self.mixer[(NAM_POWERDOWN / 2) as usize] = 0x000F;
                self.mixer[(NAM_VENDOR_ID1 / 2) as usize] = 0x4144;
                self.mixer[(NAM_VENDOR_ID2 / 2) as usize] = 0x5370;
            }
            NAM_VENDOR_ID1 | NAM_VENDOR_ID2 => {
                // Read-only
            }
            NAM_PCM_FRONT_RATE => {
                // Clamp to valid sample rates
                let rate = val.max(8000).min(48000);
                self.mixer[idx] = rate;
            }
            _ => {
                if idx < self.mixer.len() {
                    self.mixer[idx] = val;
                }
            }
        }
    }

    // ── NABM (Bus Master) Register Access ──

    fn nabm_read(&self, offset: u16, size: u8) -> u32 {
        // Determine which channel
        let (ch, ch_off) = match offset {
            0x00..=0x0F => (&self.pi, offset),
            0x10..=0x1F => (&self.po, offset - 0x10),
            0x20..=0x2B => (&self.mc, offset - 0x20),
            _ => {
                // Global registers
                return match offset {
                    0x2C => self.glob_cnt,
                    0x30 => self.glob_sta,
                    0x34 => 0x00, // CAS: Codec Access Semaphore (0=ready, 1=busy)
                    _ => 0,
                };
            }
        };

        match ch_off {
            CH_BDBAR => ch.bdbar,
            CH_CIV => ch.civ as u32,
            CH_LVI => ch.lvi as u32,
            CH_SR => ch.sr as u32,
            CH_PICB => ch.picb as u32,
            CH_PIV => ch.piv as u32,
            CH_CR => ch.cr as u32,
            _ => {
                // Multi-byte reads that span register boundaries
                if size >= 2 && ch_off == 0x04 {
                    // CIV + LVI (byte pair)
                    (ch.civ as u32) | ((ch.lvi as u32) << 8)
                } else if size >= 2 && ch_off == 0x06 {
                    ch.sr as u32
                } else {
                    0
                }
            }
        }
    }

    fn nabm_write(&mut self, offset: u16, size: u8, val: u32) {
        // Determine which channel
        let (ch, ch_off) = match offset {
            0x00..=0x0F => (&mut self.pi, offset),
            0x10..=0x1F => (&mut self.po, offset - 0x10),
            0x20..=0x2B => (&mut self.mc, offset - 0x20),
            _ => {
                // Global registers
                match offset {
                    0x2C => { // GLOB_CNT
                        let old = self.glob_cnt;
                        self.glob_cnt = val;
                        // Cold reset (bit 1)
                        if val & 0x02 != 0 && old & 0x02 == 0 {
                            self.pi.reset();
                            self.po.reset();
                            self.mc.reset();
                            self.glob_sta |= GS_PRIMARY_READY | GS_S0CR;
                        }
                    }
                    0x30 => { // GLOB_STA — write-1-to-clear for interrupt bits
                        self.glob_sta &= !(val & 0x07); // Clear MINT, POINT, PIINT
                    }
                    _ => {}
                }
                return;
            }
        };

        match ch_off {
            CH_BDBAR => {
                ch.bdbar = val & 0xFFFF_FFF8; // 8-byte aligned
            }
            CH_LVI => {
                ch.lvi = (val & 0x1F) as u8;
                // Writing LVI while running can un-halt the channel
                if ch.is_running() && (ch.sr & SR_DCH) != 0 {
                    ch.sr &= !SR_DCH;
                }
            }
            CH_SR => {
                // Write-1-to-clear for status bits
                let clear_mask = (val as u16) & (SR_LVBCI | SR_BCIS | SR_FIFOE);
                ch.sr &= !clear_mask;
            }
            CH_CR => {
                let new_cr = val as u8;
                if new_cr & CR_RR != 0 {
                    // Reset channel
                    ch.reset();
                } else {
                    let was_running = ch.is_running();
                    ch.cr = new_cr;
                    if !was_running && ch.is_running() {
                        // Starting DMA
                        ch.sr &= !SR_DCH;
                    } else if was_running && !ch.is_running() {
                        // Stopping DMA
                        ch.sr |= SR_DCH;
                    }
                }
            }
            _ => {}
        }
    }
}

/// NAM I/O port handler (BAR0).
pub struct Ac97Nam(pub *mut Ac97);
unsafe impl Send for Ac97Nam {}

impl IoHandler for Ac97Nam {
    fn read(&mut self, port: u16, size: u8) -> Result<u32> {
        let ac97 = unsafe { &*self.0 };
        let offset = port & 0xFF;
        let val = ac97.nam_read(offset);
        Ok(match size {
            1 => if offset & 1 == 0 { (val & 0xFF) as u32 } else { ((val >> 8) & 0xFF) as u32 },
            2 => val as u32,
            4 => {
                let lo = ac97.nam_read(offset) as u32;
                let hi = ac97.nam_read(offset + 2) as u32;
                lo | (hi << 16)
            }
            _ => val as u32,
        })
    }

    fn write(&mut self, port: u16, size: u8, val: u32) -> Result<()> {
        let ac97 = unsafe { &mut *self.0 };
        let offset = port & 0xFF;
        match size {
            1 => {
                let old = ac97.nam_read(offset & !1);
                let new = if offset & 1 == 0 {
                    (old & 0xFF00) | (val & 0xFF) as u16
                } else {
                    (old & 0x00FF) | ((val & 0xFF) << 8) as u16
                };
                ac97.nam_write(offset & !1, new);
            }
            2 => ac97.nam_write(offset, val as u16),
            4 => {
                ac97.nam_write(offset, val as u16);
                ac97.nam_write(offset + 2, (val >> 16) as u16);
            }
            _ => {}
        }
        Ok(())
    }
}

impl Ac97Nam {
    /// Static read for use by PCI I/O router (takes &Ac97 directly).
    pub fn read_static(ac97: &Ac97, offset: u16, size: u8) -> crate::error::Result<u32> {
        let val = ac97.nam_read(offset);
        Ok(match size {
            1 => if offset & 1 == 0 { (val & 0xFF) as u32 } else { ((val >> 8) & 0xFF) as u32 },
            2 => val as u32,
            4 => {
                let lo = ac97.nam_read(offset) as u32;
                let hi = ac97.nam_read(offset + 2) as u32;
                lo | (hi << 16)
            }
            _ => val as u32,
        })
    }

    /// Static write for use by PCI I/O router.
    pub fn write_static(ac97: &mut Ac97, offset: u16, size: u8, val: u32) -> crate::error::Result<()> {
        match size {
            1 => {
                let old = ac97.nam_read(offset & !1);
                let new = if offset & 1 == 0 {
                    (old & 0xFF00) | (val & 0xFF) as u16
                } else {
                    (old & 0x00FF) | ((val & 0xFF) << 8) as u16
                };
                ac97.nam_write(offset & !1, new);
            }
            2 => ac97.nam_write(offset, val as u16),
            4 => {
                ac97.nam_write(offset, val as u16);
                ac97.nam_write(offset + 2, (val >> 16) as u16);
            }
            _ => {}
        }
        Ok(())
    }
}

/// NABM I/O port handler (BAR1).
pub struct Ac97Nabm(pub *mut Ac97);
unsafe impl Send for Ac97Nabm {}

impl IoHandler for Ac97Nabm {
    fn read(&mut self, port: u16, size: u8) -> Result<u32> {
        let ac97 = unsafe { &*self.0 };
        let offset = port & 0x3F;
        Ok(ac97.nabm_read(offset, size))
    }

    fn write(&mut self, port: u16, size: u8, val: u32) -> Result<()> {
        let ac97 = unsafe { &mut *self.0 };
        let offset = port & 0x3F;
        ac97.nabm_write(offset, size, val);
        Ok(())
    }
}

impl Ac97Nabm {
    /// Static read for use by PCI I/O router.
    pub fn read_static(ac97: &Ac97, offset: u16, size: u8) -> crate::error::Result<u32> {
        Ok(ac97.nabm_read(offset, size))
    }

    /// Static write for use by PCI I/O router.
    pub fn write_static(ac97: &mut Ac97, offset: u16, size: u8, val: u32) -> crate::error::Result<()> {
        ac97.nabm_write(offset, size, val);
        Ok(())
    }
}

// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Intel AC'97 (ICH) audio codec driver.
//!
//! Supports the Intel 82801AA AC'97 Audio Controller (PCI 8086:2415) and
//! compatible devices. Uses DMA with a 32-entry Buffer Descriptor List (BDL)
//! for PCM output at 48 kHz, 16-bit stereo.
//!
//! QEMU: `-device AC97 -audiodev coreaudio,id=audio0`

use alloc::boxed::Box;
use crate::arch::x86::port;
use crate::memory::physical;
use crate::sync::spinlock::Spinlock;
use crate::drivers::pci::PciDevice;
use crate::serial_println;

// ---------------------------------------------------------------------------
// AC'97 Mixer registers (offsets from NAMBAR / BAR0)
// ---------------------------------------------------------------------------
const NAM_RESET: u16 = 0x00;
const NAM_MASTER_VOL: u16 = 0x02;
const NAM_PCM_OUT_VOL: u16 = 0x18;
const NAM_PCM_FRONT_DAC_RATE: u16 = 0x2C;
const NAM_EXT_AUDIO_ID: u16 = 0x28;
const NAM_EXT_AUDIO_CTRL: u16 = 0x2A;

// ---------------------------------------------------------------------------
// AC'97 Bus Master registers (offsets from NABMBAR / BAR1)
// PCM Out channel is at offset 0x10
// ---------------------------------------------------------------------------
const NABM_PO_BDBAR: u16 = 0x10; // Buffer Descriptor list Base Address (32-bit)
const NABM_PO_CIV: u16 = 0x14;   // Current Index Value (8-bit)
const NABM_PO_LVI: u16 = 0x15;   // Last Valid Index (8-bit)
const NABM_PO_SR: u16 = 0x16;    // Status Register (16-bit)
const NABM_PO_PICB: u16 = 0x18;  // Position In Current Buffer (16-bit, samples remaining)
const NABM_PO_PIV: u16 = 0x1A;   // Prefetched Index Value (8-bit)
const NABM_PO_CR: u16 = 0x1B;    // Control Register (8-bit)
const NABM_GLB_CTRL: u16 = 0x2C; // Global Control (32-bit)
const NABM_GLB_STS: u16 = 0x30;  // Global Status (32-bit)

// Control Register bits
const CR_RPBM: u8 = 0x01;  // Run/Pause Bus Master
const CR_RR: u8 = 0x02;    // Reset Registers
const CR_LVBIE: u8 = 0x04; // Last Valid Buffer Interrupt Enable
const CR_FEIE: u8 = 0x08;  // FIFO Error Interrupt Enable
const CR_IOCE: u8 = 0x10;  // Interrupt On Completion Enable

// Status Register bits
const SR_DCH: u16 = 0x01;   // DMA Controller Halted
const SR_CELV: u16 = 0x02;  // Current Equals Last Valid
const SR_LVBCI: u16 = 0x04; // Last Valid Buffer Completion Interrupt
const SR_BCIS: u16 = 0x08;  // Buffer Completion Interrupt Status
const SR_FIFOE: u16 = 0x10; // FIFO Error

// Global Control bits
const GC_GPO_INT_ENABLE: u32 = 0x01;
const GC_COLD_RESET: u32 = 0x02;
const GC_WARM_RESET: u32 = 0x04;

// BDL constants
const BDL_ENTRIES: usize = 32;
const BDL_IOC: u32 = 1 << 31;   // Interrupt On Completion
const BDL_BUP: u32 = 1 << 30;   // Buffer Underrun Policy (fill with last sample)

// Audio buffer size per BDL entry (in bytes)
// 4096 bytes = 1024 sample frames (at 4 bytes each: L16 + R16)
const BUF_SIZE: usize = 4096;
const SAMPLES_PER_BUF: u32 = (BUF_SIZE / 2) as u32; // 16-bit samples count

/// Buffer Descriptor List entry (8 bytes, must be #[repr(C)]).
#[repr(C)]
#[derive(Copy, Clone)]
struct BdlEntry {
    /// Physical address of PCM data buffer.
    buf_addr: u32,
    /// Bits [15:0] = number of 16-bit samples, bit 30 = BUP, bit 31 = IOC.
    ctl_len: u32,
}

struct Ac97State {
    nambar: u16,                  // Mixer I/O base (BAR0)
    nabmbar: u16,                 // Bus Master I/O base (BAR1)
    bdl_phys: u32,                // BDL physical address
    bufs_phys: [u32; BDL_ENTRIES],// Audio buffer physical addresses
    write_idx: u8,                // Next BDL entry to fill
    volume: u8,                   // 0-100
    playing: bool,
    irq: u8,
}

static AC97: Spinlock<Option<Ac97State>> = Spinlock::new(None);

/// Initialize the AC'97 driver from a PCI device.
///
/// Called by the HAL PCI probe when it finds a class 0x04 subclass 0x01 device.
pub fn init_from_pci(pci: &PciDevice) {
    // Extract I/O base addresses from BARs
    let bar0 = pci.bars[0];
    let bar1 = pci.bars[1];

    // AC'97 uses I/O ports (bit 0 set in BAR)
    if bar0 & 1 == 0 || bar1 & 1 == 0 {
        serial_println!("AC97: BARs are not I/O ports (BAR0={:#x}, BAR1={:#x})", bar0, bar1);
        return;
    }

    let nambar = (bar0 & 0xFFFC) as u16;
    let nabmbar = (bar1 & 0xFFFC) as u16;

    serial_println!("AC97: NAMBAR={:#06x}, NABMBAR={:#06x}, IRQ={}", nambar, nabmbar, pci.interrupt_line);

    // Enable PCI bus mastering
    crate::drivers::pci::enable_bus_master(pci);

    // Cold reset via Global Control
    unsafe {
        port::outl(nabmbar + NABM_GLB_CTRL, GC_COLD_RESET);
    }
    // Wait for codec ready (~100μs, use PIT ticks)
    for _ in 0..1000 {
        unsafe { port::io_wait(); }
    }

    // Check Global Status for codec ready
    let gsts = unsafe { port::inl(nabmbar + NABM_GLB_STS) };
    if gsts & 0x100 == 0 {
        serial_println!("AC97: Primary codec not ready (GSTS={:#010x})", gsts);
        // Try warm reset
        unsafe {
            port::outl(nabmbar + NABM_GLB_CTRL, GC_COLD_RESET | GC_WARM_RESET);
        }
        for _ in 0..1000 {
            unsafe { port::io_wait(); }
        }
    }

    // Reset codec
    unsafe { port::outw(nambar + NAM_RESET, 0x0001); }
    for _ in 0..500 {
        unsafe { port::io_wait(); }
    }

    // Set master volume: 0x0000 = max volume, unmuted
    unsafe { port::outw(nambar + NAM_MASTER_VOL, 0x0000); }
    // Set PCM out volume: 0x0808 = moderate
    unsafe { port::outw(nambar + NAM_PCM_OUT_VOL, 0x0808); }

    // Enable variable rate audio if supported
    let ext_id = unsafe { port::inw(nambar + NAM_EXT_AUDIO_ID) };
    if ext_id & 0x0001 != 0 {
        // VRA supported — enable it
        let ext_ctrl = unsafe { port::inw(nambar + NAM_EXT_AUDIO_CTRL) };
        unsafe { port::outw(nambar + NAM_EXT_AUDIO_CTRL, ext_ctrl | 0x0001); }
        // Set sample rate to 48000 Hz
        unsafe { port::outw(nambar + NAM_PCM_FRONT_DAC_RATE, 48000); }
        serial_println!("AC97: VRA enabled, sample rate = 48000 Hz");
    } else {
        serial_println!("AC97: No VRA, using fixed 48000 Hz");
    }

    // Allocate BDL (1 frame = 4096 bytes, we only need 256 bytes for 32 entries)
    let bdl_frame = match physical::alloc_frame() {
        Some(f) => f,
        None => {
            serial_println!("AC97: Failed to allocate BDL frame");
            return;
        }
    };
    let bdl_phys = bdl_frame.as_u32();

    // Zero BDL
    unsafe {
        core::ptr::write_bytes(bdl_phys as *mut u8, 0, 4096);
    }

    // Allocate audio buffers (one frame per BDL entry)
    let mut bufs_phys = [0u32; BDL_ENTRIES];
    for i in 0..BDL_ENTRIES {
        let buf_frame = match physical::alloc_frame() {
            Some(f) => f,
            None => {
                serial_println!("AC97: Failed to allocate audio buffer {}", i);
                return;
            }
        };
        bufs_phys[i] = buf_frame.as_u32();
        // Zero buffer
        unsafe {
            core::ptr::write_bytes(bufs_phys[i] as *mut u8, 0, 4096);
        }
    }

    // Set up BDL entries
    let bdl_ptr = bdl_phys as *mut BdlEntry;
    for i in 0..BDL_ENTRIES {
        unsafe {
            (*bdl_ptr.add(i)).buf_addr = bufs_phys[i];
            (*bdl_ptr.add(i)).ctl_len = SAMPLES_PER_BUF | BDL_IOC;
        }
    }

    // Reset PCM Out channel
    unsafe {
        port::outb(nabmbar + NABM_PO_CR, CR_RR);
    }
    for _ in 0..100 {
        unsafe { port::io_wait(); }
    }
    unsafe {
        port::outb(nabmbar + NABM_PO_CR, 0);
    }

    // Set BDL base address
    unsafe {
        port::outl(nabmbar + NABM_PO_BDBAR, bdl_phys);
    }

    // Clear status bits
    unsafe {
        port::outw(nabmbar + NABM_PO_SR, SR_LVBCI | SR_BCIS | SR_FIFOE);
    }

    let irq = pci.interrupt_line;

    // Store state
    {
        let mut guard = AC97.lock();
        *guard = Some(Ac97State {
            nambar,
            nabmbar,
            bdl_phys,
            bufs_phys,
            write_idx: 0,
            volume: 80,
            playing: false,
            irq,
        });
    }

    // Register IRQ handler
    crate::arch::x86::irq::register_irq(irq, ac97_irq_handler);
    if crate::arch::x86::apic::is_initialized() {
        crate::arch::x86::ioapic::unmask_irq(irq);
    } else {
        crate::arch::x86::pic::unmask(irq);
    }

    serial_println!("[OK] AC'97 initialized (48 kHz, 16-bit stereo, IRQ {})", irq);

    // Register with the generic audio subsystem
    super::register(Box::new(Ac97Driver));
}

/// Write PCM data to the next available DMA buffer.
///
/// `data` must contain 16-bit signed LE stereo samples (4 bytes per frame).
/// Returns the number of bytes actually consumed.
pub fn write_pcm(data: &[u8]) -> usize {
    let mut guard = AC97.lock();
    let state = match guard.as_mut() {
        Some(s) => s,
        None => return 0,
    };

    let mut written = 0usize;

    while written < data.len() {
        let remaining = data.len() - written;
        let chunk = remaining.min(BUF_SIZE);

        let idx = state.write_idx as usize;
        let buf_ptr = state.bufs_phys[idx] as *mut u8;

        // Copy PCM data into DMA buffer
        unsafe {
            core::ptr::copy_nonoverlapping(
                data[written..].as_ptr(),
                buf_ptr,
                chunk,
            );
            // If chunk < BUF_SIZE, zero the remainder
            if chunk < BUF_SIZE {
                core::ptr::write_bytes(buf_ptr.add(chunk), 0, BUF_SIZE - chunk);
            }
        }

        // Update BDL entry sample count
        let sample_count = (chunk / 2) as u32; // 16-bit samples
        let bdl_ptr = state.bdl_phys as *mut BdlEntry;
        unsafe {
            (*bdl_ptr.add(idx)).ctl_len = sample_count | BDL_IOC;
        }

        // Advance LVI to tell hardware about the new buffer
        unsafe {
            port::outb(state.nabmbar + NABM_PO_LVI, state.write_idx);
        }

        state.write_idx = ((state.write_idx as usize + 1) % BDL_ENTRIES) as u8;
        written += chunk;

        // Start playback if not already running
        if !state.playing {
            unsafe {
                let cr = port::inb(state.nabmbar + NABM_PO_CR);
                port::outb(state.nabmbar + NABM_PO_CR, cr | CR_RPBM | CR_IOCE | CR_LVBIE);
            }
            state.playing = true;
        }
    }

    written
}

/// Stop PCM playback.
pub fn stop() {
    let mut guard = AC97.lock();
    if let Some(state) = guard.as_mut() {
        unsafe {
            // Clear Run/Pause bit
            let cr = port::inb(state.nabmbar + NABM_PO_CR);
            port::outb(state.nabmbar + NABM_PO_CR, cr & !CR_RPBM);
        }
        state.playing = false;
        state.write_idx = 0;
    }
}

/// Set master volume (0 = mute, 100 = max).
pub fn set_volume(vol: u8) {
    let mut guard = AC97.lock();
    if let Some(state) = guard.as_mut() {
        state.volume = vol.min(100);

        // AC'97 volume: 0x00 = max (0dB), 0x3F = min (-94.5dB), bit 15 = mute
        // Map 0-100 to 63-0 (inverted)
        let attenuation = if vol == 0 {
            0x8000u16 // mute
        } else {
            let att = ((100 - vol as u16) * 63) / 100;
            (att << 8) | att // same for L and R channels
        };

        unsafe {
            port::outw(state.nambar + NAM_MASTER_VOL, attenuation);
        }
    }
}

/// Get current master volume (0-100).
pub fn get_volume() -> u8 {
    let guard = AC97.lock();
    match guard.as_ref() {
        Some(state) => state.volume,
        None => 0,
    }
}

/// Check if AC'97 hardware is available.
pub fn is_available() -> bool {
    AC97.lock().is_some()
}

/// Check if playback is active.
pub fn is_playing() -> bool {
    let guard = AC97.lock();
    match guard.as_ref() {
        Some(state) => state.playing,
        None => false,
    }
}

// ── AudioDriver trait implementation ─────────────

/// Thin wrapper that delegates to the AC97 static state.
/// The actual state lives in the `AC97` Spinlock above (needed by IRQ handler).
pub struct Ac97Driver;

impl super::AudioDriver for Ac97Driver {
    fn name(&self) -> &str { "Intel AC'97" }
    fn write_pcm(&mut self, data: &[u8]) -> usize { write_pcm(data) }
    fn stop(&mut self) { stop(); }
    fn set_volume(&mut self, vol: u8) { set_volume(vol); }
    fn get_volume(&self) -> u8 { get_volume() }
    fn is_playing(&self) -> bool { is_playing() }
    fn sample_rate(&self) -> u32 { 48000 }
}

/// AC'97 IRQ handler — acknowledges buffer completion interrupts.
fn ac97_irq_handler(_irq: u8) {
    if let Some(mut guard) = AC97.try_lock() {
        if let Some(state) = guard.as_mut() {
            let sr = unsafe { port::inw(state.nabmbar + NABM_PO_SR) };

            if sr & SR_BCIS != 0 {
                // Buffer completed — acknowledge
                unsafe {
                    port::outw(state.nabmbar + NABM_PO_SR, SR_BCIS);
                }
            }

            if sr & SR_LVBCI != 0 {
                // Last valid buffer completed — playback finished
                unsafe {
                    port::outw(state.nabmbar + NABM_PO_SR, SR_LVBCI);
                }
                state.playing = false;
            }

            if sr & SR_FIFOE != 0 {
                // FIFO error — acknowledge
                unsafe {
                    port::outw(state.nabmbar + NABM_PO_SR, SR_FIFOE);
                }
            }
        }
    }
}

/// Probe: initialize AC'97 and return a HAL driver.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    init_from_pci(pci);
    super::create_hal_driver("Intel AC'97 Audio")
}

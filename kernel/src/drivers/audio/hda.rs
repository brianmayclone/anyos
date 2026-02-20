// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Intel High Definition Audio (HDA) controller driver.
//!
//! Supports Intel ICH6/ICH9 HDA controllers as found in VirtualBox and QEMU.
//! Uses CORB/RIRB for codec communication and BDL-based DMA for PCM output
//! at 48 kHz, 16-bit stereo.
//!
//! VirtualBox: uses ICH6 HDA (8086:2668) or ICH9 (8086:293E)
//! QEMU: `-device intel-hda -device hda-output`

use alloc::boxed::Box;
use crate::memory::address::PhysAddr;
use crate::memory::{physical, virtual_mem};
use crate::drivers::pci::PciDevice;
use crate::serial_println;
use crate::sync::spinlock::Spinlock;

// No hardcoded MMIO address — uses dynamic virtual_mem::map_mmio() allocator.

// ── HDA Controller Registers (offsets from BAR0) ────────────────────────────

const REG_GCAP: u32     = 0x00; // Global Capabilities (16-bit)
const REG_GCTL: u32     = 0x08; // Global Control (32-bit)
const REG_WAKEEN: u32   = 0x0C; // Wake Enable (16-bit)
const REG_STATESTS: u32 = 0x0E; // State Change Status (16-bit)
const REG_INTCTL: u32   = 0x20; // Interrupt Control (32-bit)
const REG_INTSTS: u32   = 0x24; // Interrupt Status (32-bit)

// CORB registers
const REG_CORBLBASE: u32 = 0x40; // CORB Lower Base Address (32-bit)
const REG_CORBUBASE: u32 = 0x44; // CORB Upper Base Address (32-bit)
const REG_CORBWP: u32    = 0x48; // CORB Write Pointer (16-bit)
const REG_CORBRP: u32    = 0x4A; // CORB Read Pointer (16-bit)
const REG_CORBCTL: u32   = 0x4C; // CORB Control (8-bit)
const REG_CORBSIZE: u32  = 0x4E; // CORB Size (8-bit)

// RIRB registers
const REG_RIRBLBASE: u32 = 0x50; // RIRB Lower Base Address (32-bit)
const REG_RIRBUBASE: u32 = 0x54; // RIRB Upper Base Address (32-bit)
const REG_RIRBWP: u32    = 0x58; // RIRB Write Pointer (16-bit)
const REG_RINTCNT: u32   = 0x5A; // Response Interrupt Count (16-bit)
const REG_RIRBCTL: u32   = 0x5C; // RIRB Control (8-bit)
const REG_RIRBSTS: u32   = 0x5D; // RIRB Status (8-bit)
const REG_RIRBSIZE: u32  = 0x5E; // RIRB Size (8-bit)

// Stream Descriptor registers (output stream 0)
// Base offset depends on GCAP: input streams come first, then output streams.
// We compute the actual offset dynamically.
const SD_SIZE: u32      = 0x20; // Size of each stream descriptor block
const SD_CTL: u32       = 0x00; // Stream Control (24-bit: offset+0..+2)
const SD_STS: u32       = 0x03; // Stream Status (8-bit)
const SD_LPIB: u32      = 0x04; // Link Position In Buffer (32-bit)
const SD_CBL: u32       = 0x08; // Cyclic Buffer Length (32-bit)
const SD_LVI: u32       = 0x0C; // Last Valid Index (16-bit)
const SD_FMT: u32       = 0x12; // Stream Format (16-bit)
const SD_BDLPL: u32     = 0x18; // BDL Pointer Lower (32-bit)
const SD_BDLPU: u32     = 0x1C; // BDL Pointer Upper (32-bit)

// GCTL bits
const GCTL_CRST: u32 = 1 << 0; // Controller Reset

// CORBCTL bits
const CORBCTL_RUN: u8 = 1 << 1; // CORB DMA Engine Run

// RIRBCTL bits
const RIRBCTL_RUN: u8 = 1 << 1; // RIRB DMA Engine Run

// Stream CTL bits
const SD_CTL_RUN: u32  = 1 << 1;  // Stream Run
const SD_CTL_IOCE: u32 = 1 << 2;  // Interrupt On Completion Enable
const SD_CTL_SRST: u32 = 1 << 0;  // Stream Reset

// Stream STS bits
const SD_STS_BCIS: u8 = 1 << 2; // Buffer Completion Interrupt Status

// Stream Format: 48kHz, 16-bit, stereo
// Bits [14]: 0 = 48kHz base, [13:11]: 000 = x1, [10:8]: 000 = /1
// Bits [7:4]: 0001 = 16-bit, [3:0]: 0001 = 2 channels (stereo)
const FMT_48KHZ_16BIT_STEREO: u16 = 0x0011;

// Codec verbs
const VERB_GET_PARAM: u32      = 0xF0000; // + param_id
const VERB_SET_STREAM: u32     = 0x70600; // + (stream_id << 4) | channel
const VERB_SET_FORMAT: u32     = 0x20000; // + format
const VERB_SET_AMP_GAIN: u32   = 0x30000; // + amp gain/mute bits
const VERB_SET_PIN_WIDGET: u32 = 0x70700; // + pin widget control
const VERB_SET_POWER: u32      = 0x70500; // + power state
const VERB_SET_EAPD: u32       = 0x70C00; // + EAPD/BTL Enable
const VERB_GET_CONN_LIST: u32  = 0xF0200; // Get Connection List Length

// Codec parameters
const PARAM_VENDOR_ID: u32        = 0x00;
const PARAM_NODE_COUNT: u32       = 0x04;
const PARAM_FN_GROUP_TYPE: u32    = 0x05;
const PARAM_AUDIO_WIDGET_CAP: u32 = 0x09;
const PARAM_CONN_LIST_LEN: u32    = 0x0E;

// Widget types (from Audio Widget Capabilities parameter, bits [23:20])
const WIDGET_TYPE_AUDIO_OUTPUT: u32 = 0x0;
const WIDGET_TYPE_AUDIO_INPUT: u32  = 0x1;
const WIDGET_TYPE_AUDIO_MIXER: u32  = 0x2;
const WIDGET_TYPE_AUDIO_SELECTOR: u32 = 0x3;
const WIDGET_TYPE_PIN_COMPLEX: u32  = 0x4;

// BDL constants
const BDL_ENTRIES: usize = 32;
const BUF_SIZE: usize = 4096; // 4 KiB per buffer = 1024 sample frames at 4 bytes/frame
const IOC_FLAG: u32 = 1; // Interrupt On Completion flag in BDL entry

/// BDL entry (16 bytes each, must be aligned to 128 bytes total).
#[repr(C)]
#[derive(Copy, Clone)]
struct BdlEntry {
    addr_low: u32,
    addr_high: u32,
    length: u32,
    ioc: u32, // bit 0 = IOC
}

struct HdaState {
    mmio: u64,                      // MMIO virtual base (BAR0 registers)
    corb_virt: u64,                 // CORB virtual address (CPU access)
    rirb_virt: u64,                 // RIRB virtual address (CPU access)
    corb_phys: u64,                 // CORB physical address (for DMA)
    rirb_phys: u64,                 // RIRB physical address (for DMA)
    corb_wp: u16,                   // Current CORB write pointer
    rirb_rp: u16,                   // Current RIRB read pointer
    out_stream_base: u32,           // MMIO offset of output stream 0
    bdl_virt: u64,                  // BDL virtual address (CPU access)
    bdl_phys: u64,                  // BDL physical address (for DMA)
    bufs_phys: [u64; BDL_ENTRIES],  // PCM buffer physical addresses
    write_idx: u8,                  // Next BDL entry to fill
    volume: u8,                     // 0-100
    playing: bool,
    codec_addr: u8,                 // Detected codec address (usually 0)
    dac_nid: u16,                   // DAC widget NID
    pin_nid: u16,                   // Output pin widget NID
    irq: u8,
}

static HDA: Spinlock<Option<HdaState>> = Spinlock::new(None);

// ── MMIO helpers ────────────────────────────────────────────────────────────

#[inline]
unsafe fn mmio_read32(base: u64, offset: u32) -> u32 {
    core::ptr::read_volatile((base + offset as u64) as *const u32)
}

#[inline]
unsafe fn mmio_write32(base: u64, offset: u32, val: u32) {
    core::ptr::write_volatile((base + offset as u64) as *mut u32, val);
}

#[inline]
unsafe fn mmio_read16(base: u64, offset: u32) -> u16 {
    core::ptr::read_volatile((base + offset as u64) as *const u16)
}

#[inline]
unsafe fn mmio_write16(base: u64, offset: u32, val: u16) {
    core::ptr::write_volatile((base + offset as u64) as *mut u16, val);
}

#[inline]
unsafe fn mmio_read8(base: u64, offset: u32) -> u8 {
    core::ptr::read_volatile((base + offset as u64) as *const u8)
}

#[inline]
unsafe fn mmio_write8(base: u64, offset: u32, val: u8) {
    core::ptr::write_volatile((base + offset as u64) as *mut u8, val);
}

// ── CORB/RIRB communication ────────────────────────────────────────────────

/// Send a verb to the codec via CORB. Returns false on timeout.
fn corb_send(state: &mut HdaState, codec: u8, nid: u16, verb: u32) -> bool {
    let cmd = ((codec as u32) << 28) | ((nid as u32) << 20) | (verb & 0xFFFFF);

    // Advance write pointer
    state.corb_wp = (state.corb_wp + 1) % 256;

    // Write command to CORB
    unsafe {
        let corb_entry = (state.corb_virt + state.corb_wp as u64 * 4) as *mut u32;
        core::ptr::write_volatile(corb_entry, cmd);
    }

    // Update CORB write pointer
    unsafe {
        mmio_write16(state.mmio, REG_CORBWP, state.corb_wp);
    }

    true
}

/// Read a response from RIRB. Returns the response or None on timeout.
fn rirb_read(state: &mut HdaState) -> Option<u32> {
    // Poll for new response (wait for RIRB write pointer to advance)
    for _ in 0..100_000 {
        let wp = unsafe { mmio_read16(state.mmio, REG_RIRBWP) };
        if wp != state.rirb_rp {
            state.rirb_rp = (state.rirb_rp + 1) % 256;

            // Each RIRB entry is 8 bytes: [response:u32, response_ex:u32]
            let entry_addr = state.rirb_virt + state.rirb_rp as u64 * 8;
            let response = unsafe { core::ptr::read_volatile(entry_addr as *const u32) };

            // Clear RIRB interrupt status
            unsafe {
                mmio_write8(state.mmio, REG_RIRBSTS, 0x05);
            }

            return Some(response);
        }
        core::hint::spin_loop();
    }

    None
}

/// Send a verb and wait for the response.
fn codec_command(state: &mut HdaState, nid: u16, verb: u32) -> Option<u32> {
    let codec = state.codec_addr;
    if !corb_send(state, codec, nid, verb) {
        return None;
    }
    rirb_read(state)
}

// ── Init ────────────────────────────────────────────────────────────────────

/// Initialize the HDA controller from a PCI device.
pub fn init_from_pci(pci: &PciDevice) {
    let bar0 = pci.bars[0] & 0xFFFFFFF0;
    if bar0 == 0 {
        serial_println!("HDA: BAR0 is zero");
        return;
    }

    serial_println!("HDA: BAR0 phys = {:#010x}, IRQ = {}", bar0, pci.interrupt_line);

    // Enable bus mastering
    crate::drivers::pci::enable_bus_master(pci);

    // Map BAR0 MMIO (4 pages = 16 KiB for HDA controller registers)
    let mmio = match virtual_mem::map_mmio(PhysAddr::new(bar0 as u64), 4) {
        Some(v) => v.as_u64(),
        None => {
            serial_println!("HDA: Failed to map BAR0 MMIO");
            return;
        }
    };

    let gcap = unsafe { mmio_read16(mmio, REG_GCAP) };
    serial_println!("HDA: GCAP = {:#06x}", gcap);

    // Parse GCAP: number of input/output/bidirectional streams
    let num_iss = ((gcap >> 8) & 0x0F) as u32;  // Input streams
    let num_oss = ((gcap >> 12) & 0x0F) as u32;  // Output streams
    serial_println!("HDA: {} input stream(s), {} output stream(s)", num_iss, num_oss);

    if num_oss == 0 {
        serial_println!("HDA: No output streams available");
        return;
    }

    // Output stream 0 offset: 0x80 + (num_iss * 0x20)
    let out_stream_base = 0x80 + num_iss * SD_SIZE;

    // ── Controller Reset ──
    unsafe {
        // Clear CRST to enter reset
        mmio_write32(mmio, REG_GCTL, 0);
    }
    // Wait for CRST to read 0
    for _ in 0..100_000 {
        if unsafe { mmio_read32(mmio, REG_GCTL) } & GCTL_CRST == 0 {
            break;
        }
        core::hint::spin_loop();
    }

    // Set CRST to exit reset
    unsafe {
        mmio_write32(mmio, REG_GCTL, GCTL_CRST);
    }
    // Wait for CRST to read 1
    for _ in 0..100_000 {
        if unsafe { mmio_read32(mmio, REG_GCTL) } & GCTL_CRST != 0 {
            break;
        }
        core::hint::spin_loop();
    }

    // Short delay for codecs to enumerate
    for _ in 0..50_000 {
        core::hint::spin_loop();
    }

    // Check for codecs
    let statests = unsafe { mmio_read16(mmio, REG_STATESTS) };
    if statests == 0 {
        serial_println!("HDA: No codecs detected (STATESTS=0)");
        return;
    }

    // Find first codec address (bit position in STATESTS)
    let codec_addr = statests.trailing_zeros() as u8;
    serial_println!("HDA: Codec found at address {}", codec_addr);

    // Clear STATESTS
    unsafe {
        mmio_write16(mmio, REG_STATESTS, statests);
    }

    // ── Allocate CORB + RIRB ──
    // CORB: 256 entries × 4 bytes = 1 KiB (fits in one 4 KiB frame)
    // RIRB: 256 entries × 8 bytes = 2 KiB (fits in one 4 KiB frame)
    let corb_frame = match physical::alloc_frame() {
        Some(f) => f,
        None => { serial_println!("HDA: Failed to alloc CORB"); return; }
    };
    let rirb_frame = match physical::alloc_frame() {
        Some(f) => f,
        None => { serial_println!("HDA: Failed to alloc RIRB"); return; }
    };
    let corb_phys = corb_frame.as_u64();
    let rirb_phys = rirb_frame.as_u64();

    // Map CORB + RIRB into virtual space for CPU access (dynamic allocation)
    let corb_virt = match virtual_mem::map_mmio(corb_frame, 1) {
        Some(v) => v.as_u64(),
        None => { serial_println!("HDA: Failed to map CORB"); return; }
    };
    let rirb_virt = match virtual_mem::map_mmio(rirb_frame, 1) {
        Some(v) => v.as_u64(),
        None => { serial_println!("HDA: Failed to map RIRB"); return; }
    };

    // Zero CORB and RIRB
    unsafe {
        core::ptr::write_bytes(corb_virt as *mut u8, 0, 4096);
        core::ptr::write_bytes(rirb_virt as *mut u8, 0, 4096);
    }

    // ── Configure CORB ──
    unsafe {
        // Stop CORB
        mmio_write8(mmio, REG_CORBCTL, 0);
        for _ in 0..1000 { core::hint::spin_loop(); }

        // Set CORB size to 256 entries (size = 0x02)
        mmio_write8(mmio, REG_CORBSIZE, 0x02);

        // Set CORB base address
        mmio_write32(mmio, REG_CORBLBASE, corb_phys as u32);
        mmio_write32(mmio, REG_CORBUBASE, (corb_phys >> 32) as u32);

        // Reset CORB read pointer
        mmio_write16(mmio, REG_CORBRP, 1 << 15); // Set reset bit
        for _ in 0..10_000 {
            if mmio_read16(mmio, REG_CORBRP) & (1 << 15) != 0 { break; }
            core::hint::spin_loop();
        }
        mmio_write16(mmio, REG_CORBRP, 0); // Clear reset bit
        for _ in 0..10_000 {
            if mmio_read16(mmio, REG_CORBRP) & (1 << 15) == 0 { break; }
            core::hint::spin_loop();
        }

        // Reset CORB write pointer
        mmio_write16(mmio, REG_CORBWP, 0);

        // Start CORB
        mmio_write8(mmio, REG_CORBCTL, CORBCTL_RUN);
    }

    // ── Configure RIRB ──
    unsafe {
        // Stop RIRB
        mmio_write8(mmio, REG_RIRBCTL, 0);
        for _ in 0..1000 { core::hint::spin_loop(); }

        // Set RIRB size to 256 entries
        mmio_write8(mmio, REG_RIRBSIZE, 0x02);

        // Set RIRB base address
        mmio_write32(mmio, REG_RIRBLBASE, rirb_phys as u32);
        mmio_write32(mmio, REG_RIRBUBASE, (rirb_phys >> 32) as u32);

        // Reset RIRB write pointer
        mmio_write16(mmio, REG_RIRBWP, 1 << 15);

        // Set response interrupt count
        mmio_write16(mmio, REG_RINTCNT, 1);

        // Start RIRB
        mmio_write8(mmio, REG_RIRBCTL, RIRBCTL_RUN);
    }

    // Short delay for CORB/RIRB to start
    for _ in 0..10_000 {
        core::hint::spin_loop();
    }

    // ── Codec enumeration ──
    let mut state = HdaState {
        mmio,
        corb_virt,
        rirb_virt,
        corb_phys,
        rirb_phys,
        corb_wp: 0,
        rirb_rp: 0,
        out_stream_base,
        bdl_virt: 0,
        bdl_phys: 0,
        bufs_phys: [0; BDL_ENTRIES],
        write_idx: 0,
        volume: 80,
        playing: false,
        codec_addr,
        dac_nid: 0,
        pin_nid: 0,
        irq: pci.interrupt_line,
    };

    // Get codec vendor ID
    if let Some(vendor_id) = codec_command(&mut state, 0, VERB_GET_PARAM | PARAM_VENDOR_ID) {
        serial_println!("HDA: Codec vendor/device = {:#010x}", vendor_id);
    }

    // Get root node count to find function groups
    let node_count = codec_command(&mut state, 0, VERB_GET_PARAM | PARAM_NODE_COUNT)
        .unwrap_or(0);
    let start_nid = ((node_count >> 16) & 0xFF) as u16;
    let num_nodes = (node_count & 0xFF) as u16;

    serial_println!("HDA: Root has {} sub-node(s) starting at NID {}", num_nodes, start_nid);

    // Find Audio Function Group
    let mut afg_nid: u16 = 0;
    for nid in start_nid..start_nid + num_nodes {
        if let Some(fg_type) = codec_command(&mut state, nid, VERB_GET_PARAM | PARAM_FN_GROUP_TYPE) {
            if fg_type & 0xFF == 0x01 {
                // Audio Function Group
                afg_nid = nid;
                serial_println!("HDA: Audio Function Group at NID {}", nid);
                break;
            }
        }
    }

    if afg_nid == 0 {
        serial_println!("HDA: No Audio Function Group found");
        return;
    }

    // Power up the AFG
    codec_command(&mut state, afg_nid, VERB_SET_POWER | 0x00); // D0

    // Get widget count under the AFG
    let widget_count = codec_command(&mut state, afg_nid, VERB_GET_PARAM | PARAM_NODE_COUNT)
        .unwrap_or(0);
    let w_start = ((widget_count >> 16) & 0xFF) as u16;
    let w_num = (widget_count & 0xFF) as u16;

    serial_println!("HDA: AFG has {} widget(s) starting at NID {}", w_num, w_start);

    // Find DAC (Audio Output) and output Pin widgets
    let mut dac_nid: u16 = 0;
    let mut pin_nid: u16 = 0;

    for nid in w_start..w_start + w_num {
        if let Some(wcap) = codec_command(&mut state, nid, VERB_GET_PARAM | PARAM_AUDIO_WIDGET_CAP) {
            let wtype = (wcap >> 20) & 0xF;
            match wtype {
                WIDGET_TYPE_AUDIO_OUTPUT => {
                    if dac_nid == 0 {
                        dac_nid = nid;
                        serial_println!("HDA: DAC (Audio Output) at NID {}", nid);
                    }
                }
                WIDGET_TYPE_PIN_COMPLEX => {
                    // Check if this is an output pin (default config)
                    // For simplicity, take the first pin we find after the DAC
                    if pin_nid == 0 {
                        pin_nid = nid;
                        serial_println!("HDA: Pin Complex at NID {}", nid);
                    }
                }
                _ => {}
            }
        }
    }

    if dac_nid == 0 {
        serial_println!("HDA: No DAC widget found");
        return;
    }

    state.dac_nid = dac_nid;
    state.pin_nid = pin_nid;

    // ── Configure DAC ──
    // Set converter format: 48kHz, 16-bit, stereo
    codec_command(&mut state, dac_nid, VERB_SET_FORMAT | FMT_48KHZ_16BIT_STEREO as u32);

    // Set stream/channel: stream 1, channel 0
    codec_command(&mut state, dac_nid, VERB_SET_STREAM | (1 << 4) | 0);

    // Power up DAC
    codec_command(&mut state, dac_nid, VERB_SET_POWER | 0x00);

    // Set amp gain (output, left+right, no mute, gain = max)
    // Bit 15 = output, bits 13-12 = left+right, bits 6-0 = gain
    codec_command(&mut state, dac_nid, VERB_SET_AMP_GAIN | 0xB07F);

    // Configure output pin if found
    if pin_nid != 0 {
        // Enable output on pin (bit 6 = Out Enable)
        codec_command(&mut state, pin_nid, VERB_SET_PIN_WIDGET | 0x40);

        // Try enabling EAPD if supported
        codec_command(&mut state, pin_nid, VERB_SET_EAPD | 0x02);

        // Set pin amp gain
        codec_command(&mut state, pin_nid, VERB_SET_AMP_GAIN | 0xB07F);

        // Power up pin
        codec_command(&mut state, pin_nid, VERB_SET_POWER | 0x00);
    }

    // ── Allocate BDL + PCM buffers ──
    let bdl_frame = match physical::alloc_frame() {
        Some(f) => f,
        None => { serial_println!("HDA: Failed to alloc BDL"); return; }
    };
    state.bdl_phys = bdl_frame.as_u64();

    // Map BDL for CPU access (dynamic allocation)
    let bdl_virt = match virtual_mem::map_mmio(bdl_frame, 1) {
        Some(v) => v.as_u64(),
        None => { serial_println!("HDA: Failed to map BDL"); return; }
    };
    state.bdl_virt = bdl_virt;
    unsafe { core::ptr::write_bytes(bdl_virt as *mut u8, 0, 4096); }

    // Allocate PCM buffers
    for i in 0..BDL_ENTRIES {
        let buf_frame = match physical::alloc_frame() {
            Some(f) => f,
            None => {
                serial_println!("HDA: Failed to alloc PCM buffer {}", i);
                return;
            }
        };
        state.bufs_phys[i] = buf_frame.as_u64();
        unsafe { core::ptr::write_bytes(buf_frame.as_u64() as *mut u8, 0, 4096); }
    }

    // Set up BDL entries
    let bdl_ptr = bdl_virt as *mut BdlEntry;
    for i in 0..BDL_ENTRIES {
        unsafe {
            (*bdl_ptr.add(i)).addr_low = state.bufs_phys[i] as u32;
            (*bdl_ptr.add(i)).addr_high = (state.bufs_phys[i] >> 32) as u32;
            (*bdl_ptr.add(i)).length = BUF_SIZE as u32;
            (*bdl_ptr.add(i)).ioc = IOC_FLAG;
        }
    }

    // ── Configure Output Stream 0 ──
    let sd = out_stream_base;
    unsafe {
        // Reset stream
        let ctl = mmio_read32(mmio, sd + SD_CTL) & 0xFF;
        mmio_write32(mmio, sd + SD_CTL, ctl | SD_CTL_SRST);
        for _ in 0..10_000 {
            if mmio_read32(mmio, sd + SD_CTL) & SD_CTL_SRST != 0 { break; }
            core::hint::spin_loop();
        }
        // Clear reset
        mmio_write32(mmio, sd + SD_CTL, ctl & !SD_CTL_SRST);
        for _ in 0..10_000 {
            if mmio_read32(mmio, sd + SD_CTL) & SD_CTL_SRST == 0 { break; }
            core::hint::spin_loop();
        }

        // Set stream format
        mmio_write16(mmio, sd + SD_FMT, FMT_48KHZ_16BIT_STEREO);

        // Set cyclic buffer length (total bytes in all BDL entries)
        let total_len = (BDL_ENTRIES * BUF_SIZE) as u32;
        mmio_write32(mmio, sd + SD_CBL, total_len);

        // Set last valid index
        mmio_write16(mmio, sd + SD_LVI, (BDL_ENTRIES - 1) as u16);

        // Set BDL pointer
        mmio_write32(mmio, sd + SD_BDLPL, state.bdl_phys as u32);
        mmio_write32(mmio, sd + SD_BDLPU, (state.bdl_phys >> 32) as u32);

        // Set stream ID (stream 1 in bits [23:20] of CTL)
        let ctl_val = mmio_read32(mmio, sd + SD_CTL);
        // Clear old stream number, set stream 1
        let ctl_val = (ctl_val & 0xFF0FFFFF) | (1 << 20);
        mmio_write32(mmio, sd + SD_CTL, ctl_val | SD_CTL_IOCE);
    }

    // Enable global interrupts
    unsafe {
        // Enable stream interrupt + global interrupt enable (bit 31) + controller interrupt enable
        let stream_mask = 1u32 << (num_iss); // Output stream 0 is after input streams
        mmio_write32(mmio, REG_INTCTL, (1 << 31) | stream_mask);
    }

    // Register IRQ handler
    let irq = pci.interrupt_line;
    crate::arch::x86::irq::register_irq(irq, hda_irq_handler);
    if crate::arch::x86::apic::is_initialized() {
        crate::arch::x86::ioapic::unmask_irq(irq);
    } else {
        crate::arch::x86::pic::unmask(irq);
    }

    serial_println!("[OK] Intel HDA initialized (48 kHz, 16-bit stereo, IRQ {})", irq);

    // Store state and register with generic audio subsystem
    {
        let mut guard = HDA.lock();
        *guard = Some(state);
    }

    super::register(Box::new(HdaDriver));
}

// ── PCM Playback ────────────────────────────────────────────────────────────

/// Write PCM data to the next available DMA buffer.
pub fn write_pcm(data: &[u8]) -> usize {
    let mut guard = HDA.lock();
    let state = match guard.as_mut() {
        Some(s) => s,
        None => return 0,
    };

    let bdl_virt = state.bdl_virt;
    let mut written = 0usize;

    while written < data.len() {
        let remaining = data.len() - written;
        let chunk = remaining.min(BUF_SIZE);

        let idx = state.write_idx as usize;
        let buf_virt = state.bufs_phys[idx]; // Physical = virtual in identity-mapped low memory

        // Copy PCM data
        unsafe {
            core::ptr::copy_nonoverlapping(
                data[written..].as_ptr(),
                buf_virt as *mut u8,
                chunk,
            );
            if chunk < BUF_SIZE {
                core::ptr::write_bytes((buf_virt as *mut u8).add(chunk), 0, BUF_SIZE - chunk);
            }
        }

        // Update BDL entry length
        let bdl_ptr = bdl_virt as *mut BdlEntry;
        unsafe {
            (*bdl_ptr.add(idx)).length = chunk as u32;
        }

        // Update LVI
        unsafe {
            mmio_write16(state.mmio, state.out_stream_base + SD_LVI, state.write_idx as u16);
        }

        state.write_idx = ((state.write_idx as usize + 1) % BDL_ENTRIES) as u8;
        written += chunk;

        // Start playback if not already running
        if !state.playing {
            unsafe {
                let sd = state.out_stream_base;
                let ctl = mmio_read32(state.mmio, sd + SD_CTL);
                mmio_write32(state.mmio, sd + SD_CTL, ctl | SD_CTL_RUN | SD_CTL_IOCE);
            }
            state.playing = true;
        }
    }

    written
}

/// Stop playback.
pub fn stop() {
    let mut guard = HDA.lock();
    if let Some(state) = guard.as_mut() {
        unsafe {
            let sd = state.out_stream_base;
            let ctl = mmio_read32(state.mmio, sd + SD_CTL);
            mmio_write32(state.mmio, sd + SD_CTL, ctl & !SD_CTL_RUN);
        }
        state.playing = false;
        state.write_idx = 0;
    }
}

/// Set master volume (0-100).
pub fn set_volume(vol: u8) {
    let mut guard = HDA.lock();
    if let Some(state) = guard.as_mut() {
        state.volume = vol.min(100);

        // Map 0-100 to 0-127 gain (7-bit HDA amp gain)
        let gain = if vol == 0 {
            0x0080u32 // mute bit (bit 7)
        } else {
            ((vol as u32) * 127) / 100
        };

        // Set amp: output, left+right, gain value
        // Bit 15 = output, bit 13 = left, bit 12 = right, bits 6:0 = gain
        let verb = VERB_SET_AMP_GAIN | 0xB000 | (gain & 0x7F);

        // Send directly via CORB (already holding the lock)
        let codec = state.codec_addr;
        let nid = state.dac_nid;
        let cmd = ((codec as u32) << 28) | ((nid as u32) << 20) | (verb & 0xFFFFF);
        state.corb_wp = (state.corb_wp + 1) % 256;
        unsafe {
            let entry = (state.corb_virt + state.corb_wp as u64 * 4) as *mut u32;
            core::ptr::write_volatile(entry, cmd);
            mmio_write16(state.mmio, REG_CORBWP, state.corb_wp);
        }
    }
}

/// Get current volume (0-100).
pub fn get_volume() -> u8 {
    let guard = HDA.lock();
    match guard.as_ref() {
        Some(state) => state.volume,
        None => 0,
    }
}

/// Check if HDA is available.
pub fn is_available() -> bool {
    HDA.lock().is_some()
}

/// Check if playback is active.
pub fn is_playing() -> bool {
    let guard = HDA.lock();
    match guard.as_ref() {
        Some(state) => state.playing,
        None => false,
    }
}

// ── AudioDriver trait implementation ────────────────────────────────────────

pub struct HdaDriver;

impl super::AudioDriver for HdaDriver {
    fn name(&self) -> &str { "Intel HDA" }
    fn write_pcm(&mut self, data: &[u8]) -> usize { write_pcm(data) }
    fn stop(&mut self) { stop(); }
    fn set_volume(&mut self, vol: u8) { set_volume(vol); }
    fn get_volume(&self) -> u8 { get_volume() }
    fn is_playing(&self) -> bool { is_playing() }
    fn sample_rate(&self) -> u32 { 48000 }
}

// ── IRQ handler ─────────────────────────────────────────────────────────────

fn hda_irq_handler(_irq: u8) {
    if let Some(mut guard) = HDA.try_lock() {
        if let Some(state) = guard.as_mut() {
            let mmio = state.mmio;

            // Check global interrupt status
            let intsts = unsafe { mmio_read32(mmio, REG_INTSTS) };
            if intsts == 0 {
                return;
            }

            // Check stream status
            let sd = state.out_stream_base;
            let sts = unsafe { mmio_read8(mmio, sd + SD_STS) };

            if sts & SD_STS_BCIS != 0 {
                // Buffer completion — acknowledge
                unsafe {
                    mmio_write8(mmio, sd + SD_STS, SD_STS_BCIS);
                }
            }

            // Clear global interrupt status
            unsafe {
                mmio_write32(mmio, REG_INTSTS, intsts);
            }
        }
    }
}

/// Probe: initialize Intel HDA and return a HAL driver.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    init_from_pci(pci);
    super::create_hal_driver("Intel HDA Audio")
}

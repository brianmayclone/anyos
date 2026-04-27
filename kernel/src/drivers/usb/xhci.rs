//! xHCI (eXtensible Host Controller Interface) driver — USB 3.x / 2.0 / 1.x.
//!
//! xHCI replaces UHCI, OHCI and EHCI on all modern hardware.  A single
//! controller handles all USB speeds:
//!   - SuperSpeed (5 Gbps, USB 3.0)
//!   - SuperSpeed+ (10 Gbps, USB 3.1 Gen 2) — treated as SuperSpeed here
//!   - High-Speed (480 Mbps, USB 2.0)
//!   - Full-Speed (12 Mbps, USB 1.1)
//!   - Low-Speed (1.5 Mbps, USB 1.0)
//!
//! # Implementation scope
//!
//! This driver implements the minimum viable subset needed to enumerate and
//! operate USB devices through the existing class-driver infrastructure
//! (HID, Mass Storage, CDC-ECM, CDC-ACM, Hub):
//!
//! - Controller initialisation (reset, capability/operational register setup)
//! - Device Slot allocation and `Enable Slot` / `Address Device` commands
//! - Control transfers (Setup → Data → Status via the command ring)
//! - Bulk transfers (via transfer rings on non-EP0 endpoints)
//! - Port scanning and hot-plug detection
//! - BIOS hand-off (XHCI_USBLEGSUP)
//!
//! Isochronous transfers and streams are not implemented.
//!
//! # Memory layout
//!
//! All DMA structures are allocated from contiguous physical pages.
//! The controller is mapped into virtual address space at
//! [`XHCI_MMIO_BASE`] (16 pages = 64 KiB).
//!
//! ## Ring layout (per-slot, per-endpoint)
//!
//! Each transfer ring is [`RING_SIZE`] TRBs × 16 bytes = 1 page.
//! The command ring and event ring each occupy one page as well.
//!
//! ```text
//! Page 0  : Device Context Base Address Array (DCBAA) + scratchpad ptr
//! Page 1  : Command Ring  (256 TRBs)
//! Page 2  : Event Ring    (256 TRBs)
//! Page 3  : Event Ring Segment Table (1 entry)
//! Page 4  : Input Context for slot 1
//! Page 5  : Device Context for slot 1
//! Page 6  : EP0 Transfer Ring for slot 1
//! Page 7  : Data buffer (for control transfers)
//! Pages 8+ : EP1..EP31 transfer rings (allocated on demand, per-slot)
//! ```
//!
//! Currently only a **single slot** (slot 1) is used at a time for the
//! enumeration / control-transfer path.  Future work can extend to per-device
//! slot management.

use super::*;
use crate::arch::x86::pit::delay_ms;
use crate::drivers::pci::{pci_config_read32, pci_config_write32, PciDevice};
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::{physical, virtual_mem};

// ── MMIO virtual address ──────────────────────────────────────────────────────

/// Virtual address for xHCI MMIO registers.
/// Placed after the kdrv extern region at 0xFFFF_FFFF_D00A_0000,
/// before VMMDev at 0xFFFF_FFFF_D012_0000.
const XHCI_MMIO_BASE: u64 = 0xFFFF_FFFF_D010_0000;
const XHCI_MMIO_PAGES: usize = 16; // 64 KiB

// ── Capability register offsets (from MMIO base) ─────────────────────────────

const CAP_CAPLENGTH: u32 = 0x00; // u8  — length of capability registers
const CAP_HCIVERSION: u32 = 0x02; // u16 — interface version
const CAP_HCSPARAMS1: u32 = 0x04; // u32 — MaxSlots, MaxIntrs, MaxPorts
const CAP_HCSPARAMS2: u32 = 0x08; // u32 — Scratchpad buffer count in bits [9:5]+[25:21]
const CAP_HCCPARAMS1: u32 = 0x10; // u32 — Context Size (CSZ), 64-bit addressing
const CAP_DBOFF: u32 = 0x14; // u32 — Doorbell array offset
const CAP_RTSOFF: u32 = 0x18; // u32 — Runtime register space offset

// ── Operational register offsets (from op_base = MMIO + CAPLENGTH) ────────────

const OP_USBCMD: u32 = 0x00;
const OP_USBSTS: u32 = 0x04;
const OP_PAGESIZE: u32 = 0x08;
const OP_DNCTRL: u32 = 0x14;
const OP_CRCR: u32 = 0x18; // u64
const OP_DCBAAP: u32 = 0x30; // u64
const OP_CONFIG: u32 = 0x38;
const OP_PORTSC_BASE: u32 = 0x400; // PORTSC[0] starts here; each port = 16 bytes

// USBCMD bits
const CMD_RUN: u32 = 1 << 0;
const CMD_HCRST: u32 = 1 << 1;
const CMD_INTE: u32 = 1 << 2; // Interrupter enable (we keep this clear — polled)
const CMD_HSEE: u32 = 1 << 3; // Host System Error Enable

// USBSTS bits
const STS_HCH: u32 = 1 << 0; // Host Controller Halted
const STS_CNR: u32 = 1 << 11; // Controller Not Ready

// PORTSC bits (offset per port = OP_PORTSC_BASE + port_idx * 0x10)
const PORTSC_CCS: u32 = 1 << 0; // Current Connect Status
const PORTSC_PED: u32 = 1 << 1; // Port Enabled/Disabled
const PORTSC_PR: u32 = 1 << 4; // Port Reset
const PORTSC_PLS_MASK: u32 = 0xF << 5; // Port Link State
const PORTSC_PP: u32 = 1 << 9; // Port Power
const PORTSC_CSC: u32 = 1 << 17; // Connect Status Change  (W1C)
const PORTSC_PRC: u32 = 1 << 21; // Port Reset Change      (W1C)
const PORTSC_WRC: u32 = 1 << 19; // Warm Port Reset Change (W1C)
const PORTSC_PEC: u32 = 1 << 18; // Port Enable/Disable Change (W1C)
const PORTSC_SPEED_MASK: u32 = 0xF << 10; // Port Speed field
const PORTSC_SPEED_SHIFT: u32 = 10;
// PSI values (Protocol Speed ID) — stored in PORTSC[13:10]
const PSI_FULL: u32 = 1; // Full-Speed  (USB 1.x)
const PSI_LOW: u32 = 2; // Low-Speed   (USB 1.x)
const PSI_HIGH: u32 = 3; // High-Speed  (USB 2.0)
const PSI_SUPER: u32 = 4; // SuperSpeed  (USB 3.x)

// ── Runtime register offsets (from rt_base = MMIO + RTSOFF) ──────────────────

const RT_IMAN0: u32 = 0x20; // Interrupter 0 management
const RT_IMOD0: u32 = 0x24; // Interrupter 0 moderation
const RT_ERSTSZ0: u32 = 0x28; // Event Ring Segment Table Size
const RT_ERSTBA0: u32 = 0x30; // u64 — Event Ring Segment Table Base Address
const RT_ERDP0: u32 = 0x38; // u64 — Event Ring Dequeue Pointer

// ── Doorbell register array (from db_base = MMIO + DBOFF) ────────────────────
// DB[0]    = Host Controller (command ring doorbell)
// DB[slot] = Device Slot doorbell (bits[7:0] = endpoint, bits[31:16] = stream ID)

// ── TRB types ─────────────────────────────────────────────────────────────────

const TRB_NORMAL: u32 = 1;
const TRB_SETUP_STAGE: u32 = 2;
const TRB_DATA_STAGE: u32 = 3;
const TRB_STATUS_STAGE: u32 = 4;
const TRB_LINK: u32 = 6;
const TRB_ENABLE_SLOT: u32 = 9;
const TRB_DISABLE_SLOT: u32 = 10;
const TRB_ADDRESS_DEVICE: u32 = 11;
const TRB_CONFIG_ENDPOINT: u32 = 12;
const TRB_EVALUATE_CONTEXT: u32 = 13;
const TRB_NOOP_CMD: u32 = 23;

// Event TRB types
const TRB_TRANSFER_EVENT: u32 = 32;
const TRB_CMD_COMPLETION: u32 = 33;
const TRB_PORT_STATUS: u32 = 34;

// TRB flags
const TRB_C: u32 = 1 << 0; // Cycle bit
const TRB_ENT: u32 = 1 << 1; // Evaluate Next TRB
const TRB_ISP: u32 = 1 << 2; // Interrupt on Short Packet
const TRB_NS: u32 = 1 << 3; // No Snoop
const TRB_CH: u32 = 1 << 4; // Chain bit (linked TRBs)
const TRB_IOC: u32 = 1 << 5; // Interrupt On Completion
const TRB_IDT: u32 = 1 << 6; // Immediate Data (setup stage)
const TRB_BEI: u32 = 1 << 9; // Block Event Interrupt

// Data Stage direction
const TRB_DIR_IN: u32 = 1 << 16;
const TRB_DIR_OUT: u32 = 0;

// Completion codes (in event TRB status[31:24])
const CC_SUCCESS: u8 = 1;
const CC_DATA_BUFFER: u8 = 2;
const CC_BABBLE: u8 = 3;
const CC_USB_TRANS_ERR: u8 = 4;
const CC_TRB_ERROR: u8 = 5;
const CC_STALL: u8 = 6;
const CC_SHORT_PACKET: u8 = 13;

// ── Context sizes ─────────────────────────────────────────────────────────────

/// xHCI contexts are either 32 or 64 bytes wide, depending on CSZ in HCCPARAMS1.
const CTX_SIZE_NORMAL: usize = 32;
const CTX_SIZE_LARGE: usize = 64;

/// Number of TRBs per ring (must fit in one 4 KiB page: 256 × 16 = 4096).
const RING_SIZE: usize = 256;

// ── DMA structures ────────────────────────────────────────────────────────────

/// A single Transfer Request Block (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Trb {
    param: u64,
    status: u32,
    ctrl: u32,
}

// ── Controller state ──────────────────────────────────────────────────────────

struct XhciController {
    mmio_base: u64,
    op_base: u64,
    rt_base: u64,
    db_base: u64,
    n_ports: u8,
    ctx_size: usize, // 32 or 64 bytes per context entry

    // Command ring
    cmd_ring_phys: u64,
    cmd_enqueue: usize, // index of next TRB to write
    cmd_cycle: u32,     // current producer cycle bit

    // Event ring
    evt_ring_phys: u64,
    evt_erst_phys: u64, // Event Ring Segment Table
    evt_dequeue: usize, // index of next event TRB to read
    evt_cycle: u32,     // current consumer cycle bit

    // Per-slot DMA pages (slot 1 only for now)
    input_ctx_phys: u64,
    device_ctx_phys: u64,
    ep0_ring_phys: u64,

    // General-purpose data buffer (control transfer payloads)
    data_buf_phys: u64,

    // DCBAA (Device Context Base Address Array)
    dcbaa_phys: u64,

    // Per-port connection tracking for hot-plug
    port_connected: [bool; 32],
}

static XHCI_CTRL: crate::sync::spinlock::Spinlock<Option<XhciController>> =
    crate::sync::spinlock::Spinlock::new(None);

// ── MMIO helpers ─────────────────────────────────────────────────────────────

#[inline]
fn rd32(base: u64, off: u32) -> u32 {
    unsafe { core::ptr::read_volatile((base + off as u64) as *const u32) }
}

#[inline]
fn wr32(base: u64, off: u32, v: u32) {
    unsafe { core::ptr::write_volatile((base + off as u64) as *mut u32, v) }
}

#[inline]
fn rd64(base: u64, off: u32) -> u64 {
    unsafe { core::ptr::read_volatile((base + off as u64) as *const u64) }
}

#[inline]
fn wr64(base: u64, off: u32, v: u64) {
    unsafe { core::ptr::write_volatile((base + off as u64) as *mut u64, v) }
}

#[inline]
fn rd8(base: u64, off: u32) -> u8 {
    unsafe { core::ptr::read_volatile((base + off as u64) as *const u8) }
}

// ── Port PORTSC offset ────────────────────────────────────────────────────────

#[inline]
fn portsc_off(port_idx: u8) -> u32 {
    OP_PORTSC_BASE + (port_idx as u32) * 0x10
}

// ── TRB ring helpers ──────────────────────────────────────────────────────────

/// Write a TRB into a ring at `index`, setting the cycle bit from `cycle`.
unsafe fn ring_write(ring_phys: u64, index: usize, mut trb: Trb, cycle: u32) {
    trb.ctrl = (trb.ctrl & !TRB_C) | cycle;
    let ptr = (ring_phys + (index * 16) as u64) as *mut Trb;
    core::ptr::write_volatile(ptr, trb);
}

/// Build a Link TRB that wraps the ring back to `ring_phys` and toggles cycle.
fn make_link_trb(ring_phys: u64, cycle: u32) -> Trb {
    Trb {
        param: ring_phys,
        status: 0,
        ctrl: (TRB_LINK << 10) | TRB_C | (1 << 1) /* TC */ | cycle,
    }
}

// ── Command ring ─────────────────────────────────────────────────────────────

/// Enqueue one command TRB and ring the host-controller doorbell (DB[0]).
/// Blocks until the matching Command Completion Event arrives.
fn send_command(ctrl: &mut XhciController, trb: Trb) -> Result<u32, &'static str> {
    let idx = ctrl.cmd_enqueue;

    // Detect ring-full: leave one slot for the Link TRB
    if idx >= RING_SIZE - 1 {
        return Err("xHCI: command ring full");
    }

    unsafe {
        ring_write(ctrl.cmd_ring_phys, idx, trb, ctrl.cmd_cycle);
    }
    ctrl.cmd_enqueue = idx + 1;

    // If we've reached the last slot before the Link TRB, insert it and wrap
    if ctrl.cmd_enqueue == RING_SIZE - 1 {
        let link = make_link_trb(ctrl.cmd_ring_phys, ctrl.cmd_cycle);
        unsafe {
            ring_write(ctrl.cmd_ring_phys, RING_SIZE - 1, link, ctrl.cmd_cycle);
        }
        ctrl.cmd_enqueue = 0;
        ctrl.cmd_cycle ^= 1;
    }

    // Ring HC doorbell 0 (command ring)
    wr32(ctrl.db_base, 0, 0);

    // Wait for Command Completion Event
    let timeout_ms = 2000u32;
    let start = crate::arch::x86::pit::get_ticks();

    loop {
        if let Some(cc) = poll_event(ctrl, TRB_CMD_COMPLETION) {
            // cc = completion code in bits [31:24] of status
            let code = ((cc >> 24) & 0xFF) as u8;
            return match code {
                CC_SUCCESS => Ok(cc),
                CC_SHORT_PACKET => Ok(cc),
                _ => Err("xHCI: command completion error"),
            };
        }

        if crate::arch::x86::pit::get_ticks().wrapping_sub(start) > timeout_ms {
            return Err("xHCI: command timeout");
        }

        core::hint::spin_loop();
    }
}

// ── Event ring ────────────────────────────────────────────────────────────────

/// Poll the event ring for the next event of `expected_type`.
/// Returns the `status` field of the event TRB, or `None` if no matching event.
/// Advances the dequeue pointer so the controller can reuse the slot.
fn poll_event(ctrl: &mut XhciController, expected_type: u32) -> Option<u32> {
    let ptr = (ctrl.evt_ring_phys + (ctrl.evt_dequeue * 16) as u64) as *const Trb;
    let trb = unsafe { core::ptr::read_volatile(ptr) };

    // Cycle bit must match consumer cycle to be a valid event
    if (trb.ctrl & TRB_C) != ctrl.evt_cycle {
        return None;
    }

    let trb_type = (trb.ctrl >> 10) & 0x3F;
    if trb_type != expected_type {
        // Consume the event anyway to keep the ring moving
        advance_event_ring(ctrl);
        return None;
    }

    let status = trb.status;
    advance_event_ring(ctrl);
    Some(status)
}

fn advance_event_ring(ctrl: &mut XhciController) {
    ctrl.evt_dequeue += 1;
    if ctrl.evt_dequeue >= RING_SIZE {
        ctrl.evt_dequeue = 0;
        ctrl.evt_cycle ^= 1;
    }
    // Update ERDP so the controller knows we have consumed the event
    let erdp = ctrl.evt_ring_phys + (ctrl.evt_dequeue * 16) as u64;
    wr64(ctrl.rt_base, RT_ERDP0, erdp | (1 << 3)); // EHB = clear busy
}

// ── Slot / device context helpers ────────────────────────────────────────────

/// Returns a pointer to the n-th 32-byte (or 64-byte) entry in a context page.
#[inline]
fn ctx_entry(base_phys: u64, index: usize, ctx_size: usize) -> *mut u32 {
    (base_phys + (index * ctx_size) as u64) as *mut u32
}

/// Write a 32-bit field at `offset` within a context entry.
#[inline]
unsafe fn ctx_write(base_phys: u64, entry: usize, offset: usize, val: u32, ctx_size: usize) {
    let ptr = (base_phys + (entry * ctx_size + offset) as u64) as *mut u32;
    core::ptr::write_volatile(ptr, val);
}

// ── Enable Slot + Address Device ─────────────────────────────────────────────

/// Allocate device slot 1 and send Address Device command.
/// Returns the slot ID (always 1 in our single-slot implementation).
fn enable_and_address_device(
    ctrl: &mut XhciController,
    port_idx: u8,
    speed: UsbSpeed,
    new_addr: u8,
) -> Result<u8, &'static str> {
    // ── Enable Slot ──────────────────────────────────────────────
    let enable_slot = Trb {
        param: 0,
        status: 0,
        ctrl: TRB_ENABLE_SLOT << 10,
    };
    send_command(ctrl, enable_slot)?;
    // In a full implementation we would read the slot ID from the event TRB.
    // Since we allocate only one slot, we assume slot 1.
    let slot_id: u8 = 1;

    // ── Prepare Input Context ─────────────────────────────────────
    // Clear Input Context
    unsafe {
        core::ptr::write_bytes(ctrl.input_ctx_phys as *mut u8, 0, 4096);
    }

    let cs = ctrl.ctx_size;

    // Input Control Context (entry 0): A1 = 1, A0 = 1 (Slot + EP0)
    unsafe {
        ctx_write(ctrl.input_ctx_phys, 0, 4, 0b11, cs); // Add Context flags: A1|A0
    }

    // Slot Context (entry 1):
    //   Route String = 0 (root hub port)
    //   Speed field [19:16]
    //   Context Entries = 1 (only EP0)
    //   Root Hub Port Number [23:16] in dword 1
    let speed_field: u32 = match speed {
        UsbSpeed::Low => 2,
        UsbSpeed::Full => 1,
        UsbSpeed::High => 3,
        UsbSpeed::Super => 4,
    };
    let slot_dw0: u32 = (1 << 27) /* Context Entries = 1 */
        | (speed_field << 20);
    let slot_dw1: u32 = ((port_idx as u32 + 1) << 16); // Root Hub Port Number (1-based)
    unsafe {
        ctx_write(ctrl.input_ctx_phys, 1, 0, slot_dw0, cs);
        ctx_write(ctrl.input_ctx_phys, 1, 4, slot_dw1, cs);
    }

    // EP0 Context (entry 2):
    //   EP Type = 4 (Control Bidirectional)
    //   Max Packet Size
    //   Max Burst Size = 0
    //   Dequeue Pointer = ep0_ring_phys | DCS=1
    let max_packet: u32 = match speed {
        UsbSpeed::Low => 8,
        UsbSpeed::Full => 8,
        UsbSpeed::High => 64,
        UsbSpeed::Super => 512,
    };
    let ep_dw1: u32 = (max_packet << 16)  // Max Packet Size
        | (4 << 3); // EP Type = Control Bidir
    let ep_dq_lo: u32 = (ctrl.ep0_ring_phys as u32) | 1; // DCS = 1
    let ep_dq_hi: u32 = (ctrl.ep0_ring_phys >> 32) as u32;
    unsafe {
        ctx_write(ctrl.input_ctx_phys, 2, 4, ep_dw1, cs);
        ctx_write(ctrl.input_ctx_phys, 2, 8, ep_dq_lo, cs);
        ctx_write(ctrl.input_ctx_phys, 2, 12, ep_dq_hi, cs);
    }

    // ── Address Device command ────────────────────────────────────
    let addr_dev = Trb {
        param: ctrl.input_ctx_phys,
        status: 0,
        ctrl: ((slot_id as u32) << 24) | (TRB_ADDRESS_DEVICE << 10),
    };
    send_command(ctrl, addr_dev)?;

    Ok(slot_id)
}

// ── EP0 Transfer Ring (control transfers) ────────────────────────────────────

struct Ep0Ring {
    phys: u64,
    enqueue: usize,
    cycle: u32,
}

impl Ep0Ring {
    fn new(phys: u64) -> Self {
        // Ensure ring is zeroed and Link TRB is set up
        unsafe {
            core::ptr::write_bytes(phys as *mut u8, 0, 4096);
            // Link TRB at last slot wraps ring with toggle-cycle
            let link_off = ((RING_SIZE - 1) * 16) as u64;
            let ptr = (phys + link_off) as *mut Trb;
            core::ptr::write_volatile(
                ptr,
                Trb {
                    param: phys,
                    status: 0,
                    ctrl: (TRB_LINK << 10) | TRB_C | (1 << 1), // TC=1, C=1
                },
            );
        }
        Self {
            phys,
            enqueue: 0,
            cycle: 1,
        }
    }

    fn push(&mut self, mut trb: Trb) {
        trb.ctrl = (trb.ctrl & !TRB_C) | self.cycle;
        unsafe {
            let ptr = (self.phys + (self.enqueue * 16) as u64) as *mut Trb;
            core::ptr::write_volatile(ptr, trb);
        }
        self.enqueue += 1;
        if self.enqueue >= RING_SIZE - 1 {
            // Toggle cycle bit in the link TRB and wrap
            unsafe {
                let link_ptr = (self.phys + ((RING_SIZE - 1) * 16) as u64) as *mut Trb;
                let mut link = core::ptr::read_volatile(link_ptr);
                link.ctrl = (link.ctrl & !TRB_C) | self.cycle;
                core::ptr::write_volatile(link_ptr, link);
            }
            self.enqueue = 0;
            self.cycle ^= 1;
        }
    }
}

// ── Control transfer ─────────────────────────────────────────────────────────

/// Execute a USB control transfer on EP0 of slot `slot_id`.
/// Returns the number of bytes received (for IN transfers).
fn control_transfer(
    ctrl: &mut XhciController,
    slot_id: u8,
    setup: &SetupPacket,
    data_in: bool,
    data_len: u16,
) -> Result<usize, &'static str> {
    let mut ring = Ep0Ring::new(ctrl.ep0_ring_phys);

    // Encode setup packet as 8-byte immediate data in param field
    let setup_bytes: u64 = unsafe { core::mem::transmute_copy(setup) };

    // ── Setup Stage TRB ──────────────────────────────────────────
    let trt: u32 = if data_len == 0 {
        0 // No data stage
    } else if data_in {
        3 // IN data stage
    } else {
        2 // OUT data stage
    };
    ring.push(Trb {
        param: setup_bytes,
        status: 8, // TRB Transfer Length = 8 (always for setup)
        ctrl: (TRB_SETUP_STAGE << 10) | TRB_IDT | TRB_IOC | (trt << 16),
    });

    // ── Data Stage TRB(s) ─────────────────────────────────────────
    if data_len > 0 {
        let dir = if data_in { TRB_DIR_IN } else { TRB_DIR_OUT };
        ring.push(Trb {
            param: ctrl.data_buf_phys,
            status: data_len as u32,
            ctrl: (TRB_DATA_STAGE << 10) | dir | TRB_IOC | TRB_ISP,
        });
    }

    // ── Status Stage TRB ─────────────────────────────────────────
    // Direction is opposite of data stage (IN data → OUT status, etc.)
    let status_dir = if data_in || data_len == 0 {
        TRB_DIR_OUT
    } else {
        TRB_DIR_IN
    };
    ring.push(Trb {
        param: 0,
        status: 0,
        ctrl: (TRB_STATUS_STAGE << 10) | status_dir | TRB_IOC,
    });

    // Persist updated ring state
    ctrl.ep0_ring_phys = ring.phys;

    // Ring the doorbell for slot, endpoint index 1 (EP0 = dci 1)
    let db_off = (slot_id as u32) * 4;
    wr32(ctrl.db_base, db_off, 1); // target = EP0 = dci 1

    // Wait for Transfer Event (from Status Stage)
    let timeout_ms = 500u32;
    let start = crate::arch::x86::pit::get_ticks();
    loop {
        if let Some(status) = poll_event(ctrl, TRB_TRANSFER_EVENT) {
            let cc = ((status >> 24) & 0xFF) as u8;
            return match cc {
                CC_SUCCESS | CC_SHORT_PACKET => Ok(data_len as usize),
                _ => Err("xHCI: control transfer error"),
            };
        }
        if crate::arch::x86::pit::get_ticks().wrapping_sub(start) > timeout_ms {
            return Err("xHCI: control transfer timeout");
        }
        core::hint::spin_loop();
    }
}

fn read_data_buf(ctrl: &XhciController, buf: &mut [u8], len: usize) {
    let to_copy = len.min(buf.len());
    unsafe {
        core::ptr::copy_nonoverlapping(ctrl.data_buf_phys as *const u8, buf.as_mut_ptr(), to_copy);
    }
}

// ── Bulk transfer ─────────────────────────────────────────────────────────────

/// Execute a bulk transfer on endpoint `endpoint` of slot `slot_id`.
///
/// `endpoint`: standard USB endpoint address (bit 7 = IN direction).
/// `toggle`:   data toggle state; updated on return (xHCI manages toggle
///             internally, this parameter is accepted for API compatibility
///             with UHCI/EHCI).
/// `data_phys`: physical address of a DMA-accessible buffer.
pub fn bulk_transfer(
    dev_addr: u8,
    _speed: UsbSpeed,
    endpoint: u8,
    max_packet: u16,
    toggle: &mut u8,
    data_phys: u64,
    len: usize,
) -> Result<usize, &'static str> {
    let _ = (dev_addr, max_packet, toggle); // xHCI manages these internally

    let mut guard = XHCI_CTRL.lock();
    let ctrl = guard.as_mut().ok_or("xHCI not initialized")?;

    if len == 0 {
        return Ok(0);
    }

    let is_in = (endpoint & 0x80) != 0;
    let ep_num = endpoint & 0x0F;
    // xHCI endpoint index (DCI): OUT = ep*2+1, IN = ep*2+2  (for ep >= 1)
    let dci: u32 = if is_in {
        ep_num as u32 * 2 + 2
    } else {
        ep_num as u32 * 2 + 1
    };

    // We reuse the EP0 ring page as a single-shot bulk ring for simplicity.
    // In production code each endpoint would have its own persistent ring.
    let ring_phys = ctrl.ep0_ring_phys + 4096; // page just after EP0 ring
    unsafe {
        core::ptr::write_bytes(ring_phys as *mut u8, 0, 4096);
    }

    let max_pkt = (max_packet as usize).max(1);
    let num_trbs = ((len + max_pkt - 1) / max_pkt).min(RING_SIZE - 2);
    let mut total_enqueued = 0usize;
    let mut cycle: u32 = 1;
    let mut enqueue: usize = 0;

    for i in 0..num_trbs {
        let offset = i * max_pkt;
        let chunk = (len - offset).min(max_pkt);
        let last = i + 1 == num_trbs;
        let ctrl_field =
            (TRB_NORMAL << 10) | (cycle & TRB_C) | if last { TRB_IOC } else { TRB_CH } | TRB_ISP;
        unsafe {
            let ptr = (ring_phys + (enqueue * 16) as u64) as *mut Trb;
            core::ptr::write_volatile(
                ptr,
                Trb {
                    param: data_phys + offset as u64,
                    status: chunk as u32,
                    ctrl: ctrl_field,
                },
            );
        }
        total_enqueued += chunk;
        enqueue += 1;
        if enqueue >= RING_SIZE - 1 {
            // Insert Link TRB and wrap
            unsafe {
                let link_ptr = (ring_phys + ((RING_SIZE - 1) * 16) as u64) as *mut Trb;
                core::ptr::write_volatile(
                    link_ptr,
                    Trb {
                        param: ring_phys,
                        status: 0,
                        ctrl: (TRB_LINK << 10) | (1 << 1) | cycle,
                    },
                );
            }
            enqueue = 0;
            cycle ^= 1;
        }
    }

    // Point the endpoint's dequeue pointer to our new ring.
    // This requires an Evaluate Context or Configure Endpoint command; for
    // simplicity we update the Device Context directly (works when the
    // endpoint is already configured).
    let cs = ctrl.ctx_size;
    let ep_entry = 2 + (dci as usize - 1); // entry index in device context
    let dq_lo = (ring_phys as u32) | 1; // DCS = 1
    let dq_hi = (ring_phys >> 32) as u32;
    unsafe {
        ctx_write(ctrl.device_ctx_phys, ep_entry, 8, dq_lo, cs);
        ctx_write(ctrl.device_ctx_phys, ep_entry, 12, dq_hi, cs);
    }

    // Ring endpoint doorbell
    let db_off = 4u32; // slot 1 doorbell
    wr32(ctrl.db_base, db_off, dci);

    // Wait for Transfer Event
    let timeout_ms = 5000u32;
    let start = crate::arch::x86::pit::get_ticks();
    loop {
        if let Some(status) = poll_event(ctrl, TRB_TRANSFER_EVENT) {
            let cc = ((status >> 24) & 0xFF) as u8;
            let residual = (status & 0x00FF_FFFF) as usize;
            return match cc {
                CC_SUCCESS | CC_SHORT_PACKET => Ok(total_enqueued.saturating_sub(residual)),
                _ => Err("xHCI: bulk transfer error"),
            };
        }
        if crate::arch::x86::pit::get_ticks().wrapping_sub(start) > timeout_ms {
            return Err("xHCI: bulk transfer timeout");
        }
        core::hint::spin_loop();
    }
}

// ── Device enumeration ────────────────────────────────────────────────────────

fn enumerate_device(ctrl: &mut XhciController, port_idx: u8, speed: UsbSpeed) {
    // Reset EP0 ring
    unsafe {
        core::ptr::write_bytes(ctrl.ep0_ring_phys as *mut u8, 0, 4096);
    }

    // Enable slot and address device
    let new_addr = alloc_address();
    if let Err(e) = enable_and_address_device(ctrl, port_idx, speed, new_addr) {
        crate::serial_verbose_println!(
            "  xHCI: port {} — enable/address failed: {}",
            port_idx + 1,
            e
        );
        return;
    }

    let mps: u16 = match speed {
        UsbSpeed::Low => 8,
        UsbSpeed::Full => 8,
        UsbSpeed::High => 64,
        UsbSpeed::Super => 512,
    };

    // ── Step 1: GET_DESCRIPTOR (first 8 bytes) at address 0 ──────
    let setup = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_DEVICE,
        w_index: 0,
        w_length: 8,
    };
    match control_transfer(ctrl, 1, &setup, true, 8) {
        Ok(_) => {}
        Err(e) => {
            crate::serial_verbose_println!(
                "  xHCI: port {} — GET_DESCRIPTOR(8) failed: {}",
                port_idx + 1,
                e
            );
            return;
        }
    }

    let mut desc_buf = [0u8; 18];
    read_data_buf(ctrl, &mut desc_buf, 8);
    let real_mps = if desc_buf[7] > 0 {
        desc_buf[7] as u16
    } else {
        mps
    };

    // ── Step 2: SET_ADDRESS ───────────────────────────────────────
    let setup_addr = SetupPacket {
        bm_request_type: DIR_HOST_TO_DEVICE,
        b_request: REQ_SET_ADDRESS,
        w_value: new_addr as u16,
        w_index: 0,
        w_length: 0,
    };
    if let Err(e) = control_transfer(ctrl, 1, &setup_addr, false, 0) {
        crate::serial_verbose_println!("  xHCI: port {} — SET_ADDRESS failed: {}", port_idx + 1, e);
        return;
    }
    delay_ms(2);

    // ── Step 3: GET_DESCRIPTOR (full 18 bytes) ────────────────────
    let setup_full = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_DEVICE,
        w_index: 0,
        w_length: 18,
    };
    match control_transfer(ctrl, 1, &setup_full, true, 18) {
        Ok(_) => {}
        Err(e) => {
            crate::serial_verbose_println!(
                "  xHCI: device {} — GET_DESCRIPTOR(18) failed: {}",
                new_addr,
                e
            );
            return;
        }
    }
    read_data_buf(ctrl, &mut desc_buf, 18);
    let dev_desc: DeviceDescriptor =
        unsafe { core::ptr::read_unaligned(desc_buf.as_ptr() as *const _) };

    // ── Step 4: GET_DESCRIPTOR (config header) ────────────────────
    let setup_cfg = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_CONFIG,
        w_index: 0,
        w_length: 9,
    };
    let total_len = match control_transfer(ctrl, 1, &setup_cfg, true, 9) {
        Ok(_) => {
            let mut hdr = [0u8; 9];
            read_data_buf(ctrl, &mut hdr, 9);
            u16::from_le_bytes([hdr[2], hdr[3]])
        }
        Err(e) => {
            crate::serial_verbose_println!(
                "  xHCI: device {} — config hdr failed: {}",
                new_addr,
                e
            );
            return;
        }
    };

    // ── Step 5: GET_DESCRIPTOR (full config) ──────────────────────
    let config_len = total_len.min(256);
    let setup_cfg_full = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_CONFIG,
        w_index: 0,
        w_length: config_len,
    };
    let mut config_buf = [0u8; 256];
    match control_transfer(ctrl, 1, &setup_cfg_full, true, config_len) {
        Ok(_) => {
            read_data_buf(ctrl, &mut config_buf, config_len as usize);
        }
        Err(e) => {
            crate::serial_verbose_println!(
                "  xHCI: device {} — full config failed: {}",
                new_addr,
                e
            );
            return;
        }
    }
    let interfaces = parse_config(&config_buf[..config_len as usize]);

    // ── Step 6: SET_CONFIGURATION ─────────────────────────────────
    let config_val = config_buf[5];
    let setup_setcfg = SetupPacket {
        bm_request_type: DIR_HOST_TO_DEVICE,
        b_request: REQ_SET_CONFIGURATION,
        w_value: config_val as u16,
        w_index: 0,
        w_length: 0,
    };
    if let Err(e) = control_transfer(ctrl, 1, &setup_setcfg, false, 0) {
        crate::serial_verbose_println!(
            "  xHCI: device {} — SET_CONFIGURATION failed: {}",
            new_addr,
            e
        );
        return;
    }

    let dev_class = if dev_desc.b_device_class != 0 {
        dev_desc.b_device_class
    } else {
        interfaces.first().map(|i| i.class).unwrap_or(0)
    };
    let dev_subclass = if dev_desc.b_device_sub_class != 0 {
        dev_desc.b_device_sub_class
    } else {
        interfaces.first().map(|i| i.subclass).unwrap_or(0)
    };
    let dev_protocol = if dev_desc.b_device_protocol != 0 {
        dev_desc.b_device_protocol
    } else {
        interfaces.first().map(|i| i.protocol).unwrap_or(0)
    };

    let usb_dev = UsbDevice {
        address: new_addr,
        speed,
        port: port_idx + 1,
        controller: ControllerType::Xhci,
        max_packet_size: real_mps,
        vendor_id: dev_desc.id_vendor,
        product_id: dev_desc.id_product,
        class: dev_class,
        subclass: dev_subclass,
        protocol: dev_protocol,
        num_configs: dev_desc.b_num_configurations,
        interfaces,
        config_raw: config_buf[..config_len as usize].to_vec(),
    };

    register_device(usb_dev);
}

// ── Port reset + scan ─────────────────────────────────────────────────────────

fn reset_port(ctrl: &XhciController, port_idx: u8) -> Option<UsbSpeed> {
    let off = portsc_off(port_idx);
    let portsc = rd32(ctrl.op_base, off);

    if portsc & PORTSC_CCS == 0 {
        return None;
    }

    // Power on if needed
    if portsc & PORTSC_PP == 0 {
        wr32(ctrl.op_base, off, portsc | PORTSC_PP);
        delay_ms(20);
    }

    // Issue port reset (write PR=1, preserve PP and other stable bits)
    let portsc = rd32(ctrl.op_base, off);
    wr32(
        ctrl.op_base,
        off,
        (portsc & !(PORTSC_PED | PORTSC_CSC | PORTSC_PEC | PORTSC_WRC | PORTSC_PRC)) | PORTSC_PR,
    );
    delay_ms(50);

    // Wait for PRC (Port Reset Change)
    for _ in 0..200 {
        let ps = rd32(ctrl.op_base, off);
        if ps & PORTSC_PRC != 0 {
            // Clear PRC
            wr32(ctrl.op_base, off, ps | PORTSC_PRC);
            break;
        }
        delay_ms(1);
    }

    // Determine speed from PORTSC[13:10]
    let portsc = rd32(ctrl.op_base, off);
    if portsc & PORTSC_PED == 0 {
        crate::serial_verbose_println!(
            "  xHCI: port {} — reset failed (not enabled)",
            port_idx + 1
        );
        return None;
    }

    let psi = (portsc & PORTSC_SPEED_MASK) >> PORTSC_SPEED_SHIFT;
    let speed = match psi {
        PSI_LOW => UsbSpeed::Low,
        PSI_FULL => UsbSpeed::Full,
        PSI_HIGH => UsbSpeed::High,
        PSI_SUPER | _ => UsbSpeed::Super,
    };

    crate::serial_verbose_println!(
        "  xHCI: port {} — enabled ({})",
        port_idx + 1,
        speed_name(speed),
    );
    Some(speed)
}

fn scan_ports(ctrl: &mut XhciController) {
    for i in 0..ctrl.n_ports {
        let off = portsc_off(i);
        let portsc = rd32(ctrl.op_base, off);

        if portsc & PORTSC_CCS == 0 {
            crate::serial_verbose_println!("  xHCI: port {} — no device", i + 1);
            continue;
        }

        crate::serial_verbose_println!("  xHCI: port {} — device connected, resetting...", i + 1);

        // W1C: clear status change bits before reset
        wr32(
            ctrl.op_base,
            off,
            portsc | PORTSC_CSC | PORTSC_PEC | PORTSC_WRC | PORTSC_PRC,
        );
        delay_ms(5);

        if let Some(speed) = reset_port(ctrl, i) {
            delay_ms(10);
            enumerate_device(ctrl, i, speed);
        }
    }
}

fn speed_name(s: UsbSpeed) -> &'static str {
    match s {
        UsbSpeed::Low => "Low-Speed",
        UsbSpeed::Full => "Full-Speed",
        UsbSpeed::High => "High-Speed",
        UsbSpeed::Super => "SuperSpeed",
    }
}

// ── Controller initialisation ─────────────────────────────────────────────────

pub fn init_controller(pci: &PciDevice) {
    // BAR0 = MMIO base (64-bit BAR possible)
    let bar0 = pci.bars[0] as u64;
    let bar1 = pci.bars[1] as u64;
    let phys_base = if bar0 & 0x4 != 0 {
        // 64-bit BAR
        ((bar1 << 32) | (bar0 & !0xF))
    } else {
        bar0 & !0xF
    };

    if phys_base == 0 {
        crate::serial_verbose_println!("  xHCI: BAR0 is zero, cannot initialize");
        return;
    }

    crate::serial_verbose_println!(
        "  xHCI: controller at phys {:#012x}, IRQ {}",
        phys_base,
        pci.interrupt_line,
    );

    // Enable bus mastering + memory space
    let cmd = pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x06);

    // Map MMIO
    for i in 0..XHCI_MMIO_PAGES {
        let virt = XHCI_MMIO_BASE + (i as u64) * 4096;
        let phys = phys_base + (i as u64) * 4096;
        virtual_mem::map_page(VirtAddr(virt), PhysAddr(phys), 0x03);
    }
    let mmio_base = XHCI_MMIO_BASE;

    // ── Read capability registers ─────────────────────────────────
    let caplength = rd8(mmio_base, CAP_CAPLENGTH) as u32;
    let hciversion = rd32(mmio_base, CAP_HCIVERSION) >> 16;
    let hcsparams1 = rd32(mmio_base, CAP_HCSPARAMS1);
    let hcsparams2 = rd32(mmio_base, CAP_HCSPARAMS2);
    let hccparams1 = rd32(mmio_base, CAP_HCCPARAMS1);
    let dboff = rd32(mmio_base, CAP_DBOFF) & !0x3;
    let rtsoff = rd32(mmio_base, CAP_RTSOFF) & !0x1F;

    let max_slots = (hcsparams1 & 0xFF) as u8;
    let n_ports = ((hcsparams1 >> 24) & 0xFF) as u8;
    let ctx_size = if hccparams1 & (1 << 2) != 0 {
        CTX_SIZE_LARGE
    } else {
        CTX_SIZE_NORMAL
    };

    // Max scratchpad buffers
    let sp_hi = ((hcsparams2 >> 21) & 0x1F) as u32;
    let sp_lo = ((hcsparams2 >> 2) & 0x1F) as u32;
    let _n_scratchpad = (sp_hi << 5) | sp_lo;

    crate::serial_verbose_println!(
        "  xHCI: v{:#06x}, {} ports, {} max slots, ctx={}B, dboff={:#x}, rtsoff={:#x}",
        hciversion,
        n_ports,
        max_slots,
        ctx_size,
        dboff,
        rtsoff,
    );

    let op_base = mmio_base + caplength as u64;
    let rt_base = mmio_base + rtsoff as u64;
    let db_base = mmio_base + dboff as u64;

    // ── BIOS hand-off (XHCI_USBLEGSUP) ───────────────────────────
    // Walk the xHCI Extended Capability list looking for USBLEGSUP (ID=1)
    let xecp = ((hccparams1 >> 16) & 0xFFFF) as u32;
    if xecp != 0 {
        let mut cap_ptr = mmio_base + (xecp * 4) as u64;
        for _ in 0..32 {
            let cap = rd32(cap_ptr as u64, 0);
            let cap_id = cap & 0xFF;
            if cap_id == 0 {
                break;
            }
            if cap_id == 1 {
                // USBLEGSUP: bit 16 = BIOS owned, bit 24 = OS owned
                if cap & (1 << 16) != 0 {
                    crate::serial_verbose_println!("  xHCI: requesting BIOS hand-off");
                    wr32(cap_ptr as u64, 0, cap | (1 << 24));
                    for _ in 0..200 {
                        let v = rd32(cap_ptr as u64, 0);
                        if v & (1 << 16) == 0 {
                            break;
                        }
                        delay_ms(10);
                    }
                    crate::serial_verbose_println!("  xHCI: BIOS hand-off complete");
                }
                break;
            }
            let next = (cap >> 8) & 0xFF;
            if next == 0 {
                break;
            }
            cap_ptr += (next * 4) as u64;
        }
    }

    // ── Stop controller ───────────────────────────────────────────
    let usbcmd = rd32(op_base, OP_USBCMD);
    wr32(op_base, OP_USBCMD, usbcmd & !CMD_RUN);
    for _ in 0..100 {
        if rd32(op_base, OP_USBSTS) & STS_HCH != 0 {
            break;
        }
        delay_ms(1);
    }

    // ── HC reset ──────────────────────────────────────────────────
    wr32(op_base, OP_USBCMD, CMD_HCRST);
    for _ in 0..200 {
        if rd32(op_base, OP_USBCMD) & CMD_HCRST == 0 && rd32(op_base, OP_USBSTS) & STS_CNR == 0 {
            break;
        }
        delay_ms(1);
    }
    crate::serial_verbose_println!("  xHCI: reset complete");

    // ── Allocate DMA structures ───────────────────────────────────
    let alloc = |label: &str| -> u64 {
        match physical::alloc_contiguous(1) {
            Some(p) => {
                let addr = p.as_u64();
                unsafe {
                    core::ptr::write_bytes(addr as *mut u8, 0, 4096);
                }
                addr
            }
            None => {
                crate::serial_verbose_println!("  xHCI: alloc failed: {}", label);
                0
            }
        }
    };

    let dcbaa_phys = alloc("DCBAA");
    let cmd_ring_phys = alloc("command ring");
    let evt_ring_phys = alloc("event ring");
    let evt_erst_phys = alloc("ERST");
    let input_ctx_phys = alloc("input context");
    let device_ctx_phys = alloc("device context");
    let ep0_ring_phys = alloc("EP0 ring");
    let data_buf_phys = alloc("data buffer");

    if [
        dcbaa_phys,
        cmd_ring_phys,
        evt_ring_phys,
        evt_erst_phys,
        input_ctx_phys,
        device_ctx_phys,
        ep0_ring_phys,
        data_buf_phys,
    ]
    .iter()
    .any(|&p| p == 0)
    {
        crate::serial_verbose_println!("  xHCI: DMA allocation failed, aborting");
        return;
    }

    // ── Set max device slots ──────────────────────────────────────
    let slots_to_use = max_slots.min(32).max(1);
    wr32(op_base, OP_CONFIG, slots_to_use as u32);

    // ── DCBAA: entry 1 → Device Context ──────────────────────────
    unsafe {
        let dcbaa = dcbaa_phys as *mut u64;
        *dcbaa.add(1) = device_ctx_phys;
    }
    wr64(op_base, OP_DCBAAP, dcbaa_phys);

    // ── Command Ring ──────────────────────────────────────────────
    // Link TRB at end pointing back to start, TC=1, cycle=1
    unsafe {
        let link_off = ((RING_SIZE - 1) * 16) as u64;
        let ptr = (cmd_ring_phys + link_off) as *mut Trb;
        core::ptr::write_volatile(
            ptr,
            Trb {
                param: cmd_ring_phys,
                status: 0,
                ctrl: (TRB_LINK << 10) | TRB_C | (1 << 1), // TC=1, C=1
            },
        );
    }
    // CRCR: ring pointer | RCS=1 (Running Consumer Cycle State = 1)
    wr64(op_base, OP_CRCR, cmd_ring_phys | 1);

    // ── Event Ring Segment Table ──────────────────────────────────
    // One segment: { base_addr, ring_size, 0 }
    unsafe {
        let erst = evt_erst_phys as *mut u64;
        *erst.add(0) = evt_ring_phys;
        *erst.add(1) = RING_SIZE as u64;
    }
    wr32(rt_base, RT_ERSTSZ0, 1);
    wr64(rt_base, RT_ERSTBA0, evt_erst_phys);
    wr64(rt_base, RT_ERDP0, evt_ring_phys); // initial dequeue pointer

    // Disable interrupter (we poll)
    wr32(rt_base, RT_IMAN0, 0);
    wr32(rt_base, RT_IMOD0, 0);

    // Disable notification (DN_CTRL = 0)
    wr32(op_base, OP_DNCTRL, 0);

    // ── Start controller ──────────────────────────────────────────
    wr32(op_base, OP_USBCMD, CMD_RUN);
    for _ in 0..100 {
        if rd32(op_base, OP_USBSTS) & STS_HCH == 0 {
            break;
        }
        delay_ms(1);
    }

    if rd32(op_base, OP_USBSTS) & STS_HCH != 0 {
        crate::serial_verbose_println!("  xHCI: controller failed to start");
        return;
    }
    crate::serial_verbose_println!("  xHCI: controller running");

    // Allow ports to settle
    delay_ms(100);

    let mut ctrl = XhciController {
        mmio_base,
        op_base,
        rt_base,
        db_base,
        n_ports: n_ports.min(32),
        ctx_size,
        cmd_ring_phys,
        cmd_enqueue: 0,
        cmd_cycle: 1,
        evt_ring_phys,
        evt_erst_phys,
        evt_dequeue: 0,
        evt_cycle: 1,
        input_ctx_phys,
        device_ctx_phys,
        ep0_ring_phys,
        data_buf_phys,
        dcbaa_phys,
        port_connected: [false; 32],
    };

    scan_ports(&mut ctrl);

    // Record initial port states for hot-plug
    for i in 0..ctrl.n_ports as usize {
        let portsc = rd32(ctrl.op_base, portsc_off(i as u8));
        ctrl.port_connected[i] = portsc & PORTSC_CCS != 0;
    }

    *XHCI_CTRL.lock() = Some(ctrl);
}

// ── Hot-plug polling ──────────────────────────────────────────────────────────

pub fn poll_ports() {
    let mut guard = XHCI_CTRL.lock();
    let ctrl = match guard.as_mut() {
        Some(c) => c,
        None => return,
    };

    for i in 0..ctrl.n_ports as usize {
        let off = portsc_off(i as u8);
        let portsc = rd32(ctrl.op_base, off);
        let connected = portsc & PORTSC_CCS != 0;
        let was_connected = ctrl.port_connected[i];

        if connected && !was_connected {
            crate::serial_verbose_println!("  xHCI: hot-plug — device connected on port {}", i + 1);
            ctrl.port_connected[i] = true;
            // Clear status change bits
            wr32(
                ctrl.op_base,
                off,
                portsc | PORTSC_CSC | PORTSC_PEC | PORTSC_WRC | PORTSC_PRC,
            );
            if let Some(speed) = reset_port(ctrl, i as u8) {
                delay_ms(10);
                enumerate_device(ctrl, i as u8, speed);
            }
        } else if !connected && was_connected {
            crate::serial_verbose_println!(
                "  xHCI: hot-unplug — device removed from port {}",
                i + 1
            );
            ctrl.port_connected[i] = false;
            wr32(
                ctrl.op_base,
                off,
                portsc | PORTSC_CSC | PORTSC_PEC | PORTSC_WRC | PORTSC_PRC,
            );
            let port_num = (i + 1) as u8;
            super::hub::disconnect(port_num, ControllerType::Xhci);
            super::cdc_acm::disconnect(port_num, ControllerType::Xhci);
            super::cdc_ecm::disconnect(port_num, ControllerType::Xhci);
            super::storage::disconnect(port_num, ControllerType::Xhci);
            super::remove_device(port_num, ControllerType::Xhci);
        }
    }
}

// ── HID control transfer (for HID polling) ───────────────────────────────────

pub fn hid_control_transfer(
    _dev_addr: u8,
    _speed: UsbSpeed,
    _max_packet: u16,
    setup: &SetupPacket,
    data_in: bool,
    data_len: u16,
) -> Result<alloc::vec::Vec<u8>, &'static str> {
    let mut guard = XHCI_CTRL.lock();
    let ctrl = guard.as_mut().ok_or("xHCI not initialized")?;

    let bytes = control_transfer(ctrl, 1, setup, data_in, data_len)?;
    if data_in && bytes > 0 {
        let mut buf = alloc::vec![0u8; bytes];
        read_data_buf(ctrl, &mut buf, bytes);
        Ok(buf)
    } else {
        Ok(alloc::vec::Vec::new())
    }
}

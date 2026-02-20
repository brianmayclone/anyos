//! LSI Logic Fusion-MPT SCSI driver.
//!
//! Supports LSI Logic SAS/SCSI controllers (PCI `1000:0030` — 53c1030).
//! Uses the Fusion-MPT message passing protocol with I/O port based register
//! access, doorbell handshake for init, and request/reply FIFOs for SCSI I/O.
//!
//! Tested with VirtualBox LsiLogic SCSI controller.

use alloc::boxed::Box;
use crate::drivers::pci::{PciDevice, pci_config_read32, pci_config_write32};
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::{virtual_mem, physical};
use core::sync::atomic::{AtomicBool, Ordering};

// ── Register Offsets (from BAR0 I/O base) ─────────────

const REG_DOORBELL: u16        = 0x00;
const REG_WRITE_SEQUENCE: u16  = 0x04;
const REG_HOST_DIAG: u16       = 0x08;
const REG_HOST_INT_STATUS: u16 = 0x30;
const REG_HOST_INT_MASK: u16   = 0x34;
const REG_REQUEST_FIFO: u16    = 0x40;
const REG_REPLY_FIFO: u16      = 0x44;

// ── Doorbell function codes ───────────────────────────

const MPI_FUNCTION_IOC_INIT: u8         = 0x02;
const MPI_FUNCTION_IOC_FACTS: u8        = 0x03;
const MPI_FUNCTION_SCSI_IO_REQUEST: u8  = 0x00;

const DOORBELL_HANDSHAKE: u32     = 0x42 << 24;
const DOORBELL_RESET: u32         = 0x40 << 24;

// ── IOC States ────────────────────────────────────────

const IOC_STATE_MASK: u32        = 0xF000_0000;
const IOC_STATE_READY: u32       = 0x1000_0000;
const IOC_STATE_OPERATIONAL: u32 = 0x2000_0000;
const IOC_STATE_FAULT: u32       = 0x4000_0000;

// ── Interrupt Status bits ─────────────────────────────

const HIS_DOORBELL_INT: u32     = 0x0000_0001;
const HIS_REPLY_INT: u32        = 0x0000_0008;
const HIS_IOP_DOORBELL: u32     = 0x8000_0000;

// ── SGE flags ─────────────────────────────────────────

const SGE_FLAGS_READ: u32  = 0xD100_0000; // LAST|END_OF_BUF|SIMPLE|END_OF_LIST|IOC_TO_HOST
const SGE_FLAGS_WRITE: u32 = 0xD500_0000; // LAST|END_OF_BUF|SIMPLE|HOST_TO_IOC|END_OF_LIST

// ── SCSI Control field direction bits ─────────────────

const MPI_SCSIIO_CONTROL_READ: u32  = 0x0200_0000;
const MPI_SCSIIO_CONTROL_WRITE: u32 = 0x0100_0000;

// ── Bounce buffer ─────────────────────────────────────

const BOUNCE_SECTORS: u32 = 128;
const BOUNCE_SIZE: usize = BOUNCE_SECTORS as usize * 512;
const BOUNCE_PAGES: usize = BOUNCE_SIZE / 4096; // 16 pages = 64 KiB

// ── Message Frame Structures ──────────────────────────

/// SCSI I/O Request message frame (48 bytes) + 1 SGE (8 bytes) = 56 bytes.
/// Laid out in a single identity-mapped DMA page.
#[repr(C)]
#[derive(Clone, Copy)]
struct ScsiIoRequest {
    target_id: u8,
    bus: u8,
    chain_offset: u8,
    function: u8,
    cdb_length: u8,
    sense_buf_length: u8,
    reserved: u8,
    msg_flags: u8,
    msg_context: u32,
    lun: [u8; 8],
    control: u32,
    cdb: [u8; 16],
    data_length: u32,
    sense_buf_low_addr: u32,
    // SGE immediately follows
    sge_flags_length: u32,
    sge_data_addr: u32,
}

/// IOC Init Request (24 bytes = 6 dwords).
#[repr(C)]
#[derive(Clone, Copy)]
struct IocInitRequest {
    who_init: u8,
    reserved1: u8,
    chain_offset: u8,
    function: u8,
    flags: u8,
    max_devices: u8,
    max_buses: u8,
    msg_flags: u8,
    msg_context: u32,
    reply_frame_size: u16,
    reserved2: u16,
    host_mfa_high_addr: u32,
    sense_buffer_high_addr: u32,
}

/// IOC Init Reply (minimal, 24 bytes = 12 words of 16 bits).
#[repr(C)]
#[derive(Clone, Copy)]
struct IocInitReply {
    who_init: u8,
    reserved1: u8,
    msg_length: u8,
    function: u8,
    flags: u8,
    max_devices: u8,
    max_buses: u8,
    msg_flags: u8,
    msg_context: u32,
    reserved2: u16,
    ioc_status: u16,
    ioc_log_info: u32,
}

// ── Controller State ──────────────────────────────────

struct LsiController {
    io_base: u16,
    /// Physical address of the request frame page (identity-mapped)
    req_frame_phys: u64,
    /// Physical address of the reply buffer (identity-mapped)
    reply_buf_phys: u64,
    /// Physical address of the sense buffer (identity-mapped)
    sense_buf_phys: u64,
    /// Bounce buffer physical/virtual address (identity-mapped)
    bounce_phys: u64,
    /// Message context counter
    next_context: u32,
    /// Target ID for the disk (found during scan)
    disk_target: u8,
}

static AVAILABLE: AtomicBool = AtomicBool::new(false);
static mut CTRL: Option<LsiController> = None;

// ── I/O Port Helpers ──────────────────────────────────

#[inline]
unsafe fn inl(port: u16) -> u32 {
    crate::arch::x86::port::inl(port)
}

#[inline]
unsafe fn outl(port: u16, val: u32) {
    crate::arch::x86::port::outl(port, val);
}

// ── Doorbell Handshake ────────────────────────────────

/// Send a message via doorbell handshake and read reply.
/// `msg` is the raw dwords of the message.
/// Returns reply as a vector of u16 words (up to `reply_words` count).
unsafe fn doorbell_handshake(
    io_base: u16,
    msg_dwords: &[u32],
    reply_buf: &mut [u16],
) -> bool {
    // Wait for doorbell to be idle
    for _ in 0..100_000 {
        let db = inl(io_base + REG_DOORBELL);
        if db & 0x0800_0000 == 0 { break; } // Doorbell not active
        core::hint::spin_loop();
    }

    // Clear interrupt status
    outl(io_base + REG_HOST_INT_STATUS, 0);

    // Initiate handshake: function=0x42, msg size in dwords
    let handshake_cmd = DOORBELL_HANDSHAKE | ((msg_dwords.len() as u32) << 16);
    outl(io_base + REG_DOORBELL, handshake_cmd);

    // Wait for doorbell interrupt (IOC acknowledged handshake start)
    if !wait_doorbell_int(io_base) {
        crate::serial_println!("  LSI: handshake start timeout");
        return false;
    }
    outl(io_base + REG_HOST_INT_STATUS, 0);

    // Write each dword of the message to the doorbell
    for &dword in msg_dwords {
        outl(io_base + REG_DOORBELL, dword);
        if !wait_doorbell_int(io_base) {
            crate::serial_println!("  LSI: handshake write timeout");
            return false;
        }
        outl(io_base + REG_HOST_INT_STATUS, 0);
    }

    // Read reply 16 bits at a time from doorbell
    for word in reply_buf.iter_mut() {
        if !wait_doorbell_int(io_base) {
            crate::serial_println!("  LSI: handshake read timeout");
            return false;
        }
        *word = (inl(io_base + REG_DOORBELL) & 0xFFFF) as u16;
        outl(io_base + REG_HOST_INT_STATUS, 0);
    }

    // Final clear
    outl(io_base + REG_HOST_INT_STATUS, 0);
    true
}

unsafe fn wait_doorbell_int(io_base: u16) -> bool {
    for _ in 0..1_000_000 {
        let status = inl(io_base + REG_HOST_INT_STATUS);
        if status & HIS_DOORBELL_INT != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Wait for reply in the reply FIFO. Returns the raw 32-bit reply value.
unsafe fn wait_reply(io_base: u16) -> Option<u32> {
    for _ in 0..10_000_000 {
        let status = inl(io_base + REG_HOST_INT_STATUS);
        if status & HIS_REPLY_INT != 0 {
            let reply = inl(io_base + REG_REPLY_FIFO);
            outl(io_base + REG_HOST_INT_STATUS, 0);
            return Some(reply);
        }
        core::hint::spin_loop();
    }
    None
}

// ── Public API ──────────────────────────────────────

/// Read sectors via SCSI READ(10).
pub fn read_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    if !AVAILABLE.load(Ordering::Relaxed) {
        return false;
    }

    let ctrl = unsafe { CTRL.as_mut().unwrap() };
    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba;

    while remaining > 0 {
        let batch = remaining.min(BOUNCE_SECTORS);
        let byte_count = batch as usize * 512;

        if !unsafe { scsi_read_write(ctrl, cur_lba, batch as u16, true) } {
            return false;
        }

        // Copy from bounce buffer
        let end = (offset + byte_count).min(buf.len());
        let copy_len = end - offset;
        unsafe {
            core::ptr::copy_nonoverlapping(
                ctrl.bounce_phys as *const u8,
                buf[offset..].as_mut_ptr(),
                copy_len,
            );
        }

        offset += byte_count;
        cur_lba += batch;
        remaining -= batch;
    }

    true
}

/// Write sectors via SCSI WRITE(10).
pub fn write_sectors(lba: u32, count: u32, buf: &[u8]) -> bool {
    if !AVAILABLE.load(Ordering::Relaxed) {
        return false;
    }

    let ctrl = unsafe { CTRL.as_mut().unwrap() };
    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba;

    while remaining > 0 {
        let batch = remaining.min(BOUNCE_SECTORS);
        let byte_count = batch as usize * 512;

        // Copy to bounce buffer
        let end = (offset + byte_count).min(buf.len());
        let copy_len = end - offset;
        unsafe {
            core::ptr::copy_nonoverlapping(
                buf[offset..].as_ptr(),
                ctrl.bounce_phys as *mut u8,
                copy_len,
            );
        }

        if !unsafe { scsi_read_write(ctrl, cur_lba, batch as u16, false) } {
            return false;
        }

        offset += byte_count;
        cur_lba += batch;
        remaining -= batch;
    }

    true
}

/// Issue a SCSI READ(10) or WRITE(10) command using the bounce buffer.
unsafe fn scsi_read_write(ctrl: &mut LsiController, lba: u32, count: u16, is_read: bool) -> bool {
    let byte_count = count as u32 * 512;

    // Build SCSI I/O Request at the request frame page
    let req = ctrl.req_frame_phys as *mut ScsiIoRequest;
    core::ptr::write_bytes(req as *mut u8, 0, core::mem::size_of::<ScsiIoRequest>());

    let ctx = ctrl.next_context;
    ctrl.next_context = ctrl.next_context.wrapping_add(1);

    let mut io_req = ScsiIoRequest {
        target_id: ctrl.disk_target,
        bus: 0,
        chain_offset: 0,
        function: MPI_FUNCTION_SCSI_IO_REQUEST,
        cdb_length: 10,
        sense_buf_length: 32,
        reserved: 0,
        msg_flags: 0,
        msg_context: ctx,
        lun: [0u8; 8],
        control: if is_read { MPI_SCSIIO_CONTROL_READ } else { MPI_SCSIIO_CONTROL_WRITE },
        cdb: [0u8; 16],
        data_length: byte_count,
        sense_buf_low_addr: ctrl.sense_buf_phys as u32,
        sge_flags_length: (if is_read { SGE_FLAGS_READ } else { SGE_FLAGS_WRITE }) | byte_count,
        sge_data_addr: ctrl.bounce_phys as u32,
    };

    // Build CDB: READ(10) = 0x28, WRITE(10) = 0x2A
    io_req.cdb[0] = if is_read { 0x28 } else { 0x2A };
    io_req.cdb[2] = (lba >> 24) as u8;
    io_req.cdb[3] = (lba >> 16) as u8;
    io_req.cdb[4] = (lba >> 8) as u8;
    io_req.cdb[5] = lba as u8;
    io_req.cdb[7] = (count >> 8) as u8;
    io_req.cdb[8] = count as u8;

    core::ptr::write_volatile(req, io_req);

    // Submit request via Request FIFO
    outl(ctrl.io_base + REG_REQUEST_FIFO, ctrl.req_frame_phys as u32);

    // Wait for reply
    match wait_reply(ctrl.io_base) {
        Some(reply) => {
            if reply & 0x8000_0000 != 0 {
                // Address reply — error occurred
                let reply_addr = reply & 0x7FFF_FFFF;
                crate::serial_println!("  LSI: SCSI I/O error (reply addr={:#010x})", reply_addr);
                // Re-post reply buffer
                outl(ctrl.io_base + REG_REPLY_FIFO, ctrl.reply_buf_phys as u32);
                return false;
            }
            // Context reply — success
            true
        }
        None => {
            crate::serial_println!("  LSI: SCSI I/O timeout");
            false
        }
    }
}

// ── Initialization ──────────────────────────────────

/// Initialize LSI Logic Fusion-MPT SCSI controller. Called by HAL.
pub fn init_and_register(pci: &PciDevice) {
    // BAR0 = I/O port base
    let bar0 = pci.bars[0];
    if bar0 & 1 == 0 {
        crate::serial_println!("  LSI SCSI: BAR0 is not I/O port");
        return;
    }
    let io_base = (bar0 & 0xFFFC) as u16;
    crate::serial_println!("  LSI SCSI: I/O port base = {:#06x}", io_base);

    // Enable PCI bus mastering + I/O
    let cmd = pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x05);

    // Step 1: Reset IOC via doorbell
    unsafe { outl(io_base + REG_DOORBELL, DOORBELL_RESET); }

    // Wait for reset to complete and IOC to become READY
    let mut ready = false;
    for _ in 0..1_000_000 {
        let db = unsafe { inl(io_base + REG_DOORBELL) };
        let state = db & IOC_STATE_MASK;
        if state == IOC_STATE_READY {
            ready = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !ready {
        let db = unsafe { inl(io_base + REG_DOORBELL) };
        crate::serial_println!("  LSI SCSI: IOC not ready after reset (doorbell={:#010x})", db);
        return;
    }
    crate::serial_println!("  LSI SCSI: IOC reset OK, state=READY");

    // Step 2: Mask interrupts (we poll), then clear status
    unsafe {
        outl(io_base + REG_HOST_INT_MASK, 0x0000_0009); // Enable doorbell + reply
        outl(io_base + REG_HOST_INT_STATUS, 0);
    }

    // Step 3: Allocate DMA pages (identity-mapped)
    let req_phys = match physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => { crate::serial_println!("  LSI SCSI: alloc req frame failed"); return; }
    };
    let reply_phys = match physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => { crate::serial_println!("  LSI SCSI: alloc reply buf failed"); return; }
    };
    let sense_phys = match physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => { crate::serial_println!("  LSI SCSI: alloc sense buf failed"); return; }
    };

    // Identity-map all DMA pages
    for &phys in &[req_phys, reply_phys, sense_phys] {
        virtual_mem::map_page(VirtAddr::new(phys), PhysAddr::new(phys), 0x03);
        unsafe { core::ptr::write_bytes(phys as *mut u8, 0, 4096); }
    }

    // Allocate bounce buffer
    let bounce_phys = match physical::alloc_contiguous(BOUNCE_PAGES) {
        Some(p) => p.as_u64(),
        None => { crate::serial_println!("  LSI SCSI: alloc bounce buffer failed"); return; }
    };
    for i in 0..BOUNCE_PAGES {
        let p = bounce_phys + (i as u64) * 4096;
        virtual_mem::map_page(VirtAddr::new(p), PhysAddr::new(p), 0x03);
    }

    // Step 4: Send IOC Init via doorbell handshake
    let ioc_init = IocInitRequest {
        who_init: 0x02,  // HOST_DRIVER
        reserved1: 0,
        chain_offset: 0,
        function: MPI_FUNCTION_IOC_INIT,
        flags: 0,
        max_devices: 8,
        max_buses: 1,
        msg_flags: 0,
        msg_context: 0x1234_0001,
        reply_frame_size: 128, // reply frame size in bytes
        reserved2: 0,
        host_mfa_high_addr: 0,
        sense_buffer_high_addr: 0,
    };

    // Convert struct to dwords for handshake
    let msg_bytes = unsafe {
        core::slice::from_raw_parts(
            &ioc_init as *const IocInitRequest as *const u32,
            core::mem::size_of::<IocInitRequest>() / 4,
        )
    };

    let mut reply_words = [0u16; 12]; // 24 bytes = 12 words
    let ok = unsafe { doorbell_handshake(io_base, msg_bytes, &mut reply_words) };
    if !ok {
        crate::serial_println!("  LSI SCSI: IOC Init handshake failed");
        return;
    }

    // Check IOC status from reply (word 7 = ioc_status)
    let ioc_status = reply_words[7];
    if ioc_status != 0 {
        crate::serial_println!("  LSI SCSI: IOC Init failed (status={:#06x})", ioc_status);
        return;
    }
    crate::serial_println!("  LSI SCSI: IOC Init OK");

    // Step 5: Post reply buffer to Reply FIFO
    unsafe {
        outl(io_base + REG_REPLY_FIFO, reply_phys as u32);
    }

    // Step 6: Scan for SCSI disk (TEST UNIT READY on targets 0..7)
    let mut ctrl = LsiController {
        io_base,
        req_frame_phys: req_phys,
        reply_buf_phys: reply_phys,
        sense_buf_phys: sense_phys,
        bounce_phys,
        next_context: 0x1000,
        disk_target: 0,
    };

    let mut found_target: Option<u8> = None;
    for target_id in 0..8u8 {
        if unsafe { scsi_test_unit_ready(&mut ctrl, target_id) } {
            crate::serial_println!("  LSI SCSI: found disk at target {}", target_id);
            found_target = Some(target_id);
            break;
        }
    }

    match found_target {
        Some(tid) => {
            ctrl.disk_target = tid;
            unsafe { CTRL = Some(ctrl); }
            AVAILABLE.store(true, Ordering::Release);
            super::set_backend_lsi();
            crate::serial_println!("[OK] LSI Logic SCSI storage backend active (target={})", tid);
        }
        None => {
            crate::serial_println!("  LSI SCSI: no disk found on any target");
        }
    }
}

/// Send TEST UNIT READY to check if a target exists.
unsafe fn scsi_test_unit_ready(ctrl: &mut LsiController, target_id: u8) -> bool {
    let req = ctrl.req_frame_phys as *mut ScsiIoRequest;
    core::ptr::write_bytes(req as *mut u8, 0, core::mem::size_of::<ScsiIoRequest>());

    let ctx = ctrl.next_context;
    ctrl.next_context = ctrl.next_context.wrapping_add(1);

    let io_req = ScsiIoRequest {
        target_id,
        bus: 0,
        chain_offset: 0,
        function: MPI_FUNCTION_SCSI_IO_REQUEST,
        cdb_length: 6,
        sense_buf_length: 32,
        reserved: 0,
        msg_flags: 0,
        msg_context: ctx,
        lun: [0u8; 8],
        control: 0, // No data transfer
        cdb: [0u8; 16], // TEST UNIT READY = all zeros
        data_length: 0,
        sense_buf_low_addr: ctrl.sense_buf_phys as u32,
        sge_flags_length: 0,
        sge_data_addr: 0,
    };

    core::ptr::write_volatile(req, io_req);

    // Submit
    outl(ctrl.io_base + REG_REQUEST_FIFO, ctrl.req_frame_phys as u32);

    // Wait for reply (short timeout for scan)
    for _ in 0..500_000 {
        let status = inl(ctrl.io_base + REG_HOST_INT_STATUS);
        if status & HIS_REPLY_INT != 0 {
            let reply = inl(ctrl.io_base + REG_REPLY_FIFO);
            outl(ctrl.io_base + REG_HOST_INT_STATUS, 0);

            if reply & 0x8000_0000 != 0 {
                // Address reply — re-post buffer and check for errors
                outl(ctrl.io_base + REG_REPLY_FIFO, ctrl.reply_buf_phys as u32);
                return false;
            }
            // Context reply — success
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Probe: initialize LSI SCSI and return a HAL driver.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    init_and_register(pci);
    super::create_hal_driver("LSI Logic SCSI")
}

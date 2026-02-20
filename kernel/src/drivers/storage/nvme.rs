//! NVMe (Non-Volatile Memory Express) storage driver.
//!
//! Supports NVMe 1.0+ controllers over PCIe. Uses submission/completion queue
//! pairs for command submission and polled completion.
//!
//! Tested with VirtualBox NVMe (`80EE:4E56`) and QEMU NVMe.

use alloc::boxed::Box;
use crate::drivers::pci::{PciDevice, pci_config_read32, pci_config_write32};
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::{virtual_mem, physical};
use core::sync::atomic::{AtomicBool, Ordering};

// ── MMIO virtual base ───────────────────────────────

const NVME_MMIO_VIRT: u64 = 0xFFFF_FFFF_D014_0000;
const NVME_MMIO_PAGES: usize = 4; // 16 KiB

// ── NVMe Controller Registers ───────────────────────

const REG_CAP: u64     = 0x00;  // Controller Capabilities (64-bit)
const REG_VS: u64      = 0x08;  // Version
const REG_INTMS: u64   = 0x0C;  // Interrupt Mask Set
const REG_CC: u64      = 0x14;  // Controller Configuration
const REG_CSTS: u64    = 0x1C;  // Controller Status
const REG_AQA: u64     = 0x24;  // Admin Queue Attributes
const REG_ASQ: u64     = 0x28;  // Admin Submission Queue Base (64-bit)
const REG_ACQ: u64     = 0x30;  // Admin Completion Queue Base (64-bit)

// CC register fields
const CC_EN: u32       = 1 << 0;  // Enable
const CC_CSS_NVM: u32  = 0 << 4;  // NVM Command Set
const CC_MPS_4K: u32   = 0 << 7;  // Memory Page Size = 4 KiB (2^(12+0))
const CC_AMS_RR: u32   = 0 << 11; // Round Robin arbitration
const CC_IOSQES: u32   = 6 << 16; // I/O SQ entry size = 2^6 = 64 bytes
const CC_IOCQES: u32   = 4 << 20; // I/O CQ entry size = 2^4 = 16 bytes

// CSTS register fields
const CSTS_RDY: u32    = 1 << 0;  // Ready

// ── NVMe Command Opcodes ────────────────────────────

// Admin commands
const ADMIN_IDENTIFY: u8           = 0x06;
const ADMIN_CREATE_IO_CQ: u8      = 0x05;
const ADMIN_CREATE_IO_SQ: u8      = 0x01;

// NVM I/O commands
const NVM_CMD_READ: u8  = 0x02;
const NVM_CMD_WRITE: u8 = 0x01;

// ── Data Structures ─────────────────────────────────

/// NVMe Submission Queue Entry (64 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct NvmeCommand {
    opcode: u8,
    flags: u8,
    command_id: u16,
    nsid: u32,
    reserved: [u32; 2],
    metadata: u64,
    prp1: u64,
    prp2: u64,
    cdw10: u32,
    cdw11: u32,
    cdw12: u32,
    cdw13: u32,
    cdw14: u32,
    cdw15: u32,
}

impl NvmeCommand {
    const fn zeroed() -> Self {
        Self {
            opcode: 0, flags: 0, command_id: 0, nsid: 0,
            reserved: [0; 2], metadata: 0, prp1: 0, prp2: 0,
            cdw10: 0, cdw11: 0, cdw12: 0, cdw13: 0, cdw14: 0, cdw15: 0,
        }
    }
}

/// NVMe Completion Queue Entry (16 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct NvmeCompletion {
    result: u32,       // Command-specific result
    reserved: u32,
    sq_head: u16,      // SQ head pointer
    sq_id: u16,        // SQ identifier
    command_id: u16,   // Command ID
    status: u16,       // Status field (bit 0 = phase tag, bits 1-15 = status)
}

// ── Controller State ────────────────────────────────

struct NvmeController {
    mmio_base: u64,
    /// Doorbell stride (in bytes). Read from CAP.DSTRD.
    doorbell_stride: u32,
    /// Admin SQ/CQ physical addresses
    asq_phys: u64,
    acq_phys: u64,
    /// I/O SQ/CQ physical addresses
    iosq_phys: u64,
    iocq_phys: u64,
    /// Bounce buffer for DMA
    bounce_phys: u64,
    bounce_virt: u64,
    /// Queue depths
    admin_sq_tail: u16,
    admin_cq_head: u16,
    admin_phase: bool,
    io_sq_tail: u16,
    io_cq_head: u16,
    io_phase: bool,
    /// Command ID counter
    next_cmd_id: u16,
    /// Namespace 1 sector count
    ns1_sectors: u64,
    /// Sector size (usually 512)
    sector_size: u32,
}

static AVAILABLE: AtomicBool = AtomicBool::new(false);
static mut CTRL: Option<NvmeController> = None;

// Queue size (entries)
const ADMIN_QUEUE_SIZE: u16 = 16;
const IO_QUEUE_SIZE: u16 = 64;

// Bounce buffer: 128 KiB (256 sectors of 512 bytes)
const BOUNCE_SECTORS: u32 = 256;
const BOUNCE_SIZE: usize = BOUNCE_SECTORS as usize * 512;
const BOUNCE_PAGES: usize = BOUNCE_SIZE / 4096; // 32 pages

// ── MMIO Helpers ────────────────────────────────────

#[inline]
unsafe fn mmio_read32(base: u64, offset: u64) -> u32 {
    core::ptr::read_volatile((base + offset) as *const u32)
}

#[inline]
unsafe fn mmio_write32(base: u64, offset: u64, val: u32) {
    core::ptr::write_volatile((base + offset) as *mut u32, val);
}

#[inline]
unsafe fn mmio_read64(base: u64, offset: u64) -> u64 {
    // NVMe spec says 64-bit registers can be read as two 32-bit reads
    let lo = core::ptr::read_volatile((base + offset) as *const u32) as u64;
    let hi = core::ptr::read_volatile((base + offset + 4) as *const u32) as u64;
    lo | (hi << 32)
}

#[inline]
unsafe fn mmio_write64(base: u64, offset: u64, val: u64) {
    core::ptr::write_volatile((base + offset) as *mut u32, val as u32);
    core::ptr::write_volatile((base + offset + 4) as *mut u32, (val >> 32) as u32);
}

// ── Doorbell Writes ─────────────────────────────────

/// Doorbell offset = 0x1000 + (2*qid + tail_or_head) * doorbell_stride
#[inline]
unsafe fn write_sq_doorbell(ctrl: &NvmeController, qid: u16, tail: u16) {
    let offset = 0x1000u64 + (2 * qid as u64) * ctrl.doorbell_stride as u64;
    mmio_write32(ctrl.mmio_base, offset, tail as u32);
}

#[inline]
unsafe fn write_cq_doorbell(ctrl: &NvmeController, qid: u16, head: u16) {
    let offset = 0x1000u64 + (2 * qid as u64 + 1) * ctrl.doorbell_stride as u64;
    mmio_write32(ctrl.mmio_base, offset, head as u32);
}

// ── Admin Command Submission ────────────────────────

unsafe fn admin_submit(ctrl: &mut NvmeController, cmd: &NvmeCommand) -> Option<NvmeCompletion> {
    // Write command to admin SQ
    let sq_entry = (ctrl.asq_phys + ctrl.admin_sq_tail as u64 * 64) as *mut NvmeCommand;
    // Use identity-mapped virtual = physical for queue access
    core::ptr::write_volatile(sq_entry, *cmd);

    // Advance tail
    ctrl.admin_sq_tail = (ctrl.admin_sq_tail + 1) % ADMIN_QUEUE_SIZE;
    write_sq_doorbell(ctrl, 0, ctrl.admin_sq_tail);

    // Poll CQ for completion
    let cq_entry_ptr = (ctrl.acq_phys + ctrl.admin_cq_head as u64 * 16) as *const NvmeCompletion;
    for _ in 0..1_000_000 {
        let cqe = core::ptr::read_volatile(cq_entry_ptr);
        let phase = (cqe.status & 1) != 0;
        if phase == ctrl.admin_phase {
            // Advance CQ head
            ctrl.admin_cq_head = (ctrl.admin_cq_head + 1) % ADMIN_QUEUE_SIZE;
            if ctrl.admin_cq_head == 0 {
                ctrl.admin_phase = !ctrl.admin_phase;
            }
            write_cq_doorbell(ctrl, 0, ctrl.admin_cq_head);
            return Some(cqe);
        }
        core::hint::spin_loop();
    }
    crate::serial_println!("NVMe: admin command timeout");
    None
}

/// Submit an I/O command and wait for completion.
unsafe fn io_submit(ctrl: &mut NvmeController, cmd: &NvmeCommand) -> Option<NvmeCompletion> {
    let sq_entry = (ctrl.iosq_phys + ctrl.io_sq_tail as u64 * 64) as *mut NvmeCommand;
    core::ptr::write_volatile(sq_entry, *cmd);

    ctrl.io_sq_tail = (ctrl.io_sq_tail + 1) % IO_QUEUE_SIZE;
    write_sq_doorbell(ctrl, 1, ctrl.io_sq_tail);

    let cq_entry_ptr = (ctrl.iocq_phys + ctrl.io_cq_head as u64 * 16) as *const NvmeCompletion;
    for _ in 0..10_000_000 {
        let cqe = core::ptr::read_volatile(cq_entry_ptr);
        let phase = (cqe.status & 1) != 0;
        if phase == ctrl.io_phase {
            ctrl.io_cq_head = (ctrl.io_cq_head + 1) % IO_QUEUE_SIZE;
            if ctrl.io_cq_head == 0 {
                ctrl.io_phase = !ctrl.io_phase;
            }
            write_cq_doorbell(ctrl, 1, ctrl.io_cq_head);

            // Check status (bits 1-15, shift right 1)
            let sc = (cqe.status >> 1) & 0x7FFF;
            if sc != 0 {
                crate::serial_println!("NVMe: I/O error, status={:#06x}", sc);
                return None;
            }
            return Some(cqe);
        }
        core::hint::spin_loop();
    }
    crate::serial_println!("NVMe: I/O command timeout");
    None
}

// ── Public API ──────────────────────────────────────

/// Read sectors from NVMe namespace 1.
pub fn read_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    if !AVAILABLE.load(Ordering::Relaxed) {
        return false;
    }

    let ctrl = unsafe { CTRL.as_mut().unwrap() };
    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba as u64;

    while remaining > 0 {
        let batch = remaining.min(BOUNCE_SECTORS);
        let byte_count = batch as usize * ctrl.sector_size as usize;

        let mut cmd = NvmeCommand::zeroed();
        cmd.opcode = NVM_CMD_READ;
        cmd.command_id = ctrl.next_cmd_id;
        ctrl.next_cmd_id = ctrl.next_cmd_id.wrapping_add(1);
        cmd.nsid = 1;
        cmd.prp1 = ctrl.bounce_phys;
        // PRP2: for transfers > 4 KiB, point to next page
        if byte_count > 4096 {
            cmd.prp2 = ctrl.bounce_phys + 4096;
        }
        cmd.cdw10 = cur_lba as u32;         // Starting LBA (low 32)
        cmd.cdw11 = (cur_lba >> 32) as u32; // Starting LBA (high 32)
        cmd.cdw12 = batch - 1;              // Number of logical blocks (0-based)

        let ok = unsafe { io_submit(ctrl, &cmd).is_some() };
        if !ok {
            return false;
        }

        // Copy from bounce buffer to caller's buffer
        let src = ctrl.bounce_virt as *const u8;
        let end = (offset + byte_count).min(buf.len());
        let copy_len = end - offset;
        unsafe {
            core::ptr::copy_nonoverlapping(src, buf[offset..].as_mut_ptr(), copy_len);
        }

        offset += byte_count;
        cur_lba += batch as u64;
        remaining -= batch;
    }

    true
}

/// Write sectors to NVMe namespace 1.
pub fn write_sectors(lba: u32, count: u32, buf: &[u8]) -> bool {
    if !AVAILABLE.load(Ordering::Relaxed) {
        return false;
    }

    let ctrl = unsafe { CTRL.as_mut().unwrap() };
    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba as u64;

    while remaining > 0 {
        let batch = remaining.min(BOUNCE_SECTORS);
        let byte_count = batch as usize * ctrl.sector_size as usize;

        // Copy data to bounce buffer
        let dst = ctrl.bounce_virt as *mut u8;
        let end = (offset + byte_count).min(buf.len());
        let copy_len = end - offset;
        unsafe {
            core::ptr::copy_nonoverlapping(buf[offset..].as_ptr(), dst, copy_len);
        }

        let mut cmd = NvmeCommand::zeroed();
        cmd.opcode = NVM_CMD_WRITE;
        cmd.command_id = ctrl.next_cmd_id;
        ctrl.next_cmd_id = ctrl.next_cmd_id.wrapping_add(1);
        cmd.nsid = 1;
        cmd.prp1 = ctrl.bounce_phys;
        if byte_count > 4096 {
            cmd.prp2 = ctrl.bounce_phys + 4096;
        }
        cmd.cdw10 = cur_lba as u32;
        cmd.cdw11 = (cur_lba >> 32) as u32;
        cmd.cdw12 = batch - 1;

        let ok = unsafe { io_submit(ctrl, &cmd).is_some() };
        if !ok {
            return false;
        }

        offset += byte_count;
        cur_lba += batch as u64;
        remaining -= batch;
    }

    true
}

// ── Init ────────────────────────────────────────────

/// Initialize NVMe controller from PCI probe. Called by HAL.
pub fn init_and_register(pci: &PciDevice) {
    // BAR0 = MMIO registers
    let bar0 = pci.bars[0];
    if bar0 & 1 != 0 {
        crate::serial_println!("  NVMe: BAR0 is I/O port (expected MMIO)");
        return;
    }
    let mmio_phys = (bar0 & 0xFFFFF000) as u64;

    // Enable PCI bus mastering + memory
    let cmd = pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x06);

    // Map BAR0 MMIO
    for i in 0..NVME_MMIO_PAGES {
        let virt = VirtAddr::new(NVME_MMIO_VIRT + (i as u64) * 4096);
        let phys = PhysAddr::new(mmio_phys + (i as u64) * 4096);
        virtual_mem::map_page(virt, phys, 0x03);
    }

    let base = NVME_MMIO_VIRT;

    // Read capabilities
    let cap = unsafe { mmio_read64(base, REG_CAP) };
    let vs = unsafe { mmio_read32(base, REG_VS) };
    let mqes = (cap & 0xFFFF) as u16 + 1; // Maximum Queue Entries Supported
    let dstrd = 4 << ((cap >> 32) & 0xF);  // Doorbell Stride (bytes)
    let timeout = ((cap >> 24) & 0xFF) as u32 * 500; // Timeout in ms

    crate::serial_println!(
        "  NVMe: version {}.{}.{}, MQES={}, DSTRD={}, timeout={}ms",
        (vs >> 16) & 0xFF, (vs >> 8) & 0xFF, vs & 0xFF,
        mqes, dstrd, timeout
    );

    // Disable controller if enabled
    let cc = unsafe { mmio_read32(base, REG_CC) };
    if cc & CC_EN != 0 {
        unsafe { mmio_write32(base, REG_CC, cc & !CC_EN); }
        // Wait for not ready
        for _ in 0..1_000_000 {
            if unsafe { mmio_read32(base, REG_CSTS) } & CSTS_RDY == 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }

    // Allocate queues (identity-mapped, low memory)
    // Admin SQ: ADMIN_QUEUE_SIZE * 64 bytes (1 page)
    let asq_phys = match physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => { crate::serial_println!("  NVMe: alloc ASQ failed"); return; }
    };
    // Admin CQ: ADMIN_QUEUE_SIZE * 16 bytes (1 page)
    let acq_phys = match physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => { crate::serial_println!("  NVMe: alloc ACQ failed"); return; }
    };

    // Identity-map queue pages
    for &phys in &[asq_phys, acq_phys] {
        virtual_mem::map_page(VirtAddr::new(phys), PhysAddr::new(phys), 0x03);
    }

    // Zero queues
    unsafe {
        core::ptr::write_bytes(asq_phys as *mut u8, 0, 4096);
        core::ptr::write_bytes(acq_phys as *mut u8, 0, 4096);
    }

    // Mask all interrupts (we use polled mode)
    unsafe { mmio_write32(base, REG_INTMS, 0xFFFF_FFFF); }

    // Configure admin queues
    let aqa = ((ADMIN_QUEUE_SIZE - 1) as u32) << 16 | (ADMIN_QUEUE_SIZE - 1) as u32;
    unsafe {
        mmio_write32(base, REG_AQA, aqa);
        mmio_write64(base, REG_ASQ, asq_phys);
        mmio_write64(base, REG_ACQ, acq_phys);
    }

    // Enable controller
    let cc_val = CC_EN | CC_CSS_NVM | CC_MPS_4K | CC_AMS_RR | CC_IOSQES | CC_IOCQES;
    unsafe { mmio_write32(base, REG_CC, cc_val); }

    // Wait for ready
    for _ in 0..10_000_000 {
        if unsafe { mmio_read32(base, REG_CSTS) } & CSTS_RDY != 0 {
            break;
        }
        core::hint::spin_loop();
    }
    if unsafe { mmio_read32(base, REG_CSTS) } & CSTS_RDY == 0 {
        crate::serial_println!("  NVMe: controller failed to become ready");
        return;
    }
    crate::serial_println!("  NVMe: controller enabled and ready");

    // Allocate I/O queues
    let iosq_phys = match physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => { crate::serial_println!("  NVMe: alloc IOSQ failed"); return; }
    };
    let iocq_phys = match physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => { crate::serial_println!("  NVMe: alloc IOCQ failed"); return; }
    };
    for &phys in &[iosq_phys, iocq_phys] {
        virtual_mem::map_page(VirtAddr::new(phys), PhysAddr::new(phys), 0x03);
    }
    unsafe {
        core::ptr::write_bytes(iosq_phys as *mut u8, 0, 4096);
        core::ptr::write_bytes(iocq_phys as *mut u8, 0, 4096);
    }

    // Allocate bounce buffer (identity-mapped)
    let bounce_phys = match physical::alloc_contiguous(BOUNCE_PAGES) {
        Some(p) => p.as_u64(),
        None => { crate::serial_println!("  NVMe: alloc bounce buffer failed"); return; }
    };
    for i in 0..BOUNCE_PAGES {
        let p = bounce_phys + (i as u64) * 4096;
        virtual_mem::map_page(VirtAddr::new(p), PhysAddr::new(p), 0x03);
    }

    let mut ctrl = NvmeController {
        mmio_base: base,
        doorbell_stride: dstrd as u32,
        asq_phys,
        acq_phys,
        iosq_phys,
        iocq_phys,
        bounce_phys,
        bounce_virt: bounce_phys, // identity-mapped
        admin_sq_tail: 0,
        admin_cq_head: 0,
        admin_phase: true,
        io_sq_tail: 0,
        io_cq_head: 0,
        io_phase: true,
        next_cmd_id: 1,
        ns1_sectors: 0,
        sector_size: 512,
    };

    // Identify Controller (admin command)
    let identify_phys = match physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => { crate::serial_println!("  NVMe: alloc identify failed"); return; }
    };
    virtual_mem::map_page(VirtAddr::new(identify_phys), PhysAddr::new(identify_phys), 0x03);
    unsafe { core::ptr::write_bytes(identify_phys as *mut u8, 0, 4096); }

    let mut cmd = NvmeCommand::zeroed();
    cmd.opcode = ADMIN_IDENTIFY;
    cmd.command_id = ctrl.next_cmd_id;
    ctrl.next_cmd_id += 1;
    cmd.prp1 = identify_phys;
    cmd.cdw10 = 1; // CNS=1 → Identify Controller

    if unsafe { admin_submit(&mut ctrl, &cmd) }.is_none() {
        crate::serial_println!("  NVMe: Identify Controller failed");
        return;
    }

    // Read controller model name (bytes 24-63)
    let model = unsafe {
        let ptr = (identify_phys + 24) as *const u8;
        let slice = core::slice::from_raw_parts(ptr, 40);
        let mut end = 40;
        while end > 0 && (slice[end - 1] == 0 || slice[end - 1] == b' ') {
            end -= 1;
        }
        core::str::from_utf8_unchecked(&slice[..end])
    };
    crate::serial_println!("  NVMe: Controller: {}", model);

    // Identify Namespace 1 (CNS=0, NSID=1)
    unsafe { core::ptr::write_bytes(identify_phys as *mut u8, 0, 4096); }
    let mut cmd = NvmeCommand::zeroed();
    cmd.opcode = ADMIN_IDENTIFY;
    cmd.command_id = ctrl.next_cmd_id;
    ctrl.next_cmd_id += 1;
    cmd.nsid = 1;
    cmd.prp1 = identify_phys;
    cmd.cdw10 = 0; // CNS=0 → Identify Namespace

    if unsafe { admin_submit(&mut ctrl, &cmd) }.is_none() {
        crate::serial_println!("  NVMe: Identify Namespace 1 failed");
        return;
    }

    // Read namespace size (NSZE, bytes 0-7) and LBA format
    let nsze = unsafe { core::ptr::read_volatile(identify_phys as *const u64) };
    let flbas = unsafe { *((identify_phys + 26) as *const u8) }; // Formatted LBA Size
    let lba_format_idx = (flbas & 0x0F) as usize;
    // LBA format array starts at byte 128, each entry is 4 bytes
    let lbaf = unsafe { *((identify_phys + 128 + lba_format_idx as u64 * 4) as *const u32) };
    let lba_ds = (lbaf >> 16) & 0xFF; // LBA Data Size (power of 2)
    let sector_size = if lba_ds >= 9 { 1u32 << lba_ds } else { 512 };

    ctrl.ns1_sectors = nsze;
    ctrl.sector_size = sector_size;

    let size_mb = nsze * sector_size as u64 / (1024 * 1024);
    crate::serial_println!(
        "  NVMe: NS1: {} sectors, {} bytes/sector, {} MiB",
        nsze, sector_size, size_mb
    );

    // Create I/O Completion Queue (QID=1)
    let mut cmd = NvmeCommand::zeroed();
    cmd.opcode = ADMIN_CREATE_IO_CQ;
    cmd.command_id = ctrl.next_cmd_id;
    ctrl.next_cmd_id += 1;
    cmd.prp1 = iocq_phys;
    cmd.cdw10 = ((IO_QUEUE_SIZE - 1) as u32) << 16 | 1; // QID=1, Size
    cmd.cdw11 = 1; // Physically contiguous

    if unsafe { admin_submit(&mut ctrl, &cmd) }.is_none() {
        crate::serial_println!("  NVMe: Create I/O CQ failed");
        return;
    }

    // Create I/O Submission Queue (QID=1, linked to CQ 1)
    let mut cmd = NvmeCommand::zeroed();
    cmd.opcode = ADMIN_CREATE_IO_SQ;
    cmd.command_id = ctrl.next_cmd_id;
    ctrl.next_cmd_id += 1;
    cmd.prp1 = iosq_phys;
    cmd.cdw10 = ((IO_QUEUE_SIZE - 1) as u32) << 16 | 1; // QID=1, Size
    cmd.cdw11 = (1 << 16) | 1; // CQID=1, Physically contiguous

    if unsafe { admin_submit(&mut ctrl, &cmd) }.is_none() {
        crate::serial_println!("  NVMe: Create I/O SQ failed");
        return;
    }

    crate::serial_println!("[OK] NVMe: I/O queues created (SQ={}, CQ={})", IO_QUEUE_SIZE, IO_QUEUE_SIZE);

    // Store controller and switch backend
    unsafe { CTRL = Some(ctrl); }
    AVAILABLE.store(true, Ordering::Release);
    super::set_backend_nvme();
    crate::serial_println!("[OK] NVMe storage backend active");
}

/// Probe: initialize NVMe and return a HAL driver.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    init_and_register(pci);
    super::create_hal_driver("NVMe Controller")
}

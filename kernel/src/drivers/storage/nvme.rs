//! NVMe (Non-Volatile Memory Express) storage driver.
//!
//! Supports NVMe 1.0+ controllers over PCIe. Uses submission/completion queue
//! pairs for command submission and polled completion.
//!
//! Tested with VirtualBox NVMe (`80EE:4E56`) and QEMU NVMe.

use crate::drivers::pci::{pci_config_read32, pci_config_write32, PciDevice};
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::{physical, virtual_mem};
use alloc::boxed::Box;
use core::sync::atomic::{fence, AtomicBool, Ordering};

// ── MMIO virtual base ───────────────────────────────

const NVME_MMIO_VIRT: u64 = 0xFFFF_FFFF_D014_0000;
const NVME_MMIO_PAGES: usize = 4; // 16 KiB

// ── NVMe Controller Registers ───────────────────────

const REG_CAP: u64 = 0x00; // Controller Capabilities (64-bit)
const REG_VS: u64 = 0x08; // Version
const REG_INTMS: u64 = 0x0C; // Interrupt Mask Set
const REG_CC: u64 = 0x14; // Controller Configuration
const REG_CSTS: u64 = 0x1C; // Controller Status
const REG_AQA: u64 = 0x24; // Admin Queue Attributes
const REG_ASQ: u64 = 0x28; // Admin Submission Queue Base (64-bit)
const REG_ACQ: u64 = 0x30; // Admin Completion Queue Base (64-bit)

// CC register fields
const CC_EN: u32 = 1 << 0; // Enable
const CC_CSS_NVM: u32 = 0 << 4; // NVM Command Set
const CC_MPS_4K: u32 = 0 << 7; // Memory Page Size = 4 KiB (2^(12+0))
const CC_AMS_RR: u32 = 0 << 11; // Round Robin arbitration
const CC_IOSQES: u32 = 6 << 16; // I/O SQ entry size = 2^6 = 64 bytes
const CC_IOCQES: u32 = 4 << 20; // I/O CQ entry size = 2^4 = 16 bytes

// CSTS register fields
const CSTS_RDY: u32 = 1 << 0; // Ready

// ── NVMe Command Opcodes ────────────────────────────

// Admin commands
const ADMIN_IDENTIFY: u8 = 0x06;
const ADMIN_CREATE_IO_CQ: u8 = 0x05;
const ADMIN_CREATE_IO_SQ: u8 = 0x01;

// NVM I/O commands
const NVM_CMD_READ: u8 = 0x02;
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
            opcode: 0,
            flags: 0,
            command_id: 0,
            nsid: 0,
            reserved: [0; 2],
            metadata: 0,
            prp1: 0,
            prp2: 0,
            cdw10: 0,
            cdw11: 0,
            cdw12: 0,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }
}

/// NVMe Completion Queue Entry (16 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct NvmeCompletion {
    result: u32, // Command-specific result
    reserved: u32,
    sq_head: u16,    // SQ head pointer
    sq_id: u16,      // SQ identifier
    command_id: u16, // Command ID
    status: u16,     // Status field (bit 0 = phase tag, bits 1-15 = status)
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
    /// PRP list page used when the bounce transfer spans more than two pages.
    prp_list_phys: u64,
    prp_list_virt: u64,
    /// Per in-flight PRP list pages for queued direct I/O.
    io_prp_list_phys: [u64; NVME_IO_QUEUE_DEPTH],
    io_prp_list_virt: [u64; NVME_IO_QUEUE_DEPTH],
    io_depth: usize,
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
const PRP_LIST_ENTRIES: usize = 512; // one 4 KiB page of u64 PRP entries
const MAX_DIRECT_PRP_PAGES: usize = PRP_LIST_ENTRIES + 1;
const NVME_IO_QUEUE_DEPTH: usize = 8;
const NVME_QUEUED_CMD_SECTORS: u32 = 32;

#[derive(Clone, Copy)]
struct NvmeInflight {
    active: bool,
    command_id: u16,
}

const EMPTY_INFLIGHT: NvmeInflight = NvmeInflight {
    active: false,
    command_id: 0,
};

// ── MMIO Helpers ────────────────────────────────────

/// NVMe MMIO region size: 64 KiB covers registers + doorbells for up to ~500 queues.
const NVME_MMIO_SIZE: u64 = 0x10000;

#[inline]
unsafe fn mmio_read32(base: u64, offset: u64) -> u32 {
    debug_assert!(
        offset + 4 <= NVME_MMIO_SIZE,
        "NVMe MMIO read32 out of bounds: offset={:#x}",
        offset
    );
    core::ptr::read_volatile((base + offset) as *const u32)
}

#[inline]
unsafe fn mmio_write32(base: u64, offset: u64, val: u32) {
    debug_assert!(
        offset + 4 <= NVME_MMIO_SIZE,
        "NVMe MMIO write32 out of bounds: offset={:#x}",
        offset
    );
    core::ptr::write_volatile((base + offset) as *mut u32, val);
}

#[inline]
unsafe fn mmio_read64(base: u64, offset: u64) -> u64 {
    debug_assert!(
        offset + 8 <= NVME_MMIO_SIZE,
        "NVMe MMIO read64 out of bounds: offset={:#x}",
        offset
    );
    // NVMe spec says 64-bit registers can be read as two 32-bit reads
    let lo = core::ptr::read_volatile((base + offset) as *const u32) as u64;
    let hi = core::ptr::read_volatile((base + offset + 4) as *const u32) as u64;
    lo | (hi << 32)
}

#[inline]
unsafe fn mmio_write64(base: u64, offset: u64, val: u64) {
    debug_assert!(
        offset + 8 <= NVME_MMIO_SIZE,
        "NVMe MMIO write64 out of bounds: offset={:#x}",
        offset
    );
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
    fence(Ordering::SeqCst);
    core::ptr::write_volatile(sq_entry, *cmd);
    fence(Ordering::SeqCst);

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
            fence(Ordering::SeqCst);
            return Some(cqe);
        }
        core::hint::spin_loop();
    }
    crate::serial_verbose_println!("NVMe: admin command timeout");
    None
}

unsafe fn io_submit_nowait(ctrl: &mut NvmeController, cmd: &NvmeCommand) {
    let sq_entry = (ctrl.iosq_phys + ctrl.io_sq_tail as u64 * 64) as *mut NvmeCommand;
    fence(Ordering::SeqCst);
    core::ptr::write_volatile(sq_entry, *cmd);
    fence(Ordering::SeqCst);

    ctrl.io_sq_tail = (ctrl.io_sq_tail + 1) % IO_QUEUE_SIZE;
    write_sq_doorbell(ctrl, 1, ctrl.io_sq_tail);
}

unsafe fn io_poll_completion(ctrl: &mut NvmeController) -> Option<NvmeCompletion> {
    let cq_entry_ptr = (ctrl.iocq_phys + ctrl.io_cq_head as u64 * 16) as *const NvmeCompletion;
    let cqe = core::ptr::read_volatile(cq_entry_ptr);
    let phase = (cqe.status & 1) != 0;
    if phase != ctrl.io_phase {
        return None;
    }

    ctrl.io_cq_head = (ctrl.io_cq_head + 1) % IO_QUEUE_SIZE;
    if ctrl.io_cq_head == 0 {
        ctrl.io_phase = !ctrl.io_phase;
    }
    write_cq_doorbell(ctrl, 1, ctrl.io_cq_head);
    fence(Ordering::SeqCst);
    Some(cqe)
}

unsafe fn io_wait_completion(ctrl: &mut NvmeController) -> Option<NvmeCompletion> {
    for spin in 0..10_000_000usize {
        if let Some(cqe) = io_poll_completion(ctrl) {
            return Some(cqe);
        }
        if spin % 256 == 0 && crate::task::scheduler::current_tid() > 0 {
            crate::task::scheduler::schedule();
        } else {
            core::hint::spin_loop();
        }
    }
    crate::serial_verbose_println!("NVMe: I/O command timeout");
    None
}

#[inline]
fn cqe_status(cqe: &NvmeCompletion) -> u16 {
    (cqe.status >> 1) & 0x7FFF
}

/// Submit an I/O command and wait for completion.
unsafe fn io_submit(ctrl: &mut NvmeController, cmd: &NvmeCommand) -> Option<NvmeCompletion> {
    io_submit_nowait(ctrl, cmd);
    loop {
        let cqe = io_wait_completion(ctrl)?;
        let sc = cqe_status(&cqe);
        if sc != 0 {
            crate::serial_verbose_println!("NVMe: I/O error, status={:#06x}", sc);
            return None;
        }
        if cqe.command_id == cmd.command_id {
            return Some(cqe);
        }
        crate::serial_verbose_println!(
            "NVMe: ignoring stale completion command_id={} while waiting for {}",
            cqe.command_id,
            cmd.command_id
        );
    }
}

unsafe fn wait_for_known_completion(
    ctrl: &mut NvmeController,
    inflight: &mut [NvmeInflight; NVME_IO_QUEUE_DEPTH],
) -> bool {
    loop {
        let cqe = match io_wait_completion(ctrl) {
            Some(cqe) => cqe,
            None => return false,
        };
        let sc = cqe_status(&cqe);
        if sc != 0 {
            crate::serial_verbose_println!("NVMe: queued I/O error, status={:#06x}", sc);
            return false;
        }

        for slot in inflight.iter_mut() {
            if slot.active && slot.command_id == cqe.command_id {
                slot.active = false;
                return true;
            }
        }

        crate::serial_verbose_println!(
            "NVMe: ignoring stale completion command_id={}",
            cqe.command_id
        );
    }
}

unsafe fn setup_data_prps(
    ctrl: &mut NvmeController,
    cmd: &mut NvmeCommand,
    byte_count: usize,
) -> bool {
    if byte_count == 0 || byte_count > BOUNCE_SIZE {
        return false;
    }

    cmd.prp1 = ctrl.bounce_phys;
    cmd.prp2 = 0;

    let pages = (byte_count + 4095) / 4096;
    if pages <= 1 {
        return true;
    }
    if pages == 2 {
        cmd.prp2 = ctrl.bounce_phys + 4096;
        return true;
    }

    let list_entries = pages - 1;
    if list_entries > PRP_LIST_ENTRIES {
        return false;
    }

    let list = ctrl.prp_list_virt as *mut u64;
    core::ptr::write_bytes(list, 0, PRP_LIST_ENTRIES);
    for page in 1..pages {
        core::ptr::write_volatile(list.add(page - 1), ctrl.bounce_phys + page as u64 * 4096);
    }
    cmd.prp2 = ctrl.prp_list_phys;
    true
}

unsafe fn setup_data_prps_from_virt(
    cmd: &mut NvmeCommand,
    ptr: *const u8,
    byte_count: usize,
    prp_list_phys: u64,
    prp_list_virt: u64,
) -> bool {
    if byte_count == 0 {
        return false;
    }

    let start = ptr as usize;
    let first_phys = match virtual_mem::virt_to_phys(VirtAddr::new(start as u64)) {
        Some(phys) => phys,
        None => return false,
    };

    cmd.prp1 = first_phys;
    cmd.prp2 = 0;

    let first_page_left = 4096 - (start & 0xFFF);
    if byte_count <= first_page_left {
        return true;
    }

    let mut remaining = byte_count - first_page_left;
    let mut virt = start + first_page_left;
    let pages_after_first = (remaining + 4095) / 4096;
    if pages_after_first == 0 {
        return true;
    }
    if pages_after_first > PRP_LIST_ENTRIES {
        return false;
    }

    let second_phys = match virtual_mem::virt_to_phys(VirtAddr::new(virt as u64)) {
        Some(phys) => phys,
        None => return false,
    };
    if pages_after_first == 1 {
        cmd.prp2 = second_phys;
        return true;
    }
    if prp_list_phys == 0 || prp_list_virt == 0 {
        return false;
    }

    let list = prp_list_virt as *mut u64;
    core::ptr::write_bytes(list, 0, PRP_LIST_ENTRIES);
    core::ptr::write_volatile(list, second_phys);
    remaining = remaining.saturating_sub(4096);
    virt += 4096;

    let mut idx = 1usize;
    while remaining > 0 {
        if idx >= PRP_LIST_ENTRIES {
            return false;
        }
        let phys = match virtual_mem::virt_to_phys(VirtAddr::new(virt as u64)) {
            Some(phys) => phys,
            None => return false,
        };
        core::ptr::write_volatile(list.add(idx), phys);
        idx += 1;
        let step = remaining.min(4096);
        remaining -= step;
        virt += step;
    }

    cmd.prp2 = prp_list_phys;
    true
}

fn direct_prps_mappable(ptr: *const u8, byte_count: usize) -> bool {
    if byte_count == 0 {
        return false;
    }

    let start = ptr as usize;
    if virtual_mem::virt_to_phys(VirtAddr::new(start as u64)).is_none() {
        return false;
    }

    let first_page_left = 4096 - (start & 0xFFF);
    if byte_count <= first_page_left {
        return true;
    }

    let pages_after_first = (byte_count - first_page_left + 4095) / 4096;
    if pages_after_first > PRP_LIST_ENTRIES {
        return false;
    }

    let mut remaining = byte_count - first_page_left;
    let mut virt = start + first_page_left;
    while remaining > 0 {
        if virtual_mem::virt_to_phys(VirtAddr::new(virt as u64)).is_none() {
            return false;
        }
        let step = remaining.min(4096);
        remaining -= step;
        virt += step;
    }

    true
}

fn queued_direct_range_mappable(
    ctrl: &NvmeController,
    ptr: *const u8,
    len: usize,
    count: u32,
) -> bool {
    if ctrl.io_depth < 2 || count <= NVME_QUEUED_CMD_SECTORS {
        return false;
    }

    let mut offset = 0usize;
    let mut remaining = count;
    while remaining > 0 {
        let batch = remaining.min(NVME_QUEUED_CMD_SECTORS);
        let byte_count = batch as usize * ctrl.sector_size as usize;
        if offset + byte_count > len {
            return false;
        }
        let chunk_ptr = unsafe { ptr.add(offset) };
        if !direct_prps_mappable(chunk_ptr, byte_count) {
            return false;
        }
        offset += byte_count;
        remaining -= batch;
    }

    true
}

unsafe fn queued_direct_transfer(
    ctrl: &mut NvmeController,
    opcode: u8,
    lba: u32,
    count: u32,
    ptr: *const u8,
    len: usize,
) -> bool {
    let depth = ctrl
        .io_depth
        .min(NVME_IO_QUEUE_DEPTH)
        .min((IO_QUEUE_SIZE as usize).saturating_sub(1));
    if depth < 2 {
        return false;
    }

    let mut inflight = [EMPTY_INFLIGHT; NVME_IO_QUEUE_DEPTH];
    let mut outstanding = 0usize;
    let mut offset = 0usize;
    let mut cur_lba = lba as u64;
    let mut remaining = count;

    while remaining > 0 || outstanding > 0 {
        while remaining > 0 && outstanding < depth {
            let slot = match inflight[..depth].iter().position(|s| !s.active) {
                Some(slot) => slot,
                None => break,
            };

            let batch = remaining.min(NVME_QUEUED_CMD_SECTORS);
            let byte_count = batch as usize * ctrl.sector_size as usize;
            if offset + byte_count > len {
                return false;
            }

            let mut cmd = NvmeCommand::zeroed();
            if !setup_data_prps_from_virt(
                &mut cmd,
                ptr.add(offset),
                byte_count,
                ctrl.io_prp_list_phys[slot],
                ctrl.io_prp_list_virt[slot],
            ) {
                return false;
            }

            cmd.opcode = opcode;
            cmd.command_id = ctrl.next_cmd_id;
            ctrl.next_cmd_id = ctrl.next_cmd_id.wrapping_add(1);
            cmd.nsid = 1;
            cmd.cdw10 = cur_lba as u32;
            cmd.cdw11 = (cur_lba >> 32) as u32;
            cmd.cdw12 = batch - 1;

            inflight[slot] = NvmeInflight {
                active: true,
                command_id: cmd.command_id,
            };
            io_submit_nowait(ctrl, &cmd);

            outstanding += 1;
            offset += byte_count;
            cur_lba += batch as u64;
            remaining -= batch;
        }

        if outstanding == 0 {
            break;
        }
        if !wait_for_known_completion(ctrl, &mut inflight) {
            return false;
        }
        outstanding -= 1;
    }

    true
}

// ── Public API ──────────────────────────────────────

/// Read sectors from NVMe namespace 1.
pub fn read_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    if !AVAILABLE.load(Ordering::Relaxed) {
        return false;
    }

    let ctrl = match unsafe { CTRL.as_mut() } {
        Some(c) => c,
        None => return false,
    };
    let total_bytes = count as usize * ctrl.sector_size as usize;
    if total_bytes > buf.len() {
        return false;
    }
    if queued_direct_range_mappable(ctrl, buf.as_ptr(), buf.len(), count) {
        return unsafe {
            queued_direct_transfer(
                ctrl,
                NVM_CMD_READ,
                lba,
                count,
                buf.as_mut_ptr() as *const u8,
                buf.len(),
            )
        };
    }

    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba as u64;

    while remaining > 0 {
        let mut cmd = NvmeCommand::zeroed();
        let direct_max_batch =
            ((MAX_DIRECT_PRP_PAGES * 4096) / ctrl.sector_size as usize).max(1) as u32;
        let mut batch = remaining.min(direct_max_batch);
        let mut byte_count = batch as usize * ctrl.sector_size as usize;
        if offset + byte_count > buf.len() {
            return false;
        }

        let dst = unsafe { buf.as_mut_ptr().add(offset) };
        let direct = unsafe {
            setup_data_prps_from_virt(
                &mut cmd,
                dst as *const u8,
                byte_count,
                ctrl.prp_list_phys,
                ctrl.prp_list_virt,
            )
        };
        if !direct {
            let max_batch = (BOUNCE_SIZE / ctrl.sector_size as usize).max(1) as u32;
            batch = remaining.min(max_batch);
            byte_count = batch as usize * ctrl.sector_size as usize;
            if offset + byte_count > buf.len() {
                return false;
            }
            if !unsafe { setup_data_prps(ctrl, &mut cmd, byte_count) } {
                return false;
            }
        }

        cmd.opcode = NVM_CMD_READ;
        cmd.command_id = ctrl.next_cmd_id;
        ctrl.next_cmd_id = ctrl.next_cmd_id.wrapping_add(1);
        cmd.nsid = 1;
        cmd.cdw10 = cur_lba as u32; // Starting LBA (low 32)
        cmd.cdw11 = (cur_lba >> 32) as u32; // Starting LBA (high 32)
        cmd.cdw12 = batch - 1; // Number of logical blocks (0-based)

        let ok = unsafe { io_submit(ctrl, &cmd).is_some() };
        if !ok {
            return false;
        }

        if !direct {
            let src = ctrl.bounce_virt as *const u8;
            unsafe {
                core::ptr::copy_nonoverlapping(src, dst, byte_count);
            }
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

    let ctrl = match unsafe { CTRL.as_mut() } {
        Some(c) => c,
        None => return false,
    };
    let total_bytes = count as usize * ctrl.sector_size as usize;
    if total_bytes > buf.len() {
        return false;
    }
    if queued_direct_range_mappable(ctrl, buf.as_ptr(), buf.len(), count) {
        return unsafe {
            queued_direct_transfer(ctrl, NVM_CMD_WRITE, lba, count, buf.as_ptr(), buf.len())
        };
    }

    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba as u64;

    while remaining > 0 {
        let mut cmd = NvmeCommand::zeroed();
        let direct_max_batch =
            ((MAX_DIRECT_PRP_PAGES * 4096) / ctrl.sector_size as usize).max(1) as u32;
        let mut batch = remaining.min(direct_max_batch);
        let mut byte_count = batch as usize * ctrl.sector_size as usize;
        if offset + byte_count > buf.len() {
            return false;
        }

        let src = unsafe { buf.as_ptr().add(offset) };
        if !unsafe {
            setup_data_prps_from_virt(
                &mut cmd,
                src,
                byte_count,
                ctrl.prp_list_phys,
                ctrl.prp_list_virt,
            )
        } {
            let max_batch = (BOUNCE_SIZE / ctrl.sector_size as usize).max(1) as u32;
            batch = remaining.min(max_batch);
            byte_count = batch as usize * ctrl.sector_size as usize;
            if offset + byte_count > buf.len() {
                return false;
            }
            let dst = ctrl.bounce_virt as *mut u8;
            unsafe {
                core::ptr::copy_nonoverlapping(src, dst, byte_count);
            }
            if !unsafe { setup_data_prps(ctrl, &mut cmd, byte_count) } {
                return false;
            }
        }

        cmd.opcode = NVM_CMD_WRITE;
        cmd.command_id = ctrl.next_cmd_id;
        ctrl.next_cmd_id = ctrl.next_cmd_id.wrapping_add(1);
        cmd.nsid = 1;
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
        crate::serial_verbose_println!("  NVMe: BAR0 is I/O port (expected MMIO)");
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
    let dstrd = 4 << ((cap >> 32) & 0xF); // Doorbell Stride (bytes)
    let timeout = ((cap >> 24) & 0xFF) as u32 * 500; // Timeout in ms

    crate::serial_verbose_println!(
        "  NVMe: version {}.{}.{}, MQES={}, DSTRD={}, timeout={}ms",
        (vs >> 16) & 0xFF,
        (vs >> 8) & 0xFF,
        vs & 0xFF,
        mqes,
        dstrd,
        timeout
    );

    // Disable controller if enabled
    let cc = unsafe { mmio_read32(base, REG_CC) };
    if cc & CC_EN != 0 {
        unsafe {
            mmio_write32(base, REG_CC, cc & !CC_EN);
        }
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
        None => {
            crate::serial_verbose_println!("  NVMe: alloc ASQ failed");
            return;
        }
    };
    // Admin CQ: ADMIN_QUEUE_SIZE * 16 bytes (1 page)
    let acq_phys = match physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_verbose_println!("  NVMe: alloc ACQ failed");
            return;
        }
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
    unsafe {
        mmio_write32(base, REG_INTMS, 0xFFFF_FFFF);
    }

    // Configure admin queues
    let aqa = ((ADMIN_QUEUE_SIZE - 1) as u32) << 16 | (ADMIN_QUEUE_SIZE - 1) as u32;
    unsafe {
        mmio_write32(base, REG_AQA, aqa);
        mmio_write64(base, REG_ASQ, asq_phys);
        mmio_write64(base, REG_ACQ, acq_phys);
    }

    // Enable controller
    let cc_val = CC_EN | CC_CSS_NVM | CC_MPS_4K | CC_AMS_RR | CC_IOSQES | CC_IOCQES;
    unsafe {
        mmio_write32(base, REG_CC, cc_val);
    }

    // Wait for ready
    for _ in 0..10_000_000 {
        if unsafe { mmio_read32(base, REG_CSTS) } & CSTS_RDY != 0 {
            break;
        }
        core::hint::spin_loop();
    }
    if unsafe { mmio_read32(base, REG_CSTS) } & CSTS_RDY == 0 {
        crate::serial_verbose_println!("  NVMe: controller failed to become ready");
        return;
    }
    crate::serial_verbose_println!("  NVMe: controller enabled and ready");

    // Allocate I/O queues
    let iosq_phys = match physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_verbose_println!("  NVMe: alloc IOSQ failed");
            return;
        }
    };
    let iocq_phys = match physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_verbose_println!("  NVMe: alloc IOCQ failed");
            return;
        }
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
        None => {
            crate::serial_verbose_println!("  NVMe: alloc bounce buffer failed");
            return;
        }
    };
    for i in 0..BOUNCE_PAGES {
        let p = bounce_phys + (i as u64) * 4096;
        virtual_mem::map_page(VirtAddr::new(p), PhysAddr::new(p), 0x03);
    }

    let prp_list_phys = match physical::alloc_frame() {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_verbose_println!("  NVMe: alloc PRP list failed");
            return;
        }
    };
    virtual_mem::map_page(
        VirtAddr::new(prp_list_phys),
        PhysAddr::new(prp_list_phys),
        0x03,
    );
    unsafe {
        core::ptr::write_bytes(prp_list_phys as *mut u8, 0, 4096);
    }

    let mut io_prp_list_phys = [0u64; NVME_IO_QUEUE_DEPTH];
    let mut io_prp_list_virt = [0u64; NVME_IO_QUEUE_DEPTH];
    let mut io_depth = 0usize;
    for slot in 0..NVME_IO_QUEUE_DEPTH {
        let phys = match physical::alloc_frame() {
            Some(p) => p.as_u64(),
            None => break,
        };
        virtual_mem::map_page(VirtAddr::new(phys), PhysAddr::new(phys), 0x03);
        unsafe {
            core::ptr::write_bytes(phys as *mut u8, 0, 4096);
        }
        io_prp_list_phys[slot] = phys;
        io_prp_list_virt[slot] = phys;
        io_depth += 1;
    }
    if io_depth < 2 {
        crate::serial_verbose_println!(
            "  NVMe: queued direct I/O disabled (PRP lists={})",
            io_depth
        );
    } else {
        crate::serial_verbose_println!("  NVMe: queued direct I/O depth={}", io_depth);
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
        prp_list_phys,
        prp_list_virt: prp_list_phys,
        io_prp_list_phys,
        io_prp_list_virt,
        io_depth,
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
        None => {
            crate::serial_verbose_println!("  NVMe: alloc identify failed");
            return;
        }
    };
    virtual_mem::map_page(
        VirtAddr::new(identify_phys),
        PhysAddr::new(identify_phys),
        0x03,
    );
    unsafe {
        core::ptr::write_bytes(identify_phys as *mut u8, 0, 4096);
    }

    let mut cmd = NvmeCommand::zeroed();
    cmd.opcode = ADMIN_IDENTIFY;
    cmd.command_id = ctrl.next_cmd_id;
    ctrl.next_cmd_id += 1;
    cmd.prp1 = identify_phys;
    cmd.cdw10 = 1; // CNS=1 → Identify Controller

    if unsafe { admin_submit(&mut ctrl, &cmd) }.is_none() {
        crate::serial_verbose_println!("  NVMe: Identify Controller failed");
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
    crate::serial_verbose_println!("  NVMe: Controller: {}", model);

    // Identify Namespace 1 (CNS=0, NSID=1)
    unsafe {
        core::ptr::write_bytes(identify_phys as *mut u8, 0, 4096);
    }
    let mut cmd = NvmeCommand::zeroed();
    cmd.opcode = ADMIN_IDENTIFY;
    cmd.command_id = ctrl.next_cmd_id;
    ctrl.next_cmd_id += 1;
    cmd.nsid = 1;
    cmd.prp1 = identify_phys;
    cmd.cdw10 = 0; // CNS=0 → Identify Namespace

    if unsafe { admin_submit(&mut ctrl, &cmd) }.is_none() {
        crate::serial_verbose_println!("  NVMe: Identify Namespace 1 failed");
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
    crate::serial_verbose_println!(
        "  NVMe: NS1: {} sectors, {} bytes/sector, {} MiB",
        nsze,
        sector_size,
        size_mb
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
        crate::serial_verbose_println!("  NVMe: Create I/O CQ failed");
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
        crate::serial_verbose_println!("  NVMe: Create I/O SQ failed");
        return;
    }

    crate::serial_verbose_println!(
        "[OK] NVMe: I/O queues created (SQ={}, CQ={})",
        IO_QUEUE_SIZE,
        IO_QUEUE_SIZE
    );

    // Store controller and switch backend
    unsafe {
        CTRL = Some(ctrl);
    }
    AVAILABLE.store(true, Ordering::Release);
    super::set_backend_nvme();
    crate::serial_verbose_println!("[OK] NVMe storage backend active");
}

/// Probe: initialize NVMe and return a HAL driver.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    init_and_register(pci);
    super::create_hal_driver("NVMe Controller")
}

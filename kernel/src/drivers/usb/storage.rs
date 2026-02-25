//! USB Mass Storage class driver (Bulk-Only Transport).
//!
//! Supports SCSI transparent command set (subclass 0x06) with
//! bulk-only transport protocol (protocol 0x50).
//!
//! Implements: INQUIRY, TEST_UNIT_READY, READ_CAPACITY, READ_10, WRITE_10.

use super::{UsbDevice, UsbInterface, ControllerType, UsbSpeed};
use crate::memory::physical;
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

/// USB mass storage Command Block Wrapper (31 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Cbw {
    signature: u32,       // 0x43425355
    tag: u32,             // matches CSW
    data_transfer_length: u32,
    flags: u8,            // bit7: 0=OUT, 1=IN
    lun: u8,
    cb_length: u8,        // 1-16
    cb: [u8; 16],         // SCSI command block
}

/// USB mass storage Command Status Wrapper (13 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Csw {
    signature: u32,       // 0x53425355
    tag: u32,
    data_residue: u32,
    status: u8,           // 0=Passed, 1=Failed, 2=Phase Error
}

const CBW_SIGNATURE: u32 = 0x43425355;
const CSW_SIGNATURE: u32 = 0x53425355;
const CBW_FLAG_IN: u8 = 0x80;
const CBW_FLAG_OUT: u8 = 0x00;

// SCSI commands
const SCSI_TEST_UNIT_READY: u8 = 0x00;
const SCSI_INQUIRY: u8 = 0x12;
const SCSI_READ_CAPACITY: u8 = 0x25;
const SCSI_READ_10: u8 = 0x28;
const SCSI_WRITE_10: u8 = 0x2A;

/// Bounce buffer size: 64 KiB (128 sectors).
const BOUNCE_PAGES: usize = 16;
const BOUNCE_SIZE: usize = BOUNCE_PAGES * 4096;

// ── Device State ─────────────────────────────────

/// Per-device state for a USB mass storage device.
struct UsbStorageDevice {
    usb_addr: u8,
    controller: ControllerType,
    speed: UsbSpeed,
    port: u8,
    ep_in: u8,
    ep_out: u8,
    max_packet_in: u16,
    max_packet_out: u16,
    toggle_in: u8,
    toggle_out: u8,
    tag: u32,
    block_count: u32,
    block_size: u32,
    /// 1 page: CBW at offset 0 (31 bytes), CSW at offset 64 (13 bytes).
    cbw_csw_phys: u64,
    /// 64 KiB bounce buffer for sector data (16 contiguous pages).
    bounce_phys: u64,
    /// Assigned disk_id in the block device registry.
    disk_id: u8,
}

impl UsbStorageDevice {
    fn next_tag(&mut self) -> u32 {
        let t = self.tag;
        self.tag = self.tag.wrapping_add(1);
        t
    }
}

/// Global registry of USB storage devices.
static USB_STORAGE_DEVICES: Spinlock<Vec<UsbStorageDevice>> = Spinlock::new(Vec::new());

/// Next available disk_id for USB storage (boot disk is 0).
static NEXT_USB_DISK_ID: Spinlock<u8> = Spinlock::new(1);

fn alloc_disk_id() -> u8 {
    let mut id = NEXT_USB_DISK_ID.lock();
    let d = *id;
    *id = id.wrapping_add(1);
    d
}

// ── CBW / CSW Transfer ───────────────────────────

fn send_cbw(dev: &mut UsbStorageDevice, cbw: &Cbw) -> Result<(), &'static str> {
    // Copy CBW to DMA buffer at cbw_csw_phys
    unsafe {
        core::ptr::copy_nonoverlapping(
            cbw as *const Cbw as *const u8,
            dev.cbw_csw_phys as *mut u8,
            31,
        );
    }
    let sent = super::bulk_transfer(
        dev.usb_addr, dev.controller, dev.speed,
        dev.ep_out, dev.max_packet_out,
        &mut dev.toggle_out,
        dev.cbw_csw_phys, 31,
    )?;
    if sent < 31 { return Err("CBW short write"); }
    Ok(())
}

fn recv_csw(dev: &mut UsbStorageDevice, expected_tag: u32) -> Result<Csw, &'static str> {
    // CSW at cbw_csw_phys + 64 (avoid overlap with CBW)
    let csw_phys = dev.cbw_csw_phys + 64;
    let read = super::bulk_transfer(
        dev.usb_addr, dev.controller, dev.speed,
        dev.ep_in, dev.max_packet_in,
        &mut dev.toggle_in,
        csw_phys, 13,
    )?;
    if read < 13 { return Err("CSW short read"); }
    let csw: Csw = unsafe { core::ptr::read_unaligned(csw_phys as *const Csw) };
    if csw.signature != CSW_SIGNATURE { return Err("CSW bad signature"); }
    if csw.tag != expected_tag { return Err("CSW tag mismatch"); }
    if csw.status != 0 { return Err("CSW command failed"); }
    Ok(csw)
}

// ── SCSI Commands ────────────────────────────────

fn scsi_test_unit_ready(dev: &mut UsbStorageDevice) -> Result<(), &'static str> {
    let tag = dev.next_tag();
    let cbw = Cbw {
        signature: CBW_SIGNATURE, tag,
        data_transfer_length: 0,
        flags: CBW_FLAG_OUT, lun: 0, cb_length: 6,
        cb: [SCSI_TEST_UNIT_READY, 0,0,0,0,0, 0,0,0,0,0,0,0,0,0,0],
    };
    send_cbw(dev, &cbw)?;
    recv_csw(dev, tag)?;
    Ok(())
}

fn scsi_inquiry(dev: &mut UsbStorageDevice) -> Result<[u8; 36], &'static str> {
    let tag = dev.next_tag();
    let cbw = Cbw {
        signature: CBW_SIGNATURE, tag,
        data_transfer_length: 36,
        flags: CBW_FLAG_IN, lun: 0, cb_length: 6,
        cb: [SCSI_INQUIRY, 0, 0, 0, 36, 0, 0,0,0,0,0,0,0,0,0,0],
    };
    send_cbw(dev, &cbw)?;
    // Data phase: read 36 bytes into bounce buffer
    super::bulk_transfer(
        dev.usb_addr, dev.controller, dev.speed,
        dev.ep_in, dev.max_packet_in,
        &mut dev.toggle_in,
        dev.bounce_phys, 36,
    )?;
    let mut result = [0u8; 36];
    unsafe {
        core::ptr::copy_nonoverlapping(dev.bounce_phys as *const u8, result.as_mut_ptr(), 36);
    }
    recv_csw(dev, tag)?;
    Ok(result)
}

fn scsi_read_capacity(dev: &mut UsbStorageDevice) -> Result<(u32, u32), &'static str> {
    let tag = dev.next_tag();
    let cbw = Cbw {
        signature: CBW_SIGNATURE, tag,
        data_transfer_length: 8,
        flags: CBW_FLAG_IN, lun: 0, cb_length: 10,
        cb: [SCSI_READ_CAPACITY, 0,0,0,0,0,0,0,0,0, 0,0,0,0,0,0],
    };
    send_cbw(dev, &cbw)?;
    super::bulk_transfer(
        dev.usb_addr, dev.controller, dev.speed,
        dev.ep_in, dev.max_packet_in,
        &mut dev.toggle_in,
        dev.bounce_phys, 8,
    )?;
    let buf: [u8; 8] = unsafe {
        let mut b = [0u8; 8];
        core::ptr::copy_nonoverlapping(dev.bounce_phys as *const u8, b.as_mut_ptr(), 8);
        b
    };
    recv_csw(dev, tag)?;
    let last_lba = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let block_size = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    Ok((last_lba.wrapping_add(1), block_size))
}

fn scsi_read_10(dev: &mut UsbStorageDevice, lba: u32, count: u16) -> Result<(), &'static str> {
    let byte_count = count as u32 * dev.block_size;
    let tag = dev.next_tag();
    let mut cb = [0u8; 16];
    cb[0] = SCSI_READ_10;
    cb[2] = (lba >> 24) as u8;
    cb[3] = (lba >> 16) as u8;
    cb[4] = (lba >> 8) as u8;
    cb[5] = lba as u8;
    cb[7] = (count >> 8) as u8;
    cb[8] = count as u8;
    let cbw = Cbw {
        signature: CBW_SIGNATURE, tag,
        data_transfer_length: byte_count,
        flags: CBW_FLAG_IN, lun: 0, cb_length: 10, cb,
    };
    send_cbw(dev, &cbw)?;

    // Data phase: may need multiple bulk transfers
    let mut received = 0usize;
    while received < byte_count as usize {
        let remaining = byte_count as usize - received;
        let chunk = remaining.min(dev.max_packet_in as usize);
        let got = super::bulk_transfer(
            dev.usb_addr, dev.controller, dev.speed,
            dev.ep_in, dev.max_packet_in,
            &mut dev.toggle_in,
            dev.bounce_phys + received as u64, chunk,
        )?;
        received += got;
        if got < chunk { break; } // short packet
    }

    recv_csw(dev, tag)?;
    Ok(())
}

fn scsi_write_10(dev: &mut UsbStorageDevice, lba: u32, count: u16) -> Result<(), &'static str> {
    let byte_count = count as u32 * dev.block_size;
    let tag = dev.next_tag();
    let mut cb = [0u8; 16];
    cb[0] = SCSI_WRITE_10;
    cb[2] = (lba >> 24) as u8;
    cb[3] = (lba >> 16) as u8;
    cb[4] = (lba >> 8) as u8;
    cb[5] = lba as u8;
    cb[7] = (count >> 8) as u8;
    cb[8] = count as u8;
    let cbw = Cbw {
        signature: CBW_SIGNATURE, tag,
        data_transfer_length: byte_count,
        flags: CBW_FLAG_OUT, lun: 0, cb_length: 10, cb,
    };
    send_cbw(dev, &cbw)?;

    // Data phase: send data from bounce buffer
    let mut sent = 0usize;
    while sent < byte_count as usize {
        let remaining = byte_count as usize - sent;
        let chunk = remaining.min(dev.max_packet_out as usize);
        let wrote = super::bulk_transfer(
            dev.usb_addr, dev.controller, dev.speed,
            dev.ep_out, dev.max_packet_out,
            &mut dev.toggle_out,
            dev.bounce_phys + sent as u64, chunk,
        )?;
        sent += wrote;
        if wrote < chunk { break; }
    }

    recv_csw(dev, tag)?;
    Ok(())
}

// ── Block Device Read/Write Dispatch ─────────────

/// Read sectors from a USB storage device. Registered as I/O override.
pub fn usb_storage_read(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
    let mut devs = USB_STORAGE_DEVICES.lock();
    let dev = match devs.iter_mut().find(|d| d.disk_id == disk_id) {
        Some(d) => d,
        None => return false,
    };

    let bs = dev.block_size as usize;
    let max_sectors = (BOUNCE_SIZE / bs) as u32;
    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba;

    while remaining > 0 {
        let batch = remaining.min(max_sectors);
        if scsi_read_10(dev, cur_lba, batch as u16).is_err() {
            return false;
        }
        let bytes = batch as usize * bs;
        unsafe {
            core::ptr::copy_nonoverlapping(
                dev.bounce_phys as *const u8,
                buf[offset..].as_mut_ptr(),
                bytes,
            );
        }
        offset += bytes;
        cur_lba += batch;
        remaining -= batch;
    }
    true
}

/// Write sectors to a USB storage device. Registered as I/O override.
pub fn usb_storage_write(disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
    let mut devs = USB_STORAGE_DEVICES.lock();
    let dev = match devs.iter_mut().find(|d| d.disk_id == disk_id) {
        Some(d) => d,
        None => return false,
    };

    let bs = dev.block_size as usize;
    let max_sectors = (BOUNCE_SIZE / bs) as u32;
    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba;

    while remaining > 0 {
        let batch = remaining.min(max_sectors);
        let bytes = batch as usize * bs;
        unsafe {
            core::ptr::copy_nonoverlapping(
                buf[offset..].as_ptr(),
                dev.bounce_phys as *mut u8,
                bytes,
            );
        }
        if scsi_write_10(dev, cur_lba, batch as u16).is_err() {
            return false;
        }
        offset += bytes;
        cur_lba += batch;
        remaining -= batch;
    }
    true
}

// ── Probe + Initialization ───────────────────────

/// Called when a mass storage interface is detected during USB enumeration.
pub fn probe(dev: &UsbDevice, iface: &UsbInterface) {
    let subclass_desc = match iface.subclass {
        0x01 => "RBC",
        0x02 => "ATAPI",
        0x03 => "QIC-157",
        0x04 => "UFI (floppy)",
        0x05 => "SFF-8070i",
        0x06 => "SCSI transparent",
        _ => "Unknown",
    };

    let protocol_desc = match iface.protocol {
        0x00 => "CBI with interrupt",
        0x01 => "CBI without interrupt",
        0x50 => "Bulk-Only",
        0x62 => "UAS",
        _ => "Unknown",
    };

    crate::serial_println!(
        "  USB Storage: detected (subclass={:#04x} [{}], protocol={:#04x} [{}], addr={})",
        iface.subclass, subclass_desc,
        iface.protocol, protocol_desc,
        dev.address,
    );

    // Find bulk IN and bulk OUT endpoints
    let bulk_in = iface.endpoints.iter().find(|ep| {
        (ep.attributes & 0x03) == 2    // Bulk transfer type
            && (ep.address & 0x80) != 0 // IN direction
    });

    let bulk_out = iface.endpoints.iter().find(|ep| {
        (ep.attributes & 0x03) == 2    // Bulk transfer type
            && (ep.address & 0x80) == 0 // OUT direction
    });

    let (ep_in, ep_out) = match (bulk_in, bulk_out) {
        (Some(i), Some(o)) => {
            crate::serial_println!(
                "  USB Storage: bulk IN ep={:#04x} (max={}), bulk OUT ep={:#04x} (max={})",
                i.address, i.max_packet_size,
                o.address, o.max_packet_size,
            );
            (i, o)
        }
        _ => {
            crate::serial_println!("  USB Storage: missing bulk endpoints");
            return;
        }
    };

    // Only support SCSI transparent (0x06) + Bulk-Only (0x50) for now
    if iface.subclass != 0x06 || iface.protocol != 0x50 {
        crate::serial_println!(
            "  USB Storage: unsupported subclass/protocol combination"
        );
        return;
    }

    // Allocate DMA memory for CBW/CSW (1 page, identity-mapped)
    let cbw_csw_phys = match physical::alloc_frame() {
        Some(f) => f.as_u64(),
        None => {
            crate::serial_println!("  USB Storage: failed to allocate CBW/CSW page");
            return;
        }
    };
    // Zero the page
    unsafe { core::ptr::write_bytes(cbw_csw_phys as *mut u8, 0, 4096); }

    // Allocate bounce buffer (16 contiguous pages = 64 KiB)
    let bounce_phys = match physical::alloc_contiguous(BOUNCE_PAGES) {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_println!("  USB Storage: failed to allocate bounce buffer");
            return;
        }
    };
    unsafe { core::ptr::write_bytes(bounce_phys as *mut u8, 0, BOUNCE_SIZE); }

    let mut stor_dev = UsbStorageDevice {
        usb_addr: dev.address,
        controller: dev.controller,
        speed: dev.speed,
        port: dev.port,
        ep_in: ep_in.address,
        ep_out: ep_out.address,
        max_packet_in: ep_in.max_packet_size,
        max_packet_out: ep_out.max_packet_size,
        toggle_in: 0,
        toggle_out: 0,
        tag: 1,
        block_count: 0,
        block_size: 512,
        cbw_csw_phys,
        bounce_phys,
        disk_id: 0,
    };

    // TEST_UNIT_READY with retry (device may be spinning up)
    let mut ready = false;
    for attempt in 0..10 {
        if scsi_test_unit_ready(&mut stor_dev).is_ok() {
            ready = true;
            break;
        }
        crate::serial_println!("  USB Storage: TEST_UNIT_READY attempt {} failed", attempt + 1);
        crate::arch::x86::pit::delay_ms(200);
    }
    if !ready {
        crate::serial_println!("  USB Storage: device not ready after retries");
        return;
    }

    // INQUIRY
    match scsi_inquiry(&mut stor_dev) {
        Ok(inquiry) => {
            let vendor = core::str::from_utf8(&inquiry[8..16]).unwrap_or("?").trim();
            let product = core::str::from_utf8(&inquiry[16..32]).unwrap_or("?").trim();
            crate::serial_println!("  USB Storage: {} {}", vendor, product);
        }
        Err(e) => {
            crate::serial_println!("  USB Storage: INQUIRY failed: {}", e);
        }
    }

    // READ_CAPACITY
    match scsi_read_capacity(&mut stor_dev) {
        Ok((blocks, block_size)) => {
            stor_dev.block_count = blocks;
            stor_dev.block_size = block_size;
            let size_mib = blocks as u64 * block_size as u64 / (1024 * 1024);
            crate::serial_println!(
                "  USB Storage: {} sectors x {} bytes = {} MiB",
                blocks, block_size, size_mib
            );
        }
        Err(e) => {
            crate::serial_println!("  USB Storage: READ_CAPACITY failed: {}", e);
            return;
        }
    }

    // Register as block device
    let disk_id = alloc_disk_id();
    stor_dev.disk_id = disk_id;

    use crate::drivers::storage::blockdev;
    blockdev::register_device(blockdev::BlockDevice {
        id: 0,
        disk_id,
        partition: None,
        start_lba: 0,
        size_sectors: stor_dev.block_count as u64,
    });

    // Register I/O override
    crate::drivers::storage::register_device_io(
        disk_id,
        usb_storage_read,
        usb_storage_write,
    );

    // Store device before partition scanning (read dispatch needs it)
    USB_STORAGE_DEVICES.lock().push(stor_dev);

    // Scan for partitions
    blockdev::scan_and_register_partitions(disk_id);

    crate::serial_println!("  USB Storage: registered as disk {} (hd{})", disk_id, disk_id);
}

/// Called when a USB device is disconnected. Cleans up storage state.
pub fn disconnect(port: u8, controller: ControllerType) {
    let mut devs = USB_STORAGE_DEVICES.lock();
    if let Some(idx) = devs.iter().position(|d| d.port == port && d.controller == controller) {
        let dev = devs.remove(idx);
        let disk_id = dev.disk_id;
        drop(devs);

        // Remove block devices
        use crate::drivers::storage::blockdev;
        blockdev::remove_partition_devices(disk_id);

        // Unregister I/O override
        crate::drivers::storage::unregister_device_io(disk_id);

        // TODO: free DMA memory (cbw_csw_phys, bounce_phys) back to physical allocator

        crate::serial_println!("  USB Storage: disk {} (hd{}) removed", disk_id, disk_id);
    }
}

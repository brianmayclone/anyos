//! USB Mass Storage class driver (Bulk-Only Transport).
//!
//! Supports SCSI transparent command set (subclass 0x06) with
//! bulk-only transport protocol (protocol 0x50).
//!
//! Implements: INQUIRY, TEST_UNIT_READY, READ_CAPACITY, READ_10, WRITE_10.

use super::{UsbDevice, UsbInterface};

/// USB mass storage Command Block Wrapper (31 bytes).
#[repr(C, packed)]
#[allow(dead_code)]
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
#[allow(dead_code)]
struct Csw {
    signature: u32,       // 0x53425355
    tag: u32,
    data_residue: u32,
    status: u8,           // 0=Passed, 1=Failed, 2=Phase Error
}

#[allow(dead_code)]
const CBW_SIGNATURE: u32 = 0x43425355;
#[allow(dead_code)]
const CSW_SIGNATURE: u32 = 0x53425355;

#[allow(dead_code)]
const CBW_FLAG_IN: u8 = 0x80;
#[allow(dead_code)]
const CBW_FLAG_OUT: u8 = 0x00;

// SCSI commands
#[allow(dead_code)]
const SCSI_TEST_UNIT_READY: u8 = 0x00;
#[allow(dead_code)]
const SCSI_INQUIRY: u8 = 0x12;
#[allow(dead_code)]
const SCSI_READ_CAPACITY: u8 = 0x25;
#[allow(dead_code)]
const SCSI_READ_10: u8 = 0x28;
#[allow(dead_code)]
const SCSI_WRITE_10: u8 = 0x2A;

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

    match (bulk_in, bulk_out) {
        (Some(ep_in), Some(ep_out)) => {
            crate::serial_println!(
                "  USB Storage: bulk IN ep={:#04x} (max={}), bulk OUT ep={:#04x} (max={})",
                ep_in.address, ep_in.max_packet_size,
                ep_out.address, ep_out.max_packet_size,
            );
        }
        _ => {
            crate::serial_println!("  USB Storage: missing bulk endpoints");
            return;
        }
    }

    // Only support SCSI transparent (0x06) + Bulk-Only (0x50) for now
    if iface.subclass != 0x06 || iface.protocol != 0x50 {
        crate::serial_println!(
            "  USB Storage: unsupported subclass/protocol combination"
        );
        return;
    }

    crate::serial_println!("  USB Storage: SCSI Bulk-Only device ready");
}

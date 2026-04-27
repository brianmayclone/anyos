//! Bluetooth USB HCI Transport.
//!
//! Bluetooth USB adapters use a well-defined interface class (E0:01:01):
//! - Endpoint 0 (Control): HCI commands (host → controller)
//! - Interrupt IN endpoint: HCI events (controller → host)
//! - Bulk OUT endpoint: ACL data (host → controller)
//! - Bulk IN endpoint: ACL data (controller → host)
//!
//! This module handles device detection, endpoint discovery, and raw
//! packet send/receive over USB.

use crate::drivers::usb::{ControllerType, SetupPacket, UsbDevice, UsbInterface, UsbSpeed};
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Whether a Bluetooth USB adapter has been detected and attached.
static ATTACHED: AtomicBool = AtomicBool::new(false);
static BT_ADDRESS: AtomicU8 = AtomicU8::new(0);
static BT_CONTROLLER: AtomicU8 = AtomicU8::new(0); // ControllerType as u8
static BT_INT_EP: AtomicU8 = AtomicU8::new(0);
static BT_BULK_IN_EP: AtomicU8 = AtomicU8::new(0);
static BT_BULK_OUT_EP: AtomicU8 = AtomicU8::new(0);
static BT_MAX_PACKET: AtomicU8 = AtomicU8::new(0);

/// Check if a BT adapter is attached.
pub fn is_attached() -> bool {
    ATTACHED.load(Ordering::Acquire)
}

/// Called during USB enumeration when a Bluetooth interface is detected.
/// USB class E0h, subclass 01h, protocol 01h = Bluetooth HCI.
pub fn probe(dev: &UsbDevice, iface: &UsbInterface) {
    // Verify interface class
    if iface.class != 0xE0 || iface.subclass != 0x01 || iface.protocol != 0x01 {
        return;
    }

    // Find endpoints: interrupt IN, bulk IN, bulk OUT
    let mut int_in: Option<u8> = None;
    let mut bulk_in: Option<u8> = None;
    let mut bulk_out: Option<u8> = None;

    for ep in &iface.endpoints {
        let is_in = ep.address & 0x80 != 0;
        let transfer_type = ep.attributes & 0x03;
        match (transfer_type, is_in) {
            (3, true) if int_in.is_none() => int_in = Some(ep.address), // Interrupt IN
            (2, true) if bulk_in.is_none() => bulk_in = Some(ep.address), // Bulk IN
            (2, false) if bulk_out.is_none() => bulk_out = Some(ep.address), // Bulk OUT
            _ => {}
        }
    }

    let int_ep = match int_in {
        Some(ep) => ep,
        None => {
            crate::serial_println!("  BT USB: no interrupt IN endpoint, skipping");
            return;
        }
    };

    BT_ADDRESS.store(dev.address, Ordering::Release);
    BT_CONTROLLER.store(dev.controller as u8, Ordering::Release);
    BT_INT_EP.store(int_ep, Ordering::Release);
    BT_BULK_IN_EP.store(bulk_in.unwrap_or(0), Ordering::Release);
    BT_BULK_OUT_EP.store(bulk_out.unwrap_or(0), Ordering::Release);
    BT_MAX_PACKET.store(dev.max_packet_size as u8, Ordering::Release);
    ATTACHED.store(true, Ordering::Release);

    crate::serial_println!(
        "[BT] USB adapter detected (addr={}, int_ep={:#04x}, bulk_in={:#04x}, bulk_out={:#04x})",
        dev.address,
        int_ep,
        bulk_in.unwrap_or(0),
        bulk_out.unwrap_or(0),
    );

    // Initialize the controller
    super::hci::reset();
}

/// Send an HCI command via USB control transfer (endpoint 0).
/// HCI commands use: bmRequestType=0x20 (class, host-to-device),
/// bRequest=0x00, wIndex=interface.
pub fn send_command(cmd: &[u8]) {
    if !ATTACHED.load(Ordering::Acquire) {
        return;
    }

    let address = BT_ADDRESS.load(Ordering::Acquire);
    let controller = BT_CONTROLLER.load(Ordering::Acquire);
    let max_packet = BT_MAX_PACKET.load(Ordering::Acquire);

    let setup = SetupPacket {
        bm_request_type: 0x20, // Class, Host-to-Device, Interface
        b_request: 0x00,       // HCI command
        w_value: 0,
        w_index: 0, // Interface 0
        w_length: cmd.len() as u16,
    };

    // Use the USB subsystem's control transfer with data phase
    let controller_type = match controller {
        0 => ControllerType::Uhci,
        1 => ControllerType::Ehci,
        2 => ControllerType::Xhci,
        _ => return,
    };

    let _ = crate::drivers::usb::hid_control_transfer(
        address,
        controller_type,
        UsbSpeed::Full,
        max_packet as u16,
        &setup,
        true,
        cmd.len() as u16,
    );
}

/// Send ACL data via USB bulk OUT endpoint.
pub fn send_acl(_data: &[u8]) {
    // TODO: implement bulk OUT transfer via USB subsystem
    // For now, this is a stub — L2CAP data sending will use this
}

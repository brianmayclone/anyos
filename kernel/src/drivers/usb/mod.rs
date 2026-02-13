//! USB subsystem — host controller drivers and class drivers.
//!
//! Supports UHCI (USB 1.x, I/O port based) and EHCI (USB 2.0, MMIO based).
//! Class drivers: HID (keyboard/mouse) and Mass Storage (SCSI bulk-only).

pub mod uhci;
pub mod ehci;
pub mod hid;
pub mod storage;

use crate::drivers::pci::PciDevice;
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

// ── USB Speed ────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UsbSpeed {
    Low,  // 1.5 Mbps (USB 1.0)
    Full, // 12 Mbps (USB 1.1)
    High, // 480 Mbps (USB 2.0)
}

// ── USB Controller Type ──────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControllerType {
    Uhci,
    Ehci,
}

// ── USB Device ───────────────────────────────────

#[derive(Debug, Clone)]
pub struct UsbDevice {
    pub address: u8,
    pub speed: UsbSpeed,
    pub port: u8,
    pub controller: ControllerType,
    pub max_packet_size: u16,
    pub vendor_id: u16,
    pub product_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub num_configs: u8,
    pub interfaces: Vec<UsbInterface>,
}

#[derive(Debug, Clone)]
pub struct UsbInterface {
    pub number: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub endpoints: Vec<UsbEndpoint>,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbEndpoint {
    pub address: u8,    // bit7 = direction (1=IN, 0=OUT), bits3-0 = endpoint number
    pub attributes: u8, // bits1-0 = transfer type (0=control, 1=iso, 2=bulk, 3=interrupt)
    pub max_packet_size: u16,
    pub interval: u8,
}

// ── Global Device List ───────────────────────────

static USB_DEVICES: Spinlock<Vec<UsbDevice>> = Spinlock::new(Vec::new());
static NEXT_ADDRESS: Spinlock<u8> = Spinlock::new(1);

pub fn alloc_address() -> u8 {
    let mut addr = NEXT_ADDRESS.lock();
    let a = *addr;
    if a < 127 { *addr = a + 1; }
    a
}

pub fn register_device(dev: UsbDevice) {
    crate::serial_println!(
        "  USB: device {} — {:04x}:{:04x} class={:02x}:{:02x} proto={:02x} ({}) [{}]",
        dev.address, dev.vendor_id, dev.product_id,
        dev.class, dev.subclass, dev.protocol,
        class_name(dev.class), speed_name(dev.speed),
    );
    for iface in &dev.interfaces {
        crate::serial_println!(
            "    Interface {}: class={:02x}:{:02x} proto={:02x} ({}) endpoints={}",
            iface.number, iface.class, iface.subclass, iface.protocol,
            class_name(iface.class), iface.endpoints.len(),
        );
    }

    // Dispatch to class drivers
    for iface in &dev.interfaces {
        match iface.class {
            0x03 => hid::probe(&dev, iface),
            0x08 => storage::probe(&dev, iface),
            _ => {}
        }
    }

    USB_DEVICES.lock().push(dev);
}

pub fn devices() -> Vec<UsbDevice> {
    USB_DEVICES.lock().clone()
}

/// Remove a device by port and controller type (for hot-unplug).
pub fn remove_device(port: u8, controller: ControllerType) {
    let mut devs = USB_DEVICES.lock();
    if let Some(pos) = devs.iter().position(|d| d.port == port && d.controller == controller) {
        let dev = devs.remove(pos);
        crate::serial_println!(
            "  USB: removed device {} (port {}, {:04x}:{:04x})",
            dev.address, port, dev.vendor_id, dev.product_id,
        );
    }
}

/// Poll all USB controllers for hot-plug events.
pub fn poll_all_controllers() {
    uhci::poll_ports();
    ehci::poll_ports();
}

fn class_name(class: u8) -> &'static str {
    match class {
        0x00 => "Composite",
        0x01 => "Audio",
        0x02 => "CDC",
        0x03 => "HID",
        0x05 => "Physical",
        0x06 => "Image",
        0x07 => "Printer",
        0x08 => "Mass Storage",
        0x09 => "Hub",
        0x0A => "CDC-Data",
        0x0E => "Video",
        0xFF => "Vendor-specific",
        _ => "Unknown",
    }
}

fn speed_name(speed: UsbSpeed) -> &'static str {
    match speed {
        UsbSpeed::Low => "Low-Speed",
        UsbSpeed::Full => "Full-Speed",
        UsbSpeed::High => "High-Speed",
    }
}

// ── USB Descriptor Structures ────────────────────

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DeviceDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_sub_class: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size0: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
    pub b_num_configurations: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ConfigDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub w_total_length: u16,
    pub b_num_interfaces: u8,
    pub b_configuration_value: u8,
    pub i_configuration: u8,
    pub bm_attributes: u8,
    pub b_max_power: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct InterfaceDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub b_interface_number: u8,
    pub b_alternate_setting: u8,
    pub b_num_endpoints: u8,
    pub b_interface_class: u8,
    pub b_interface_sub_class: u8,
    pub b_interface_protocol: u8,
    pub i_interface: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct EndpointDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub b_endpoint_address: u8,
    pub bm_attributes: u8,
    pub w_max_packet_size: u16,
    pub b_interval: u8,
}

// ── Setup Packet ─────────────────────────────────

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SetupPacket {
    pub bm_request_type: u8,
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
    pub w_length: u16,
}

// Standard request codes
pub const REQ_GET_DESCRIPTOR: u8 = 0x06;
pub const REQ_SET_ADDRESS: u8 = 0x05;
pub const REQ_SET_CONFIGURATION: u8 = 0x09;
pub const REQ_SET_PROTOCOL: u8 = 0x0B;

// Descriptor types
pub const DESC_DEVICE: u16 = 0x0100;
pub const DESC_CONFIG: u16 = 0x0200;

// Transfer direction
pub const DIR_HOST_TO_DEVICE: u8 = 0x00;
pub const DIR_DEVICE_TO_HOST: u8 = 0x80;

// PID tokens
pub const PID_SETUP: u8 = 0x2D;
pub const PID_IN: u8 = 0x69;
pub const PID_OUT: u8 = 0xE1;

// ── Configuration Descriptor Parser ──────────────

/// Parse a full configuration descriptor buffer into interfaces + endpoints.
pub fn parse_config(data: &[u8]) -> Vec<UsbInterface> {
    let mut interfaces = Vec::new();
    let mut offset = 0;

    while offset + 1 < data.len() {
        let len = data[offset] as usize;
        let desc_type = data[offset + 1];
        if len < 2 || offset + len > data.len() {
            break;
        }

        match desc_type {
            // Interface descriptor (type 4)
            4 if len >= 9 => {
                let iface = UsbInterface {
                    number: data[offset + 2],
                    class: data[offset + 5],
                    subclass: data[offset + 6],
                    protocol: data[offset + 7],
                    endpoints: Vec::new(),
                };
                interfaces.push(iface);
            }
            // Endpoint descriptor (type 5)
            5 if len >= 7 => {
                if let Some(iface) = interfaces.last_mut() {
                    let ep = UsbEndpoint {
                        address: data[offset + 2],
                        attributes: data[offset + 3],
                        max_packet_size: u16::from_le_bytes([data[offset + 4], data[offset + 5]]),
                        interval: data[offset + 6],
                    };
                    iface.endpoints.push(ep);
                }
            }
            _ => {}
        }

        offset += len;
    }

    interfaces
}

// ── Init entry point ─────────────────────────────

/// Initialize USB for a PCI USB controller, dispatching by prog_if.
pub fn init(pci: &PciDevice) {
    match pci.prog_if {
        0x00 => {
            crate::serial_println!("  USB: UHCI controller detected (prog_if=0x00)");
            uhci::init_controller(pci);
        }
        0x20 => {
            crate::serial_println!("  USB: EHCI controller detected (prog_if=0x20)");
            ehci::init_controller(pci);
        }
        0x10 => {
            crate::serial_println!("  USB: OHCI controller detected (prog_if=0x10) — not supported");
        }
        0x30 => {
            crate::serial_println!("  USB: xHCI controller detected (prog_if=0x30) — not supported");
        }
        _ => {
            crate::serial_println!("  USB: unknown controller type (prog_if={:#04x})", pci.prog_if);
        }
    }
}

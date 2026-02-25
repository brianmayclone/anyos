//! USB CDC-ECM (Ethernet Networking Control Model) driver.
//!
//! Supports USB Ethernet adapters using Communication Device Class with
//! ECM subclass (class 0x02, subclass 0x06). Registers as a `NetworkDriver`
//! so the network stack can transmit/receive Ethernet frames.

use super::{ControllerType, SetupPacket, UsbDevice, UsbInterface, UsbSpeed};
use crate::memory::physical;
use crate::sync::spinlock::Spinlock;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

// ── CDC-ECM Constants ──────────────────────────────

/// SET_ETHERNET_PACKET_FILTER — CDC class request.
const SET_ETHERNET_PACKET_FILTER: u8 = 0x43;

/// Packet filter bits (USB CDC spec Table 62).
const PACKET_TYPE_DIRECTED: u16 = 0x01;
const PACKET_TYPE_BROADCAST: u16 = 0x08;

/// CDC functional descriptor type marker.
const CS_INTERFACE: u8 = 0x24;
/// Ethernet Networking Functional Descriptor subtype.
const ECM_FUNC_DESC_SUBTYPE: u8 = 0x0F;

// ── Device State ─────────────────────────────────

const MAX_RX_QUEUE: usize = 64;

struct CdcEcmDevice {
    usb_addr: u8,
    controller: ControllerType,
    speed: UsbSpeed,
    max_packet: u16,
    port: u8,
    comm_iface: u8,
    ep_bulk_in: u8,
    ep_bulk_out: u8,
    max_packet_in: u16,
    max_packet_out: u16,
    toggle_in: u8,
    toggle_out: u8,
    rx_bounce_phys: u64,
    tx_bounce_phys: u64,
    mac: [u8; 6],
    rx_queue: VecDeque<Vec<u8>>,
}

static CDC_ECM_DEVICES: Spinlock<Vec<CdcEcmDevice>> = Spinlock::new(Vec::new());

// ── Config Descriptor Parsing ───────────────────

/// Parse the raw configuration descriptor to find the Ethernet Networking
/// Functional Descriptor (CS_INTERFACE / subtype 0x0F) and return the
/// `iMACAddress` string descriptor index.
fn find_mac_string_index(config_raw: &[u8]) -> Option<u8> {
    let mut offset = 0;
    while offset + 1 < config_raw.len() {
        let b_length = config_raw[offset] as usize;
        if b_length < 2 {
            break;
        }
        let b_descriptor_type = config_raw[offset + 1];

        // Ethernet Networking Functional Descriptor:
        //   bLength >= 13, bDescriptorType = 0x24, bDescriptorSubtype = 0x0F
        //   offset+3 = iMACAddress (string index)
        if b_descriptor_type == CS_INTERFACE
            && b_length >= 13
            && offset + 3 < config_raw.len()
            && config_raw[offset + 2] == ECM_FUNC_DESC_SUBTYPE
        {
            return Some(config_raw[offset + 3]);
        }

        offset += b_length;
    }
    None
}

/// Parse a 12-character hex MAC string ("AABBCCDDEEFF") into [u8; 6].
fn parse_mac_string(s: &str) -> Option<[u8; 6]> {
    if s.len() != 12 {
        return None;
    }
    let mut mac = [0u8; 6];
    let bytes = s.as_bytes();
    for i in 0..6 {
        let hi = hex_nibble(bytes[i * 2])?;
        let lo = hex_nibble(bytes[i * 2 + 1])?;
        mac[i] = (hi << 4) | lo;
    }
    Some(mac)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ── Probe ────────────────────────────────────────

/// Called when a CDC Communication Interface (class=0x02, subclass=0x06) is detected.
pub fn probe(dev: &UsbDevice, iface: &UsbInterface) {
    crate::serial_println!(
        "  CDC-ECM: probing (addr={}, iface={})",
        dev.address, iface.number
    );

    // 1. Parse config_raw for the Ethernet Networking Functional Descriptor
    let mac_str_idx = match find_mac_string_index(&dev.config_raw) {
        Some(idx) if idx != 0 => idx,
        _ => {
            crate::serial_println!("  CDC-ECM: no Ethernet Functional Descriptor found");
            return;
        }
    };

    // 2. Read MAC address string descriptor
    let mac = match super::get_string_descriptor(
        dev.address, dev.controller, dev.speed, dev.max_packet_size, mac_str_idx,
    ) {
        Ok(s) => {
            match parse_mac_string(&s) {
                Some(m) => {
                    crate::serial_println!(
                        "  CDC-ECM: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                        m[0], m[1], m[2], m[3], m[4], m[5]
                    );
                    m
                }
                None => {
                    crate::serial_println!("  CDC-ECM: bad MAC string '{}'", s);
                    return;
                }
            }
        }
        Err(e) => {
            crate::serial_println!("  CDC-ECM: failed to read MAC string: {}", e);
            return;
        }
    };

    // 3. Find the companion Data Interface (class 0x0A)
    let data_iface = match dev.interfaces.iter().find(|i| i.class == 0x0A) {
        Some(di) => di,
        None => {
            crate::serial_println!("  CDC-ECM: no Data Interface (class 0x0A) found");
            return;
        }
    };

    // 4. Find bulk IN and bulk OUT endpoints on the Data Interface
    let bulk_in = data_iface.endpoints.iter().find(|ep| {
        (ep.attributes & 0x03) == 2 && (ep.address & 0x80) != 0
    });
    let bulk_out = data_iface.endpoints.iter().find(|ep| {
        (ep.attributes & 0x03) == 2 && (ep.address & 0x80) == 0
    });

    let (ep_in, ep_out) = match (bulk_in, bulk_out) {
        (Some(i), Some(o)) => {
            crate::serial_println!(
                "  CDC-ECM: bulk IN ep={:#04x} (max={}), bulk OUT ep={:#04x} (max={})",
                i.address, i.max_packet_size, o.address, o.max_packet_size
            );
            (i, o)
        }
        _ => {
            crate::serial_println!("  CDC-ECM: missing bulk endpoints on Data Interface");
            return;
        }
    };

    // 5. Allocate two DMA bounce pages (RX and TX)
    let rx_bounce_phys = match physical::alloc_frame() {
        Some(f) => f.as_u64(),
        None => {
            crate::serial_println!("  CDC-ECM: failed to allocate RX DMA page");
            return;
        }
    };
    let tx_bounce_phys = match physical::alloc_frame() {
        Some(f) => f.as_u64(),
        None => {
            crate::serial_println!("  CDC-ECM: failed to allocate TX DMA page");
            return;
        }
    };
    unsafe {
        core::ptr::write_bytes(rx_bounce_phys as *mut u8, 0, 4096);
        core::ptr::write_bytes(tx_bounce_phys as *mut u8, 0, 4096);
    }

    // 6. SET_ETHERNET_PACKET_FILTER — enable directed + broadcast
    let setup = SetupPacket {
        bm_request_type: 0x21, // Host-to-device, Class, Interface
        b_request: SET_ETHERNET_PACKET_FILTER,
        w_value: PACKET_TYPE_DIRECTED | PACKET_TYPE_BROADCAST,
        w_index: iface.number as u16,
        w_length: 0,
    };
    let _ = super::hid_control_transfer(
        dev.address, dev.controller, dev.speed, dev.max_packet_size,
        &setup, false, 0,
    );

    let ecm_dev = CdcEcmDevice {
        usb_addr: dev.address,
        controller: dev.controller,
        speed: dev.speed,
        max_packet: dev.max_packet_size,
        port: dev.port,
        comm_iface: iface.number,
        ep_bulk_in: ep_in.address,
        ep_bulk_out: ep_out.address,
        max_packet_in: ep_in.max_packet_size,
        max_packet_out: ep_out.max_packet_size,
        toggle_in: 0,
        toggle_out: 0,
        rx_bounce_phys,
        tx_bounce_phys,
        mac,
        rx_queue: VecDeque::new(),
    };

    crate::serial_println!(
        "  CDC-ECM: registered USB Ethernet (addr={})",
        dev.address
    );

    CDC_ECM_DEVICES.lock().push(ecm_dev);

    // 7. Register as NetworkDriver (if no other driver is registered)
    if !crate::drivers::network::is_available() {
        crate::drivers::network::register(
            alloc::boxed::Box::new(CdcEcmNetworkDriver),
        );
    } else {
        crate::serial_println!("  CDC-ECM: NetworkDriver already registered, skipping");
    }
}

// ── Transmit / Receive ──────────────────────────

/// Transmit an Ethernet frame via the first CDC-ECM device.
fn ecm_transmit(data: &[u8]) -> bool {
    let mut devs = CDC_ECM_DEVICES.lock();
    let dev = match devs.first_mut() {
        Some(d) => d,
        None => return false,
    };

    let to_send = data.len().min(4096);
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), dev.tx_bounce_phys as *mut u8, to_send);
    }
    super::bulk_transfer(
        dev.usb_addr, dev.controller, dev.speed,
        dev.ep_bulk_out, dev.max_packet_out,
        &mut dev.toggle_out,
        dev.tx_bounce_phys, to_send,
    ).is_ok()
}

/// Receive the next queued Ethernet frame (called from net::poll integration).
pub fn recv_packet() -> Option<Vec<u8>> {
    let mut devs = CDC_ECM_DEVICES.lock();
    for dev in devs.iter_mut() {
        if let Some(pkt) = dev.rx_queue.pop_front() {
            return Some(pkt);
        }
    }
    None
}

/// Get the MAC address of the first CDC-ECM device.
fn ecm_get_mac() -> [u8; 6] {
    let devs = CDC_ECM_DEVICES.lock();
    devs.first().map(|d| d.mac).unwrap_or([0; 6])
}

// ── Polling ──────────────────────────────────────

/// Poll all CDC-ECM devices for incoming Ethernet frames on bulk IN.
pub fn poll_all() {
    let mut devs = CDC_ECM_DEVICES.lock();
    for dev in devs.iter_mut() {
        // Try to receive up to one frame per poll cycle
        match super::bulk_transfer(
            dev.usb_addr, dev.controller, dev.speed,
            dev.ep_bulk_in, dev.max_packet_in,
            &mut dev.toggle_in,
            dev.rx_bounce_phys, 4096,
        ) {
            Ok(n) if n > 0 => {
                let mut frame = alloc::vec![0u8; n];
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        dev.rx_bounce_phys as *const u8,
                        frame.as_mut_ptr(),
                        n,
                    );
                }
                if dev.rx_queue.len() < MAX_RX_QUEUE {
                    dev.rx_queue.push_back(frame);
                }
            }
            _ => {}
        }
    }
}

/// Disconnect handler for hot-unplug.
pub fn disconnect(port: u8, controller: ControllerType) {
    let mut devs = CDC_ECM_DEVICES.lock();
    if let Some(idx) = devs.iter().position(|d| d.port == port && d.controller == controller) {
        let dev = devs.remove(idx);
        crate::serial_println!(
            "  CDC-ECM: USB Ethernet removed (addr={})",
            dev.usb_addr
        );
    }
}

// ── NetworkDriver Trait ─────────────────────────

use crate::drivers::network::NetworkDriver;

struct CdcEcmNetworkDriver;

impl NetworkDriver for CdcEcmNetworkDriver {
    fn name(&self) -> &str { "USB CDC-ECM Ethernet" }

    fn transmit(&mut self, data: &[u8]) -> bool {
        ecm_transmit(data)
    }

    fn get_mac(&self) -> [u8; 6] {
        ecm_get_mac()
    }

    fn link_up(&self) -> bool {
        !CDC_ECM_DEVICES.lock().is_empty()
    }
}

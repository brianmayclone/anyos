//! USB Hub class driver (class 0x09).
//!
//! Enumerates downstream ports, powers them, resets connected devices,
//! and runs the standard USB enumeration sequence for each.

use super::{
    ControllerType, SetupPacket, UsbDevice, UsbInterface, UsbSpeed,
    hid_control_transfer, alloc_address, register_device, parse_config,
    REQ_GET_DESCRIPTOR, REQ_SET_ADDRESS, REQ_SET_CONFIGURATION,
    DESC_DEVICE, DESC_CONFIG, DIR_DEVICE_TO_HOST, DIR_HOST_TO_DEVICE,
};
use crate::memory::physical;
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

// ── Hub Constants ────────────────────────────────

// bmRequestType values
const RT_HUB_CLASS_IN: u8 = 0xA0;   // Device-to-host, Class, Device
const RT_PORT_CLASS_IN: u8 = 0xA3;   // Device-to-host, Class, Other (port)
const RT_PORT_CLASS_OUT: u8 = 0x23;  // Host-to-device, Class, Other (port)

// Hub class requests
const HUB_REQ_GET_STATUS: u8 = 0x00;
const HUB_REQ_CLEAR_FEATURE: u8 = 0x01;
const HUB_REQ_SET_FEATURE: u8 = 0x03;
const HUB_REQ_GET_DESCRIPTOR: u8 = 0x06;

// Port features
const PORT_RESET: u16 = 4;
const PORT_POWER: u16 = 8;
const C_PORT_CONNECTION: u16 = 16;
const C_PORT_ENABLE: u16 = 17;
const C_PORT_RESET: u16 = 20;

// Port status bits (wPortStatus)
const PORT_STATUS_CONNECTION: u16 = 1 << 0;
const PORT_STATUS_ENABLE: u16 = 1 << 1;
const PORT_STATUS_RESET: u16 = 1 << 4;
const PORT_STATUS_POWER: u16 = 1 << 8;
const PORT_STATUS_LOW_SPEED: u16 = 1 << 9;
const PORT_STATUS_HIGH_SPEED: u16 = 1 << 10;

// Port change bits (wPortChange)
const PORT_CHANGE_CONNECTION: u16 = 1 << 0;

// ── Hub State ────────────────────────────────────

struct HubPort {
    connected: bool,
}

struct HubDevice {
    usb_addr: u8,
    controller: ControllerType,
    speed: UsbSpeed,
    max_packet: u16,
    port: u8,
    num_ports: u8,
    ports: Vec<HubPort>,
    data_phys: u64,
}

static HUB_DEVICES: Spinlock<Vec<HubDevice>> = Spinlock::new(Vec::new());

// ── Hub Control Requests ─────────────────────────

fn get_hub_descriptor(hub: &HubDevice) -> Result<(u8, u8), &'static str> {
    let setup = SetupPacket {
        bm_request_type: RT_HUB_CLASS_IN,
        b_request: HUB_REQ_GET_DESCRIPTOR,
        w_value: 0x2900, // Hub descriptor type
        w_index: 0,
        w_length: 8,
    };
    let data = hid_control_transfer(
        hub.usb_addr, hub.controller, hub.speed, hub.max_packet,
        &setup, true, 8,
    )?;
    if data.len() < 3 {
        return Err("hub descriptor too short");
    }
    let num_ports = data[2];
    let pwr_on_delay = if data.len() >= 6 { data[5] } else { 50 };
    Ok((num_ports, pwr_on_delay))
}

fn get_port_status(hub: &HubDevice, port: u8) -> Result<(u16, u16), &'static str> {
    let setup = SetupPacket {
        bm_request_type: RT_PORT_CLASS_IN,
        b_request: HUB_REQ_GET_STATUS,
        w_value: 0,
        w_index: port as u16,
        w_length: 4,
    };
    let data = hid_control_transfer(
        hub.usb_addr, hub.controller, hub.speed, hub.max_packet,
        &setup, true, 4,
    )?;
    if data.len() < 4 {
        return Err("port status too short");
    }
    let status = u16::from_le_bytes([data[0], data[1]]);
    let change = u16::from_le_bytes([data[2], data[3]]);
    Ok((status, change))
}

fn set_port_feature(hub: &HubDevice, port: u8, feature: u16) -> Result<(), &'static str> {
    let setup = SetupPacket {
        bm_request_type: RT_PORT_CLASS_OUT,
        b_request: HUB_REQ_SET_FEATURE,
        w_value: feature,
        w_index: port as u16,
        w_length: 0,
    };
    hid_control_transfer(
        hub.usb_addr, hub.controller, hub.speed, hub.max_packet,
        &setup, false, 0,
    )?;
    Ok(())
}

fn clear_port_feature(hub: &HubDevice, port: u8, feature: u16) -> Result<(), &'static str> {
    let setup = SetupPacket {
        bm_request_type: RT_PORT_CLASS_OUT,
        b_request: HUB_REQ_CLEAR_FEATURE,
        w_value: feature,
        w_index: port as u16,
        w_length: 0,
    };
    hid_control_transfer(
        hub.usb_addr, hub.controller, hub.speed, hub.max_packet,
        &setup, false, 0,
    )?;
    Ok(())
}

/// Reset a hub port and return the speed of the attached device.
fn reset_hub_port(hub: &HubDevice, port: u8) -> Result<UsbSpeed, &'static str> {
    set_port_feature(hub, port, PORT_RESET)?;

    // Poll until reset completes (PORT_STATUS_RESET clears)
    for _ in 0..20 {
        crate::arch::x86::pit::delay_ms(10);
        let (status, _) = get_port_status(hub, port)?;
        if status & PORT_STATUS_RESET == 0 {
            // Clear reset change
            let _ = clear_port_feature(hub, port, C_PORT_RESET);

            // Determine speed
            let speed = if status & PORT_STATUS_HIGH_SPEED != 0 {
                UsbSpeed::High
            } else if status & PORT_STATUS_LOW_SPEED != 0 {
                UsbSpeed::Low
            } else {
                UsbSpeed::Full
            };
            return Ok(speed);
        }
    }
    Err("hub port reset timeout")
}

// ── Downstream Device Enumeration ────────────────

/// Enumerate a device behind a hub port using the public control transfer API.
/// This mirrors the UHCI/EHCI enumerate_device() flow but is controller-agnostic.
fn enumerate_downstream(hub: &HubDevice, hub_port: u8, speed: UsbSpeed) {
    let controller = hub.controller;

    // Step 1: GET_DESCRIPTOR to address 0 (first 8 bytes for max packet size)
    let setup_dev8 = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_DEVICE,
        w_index: 0,
        w_length: 8,
    };
    let data8 = match hid_control_transfer(0, controller, speed, 8, &setup_dev8, true, 8) {
        Ok(d) if d.len() >= 8 => d,
        _ => {
            crate::serial_println!("  Hub: downstream port {} — GET_DESCRIPTOR(8) failed", hub_port);
            return;
        }
    };
    let max_packet = data8[7] as u16;
    if max_packet == 0 {
        crate::serial_println!("  Hub: downstream port {} — max_packet=0", hub_port);
        return;
    }

    // Step 2: SET_ADDRESS
    let new_addr = alloc_address();
    let setup_addr = SetupPacket {
        bm_request_type: DIR_HOST_TO_DEVICE,
        b_request: REQ_SET_ADDRESS,
        w_value: new_addr as u16,
        w_index: 0,
        w_length: 0,
    };
    if hid_control_transfer(0, controller, speed, max_packet, &setup_addr, false, 0).is_err() {
        crate::serial_println!("  Hub: downstream port {} — SET_ADDRESS failed", hub_port);
        return;
    }
    crate::arch::x86::pit::delay_ms(5);

    // Step 3: GET_DESCRIPTOR full (18 bytes)
    let setup_dev18 = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_DEVICE,
        w_index: 0,
        w_length: 18,
    };
    let dev_data = match hid_control_transfer(new_addr, controller, speed, max_packet, &setup_dev18, true, 18) {
        Ok(d) if d.len() >= 18 => d,
        _ => {
            crate::serial_println!("  Hub: device {} — full device descriptor failed", new_addr);
            return;
        }
    };

    let vendor_id = u16::from_le_bytes([dev_data[8], dev_data[9]]);
    let product_id = u16::from_le_bytes([dev_data[10], dev_data[11]]);
    let b_device_class = dev_data[4];
    let b_device_sub_class = dev_data[5];
    let b_device_protocol = dev_data[6];
    let b_num_configurations = dev_data[17];

    // Step 4: GET_DESCRIPTOR config header (9 bytes)
    let setup_cfg9 = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_CONFIG,
        w_index: 0,
        w_length: 9,
    };
    let cfg_header = match hid_control_transfer(new_addr, controller, speed, max_packet, &setup_cfg9, true, 9) {
        Ok(d) if d.len() >= 4 => d,
        _ => {
            crate::serial_println!("  Hub: device {} — config header failed", new_addr);
            return;
        }
    };
    let total_len = u16::from_le_bytes([cfg_header[2], cfg_header[3]]).min(256);

    // Step 5: GET_DESCRIPTOR full config
    let setup_cfg_full = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_CONFIG,
        w_index: 0,
        w_length: total_len,
    };
    let config_data = match hid_control_transfer(new_addr, controller, speed, max_packet, &setup_cfg_full, true, total_len) {
        Ok(d) if d.len() >= 9 => d,
        _ => {
            crate::serial_println!("  Hub: device {} — full config failed", new_addr);
            return;
        }
    };

    let interfaces = parse_config(&config_data[..total_len as usize]);

    // Step 6: SET_CONFIGURATION
    let config_val = if config_data.len() > 5 { config_data[5] } else { 1 };
    let setup_setcfg = SetupPacket {
        bm_request_type: DIR_HOST_TO_DEVICE,
        b_request: REQ_SET_CONFIGURATION,
        w_value: config_val as u16,
        w_index: 0,
        w_length: 0,
    };
    if hid_control_transfer(new_addr, controller, speed, max_packet, &setup_setcfg, false, 0).is_err() {
        crate::serial_println!("  Hub: device {} — SET_CONFIGURATION failed", new_addr);
        return;
    }

    // Determine class
    let dev_class = if b_device_class != 0 { b_device_class }
        else { interfaces.first().map(|i| i.class).unwrap_or(0) };
    let dev_subclass = if b_device_sub_class != 0 { b_device_sub_class }
        else { interfaces.first().map(|i| i.subclass).unwrap_or(0) };
    let dev_protocol = if b_device_protocol != 0 { b_device_protocol }
        else { interfaces.first().map(|i| i.protocol).unwrap_or(0) };

    // Encode port as (hub_addr << 4) | hub_port to distinguish from root ports
    let encoded_port = (hub.usb_addr << 4) | (hub_port & 0x0F);

    let usb_dev = UsbDevice {
        address: new_addr,
        speed,
        port: encoded_port,
        controller,
        max_packet_size: max_packet,
        vendor_id,
        product_id,
        class: dev_class,
        subclass: dev_subclass,
        protocol: dev_protocol,
        num_configs: b_num_configurations,
        interfaces,
        config_raw: config_data[..total_len as usize].to_vec(),
    };

    crate::serial_println!(
        "  Hub: enumerated device {} on hub {} port {} ({:04x}:{:04x})",
        new_addr, hub.usb_addr, hub_port, vendor_id, product_id
    );

    register_device(usb_dev);
}

// ── Probe ────────────────────────────────────────

/// Called when a Hub interface (class 0x09) is detected.
pub fn probe(dev: &UsbDevice, _iface: &UsbInterface) {
    crate::serial_println!("  Hub: probing hub at address {}", dev.address);

    // Allocate DMA buffer
    let data_phys = match physical::alloc_frame() {
        Some(f) => f.as_u64(),
        None => {
            crate::serial_println!("  Hub: failed to allocate DMA page");
            return;
        }
    };
    unsafe { core::ptr::write_bytes(data_phys as *mut u8, 0, 4096); }

    let mut hub = HubDevice {
        usb_addr: dev.address,
        controller: dev.controller,
        speed: dev.speed,
        max_packet: dev.max_packet_size,
        port: dev.port,
        num_ports: 0,
        ports: Vec::new(),
        data_phys,
    };

    // Get hub descriptor
    let (num_ports, pwr_delay) = match get_hub_descriptor(&hub) {
        Ok(v) => v,
        Err(e) => {
            crate::serial_println!("  Hub: GET_HUB_DESCRIPTOR failed: {}", e);
            return;
        }
    };
    hub.num_ports = num_ports;
    crate::serial_println!("  Hub: {} ports, power-on delay {}ms", num_ports, pwr_delay as u32 * 2);

    // Initialize port state
    for _ in 0..num_ports {
        hub.ports.push(HubPort { connected: false });
    }

    // Power all ports
    for p in 1..=num_ports {
        let _ = set_port_feature(&hub, p, PORT_POWER);
    }
    crate::arch::x86::pit::delay_ms(pwr_delay as u32 * 2 + 10);

    // Check each port for connected devices
    for p in 1..=num_ports {
        if let Ok((status, _change)) = get_port_status(&hub, p) {
            // Clear any pending connection change
            let _ = clear_port_feature(&hub, p, C_PORT_CONNECTION);

            if status & PORT_STATUS_CONNECTION != 0 {
                crate::serial_println!("  Hub: port {} — device connected", p);
                hub.ports[(p - 1) as usize].connected = true;

                match reset_hub_port(&hub, p) {
                    Ok(speed) => {
                        crate::arch::x86::pit::delay_ms(10);
                        enumerate_downstream(&hub, p, speed);
                    }
                    Err(e) => {
                        crate::serial_println!("  Hub: port {} reset failed: {}", p, e);
                    }
                }
            }
        }
    }

    HUB_DEVICES.lock().push(hub);
}

// ── Polling ──────────────────────────────────────

/// Poll all hubs for port status changes. Called from USB poll thread.
pub fn poll_all() {
    let mut hubs = HUB_DEVICES.lock();
    for hub in hubs.iter_mut() {
        for p in 1..=hub.num_ports {
            let (status, change) = match get_port_status(hub, p) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if change & PORT_CHANGE_CONNECTION == 0 {
                continue;
            }

            // Acknowledge change
            let _ = clear_port_feature(hub, p, C_PORT_CONNECTION);

            let idx = (p - 1) as usize;
            let connected = status & PORT_STATUS_CONNECTION != 0;
            let was_connected = hub.ports[idx].connected;

            if connected && !was_connected {
                crate::serial_println!("  Hub: hot-plug on hub {} port {}", hub.usb_addr, p);
                hub.ports[idx].connected = true;

                match reset_hub_port(hub, p) {
                    Ok(speed) => {
                        crate::arch::x86::pit::delay_ms(10);
                        enumerate_downstream(hub, p, speed);
                    }
                    Err(e) => {
                        crate::serial_println!("  Hub: port {} reset failed: {}", p, e);
                    }
                }
            } else if !connected && was_connected {
                crate::serial_println!("  Hub: hot-unplug on hub {} port {}", hub.usb_addr, p);
                hub.ports[idx].connected = false;

                let encoded_port = (hub.usb_addr << 4) | (p & 0x0F);
                super::storage::disconnect(encoded_port, hub.controller);
                super::cdc_acm::disconnect(encoded_port, hub.controller);
                super::cdc_ecm::disconnect(encoded_port, hub.controller);
                super::remove_device(encoded_port, hub.controller);
            }
        }
    }
}

/// Disconnect handler — remove a hub and cascade disconnect downstream devices.
pub fn disconnect(port: u8, controller: ControllerType) {
    let mut hubs = HUB_DEVICES.lock();
    if let Some(idx) = hubs.iter().position(|h| h.port == port && h.controller == controller) {
        let hub = hubs.remove(idx);
        crate::serial_println!("  Hub: hub {} removed (port {})", hub.usb_addr, port);

        // Cascade: disconnect all downstream devices
        for p in 1..=hub.num_ports {
            if hub.ports[(p - 1) as usize].connected {
                let encoded_port = (hub.usb_addr << 4) | (p & 0x0F);
                super::storage::disconnect(encoded_port, hub.controller);
                super::cdc_acm::disconnect(encoded_port, hub.controller);
                super::cdc_ecm::disconnect(encoded_port, hub.controller);
                super::remove_device(encoded_port, hub.controller);
            }
        }
    }
}

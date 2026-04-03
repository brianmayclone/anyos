//! Bluetooth HCI (Host Controller Interface) command/event processing.
//!
//! Implements the HCI command set needed for device discovery and connection
//! management. Commands are sent via USB bulk/control endpoints; events arrive
//! via USB interrupt endpoint.

use super::{BdAddr, BtDevice};

// ── HCI Packet Types ────────────────────────────────────────────────────────

/// HCI packet indicator (prepended to USB data).
pub const HCI_COMMAND: u8 = 0x01;
pub const HCI_ACL_DATA: u8 = 0x02;
pub const HCI_EVENT: u8 = 0x04;

// ── HCI Command OpCodes (OGF << 10 | OCF) ──────────────────────────────────

// Link Control (OGF = 0x01)
pub const HCI_INQUIRY: u16 = 0x0401;
pub const HCI_INQUIRY_CANCEL: u16 = 0x0402;
pub const HCI_CREATE_CONNECTION: u16 = 0x0405;
pub const HCI_DISCONNECT: u16 = 0x0406;

// Link Policy (OGF = 0x02)
pub const HCI_WRITE_DEFAULT_LINK_POLICY: u16 = 0x080F;

// Controller & Baseband (OGF = 0x03)
pub const HCI_RESET: u16 = 0x0C03;
pub const HCI_WRITE_LOCAL_NAME: u16 = 0x0C13;
pub const HCI_WRITE_SCAN_ENABLE: u16 = 0x0C1A;
pub const HCI_WRITE_CLASS_OF_DEVICE: u16 = 0x0C24;
pub const HCI_WRITE_SIMPLE_PAIRING_MODE: u16 = 0x0C56;

// Informational (OGF = 0x04)
pub const HCI_READ_BD_ADDR: u16 = 0x1009;
pub const HCI_READ_LOCAL_VERSION: u16 = 0x1001;

// ── HCI Event Codes ─────────────────────────────────────────────────────────

pub const EVT_INQUIRY_COMPLETE: u8 = 0x01;
pub const EVT_INQUIRY_RESULT: u8 = 0x02;
pub const EVT_CONNECTION_COMPLETE: u8 = 0x03;
pub const EVT_DISCONNECTION_COMPLETE: u8 = 0x05;
pub const EVT_COMMAND_COMPLETE: u8 = 0x0E;
pub const EVT_COMMAND_STATUS: u8 = 0x0F;
pub const EVT_INQUIRY_RESULT_WITH_RSSI: u8 = 0x22;
pub const EVT_EXTENDED_INQUIRY_RESULT: u8 = 0x2F;

// ── HCI Command Building ────────────────────────────────────────────────────

/// Build an HCI command packet. Returns the raw bytes to send.
fn build_command(opcode: u16, params: &[u8]) -> [u8; 259] {
    let mut buf = [0u8; 259]; // Max HCI command = 3 header + 255 params
    buf[0] = (opcode & 0xFF) as u8;
    buf[1] = (opcode >> 8) as u8;
    buf[2] = params.len() as u8;
    buf[3..3 + params.len()].copy_from_slice(params);
    buf
}

fn command_len(params_len: usize) -> usize {
    3 + params_len
}

/// Send HCI Reset command.
pub fn reset() {
    let cmd = build_command(HCI_RESET, &[]);
    super::usb_transport::send_command(&cmd[..command_len(0)]);
    crate::serial_println!("  BT HCI: Reset sent");
}

/// Read the controller's BD_ADDR.
pub fn read_bd_addr() {
    let cmd = build_command(HCI_READ_BD_ADDR, &[]);
    super::usb_transport::send_command(&cmd[..command_len(0)]);
}

/// Start inquiry (device discovery).
/// LAP = GIAC (General Inquiry Access Code), length = 8 (10.24s), max responses = 0 (unlimited).
pub fn inquiry_start() {
    // LAP: 0x9E8B33 (GIAC), Inquiry Length: 8 (10.24s), Num Responses: 0
    let params = [0x33, 0x8B, 0x9E, 0x08, 0x00];
    let cmd = build_command(HCI_INQUIRY, &params);
    super::usb_transport::send_command(&cmd[..command_len(params.len())]);
    crate::serial_println!("  BT HCI: Inquiry started (10s scan)");
}

/// Cancel an ongoing inquiry.
pub fn inquiry_cancel() {
    let cmd = build_command(HCI_INQUIRY_CANCEL, &[]);
    super::usb_transport::send_command(&cmd[..command_len(0)]);
}

/// Set local device name.
pub fn write_local_name(name: &str) {
    let mut params = [0u8; 248];
    let len = name.len().min(247);
    params[..len].copy_from_slice(&name.as_bytes()[..len]);
    let cmd = build_command(HCI_WRITE_LOCAL_NAME, &params[..248]);
    super::usb_transport::send_command(&cmd[..command_len(248)]);
}

/// Enable scan (discoverable + connectable).
pub fn write_scan_enable(discoverable: bool, connectable: bool) {
    let val = if discoverable { 0x02 } else { 0 } | if connectable { 0x01 } else { 0 };
    let cmd = build_command(HCI_WRITE_SCAN_ENABLE, &[val]);
    super::usb_transport::send_command(&cmd[..command_len(1)]);
}

// ── HCI Event Processing ────────────────────────────────────────────────────

/// Process an HCI event packet received from the controller.
pub fn process_event(data: &[u8]) {
    if data.len() < 2 { return; }

    let event_code = data[0];
    let param_len = data[1] as usize;
    let params = if data.len() >= 2 + param_len { &data[2..2 + param_len] } else { &data[2..] };

    match event_code {
        EVT_COMMAND_COMPLETE => {
            if params.len() >= 3 {
                let opcode = u16::from_le_bytes([params[1], params[2]]);
                let status = if params.len() > 3 { params[3] } else { 0xFF };
                handle_command_complete(opcode, status, &params[3..]);
            }
        }
        EVT_COMMAND_STATUS => {
            if params.len() >= 4 {
                let status = params[0];
                let opcode = u16::from_le_bytes([params[2], params[3]]);
                if status != 0 {
                    crate::serial_println!("  BT HCI: command {:#06x} status={:#04x}", opcode, status);
                }
            }
        }
        EVT_INQUIRY_RESULT | EVT_INQUIRY_RESULT_WITH_RSSI => {
            handle_inquiry_result(params, event_code == EVT_INQUIRY_RESULT_WITH_RSSI);
        }
        EVT_EXTENDED_INQUIRY_RESULT => {
            handle_extended_inquiry_result(params);
        }
        EVT_INQUIRY_COMPLETE => {
            crate::serial_println!("  BT HCI: Inquiry complete");
        }
        EVT_CONNECTION_COMPLETE => {
            if params.len() >= 11 {
                let status = params[0];
                let handle = u16::from_le_bytes([params[1], params[2]]);
                let mut addr = BdAddr::default();
                addr.0.copy_from_slice(&params[3..9]);
                crate::serial_println!("  BT HCI: Connection {} status={} handle={}",
                    addr, status, handle);
            }
        }
        EVT_DISCONNECTION_COMPLETE => {
            if params.len() >= 4 {
                let handle = u16::from_le_bytes([params[1], params[2]]);
                let reason = params[3];
                crate::serial_println!("  BT HCI: Disconnected handle={} reason={:#04x}",
                    handle, reason);
            }
        }
        _ => {
            crate::serial_verbose_println!("  BT HCI: unhandled event {:#04x} (len={})",
                event_code, param_len);
        }
    }
}

fn handle_command_complete(opcode: u16, status: u8, params: &[u8]) {
    match opcode {
        HCI_RESET => {
            crate::serial_println!("  BT HCI: Reset complete (status={})", status);
            if status == 0 {
                // After reset: read BD_ADDR, set name, enable scans
                read_bd_addr();
            }
        }
        HCI_READ_BD_ADDR => {
            if status == 0 && params.len() >= 7 {
                let mut addr = BdAddr::default();
                addr.0.copy_from_slice(&params[1..7]);
                crate::serial_println!("  BT HCI: Local address: {}", addr);
                // Continue initialization
                write_local_name("anyOS");
                write_scan_enable(true, true);
            }
        }
        _ => {
            if status != 0 {
                crate::serial_verbose_println!("  BT HCI: cmd {:#06x} failed (status={})",
                    opcode, status);
            }
        }
    }
}

fn handle_inquiry_result(params: &[u8], with_rssi: bool) {
    if params.is_empty() { return; }
    let num_responses = params[0] as usize;
    let entry_size = if with_rssi { 14 } else { 14 }; // Standard inquiry result entry

    for i in 0..num_responses {
        let base = 1 + i * entry_size;
        if base + 6 > params.len() { break; }

        let mut addr = BdAddr::default();
        addr.0.copy_from_slice(&params[base..base + 6]);

        let cod = if base + 11 <= params.len() {
            u32::from_le_bytes([params[base + 8], params[base + 9], params[base + 10], 0])
        } else { 0 };

        let rssi = if with_rssi && base + 14 <= params.len() {
            params[base + 13] as i8
        } else { 0 };

        crate::serial_println!("  BT: found {} (class={:#08x} rssi={})", addr, cod, rssi);

        super::add_discovered(BtDevice {
            addr,
            class_of_device: cod,
            rssi,
            name: [0; 248],
            name_len: 0,
        });
    }
}

fn handle_extended_inquiry_result(params: &[u8]) {
    if params.len() < 15 { return; }

    let mut addr = BdAddr::default();
    addr.0.copy_from_slice(&params[1..7]);
    let cod = u32::from_le_bytes([params[9], params[10], params[11], 0]);
    let rssi = params[13] as i8;

    // EIR data starts at offset 15
    let mut name = [0u8; 248];
    let mut name_len = 0u8;

    let mut pos = 15;
    while pos < params.len() {
        let len = params[pos] as usize;
        if len == 0 || pos + 1 + len > params.len() { break; }
        let eir_type = params[pos + 1];
        let eir_data = &params[pos + 2..pos + 1 + len];

        // EIR type 0x08 = Shortened Local Name, 0x09 = Complete Local Name
        if (eir_type == 0x08 || eir_type == 0x09) && !eir_data.is_empty() {
            let n = eir_data.len().min(247);
            name[..n].copy_from_slice(&eir_data[..n]);
            name_len = n as u8;
        }

        pos += 1 + len;
    }

    let name_str = core::str::from_utf8(&name[..name_len as usize]).unwrap_or("?");
    crate::serial_println!("  BT: found {} '{}' (class={:#08x} rssi={})",
        addr, name_str, cod, rssi);

    super::add_discovered(BtDevice {
        addr,
        class_of_device: cod,
        rssi,
        name,
        name_len,
    });
}

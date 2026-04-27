//! Bluetooth L2CAP (Logical Link Control and Adaptation Protocol).
//!
//! L2CAP provides connection-oriented and connectionless data channels
//! over the HCI ACL link. It handles:
//! - Channel multiplexing (CID-based)
//! - Connection setup/teardown
//! - MTU negotiation
//! - Segmentation and reassembly (basic mode)
//!
//! # Channels
//!
//! Fixed channels:
//! - CID 0x0001: L2CAP Signaling
//! - CID 0x0003: AMP Manager (not used)
//! - CID 0x0004: Attribute Protocol (BLE, not used here)
//!
//! Dynamic channels (CID 0x0040+): allocated per connection.

use super::BdAddr;
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

// ── L2CAP Header ────────────────────────────────────────────────────────────

/// L2CAP fixed channel for signaling.
const L2CAP_CID_SIGNALING: u16 = 0x0001;

/// L2CAP signaling command codes.
const L2CAP_CONNECT_REQ: u8 = 0x02;
const L2CAP_CONNECT_RSP: u8 = 0x03;
const L2CAP_CONFIG_REQ: u8 = 0x04;
const L2CAP_CONFIG_RSP: u8 = 0x05;
const L2CAP_DISCONNECT_REQ: u8 = 0x06;
const L2CAP_DISCONNECT_RSP: u8 = 0x07;
const L2CAP_INFO_REQ: u8 = 0x0A;
const L2CAP_INFO_RSP: u8 = 0x0B;

/// Well-known Protocol/Service Multiplexer (PSM) values.
pub const PSM_SDP: u16 = 0x0001; // Service Discovery Protocol
pub const PSM_HID_CONTROL: u16 = 0x0011; // HID Control Channel
pub const PSM_HID_INTERRUPT: u16 = 0x0013; // HID Interrupt Channel

/// Default L2CAP MTU.
const DEFAULT_MTU: u16 = 672;

/// Maximum number of concurrent L2CAP channels.
const MAX_CHANNELS: usize = 32;

/// Next dynamic CID to allocate.
const FIRST_DYNAMIC_CID: u16 = 0x0040;

// ── Channel State ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum ChannelState {
    Closed,
    WaitConnect,
    WaitConfig,
    Open,
    WaitDisconnect,
}

#[derive(Debug, Clone)]
struct L2capChannel {
    /// Local channel ID.
    local_cid: u16,
    /// Remote channel ID.
    remote_cid: u16,
    /// HCI connection handle.
    handle: u16,
    /// Protocol/Service Multiplexer.
    psm: u16,
    /// Channel state.
    state: ChannelState,
    /// Negotiated MTU.
    mtu: u16,
    /// Receive buffer.
    rx_buf: Vec<u8>,
}

/// Global L2CAP channel table.
static CHANNELS: Spinlock<Vec<L2capChannel>> = Spinlock::new(Vec::new());

/// Next CID to allocate.
static NEXT_CID: core::sync::atomic::AtomicU16 =
    core::sync::atomic::AtomicU16::new(FIRST_DYNAMIC_CID);

/// Allocate a local CID.
fn alloc_cid() -> u16 {
    NEXT_CID.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
}

/// Open an L2CAP channel to a remote device.
/// `handle` = HCI connection handle, `psm` = Protocol/Service Multiplexer.
pub fn connect(handle: u16, psm: u16) -> u16 {
    let local_cid = alloc_cid();

    let channel = L2capChannel {
        local_cid,
        remote_cid: 0,
        handle,
        psm,
        state: ChannelState::WaitConnect,
        mtu: DEFAULT_MTU,
        rx_buf: Vec::new(),
    };

    CHANNELS.lock().push(channel);

    // Send L2CAP Connect Request
    let mut req = [0u8; 12];
    // L2CAP header: length(2) + CID(2) = signaling channel
    let payload_len = 4u16; // PSM(2) + Source CID(2)
    let sig_len = 4 + payload_len; // Code(1) + ID(1) + Length(2) + data
    let l2cap_len = sig_len;

    req[0] = (l2cap_len & 0xFF) as u8;
    req[1] = (l2cap_len >> 8) as u8;
    req[2] = (L2CAP_CID_SIGNALING & 0xFF) as u8;
    req[3] = (L2CAP_CID_SIGNALING >> 8) as u8;
    // Signaling: Connect Request
    req[4] = L2CAP_CONNECT_REQ;
    req[5] = 1; // Identifier (incremented per request)
    req[6] = (payload_len & 0xFF) as u8;
    req[7] = (payload_len >> 8) as u8;
    req[8] = (psm & 0xFF) as u8;
    req[9] = (psm >> 8) as u8;
    req[10] = (local_cid & 0xFF) as u8;
    req[11] = (local_cid >> 8) as u8;

    send_acl(handle, &req);
    crate::serial_println!(
        "  L2CAP: connect request PSM={:#06x} local_cid={}",
        psm,
        local_cid
    );

    local_cid
}

/// Process incoming L2CAP data on a connection.
pub fn process_acl(handle: u16, data: &[u8]) {
    if data.len() < 4 {
        return;
    }

    let _l2cap_len = u16::from_le_bytes([data[0], data[1]]);
    let cid = u16::from_le_bytes([data[2], data[3]]);
    let payload = &data[4..];

    if cid == L2CAP_CID_SIGNALING {
        process_signaling(handle, payload);
    } else {
        // Data on a dynamic channel — route to the channel's receive buffer
        let mut channels = CHANNELS.lock();
        if let Some(ch) = channels
            .iter_mut()
            .find(|c| c.local_cid == cid && c.handle == handle)
        {
            ch.rx_buf.extend_from_slice(payload);
        }
    }
}

fn process_signaling(handle: u16, data: &[u8]) {
    if data.len() < 4 {
        return;
    }

    let code = data[0];
    let _identifier = data[1];
    let _length = u16::from_le_bytes([data[2], data[3]]);
    let params = &data[4..];

    match code {
        L2CAP_CONNECT_RSP => {
            if params.len() >= 8 {
                let dest_cid = u16::from_le_bytes([params[0], params[1]]);
                let source_cid = u16::from_le_bytes([params[2], params[3]]);
                let result = u16::from_le_bytes([params[4], params[5]]);
                crate::serial_println!(
                    "  L2CAP: connect response dest={} src={} result={}",
                    dest_cid,
                    source_cid,
                    result
                );

                if result == 0 {
                    // Success — update channel and send config request
                    let mut channels = CHANNELS.lock();
                    if let Some(ch) = channels.iter_mut().find(|c| c.local_cid == source_cid) {
                        ch.remote_cid = dest_cid;
                        ch.state = ChannelState::WaitConfig;
                    }
                }
            }
        }
        L2CAP_CONFIG_RSP => {
            if params.len() >= 6 {
                let source_cid = u16::from_le_bytes([params[0], params[1]]);
                let result = u16::from_le_bytes([params[4], params[5]]);
                if result == 0 {
                    let mut channels = CHANNELS.lock();
                    if let Some(ch) = channels.iter_mut().find(|c| c.local_cid == source_cid) {
                        ch.state = ChannelState::Open;
                        crate::serial_println!(
                            "  L2CAP: channel {} open (PSM={:#06x})",
                            source_cid,
                            ch.psm
                        );
                    }
                }
            }
        }
        L2CAP_DISCONNECT_REQ => {
            if params.len() >= 4 {
                let dest_cid = u16::from_le_bytes([params[0], params[1]]);
                let source_cid = u16::from_le_bytes([params[2], params[3]]);
                // Send disconnect response
                let mut rsp = [0u8; 12];
                rsp[0] = 4;
                rsp[1] = 0; // L2CAP length
                rsp[2] = 1;
                rsp[3] = 0; // CID = signaling
                rsp[4] = L2CAP_DISCONNECT_RSP;
                rsp[5] = data[1]; // Same identifier
                rsp[6] = 4;
                rsp[7] = 0; // Param length
                rsp[8] = (dest_cid & 0xFF) as u8;
                rsp[9] = (dest_cid >> 8) as u8;
                rsp[10] = (source_cid & 0xFF) as u8;
                rsp[11] = (source_cid >> 8) as u8;
                send_acl(handle, &rsp);

                let mut channels = CHANNELS.lock();
                channels.retain(|c| c.local_cid != dest_cid || c.handle != handle);
            }
        }
        _ => {
            crate::serial_verbose_println!("  L2CAP: unhandled signaling code {:#04x}", code);
        }
    }
}

/// Send ACL data on a connection.
fn send_acl(handle: u16, l2cap_data: &[u8]) {
    // ACL header: Handle(2, with PB=0x20 first fragment) + Total Length(2)
    let mut pkt = alloc::vec![0u8; 4 + l2cap_data.len()];
    let handle_flags = handle | (0x02 << 12); // PB = 0b10 (first automatically-flushable)
    pkt[0] = (handle_flags & 0xFF) as u8;
    pkt[1] = (handle_flags >> 8) as u8;
    pkt[2] = (l2cap_data.len() & 0xFF) as u8;
    pkt[3] = (l2cap_data.len() >> 8) as u8;
    pkt[4..].copy_from_slice(l2cap_data);

    super::usb_transport::send_acl(&pkt);
}

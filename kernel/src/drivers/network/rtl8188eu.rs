//! RTL8188EU USB WiFi driver.
//!
//! Supports the Realtek RTL8188EU 802.11n (2.4 GHz) USB WiFi chip, which is
//! extremely common in USB "nano" dongles and embedded WiFi modules.
//!
//! Supported USB IDs:
//!   0x0BDA:0x8179  (RTL8188EUS — primary)
//!   0x0BDA:0x8176  (RTL8188CE)
//!   0x0BDA:0x8187  (RTL8187)
//!   0x2001:0x3319  (D-Link DWA-131)
//!   0x7392:0x7811  (Edimax EW-7811Un)
//!
//! # Hardware overview
//!
//! The RTL8188EU uses USB 2.0 (High-Speed, 480 Mbps).  All register access
//! goes over USB control transfers; TX and RX use USB bulk endpoints.
//!
//! Register map:
//!   The chip exposes a 16-bit register address space accessed via vendor
//!   control requests (bRequest = 0x05).  The upper 4 bits of the address
//!   select the "page"; the chip's HAL documentation refers to registers as
//!   "0xYZZZ" where Y = page, ZZZ = offset.
//!
//! Firmware:
//!   The chip requires an MCU firmware image to be uploaded at startup.
//!   Without firmware, the MAC and RF are non-functional.  We attempt to load
//!   `/System/Drivers/wifi/rtl8188eu.bin` at runtime; if absent we continue
//!   in reduced mode (register access still works, useful for bringup).
//!
//! Frame format:
//!   TX: 8-byte descriptor prepended to the 802.11 frame, sent over bulk OUT.
//!   RX: 24-byte descriptor prepended to the received 802.11 frame, arrives on
//!       bulk IN.  We strip the descriptor and dispatch the frame.

use alloc::vec::Vec;
use crate::sync::spinlock::Spinlock;
use super::super::usb::{
    ControllerType, SetupPacket, UsbDevice, UsbInterface, UsbSpeed,
};

// ── Supported USB IDs ─────────────────────────────────────────────────────────

/// List of (VID, PID) pairs supported by this driver.
const SUPPORTED_IDS: &[(u16, u16)] = &[
    (0x0BDA, 0x8179), // RTL8188EUS (most common)
    (0x0BDA, 0x8176), // RTL8188CE-VAU
    (0x0BDA, 0x8187), // RTL8187
    (0x2001, 0x3319), // D-Link DWA-131 rev E1
    (0x7392, 0x7811), // Edimax EW-7811Un
];

// ── Key Registers ─────────────────────────────────────────────────────────────

/// Command register: bit0 = RST, bit2 = RE (RX enable), bit3 = TE (TX enable).
const REG_CR: u16 = 0x0100;
/// Media Status Register: bits[1:0]: 0=no link, 1=WiFi, 3=USB
const REG_MSR: u16 = 0x0102;
/// SIFS timing register (CCK mode). Reserved for future RF configuration.
#[allow(dead_code)]
const REG_SIFS_CCK: u16 = 0x0114;
/// TX configuration register.
const REG_TCR: u16 = 0x0604;
/// RX configuration register: bit12=CBSSID_BCN, bit10=CBSSID_DATA, bit6=ACRC32 (accept bad CRC)
const REG_RCR: u16 = 0x0608;
/// MACID register: 6 bytes, read sequentially starting here.
const REG_MACID: u16 = 0x0610;
/// BSSID register (6 bytes).
const REG_BSSID: u16 = 0x0618;
/// RXDMA control register.
const REG_RXDMA: u16 = 0x0228;

// ── USB vendor request codes ──────────────────────────────────────────────────

/// Vendor control request: write register.
const VENDOR_WRITE: u8 = 0x05;
/// bmRequestType for vendor write (host-to-device, vendor, device).
const BM_WRITE: u8 = 0x40;
/// bmRequestType for vendor read (device-to-host, vendor, device).
const BM_READ: u8 = 0xC0;

// ── TX descriptor format (8 bytes) ───────────────────────────────────────────
//
// Word 0 (bytes 0-3, little-endian):
//   bits[15:0]  = packet length (total bytes including descriptor)
//   bits[30:16] = reserved
//   bit[31]     = OWN (set by host, cleared by chip when transmitted)
//
// Word 1 (bytes 4-7, little-endian):
//   bits[12:8]  = queue select: 0x12 = Best Effort (BE) AC queue

const TX_DESC_SIZE: usize = 8;
/// BE queue ID in TX descriptor word 1 bits[12:8].
const TX_QUEUE_BE: u8 = 0x12;

// ── RX descriptor format (24 bytes) ──────────────────────────────────────────
//
// Word 0 (bytes 0-3, little-endian):
//   bits[13:0]  = packet length
//   bit[14]     = CRC error
//   bit[31]     = PHY status included before packet

const RX_DESC_SIZE: usize = 24;

// ── Buffer sizes ──────────────────────────────────────────────────────────────

/// Maximum 802.11 MSDU + headers + padding, rounded to page boundary.
const TX_BUF_SIZE: usize = 2 * 4096; // 2 pages = 8 KB
const RX_BUF_SIZE: usize = 2 * 4096;

// ── RX queue ──────────────────────────────────────────────────────────────────

/// Decoded Ethernet frames ready for the network stack.
static RX_QUEUE: Spinlock<Vec<Vec<u8>>> = Spinlock::new(Vec::new());

/// Maximum number of buffered RX frames before we drop.
const MAX_RX_QUEUE: usize = 64;

// ── Driver instance ───────────────────────────────────────────────────────────

pub struct Rtl8188euDriver {
    usb_addr:       u8,
    controller:     ControllerType,
    speed:          UsbSpeed,
    bulk_in_ep:     u8,
    bulk_out_ep:    u8,
    max_packet_in:  u16,
    max_packet_out: u16,
    mac:            [u8; 6],
    tx_toggle:      u8,
    rx_toggle:      u8,
    /// Physical address of TX DMA bounce buffer (2 pages).
    tx_buf_phys:    u64,
    /// Physical address of RX DMA bounce buffer (2 pages).
    rx_buf_phys:    u64,
}

/// Global driver instance (only one RTL8188EU at a time).
static DRIVER: Spinlock<Option<Rtl8188euDriver>> = Spinlock::new(None);

// ── Register access ───────────────────────────────────────────────────────────

/// Write a 16-bit value to a register at `addr`.
fn reg_write16(dev_addr: u8, controller: ControllerType, speed: UsbSpeed, max_pkt: u16,
               addr: u16, value: u16)
{
    let setup = SetupPacket {
        bm_request_type: BM_WRITE,
        b_request:       VENDOR_WRITE,
        w_value:         value,
        w_index:         addr,
        w_length:        2,
    };
    let _ = super::super::usb::hid_control_transfer(
        dev_addr, controller, speed, max_pkt, &setup, false, 0,
    );
}

/// Write a single byte to a register at `addr`.
fn reg_write8(dev_addr: u8, controller: ControllerType, speed: UsbSpeed, max_pkt: u16,
              addr: u16, value: u8)
{
    let setup = SetupPacket {
        bm_request_type: BM_WRITE,
        b_request:       VENDOR_WRITE,
        w_value:         value as u16,
        w_index:         addr,
        w_length:        1,
    };
    let _ = super::super::usb::hid_control_transfer(
        dev_addr, controller, speed, max_pkt, &setup, false, 0,
    );
}

/// Read `len` bytes from register `addr`. Returns data or empty vec on error.
fn reg_read(dev_addr: u8, controller: ControllerType, speed: UsbSpeed, max_pkt: u16,
            addr: u16, len: u16) -> Vec<u8>
{
    let setup = SetupPacket {
        bm_request_type: BM_READ,
        b_request:       VENDOR_WRITE,
        w_value:         0,
        w_index:         addr,
        w_length:        len,
    };
    super::super::usb::hid_control_transfer(
        dev_addr, controller, speed, max_pkt, &setup, true, len,
    ).unwrap_or_default()
}

// ── Hardware initialisation ───────────────────────────────────────────────────

/// Attempt to upload firmware from the VFS.
/// Returns true if firmware was uploaded successfully.
fn upload_firmware(dev_addr: u8, controller: ControllerType, speed: UsbSpeed, max_pkt: u16) -> bool {
    crate::serial_verbose_println!("[rtl8188eu] Attempting firmware load from /System/Drivers/wifi/rtl8188eu.bin");

    // Try to open the firmware file via VFS
    let fw_data = crate::fs::vfs::read_file_to_vec("/System/Drivers/wifi/rtl8188eu.bin");
    let fw = match fw_data {
        Ok(data) => {
            crate::serial_verbose_println!("[rtl8188eu] Firmware: {} bytes", data.len());
            data
        }
        Err(_) => {
            crate::serial_verbose_println!("[rtl8188eu] Firmware not found — reduced mode");
            return false;
        }
    };

    if fw.len() < 32 {
        crate::serial_verbose_println!("[rtl8188eu] Firmware file too small");
        return false;
    }

    // RTL8188EU firmware upload sequence:
    //
    // 1. Assert MCU reset (write 0x04 to SYS_FUNC_EN to reset 8051)
    // 2. Transfer firmware payload in 64-byte chunks via control transfers
    //    to the firmware load register (0x1000 + offset)
    // 3. De-assert reset and wait for MCU ready (poll MCUFWDL register)
    //
    // Firmware load register base
    const FW_REG_BASE: u16 = 0x1000;
    const CHUNK_SIZE: usize = 64;

    let mut offset = 0usize;
    let mut page: u16 = 0;

    while offset < fw.len() {
        let chunk_len = (fw.len() - offset).min(CHUNK_SIZE);
        let reg_addr = FW_REG_BASE + (offset as u16 % 0x1000);

        // Advance page if we've wrapped the 4 KB register window
        if offset > 0 && (offset % 0x1000) == 0 {
            page += 1;
            // Write page select to MCUFWDL register (0x0080), bits[7:4]
            reg_write8(dev_addr, controller, speed, max_pkt, 0x0080, page as u8);
        }

        // Upload this chunk byte-by-byte via reg_write8.
        // The hid_control_transfer API does not currently support a write-data
        // buffer path, so we fall back to individual byte writes.
        // In a production driver you would extend the USB layer with a
        // bulk-write path (bm_request_type=0x40, w_index=addr, data payload).
        let _ = reg_addr; // address is encoded per-byte below
        for i in 0..chunk_len {
            reg_write8(dev_addr, controller, speed, max_pkt,
                       FW_REG_BASE + (offset + i) as u16, fw[offset + i]);
        }

        offset += chunk_len;
    }

    // Set MCUFWDL_EN bit in 0x0080 to trigger load
    reg_write8(dev_addr, controller, speed, max_pkt, 0x0080, 0x01);
    // Wait for MCU ready (poll bit 1 of 0x0080 up to 50 ms)
    for _ in 0..50 {
        crate::arch::x86::pit::delay_ms(1);
        let status = reg_read(dev_addr, controller, speed, max_pkt, 0x0080, 1);
        if !status.is_empty() && (status[0] & 0x02) != 0 {
            crate::serial_verbose_println!("[rtl8188eu] Firmware loaded OK");
            return true;
        }
    }

    crate::serial_verbose_println!("[rtl8188eu] Firmware load timeout");
    false
}

/// Read the 6-byte MAC address from the MACID hardware register.
fn read_mac(dev_addr: u8, controller: ControllerType, speed: UsbSpeed, max_pkt: u16) -> [u8; 6] {
    let data = reg_read(dev_addr, controller, speed, max_pkt, REG_MACID, 6);
    if data.len() >= 6 {
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&data[..6]);
        mac
    } else {
        // Fallback: locally administered unicast address
        [0x02, 0x00, 0x00, 0xFF, 0x88, 0x1E]
    }
}

/// Configure the AP's BSSID into hardware for frame filtering.
fn set_bssid(dev_addr: u8, controller: ControllerType, speed: UsbSpeed, max_pkt: u16,
             bssid: &[u8; 6])
{
    for i in 0..6 {
        reg_write8(dev_addr, controller, speed, max_pkt, REG_BSSID + i as u16, bssid[i]);
    }
}

/// Set the channel by writing the RF control registers.
///
/// A full channel-change sequence involves PHY/RF register writes via
/// indirect access registers.  For now we log the channel change; a
/// production driver would perform the complete RF programming sequence.
fn set_channel(_dev_addr: u8, _controller: ControllerType, _speed: UsbSpeed, _max_pkt: u16,
               channel: u8)
{
    crate::serial_verbose_println!("[rtl8188eu] Set channel {}", channel);
    // In a full implementation this would write to:
    //   REG_FPGA0_RFMOD  (0x800)
    //   REG_RF_A_CHANNEL (0x7C, via RF indirect access)
    // using the rtl8188eu-specific RF band/channel tables.
}

/// Perform the hardware reset and basic MAC initialisation.
fn hw_init(dev_addr: u8, controller: ControllerType, speed: UsbSpeed, max_pkt: u16) {
    // 1. Reset: write CR[0] = 1
    reg_write8(dev_addr, controller, speed, max_pkt, REG_CR, 0x00);
    crate::arch::x86::pit::delay_ms(5);
    reg_write8(dev_addr, controller, speed, max_pkt, REG_CR, 0xFF);
    crate::arch::x86::pit::delay_ms(5);

    // 2. Configure RX: accept all, no CRC filtering during scan
    //    RCR bits: ACRC32(6) + AB(3) + AM(2) + APM(1) + AAP(0) = 0x0000004F
    reg_write16(dev_addr, controller, speed, max_pkt, REG_RCR, 0x004F);

    // 3. TX config: normal mode
    reg_write16(dev_addr, controller, speed, max_pkt, REG_TCR, 0x0000);

    // 4. RXDMA: enable
    reg_write8(dev_addr, controller, speed, max_pkt, REG_RXDMA, 0x06);

    // 5. Set media status to WiFi (bits[1:0] = 01)
    reg_write8(dev_addr, controller, speed, max_pkt, REG_MSR, 0x01);

    // 6. Enable TX + RX in CR (TE|RE bits 3,2)
    reg_write8(dev_addr, controller, speed, max_pkt, REG_CR, 0x0C);
}

// ── Channel scanning ──────────────────────────────────────────────────────────

/// Perform an active scan on channels 1–13.
/// Sends a Probe Request on each channel and polls for responses.
fn do_scan(drv: &mut Rtl8188euDriver) {
    crate::serial_verbose_println!("[rtl8188eu] Starting channel scan");

    let probe_frame = crate::net::wifi::build_probe_request(&drv.mac);

    for ch in 1u8..=13 {
        // Tune to channel
        set_channel(drv.usb_addr, drv.controller, drv.speed, 0, ch);
        crate::arch::x86::pit::delay_ms(10);

        // Transmit probe request
        rtl_transmit_inner(drv, &probe_frame);

        // Dwell for beacons / probe responses (~100 ms)
        for _ in 0..10 {
            crate::arch::x86::pit::delay_ms(10);
            rtl_poll_rx_inner(drv);
        }
    }

    crate::serial_verbose_println!("[rtl8188eu] Scan complete ({} BSS found)",
        crate::net::wifi::get_scan_results().len());
}

// ── TX / RX ───────────────────────────────────────────────────────────────────

/// Build an 8-byte RTL8188EU TX descriptor for a packet of `pkt_len` bytes
/// (not including the descriptor itself).
fn build_tx_desc(pkt_len: usize) -> [u8; TX_DESC_SIZE] {
    let mut desc = [0u8; TX_DESC_SIZE];
    // Word 0: OWN(31) | length[15:0]
    // Total length = descriptor + packet
    let total_len = (TX_DESC_SIZE + pkt_len) as u32;
    let word0 = (1u32 << 31) | (total_len & 0xFFFF);
    desc[0..4].copy_from_slice(&word0.to_le_bytes());
    // Word 1: queue select in bits[12:8] = TX_QUEUE_BE
    let word1 = (TX_QUEUE_BE as u32) << 8;
    desc[4..8].copy_from_slice(&word1.to_le_bytes());
    desc
}

/// Inner transmit — does not lock DRIVER.
fn rtl_transmit_inner(drv: &mut Rtl8188euDriver, data: &[u8]) -> bool {
    let total = TX_DESC_SIZE + data.len();
    if total > TX_BUF_SIZE {
        crate::serial_verbose_println!("[rtl8188eu] TX: frame too large ({})", data.len());
        return false;
    }

    let desc = build_tx_desc(data.len());

    // Copy descriptor + frame into TX DMA buffer
    unsafe {
        let buf = drv.tx_buf_phys as *mut u8;
        core::ptr::copy_nonoverlapping(desc.as_ptr(), buf, TX_DESC_SIZE);
        core::ptr::copy_nonoverlapping(data.as_ptr(), buf.add(TX_DESC_SIZE), data.len());
    }

    super::super::usb::bulk_transfer(
        drv.usb_addr,
        drv.controller,
        drv.speed,
        drv.bulk_out_ep,
        drv.max_packet_out,
        &mut drv.tx_toggle,
        drv.tx_buf_phys,
        total,
    ).is_ok()
}

/// Parse an incoming bulk IN buffer and dispatch frames.
fn rtl_poll_rx_inner(drv: &mut Rtl8188euDriver) {
    let result = super::super::usb::bulk_transfer(
        drv.usb_addr,
        drv.controller,
        drv.speed,
        drv.bulk_in_ep,
        drv.max_packet_in,
        &mut drv.rx_toggle,
        drv.rx_buf_phys,
        RX_BUF_SIZE,
    );

    let rx_len = match result {
        Ok(n) if n > RX_DESC_SIZE => n,
        _ => return,
    };

    // Copy from DMA buffer
    let mut raw = alloc::vec![0u8; rx_len];
    unsafe {
        core::ptr::copy_nonoverlapping(
            drv.rx_buf_phys as *const u8,
            raw.as_mut_ptr(),
            rx_len,
        );
    }

    // Parse RX descriptor (24 bytes)
    // Word 0: packet length in bits[13:0], CRC error in bit[14]
    let word0 = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    let pkt_len = (word0 & 0x3FFF) as usize;
    let crc_error = (word0 >> 14) & 1 != 0;

    if crc_error {
        crate::serial_verbose_println!("[rtl8188eu] RX: CRC error, dropping");
        return;
    }

    if pkt_len == 0 || RX_DESC_SIZE + pkt_len > rx_len {
        return;
    }

    let frame = &raw[RX_DESC_SIZE..RX_DESC_SIZE + pkt_len];
    dispatch_rx_frame(frame, drv);
}

/// Classify and dispatch a received 802.11 frame.
///
/// Management frames are passed to the wifi module.
/// Data frames are decapsulated (802.11 → Ethernet) and queued for poll_rx().
fn dispatch_rx_frame(frame: &[u8], drv: &mut Rtl8188euDriver) {
    if frame.len() < 2 {
        return;
    }

    // 802.11 Frame Control
    let fc_byte0 = frame[0];
    // bits[1:0] = protocol version (must be 0)
    // bits[3:2] = type: 00=management, 01=control, 10=data
    // bits[7:4] = subtype
    let frame_type    = (fc_byte0 >> 2) & 0x03;
    let frame_subtype = (fc_byte0 >> 4) & 0x0F;

    // Collect frames to transmit after the handler returns.
    // This avoids re-acquiring the DRIVER lock (or using the &mut drv) inside
    // a closure that is called while dispatch_rx_frame holds the borrow.
    let mut tx_pending: Vec<Vec<u8>> = Vec::new();

    match frame_type {
        // Management frames
        0b00 => {
            match frame_subtype {
                0x00 => {} // Association Request (we sent it, ignore)
                0x01 => {
                    // Association Response
                    crate::net::wifi::handle_assoc_response(frame, &mut |tx_frame: &[u8]| {
                        tx_pending.push(tx_frame.to_vec());
                    });
                }
                0x04 | 0x05 => {
                    // Probe Request / Probe Response
                    crate::net::wifi::handle_probe_response(frame);
                }
                0x08 => {
                    // Beacon
                    crate::net::wifi::handle_beacon(frame);

                    // If we are in Scanning state and there's a connect config,
                    // check if this is our target AP and start auth
                    maybe_start_auth(frame, drv);
                    // maybe_start_auth transmits directly using the &mut drv it receives
                }
                0x0B => {
                    // Authentication Response
                    crate::net::wifi::handle_auth_response(frame, &mut |tx_frame: &[u8]| {
                        tx_pending.push(tx_frame.to_vec());
                    });
                }
                _ => {}
            }
        }

        // Data frames
        0b10 => {
            // Decapsulate 802.11 → Ethernet for all data frames
            if let Some(eth_frame) = decap_data_frame(frame) {
                // Check EtherType to route EAPOL frames before general queuing
                if eth_frame.len() >= 14 {
                    let ethertype = u16::from_be_bytes([eth_frame[12], eth_frame[13]]);
                    if ethertype == 0x888E {
                        // EAPOL frame — feed to the WPA2 handshake engine
                        crate::net::wifi::handle_eapol(&eth_frame, &mut |tx_frame: &[u8]| {
                            tx_pending.push(tx_frame.to_vec());
                        });
                        // Transmit any pending frames (EAPOL responses)
                        for pkt in &tx_pending {
                            rtl_transmit_inner(drv, pkt);
                        }
                        return;
                    }
                }

                // Normal data frame — queue for net stack (only if associated)
                if crate::net::wifi::is_connected() {
                    let mut queue = RX_QUEUE.lock();
                    if queue.len() < MAX_RX_QUEUE {
                        queue.push(eth_frame);
                    }
                }
            }
        }

        _ => {}
    }

    // Transmit any management-frame responses (assoc req, auth req)
    for pkt in &tx_pending {
        rtl_transmit_inner(drv, pkt);
    }
}

/// If we are scanning and see a beacon for our target SSID, initiate auth.
/// Takes a mutable reference so it can transmit the auth frame directly
/// (avoiding re-locking the DRIVER global and causing a deadlock).
fn maybe_start_auth(frame: &[u8], drv: &mut Rtl8188euDriver) {
    use crate::net::wifi::WifiState;

    // Only act during Scanning
    let state = crate::net::wifi::get_state();
    if !matches!(state, WifiState::Scanning) {
        return;
    }

    // Only act if there is a pending connect config
    let (target_ssid, target_ssid_len) = match crate::net::wifi::get_connect_ssid() {
        Some(t) => t,
        None => return,
    };

    // Parse BSSID, SSID, and channel from the beacon frame
    if frame.len() < 36 {
        return;
    }

    let mut beacon_ssid = [0u8; 32];
    let mut beacon_ssid_len = 0usize;
    let mut beacon_bssid = [0u8; 6];
    let mut beacon_channel = 1u8;

    beacon_bssid.copy_from_slice(&frame[16..22]);

    let mut offset = 36;
    while offset + 1 < frame.len() {
        let tag = frame[offset];
        let tlen = frame[offset + 1] as usize;
        if offset + 2 + tlen > frame.len() { break; }
        let td = &frame[offset + 2..offset + 2 + tlen];
        match tag {
            0 => {
                // SSID tag
                let clen = tlen.min(32);
                beacon_ssid[..clen].copy_from_slice(&td[..clen]);
                beacon_ssid_len = clen;
            }
            3 if tlen >= 1 => { beacon_channel = td[0]; }
            _ => {}
        }
        offset += 2 + tlen;
    }

    // Check if beacon SSID matches our target
    if beacon_ssid_len != target_ssid_len
        || beacon_ssid[..beacon_ssid_len] != target_ssid[..target_ssid_len]
    {
        return;
    }

    crate::serial_verbose_println!(
        "[rtl8188eu] Target AP found on channel {}, starting auth",
        beacon_channel
    );

    let our_mac = drv.mac;
    let auth_frame = crate::net::wifi::build_auth_request(&beacon_bssid, &our_mac, 2);

    // Configure BSSID filter and channel directly via the driver ref we already hold
    set_bssid(drv.usb_addr, drv.controller, drv.speed, 0, &beacon_bssid);
    set_channel(drv.usb_addr, drv.controller, drv.speed, 0, beacon_channel);
    rtl_transmit_inner(drv, &auth_frame);

    crate::serial_verbose_println!(
        "[rtl8188eu] Auth request sent to {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} ch={}",
        beacon_bssid[0], beacon_bssid[1], beacon_bssid[2],
        beacon_bssid[3], beacon_bssid[4], beacon_bssid[5],
        beacon_channel
    );
}

/// Decapsulate an 802.11 data frame to a raw Ethernet II frame.
///
/// 802.11 data frame layout (infrastructure mode, from AP):
///   Frame Control   [0..2]
///   Duration/ID     [2..4]
///   Address 1 (DA)  [4..10]
///   Address 2 (SA)  [10..16]  — transmitter (AP)
///   Address 3 (BSSID/SA)[16..22]
///   Sequence Ctrl   [22..24]
///   QoS Control     [24..26]  (if QoS Data subtype, bit 7 of FC byte 0)
///   LLC/SNAP header [26..34] or [24..32] depending on QoS
///     AA AA 03 00 00 00 <EtherType 2 bytes>
///   Payload         [34..] or [32..]
///
/// We build: dst[6] + src[6] + ethertype[2] + payload
fn decap_data_frame(frame: &[u8]) -> Option<Vec<u8>> {
    if frame.len() < 24 {
        return None;
    }

    let fc0 = frame[0];
    // bit 7 of FC byte 0 = subtype bit 3 (QoS bit for data subtypes)
    let is_qos = (fc0 & 0x80) != 0;
    // FC byte 1 bit 6 = From DS
    let from_ds = (frame[1] & 0x02) != 0;

    // Determine header length
    let header_len = if is_qos { 26 } else { 24 };

    if frame.len() < header_len + 8 {
        return None;
    }

    // LLC/SNAP: must be AA AA 03 00 00 00 <EtherType>
    let llc = &frame[header_len..header_len + 8];
    if llc[0] != 0xAA || llc[1] != 0xAA || llc[2] != 0x03
        || llc[3] != 0x00 || llc[4] != 0x00 || llc[5] != 0x00
    {
        // Not a standard Ethernet encapsulation; drop
        return None;
    }

    let ethertype = [llc[6], llc[7]];
    let payload_start = header_len + 8;
    let payload = &frame[payload_start..];

    // Destination address: Address 1
    let da = &frame[4..10];
    // Source address: depends on DS bits
    //   From DS=1: Address 3 is the original SA
    //   From DS=0: Address 2 is the SA
    let sa = if from_ds { &frame[16..22] } else { &frame[10..16] };

    let mut eth = Vec::with_capacity(14 + payload.len());
    eth.extend_from_slice(da);
    eth.extend_from_slice(sa);
    eth.extend_from_slice(&ethertype);
    eth.extend_from_slice(payload);

    Some(eth)
}

// ── Probe ─────────────────────────────────────────────────────────────────────

/// Called from USB device dispatch for vendor-class (0xFF) interfaces.
/// Returns without doing anything if the VID/PID is not a supported RTL8188EU.
pub fn probe(dev: &UsbDevice, iface: &UsbInterface) {
    // Check VID/PID
    if !SUPPORTED_IDS.iter().any(|&(vid, pid)| vid == dev.vendor_id && pid == dev.product_id) {
        return;
    }

    crate::serial_verbose_println!(
        "[rtl8188eu] Detected {:04x}:{:04x} (addr={})",
        dev.vendor_id, dev.product_id, dev.address
    );

    // Find bulk IN and bulk OUT endpoints on this interface
    let bulk_in = iface.endpoints.iter().find(|ep| {
        (ep.attributes & 0x03) == 2 && (ep.address & 0x80) != 0
    });
    let bulk_out = iface.endpoints.iter().find(|ep| {
        (ep.attributes & 0x03) == 2 && (ep.address & 0x80) == 0
    });

    let (ep_in, ep_out) = match (bulk_in, bulk_out) {
        (Some(i), Some(o)) => {
            crate::serial_verbose_println!(
                "[rtl8188eu] bulk IN ep={:#04x} (max={}), bulk OUT ep={:#04x} (max={})",
                i.address, i.max_packet_size, o.address, o.max_packet_size
            );
            (i, o)
        }
        _ => {
            // Try finding endpoints across all interfaces
            let i = dev.interfaces.iter()
                .flat_map(|i| i.endpoints.iter())
                .find(|ep| (ep.attributes & 0x03) == 2 && (ep.address & 0x80) != 0);
            let o = dev.interfaces.iter()
                .flat_map(|i| i.endpoints.iter())
                .find(|ep| (ep.attributes & 0x03) == 2 && (ep.address & 0x80) == 0);
            match (i, o) {
                (Some(i), Some(o)) => (i, o),
                _ => {
                    crate::serial_verbose_println!("[rtl8188eu] no bulk endpoints found");
                    return;
                }
            }
        }
    };

    // Allocate 2 contiguous pages for each DMA buffer
    let tx_phys = match crate::memory::physical::alloc_contiguous(2) {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_verbose_println!("[rtl8188eu] failed to alloc TX DMA buffer");
            return;
        }
    };
    let rx_phys = match crate::memory::physical::alloc_contiguous(2) {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_verbose_println!("[rtl8188eu] failed to alloc RX DMA buffer");
            return;
        }
    };
    unsafe {
        core::ptr::write_bytes(tx_phys as *mut u8, 0, TX_BUF_SIZE);
        core::ptr::write_bytes(rx_phys as *mut u8, 0, RX_BUF_SIZE);
    }

    // Upload firmware
    upload_firmware(dev.address, dev.controller, dev.speed, dev.max_packet_size);

    // Initialise hardware
    hw_init(dev.address, dev.controller, dev.speed, dev.max_packet_size);

    // Read MAC address
    let mac = read_mac(dev.address, dev.controller, dev.speed, dev.max_packet_size);
    crate::serial_verbose_println!(
        "[rtl8188eu] MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    let drv = Rtl8188euDriver {
        usb_addr:       dev.address,
        controller:     dev.controller,
        speed:          dev.speed,
        bulk_in_ep:     ep_in.address,
        bulk_out_ep:    ep_out.address,
        max_packet_in:  ep_in.max_packet_size,
        max_packet_out: ep_out.max_packet_size,
        mac,
        tx_toggle:      0,
        rx_toggle:      0,
        tx_buf_phys:    tx_phys,
        rx_buf_phys:    rx_phys,
    };

    *DRIVER.lock() = Some(drv);

    // Register as WiFi NetworkDriver
    crate::drivers::network::register_wifi(
        alloc::boxed::Box::new(Rtl8188euNetworkDriver),
    );

    // Start initial scan
    crate::net::wifi::start_scan();
    {
        let mut guard = DRIVER.lock();
        if let Some(d) = guard.as_mut() {
            do_scan(d);
        }
    }

    crate::serial_verbose_println!("[rtl8188eu] Init complete");
}

// ── Public RX poll ────────────────────────────────────────────────────────────

/// Poll the RTL8188EU for incoming frames.
/// Called from the USB poll thread (and from net::poll_rx).
pub fn poll_rx() {
    let mut guard = DRIVER.lock();
    if let Some(drv) = guard.as_mut() {
        rtl_poll_rx_inner(drv);
    }
}

/// Pop one decoded Ethernet frame from the RX queue (FIFO order).
/// Called from net::poll_rx() to feed the TCP/IP stack.
pub fn recv_packet() -> Option<Vec<u8>> {
    let mut q = RX_QUEUE.lock();
    if q.is_empty() { None } else { Some(q.remove(0)) }
}

// ── NetworkDriver trait impl ──────────────────────────────────────────────────

use crate::drivers::network::NetworkDriver;

struct Rtl8188euNetworkDriver;

impl NetworkDriver for Rtl8188euNetworkDriver {
    fn name(&self) -> &str {
        "RTL8188EU 802.11n WiFi"
    }

    fn transmit(&mut self, data: &[u8]) -> bool {
        let mut guard = DRIVER.lock();
        match guard.as_mut() {
            Some(drv) => rtl_transmit_inner(drv, data),
            None => false,
        }
    }

    fn get_mac(&self) -> [u8; 6] {
        DRIVER.lock().as_ref().map(|d| d.mac).unwrap_or([0; 6])
    }

    fn link_up(&self) -> bool {
        crate::net::wifi::is_connected()
    }
}

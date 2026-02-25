//! EHCI (Enhanced Host Controller Interface) driver — USB 2.0.
//!
//! MMIO-based controller. Uses BAR0 for register access.
//! Supports high-speed (480 Mbps) devices. Ports that fail high-speed
//! handshake are released to companion controllers (UHCI/OHCI).

use crate::arch::x86::pit::delay_ms;
use crate::drivers::pci::{PciDevice, pci_config_read32, pci_config_write32};
use crate::memory::address::PhysAddr;
use crate::memory::physical;
use crate::memory::virtual_mem;
use super::*;

// ── EHCI MMIO Virtual Address ──────────────────
// After AHCI at 0xFFFF_FFFF_D006_0000 (8 pages)
// After E1000 at 0xFFFF_FFFF_D000_0000 (32 pages)
// After SVGA FIFO at 0xFFFF_FFFF_D002_0000 (64 pages)
const EHCI_MMIO_BASE: u64 = 0xFFFF_FFFF_D008_0000;
const EHCI_MMIO_PAGES: usize = 16; // 64 KiB

/// EHCI spec: N_PORTS is a 4-bit field → hardware maximum is 15 ports per controller.
const EHCI_MAX_PORTS: usize = 15;

// ── Capability Register Offsets ────────────────

const CAP_CAPLENGTH: u32 = 0x00;  // 8-bit
const CAP_HCSPARAMS: u32 = 0x04;  // 32-bit
const CAP_HCCPARAMS: u32 = 0x08;  // 32-bit

// ── Operational Register Offsets (from op base) ──

const OP_USBCMD: u32 = 0x00;
const OP_USBSTS: u32 = 0x04;
const OP_USBINTR: u32 = 0x08;
const OP_FRINDEX: u32 = 0x0C;
const OP_PERIODICLISTBASE: u32 = 0x14;
const OP_ASYNCLISTADDR: u32 = 0x18;
const OP_CONFIGFLAG: u32 = 0x40;
const OP_PORTSC_BASE: u32 = 0x44;

// USBCMD bits
const CMD_RUN: u32 = 1 << 0;
const CMD_HCRESET: u32 = 1 << 1;
const CMD_ASYNC_ENABLE: u32 = 1 << 5;
const CMD_PERIODIC_ENABLE: u32 = 1 << 4;
const CMD_ITC_8: u32 = 8 << 16; // Interrupt threshold = 8 microframes

// USBSTS bits
const STS_HALTED: u32 = 1 << 12;
const STS_ASYNC_ADVANCE: u32 = 1 << 5;
const STS_PORT_CHANGE: u32 = 1 << 2;
const STS_ERROR: u32 = 1 << 1;
const STS_INT: u32 = 1 << 0;

// PORTSC bits
const PORTSC_CCS: u32 = 1 << 0;    // Current Connect Status
const PORTSC_CSC: u32 = 1 << 1;    // Connect Status Change
const PORTSC_PE: u32 = 1 << 2;     // Port Enabled
const PORTSC_PEC: u32 = 1 << 3;    // Port Enable Change
const PORTSC_OCA: u32 = 1 << 4;    // Over-current Active
const PORTSC_PR: u32 = 1 << 8;     // Port Reset
const PORTSC_LINE_STATUS: u32 = 3 << 10; // Line status (bits 11:10)
const PORTSC_PP: u32 = 1 << 12;    // Port Power
const PORTSC_PO: u32 = 1 << 13;    // Port Owner (1=companion)

// QH/qTD link pointer bits
const QH_TYPE_QH: u32 = 1 << 1;
const LP_T: u32 = 1;  // Terminate

// qTD token bits
const QTD_ACTIVE: u32 = 1 << 7;
const QTD_IOC: u32 = 1 << 15;
const QTD_PID_OUT: u32 = 0 << 8;
const QTD_PID_IN: u32 = 1 << 8;
const QTD_PID_SETUP: u32 = 2 << 8;
const QTD_ERR_MASK: u32 = 0x7C; // bits 6-2: error flags
const QTD_DT: u32 = 1 << 31;    // Data Toggle

// ── DMA Structures ─────────────────────────────

/// EHCI Queue Head (48 bytes, aligned to 32).
#[repr(C)]
struct EhciQh {
    horiz_link: u32,         // next QH pointer
    characteristics: u32,    // endpoint characteristics
    capabilities: u32,       // endpoint capabilities (split transaction)
    current_qtd: u32,        // current qTD pointer
    // Overlay area (mirrors qTD)
    next_qtd: u32,
    alt_next_qtd: u32,
    token: u32,
    buffer: [u32; 5],
    // Pad to 64 bytes for alignment
    _pad: [u32; 4],
}

/// EHCI Queue Element Transfer Descriptor (32 bytes, aligned to 32).
#[repr(C)]
struct EhciQtd {
    next_qtd: u32,
    alt_next_qtd: u32,
    token: u32,
    buffer: [u32; 5],
}

// ── Controller State ───────────────────────────

struct EhciController {
    mmio_base: u64,       // virtual MMIO base
    op_base: u64,         // operational registers base (mmio_base + CAPLENGTH)
    n_ports: u8,
    async_qh_phys: u64,  // async schedule head QH (physical)
    td_pool_phys: u64,    // qTD pool (physical)
    data_buf_phys: u64,   // data buffer (physical)
    port_connected: [bool; EHCI_MAX_PORTS], // per-port connection state
}

static EHCI_CTRL: crate::sync::spinlock::Spinlock<Option<EhciController>> =
    crate::sync::spinlock::Spinlock::new(None);

fn mmio_read32(base: u64, offset: u32) -> u32 {
    unsafe { core::ptr::read_volatile((base + offset as u64) as *const u32) }
}

fn mmio_write32(base: u64, offset: u32, val: u32) {
    unsafe { core::ptr::write_volatile((base + offset as u64) as *mut u32, val) }
}

fn mmio_read8(base: u64, offset: u32) -> u8 {
    unsafe { core::ptr::read_volatile((base + offset as u64) as *const u8) }
}

// ── QH Characteristics Builder ─────────────────

fn make_qh_chars(dev_addr: u8, endpoint: u8, speed: UsbSpeed, max_packet: u16) -> u32 {
    let eps = match speed {
        UsbSpeed::High => 2u32,
        UsbSpeed::Full => 0u32,
        UsbSpeed::Low => 1u32,
    };
    let rl = 15u32;   // Nak Count Reload
    let dtc = 1u32;   // Data Toggle Control (from qTD)
    let h = 1u32;     // Head of Reclamation List

    (rl << 28)
        | (max_packet as u32 & 0x7FF) << 16
        | (h << 15)
        | (dtc << 14)
        | (eps << 12)
        | ((endpoint as u32 & 0xF) << 8)
        | (dev_addr as u32 & 0x7F)
}

fn make_qh_caps(speed: UsbSpeed) -> u32 {
    // For high-speed, mult=1 (one transaction per microframe)
    // For full/low-speed through transaction translator, set hub/port
    match speed {
        UsbSpeed::High => 1 << 30,  // mult=1
        _ => 1 << 30,               // mult=1 (simplified — no TT support yet)
    }
}

// ── qTD Token Builder ──────────────────────────

fn make_qtd_token(pid: u32, toggle: u8, bytes: u16) -> u32 {
    let cerr = 3u32; // Error counter = 3
    QTD_ACTIVE
        | ((toggle as u32 & 1) << 31)
        | ((bytes as u32 & 0x7FFF) << 16)
        | (cerr << 10)
        | pid
}

// ── Control Transfer ───────────────────────────

fn control_transfer(
    ctrl: &EhciController,
    dev_addr: u8,
    speed: UsbSpeed,
    max_packet: u16,
    setup: &SetupPacket,
    data_in: bool,
    data_len: u16,
) -> Result<usize, &'static str> {
    let qh_phys = ctrl.async_qh_phys;
    let td_base = ctrl.td_pool_phys;
    let data_buf = ctrl.data_buf_phys;

    // Copy setup packet to first 8 bytes of data buffer
    let setup_bytes: [u8; 8] = unsafe { core::mem::transmute_copy(setup) };
    let setup_buf_phys = data_buf;
    unsafe {
        core::ptr::write_volatile(setup_buf_phys as *mut [u8; 8], setup_bytes);
    }

    // Data buffer at offset 64
    let data_phys = data_buf + 64;

    // Build qTDs: Setup → Data → Status
    let mut td_idx = 0u32;

    // Setup qTD
    let setup_qtd = td_base;
    let next_qtd = td_base + 32;
    unsafe {
        let qtd = setup_qtd as *mut EhciQtd;
        (*qtd).next_qtd = next_qtd as u32;
        (*qtd).alt_next_qtd = LP_T;
        (*qtd).token = make_qtd_token(QTD_PID_SETUP, 0, 8);
        (*qtd).buffer[0] = setup_buf_phys as u32;
        (*qtd).buffer[1] = 0;
        (*qtd).buffer[2] = 0;
        (*qtd).buffer[3] = 0;
        (*qtd).buffer[4] = 0;
    }
    td_idx += 1;

    // Data qTDs
    let mut remaining = data_len;
    let mut offset = 0u16;
    let mut toggle = 1u8;
    let pid = if data_in { QTD_PID_IN } else { QTD_PID_OUT };

    while remaining > 0 {
        let chunk = remaining.min(max_packet);
        let this_qtd = td_base + (td_idx as u64) * 32;
        let next = td_base + ((td_idx + 1) as u64) * 32;

        unsafe {
            let qtd = this_qtd as *mut EhciQtd;
            (*qtd).next_qtd = next as u32;
            (*qtd).alt_next_qtd = LP_T;
            (*qtd).token = make_qtd_token(pid, toggle, chunk);
            (*qtd).buffer[0] = (data_phys + offset as u64) as u32;
            (*qtd).buffer[1] = 0;
            (*qtd).buffer[2] = 0;
            (*qtd).buffer[3] = 0;
            (*qtd).buffer[4] = 0;
        }

        toggle ^= 1;
        offset += chunk;
        remaining -= chunk;
        td_idx += 1;
    }

    // Status qTD
    // Status PID is opposite of data direction.
    // IN transfers: status = OUT. OUT/no-data transfers: status = IN.
    let status_pid = if data_in { QTD_PID_OUT } else { QTD_PID_IN };
    let status_qtd = td_base + (td_idx as u64) * 32;
    unsafe {
        let qtd = status_qtd as *mut EhciQtd;
        (*qtd).next_qtd = LP_T;
        (*qtd).alt_next_qtd = LP_T;
        (*qtd).token = make_qtd_token(status_pid, 1, 0) | QTD_IOC;
        (*qtd).buffer[0] = 0;
        (*qtd).buffer[1] = 0;
        (*qtd).buffer[2] = 0;
        (*qtd).buffer[3] = 0;
        (*qtd).buffer[4] = 0;
    }
    let _ = td_idx;

    // Set up async QH to point to the first qTD
    let mps = if max_packet == 0 { 8 } else { max_packet };
    unsafe {
        let qh = qh_phys as *mut EhciQh;
        (*qh).characteristics = make_qh_chars(dev_addr, 0, speed, mps);
        (*qh).capabilities = make_qh_caps(speed);
        (*qh).current_qtd = 0;
        (*qh).next_qtd = setup_qtd as u32;
        (*qh).alt_next_qtd = LP_T;
        (*qh).token = 0; // clear overlay
        for b in &mut (*qh).buffer { *b = 0; }
    }

    // Enable async schedule
    let usbcmd = mmio_read32(ctrl.op_base, OP_USBCMD);
    if usbcmd & CMD_ASYNC_ENABLE == 0 {
        mmio_write32(ctrl.op_base, OP_USBCMD, usbcmd | CMD_ASYNC_ENABLE);
    }

    // Poll for completion
    let timeout = 500u32;
    let start = crate::arch::x86::pit::get_ticks();

    loop {
        let token = unsafe {
            let qtd = status_qtd as *mut EhciQtd;
            core::ptr::read_volatile(&(*qtd).token)
        };

        if token & QTD_ACTIVE == 0 {
            if token & QTD_ERR_MASK != 0 {
                return Err("EHCI transfer error");
            }
            break;
        }

        if crate::arch::x86::pit::get_ticks().wrapping_sub(start) > timeout {
            // Deactivate by clearing QH overlay
            unsafe {
                let qh = qh_phys as *mut EhciQh;
                (*qh).next_qtd = LP_T;
                (*qh).token = 0;
            }
            return Err("EHCI transfer timeout");
        }

        core::hint::spin_loop();
    }

    // Deactivate QH
    unsafe {
        let qh = qh_phys as *mut EhciQh;
        (*qh).next_qtd = LP_T;
        (*qh).token = 0;
    }

    // For completed transfers, trust data_len (errors caught above)
    if data_in {
        Ok(data_len as usize)
    } else {
        Ok(0)
    }
}

fn read_transfer_data(ctrl: &EhciController, buf: &mut [u8], len: usize) {
    let data_phys = ctrl.data_buf_phys + 64;
    let to_copy = len.min(buf.len());
    unsafe {
        core::ptr::copy_nonoverlapping(data_phys as *const u8, buf.as_mut_ptr(), to_copy);
    }
}

// ── Bulk Transfer ──────────────────────────────

/// Execute a bulk transfer on a non-zero endpoint.
/// `endpoint`: endpoint address (bit 7 = direction: 0x80=IN, 0x00=OUT)
/// `toggle`: pointer to caller's data toggle state (0 or 1), updated on return
/// `data_phys`: physical address of DMA-accessible buffer
/// `len`: number of bytes to transfer
/// Returns number of bytes actually transferred.
fn bulk_transfer_inner(
    ctrl: &EhciController,
    dev_addr: u8,
    speed: UsbSpeed,
    endpoint: u8,
    max_packet: u16,
    toggle: &mut u8,
    data_phys: u64,
    len: usize,
) -> Result<usize, &'static str> {
    if len == 0 { return Ok(0); }
    let max_pkt = (max_packet as usize).max(1);

    let qh_phys = ctrl.async_qh_phys;
    let td_base = ctrl.td_pool_phys;
    let ep_num = endpoint & 0x0F;
    let is_in = (endpoint & 0x80) != 0;
    let pid = if is_in { QTD_PID_IN } else { QTD_PID_OUT };

    // Max qTDs: (4096 - 256) / 32 = 120
    let max_tds: usize = 120;
    let num_tds = ((len + max_pkt - 1) / max_pkt).min(max_tds);

    // Build qTD chain
    for i in 0..num_tds {
        let this_qtd = td_base + (i as u64) * 32;
        let offset = i * max_pkt;
        let chunk = if i == num_tds - 1 {
            (len - offset).min(max_pkt)
        } else {
            max_pkt
        };

        let next_link = if i + 1 < num_tds {
            (td_base + ((i + 1) as u64) * 32) as u32
        } else {
            LP_T
        };

        let mut token = make_qtd_token(pid, *toggle, chunk as u16);
        if i + 1 == num_tds { token |= QTD_IOC; }

        unsafe {
            let qtd = this_qtd as *mut EhciQtd;
            (*qtd).next_qtd = next_link;
            (*qtd).alt_next_qtd = LP_T;
            (*qtd).token = token;
            (*qtd).buffer[0] = (data_phys + offset as u64) as u32;
            // EHCI buffer pointers are per-4K page; set additional pages if crossing
            let start_page = (data_phys + offset as u64) & !0xFFF;
            for b in 1..5u64 {
                (*qtd).buffer[b as usize] = (start_page + b * 4096) as u32;
            }
        }
        *toggle ^= 1;
    }

    // Configure QH for this endpoint
    let mps = if max_packet == 0 { 8 } else { max_packet };
    unsafe {
        let qh = qh_phys as *mut EhciQh;
        // Keep H=1, DTC=1, set endpoint, device address, speed, max packet
        (*qh).characteristics = make_qh_chars(dev_addr, ep_num, speed, mps);
        (*qh).capabilities = make_qh_caps(speed);
        (*qh).current_qtd = 0;
        (*qh).next_qtd = td_base as u32;
        (*qh).alt_next_qtd = LP_T;
        (*qh).token = 0; // clear overlay
        for b in &mut (*qh).buffer { *b = 0; }
    }

    // Ensure async schedule is enabled
    let usbcmd = mmio_read32(ctrl.op_base, OP_USBCMD);
    if usbcmd & CMD_ASYNC_ENABLE == 0 {
        mmio_write32(ctrl.op_base, OP_USBCMD, usbcmd | CMD_ASYNC_ENABLE);
    }

    // Poll last qTD for completion
    let last_qtd = td_base + ((num_tds - 1) as u64) * 32;
    let timeout = 5000u32; // 5 seconds
    let start = crate::arch::x86::pit::get_ticks();

    loop {
        let token = unsafe {
            let qtd = last_qtd as *mut EhciQtd;
            core::ptr::read_volatile(&(*qtd).token)
        };

        if token & QTD_ACTIVE == 0 {
            if token & QTD_ERR_MASK != 0 {
                // Deactivate QH
                unsafe {
                    let qh = qh_phys as *mut EhciQh;
                    (*qh).next_qtd = LP_T;
                    (*qh).token = 0;
                }
                return Err("EHCI bulk transfer error");
            }
            break;
        }

        if crate::arch::x86::pit::get_ticks().wrapping_sub(start) > timeout {
            unsafe {
                let qh = qh_phys as *mut EhciQh;
                (*qh).next_qtd = LP_T;
                (*qh).token = 0;
            }
            return Err("EHCI bulk transfer timeout");
        }

        core::hint::spin_loop();
    }

    // Deactivate QH
    unsafe {
        let qh = qh_phys as *mut EhciQh;
        (*qh).next_qtd = LP_T;
        (*qh).token = 0;
    }

    // Calculate actual bytes transferred from qTD token fields
    let mut total = 0usize;
    for i in 0..num_tds {
        let qtd_phys = td_base + (i as u64) * 32;
        let token = unsafe {
            let qtd = qtd_phys as *mut EhciQtd;
            core::ptr::read_volatile(&(*qtd).token)
        };
        if token & QTD_ACTIVE != 0 {
            break;
        }
        let bytes_left = ((token >> 16) & 0x7FFF) as usize;
        let expected = if i == num_tds - 1 {
            (len - i * max_pkt).min(max_pkt)
        } else {
            max_pkt
        };
        let actual = expected.saturating_sub(bytes_left);
        total += actual;
        if actual < max_pkt {
            break; // short packet
        }
    }

    Ok(total)
}

/// Public bulk transfer. Locks EHCI_CTRL internally.
pub fn bulk_transfer(
    dev_addr: u8,
    speed: UsbSpeed,
    endpoint: u8,
    max_packet: u16,
    toggle: &mut u8,
    data_phys: u64,
    len: usize,
) -> Result<usize, &'static str> {
    let guard = EHCI_CTRL.lock();
    let ctrl = guard.as_ref().ok_or("EHCI not initialized")?;
    bulk_transfer_inner(ctrl, dev_addr, speed, endpoint, max_packet, toggle, data_phys, len)
}

// ── Device Enumeration ─────────────────────────

fn enumerate_device(ctrl: &EhciController, port: u8, speed: UsbSpeed) {
    let mps = match speed {
        UsbSpeed::High => 64u16,
        _ => 8u16,
    };

    // Step 1: GET_DESCRIPTOR (first 8 bytes) to address 0
    let setup = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_DEVICE,
        w_index: 0,
        w_length: 8,
    };

    match control_transfer(ctrl, 0, speed, mps, &setup, true, 8) {
        Ok(n) if n >= 8 => {}
        Ok(_) => {
            crate::serial_println!("  EHCI: port {} — short device descriptor", port);
            return;
        }
        Err(e) => {
            crate::serial_println!("  EHCI: port {} — GET_DESCRIPTOR(8) failed: {}", port, e);
            return;
        }
    }

    let mut desc_buf = [0u8; 18];
    read_transfer_data(ctrl, &mut desc_buf, 8);
    let real_mps = desc_buf[7] as u16;
    let max_packet = if real_mps > 0 { real_mps } else { mps };

    // Step 2: SET_ADDRESS
    let new_addr = alloc_address();
    let setup_addr = SetupPacket {
        bm_request_type: DIR_HOST_TO_DEVICE,
        b_request: REQ_SET_ADDRESS,
        w_value: new_addr as u16,
        w_index: 0,
        w_length: 0,
    };

    if let Err(e) = control_transfer(ctrl, 0, speed, max_packet, &setup_addr, false, 0) {
        crate::serial_println!("  EHCI: port {} — SET_ADDRESS failed: {}", port, e);
        return;
    }
    delay_ms(2);

    // Step 3: GET_DESCRIPTOR (full 18 bytes)
    let setup_full = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_DEVICE,
        w_index: 0,
        w_length: 18,
    };

    match control_transfer(ctrl, new_addr, speed, max_packet, &setup_full, true, 18) {
        Ok(n) if n >= 18 => {}
        Ok(n) => {
            crate::serial_println!("  EHCI: device {} — short descriptor ({} bytes)", new_addr, n);
            return;
        }
        Err(e) => {
            crate::serial_println!("  EHCI: device {} — GET_DESCRIPTOR(18) failed: {}", new_addr, e);
            return;
        }
    }

    read_transfer_data(ctrl, &mut desc_buf, 18);
    let dev_desc: DeviceDescriptor = unsafe { core::ptr::read_unaligned(desc_buf.as_ptr() as *const _) };

    // Step 4: GET_DESCRIPTOR (config header)
    let setup_cfg = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_CONFIG,
        w_index: 0,
        w_length: 9,
    };

    let total_len = match control_transfer(ctrl, new_addr, speed, max_packet, &setup_cfg, true, 9) {
        Ok(n) if n >= 9 => {
            let mut hdr = [0u8; 9];
            read_transfer_data(ctrl, &mut hdr, 9);
            u16::from_le_bytes([hdr[2], hdr[3]])
        }
        _ => {
            crate::serial_println!("  EHCI: device {} — config descriptor header failed", new_addr);
            return;
        }
    };

    // Step 5: GET_DESCRIPTOR (full config)
    let config_len = total_len.min(256);
    let setup_cfg_full = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_CONFIG,
        w_index: 0,
        w_length: config_len,
    };

    let mut config_buf = [0u8; 256];
    match control_transfer(ctrl, new_addr, speed, max_packet, &setup_cfg_full, true, config_len) {
        Ok(n) if n >= 9 => {
            read_transfer_data(ctrl, &mut config_buf, n);
        }
        _ => {
            crate::serial_println!("  EHCI: device {} — full config descriptor failed", new_addr);
            return;
        }
    }

    let interfaces = parse_config(&config_buf[..config_len as usize]);

    // Step 6: SET_CONFIGURATION
    let config_val = config_buf[5];
    let setup_setcfg = SetupPacket {
        bm_request_type: DIR_HOST_TO_DEVICE,
        b_request: REQ_SET_CONFIGURATION,
        w_value: config_val as u16,
        w_index: 0,
        w_length: 0,
    };

    if let Err(e) = control_transfer(ctrl, new_addr, speed, max_packet, &setup_setcfg, false, 0) {
        crate::serial_println!("  EHCI: device {} — SET_CONFIGURATION failed: {}", new_addr, e);
        return;
    }

    let dev_class = if dev_desc.b_device_class != 0 {
        dev_desc.b_device_class
    } else {
        interfaces.first().map(|i| i.class).unwrap_or(0)
    };

    let dev_subclass = if dev_desc.b_device_sub_class != 0 {
        dev_desc.b_device_sub_class
    } else {
        interfaces.first().map(|i| i.subclass).unwrap_or(0)
    };

    let dev_protocol = if dev_desc.b_device_protocol != 0 {
        dev_desc.b_device_protocol
    } else {
        interfaces.first().map(|i| i.protocol).unwrap_or(0)
    };

    let usb_dev = UsbDevice {
        address: new_addr,
        speed,
        port,
        controller: ControllerType::Ehci,
        max_packet_size: max_packet,
        vendor_id: dev_desc.id_vendor,
        product_id: dev_desc.id_product,
        class: dev_class,
        subclass: dev_subclass,
        protocol: dev_protocol,
        num_configs: dev_desc.b_num_configurations,
        interfaces,
    };

    register_device(usb_dev);
}

// ── Port Reset + Scan ──────────────────────────

fn reset_port(ctrl: &EhciController, port_idx: u8) -> Option<UsbSpeed> {
    let port_offset = OP_PORTSC_BASE + (port_idx as u32) * 4;

    // Check connection
    let portsc = mmio_read32(ctrl.op_base, port_offset);
    if portsc & PORTSC_CCS == 0 {
        return None;
    }

    // Clear status change bits
    mmio_write32(ctrl.op_base, port_offset, portsc | PORTSC_CSC | PORTSC_PEC);

    // Port reset: set PR, wait, clear PR
    let portsc = mmio_read32(ctrl.op_base, port_offset);
    mmio_write32(ctrl.op_base, port_offset, (portsc | PORTSC_PR) & !PORTSC_PE);
    delay_ms(50);

    let portsc = mmio_read32(ctrl.op_base, port_offset);
    mmio_write32(ctrl.op_base, port_offset, portsc & !PORTSC_PR);
    delay_ms(10);

    // Check if port enabled (high-speed handshake succeeded)
    let portsc = mmio_read32(ctrl.op_base, port_offset);
    if portsc & PORTSC_PE != 0 {
        // High-speed device
        Some(UsbSpeed::High)
    } else {
        // Not high-speed — release to companion controller
        let portsc = mmio_read32(ctrl.op_base, port_offset);
        mmio_write32(ctrl.op_base, port_offset, portsc | PORTSC_PO);
        crate::serial_println!(
            "  EHCI: port {} — not high-speed, released to companion",
            port_idx + 1
        );
        None
    }
}

fn scan_ports(ctrl: &EhciController) {
    for i in 0..ctrl.n_ports {
        let port_offset = OP_PORTSC_BASE + (i as u32) * 4;
        let portsc = mmio_read32(ctrl.op_base, port_offset);

        if portsc & PORTSC_CCS == 0 {
            crate::serial_println!("  EHCI: port {} — no device", i + 1);
            continue;
        }

        crate::serial_println!("  EHCI: port {} — device connected, resetting...", i + 1);

        if let Some(speed) = reset_port(ctrl, i) {
            crate::serial_println!(
                "  EHCI: port {} — enabled (High-Speed)",
                i + 1
            );
            delay_ms(10);
            enumerate_device(ctrl, i + 1, speed);
        }
    }
}

// ── Controller Init ────────────────────────────

pub fn init_controller(pci: &PciDevice) {
    let bar0 = pci.bars[0];
    if bar0 == 0 {
        crate::serial_println!("  EHCI: BAR0 is zero, cannot initialize");
        return;
    }
    let phys_base = (bar0 & 0xFFFFF000) as u64;

    crate::serial_println!(
        "  EHCI: controller at phys {:#010x}, IRQ {}",
        phys_base, pci.interrupt_line
    );

    // Map MMIO pages
    use crate::memory::address::VirtAddr;
    for i in 0..EHCI_MMIO_PAGES {
        let virt = EHCI_MMIO_BASE + (i as u64) * 4096;
        let phys = phys_base + (i as u64) * 4096;
        virtual_mem::map_page(VirtAddr(virt), PhysAddr(phys), 0x03);
    }

    let mmio_base = EHCI_MMIO_BASE;

    // Read capability length
    let caplength = mmio_read8(mmio_base, CAP_CAPLENGTH) as u64;
    let op_base = mmio_base + caplength;

    // Read structural parameters
    let hcsparams = mmio_read32(mmio_base, CAP_HCSPARAMS);
    let n_ports = ((hcsparams & 0x0F) as u8).min(EHCI_MAX_PORTS as u8);

    crate::serial_println!(
        "  EHCI: CAPLENGTH={}, {} port(s)",
        caplength, n_ports
    );

    // Enable bus mastering + memory space
    let cmd = pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x06);

    // Take ownership from BIOS (EECP/USBLEGSUP)
    let hccparams = mmio_read32(mmio_base, CAP_HCCPARAMS);
    let eecp = ((hccparams >> 8) & 0xFF) as u8;
    if eecp >= 0x40 {
        let legsup = pci_config_read32(pci.bus, pci.device, pci.function, eecp);
        if legsup & (1 << 16) != 0 {
            // BIOS owns it — request ownership
            pci_config_write32(
                pci.bus, pci.device, pci.function, eecp,
                legsup | (1 << 24) // Set OS ownership
            );
            // Wait for BIOS to release
            for _ in 0..100 {
                let val = pci_config_read32(pci.bus, pci.device, pci.function, eecp);
                if val & (1 << 16) == 0 {
                    break;
                }
                delay_ms(10);
            }
        }
    }

    // Stop controller
    let usbcmd = mmio_read32(op_base, OP_USBCMD);
    mmio_write32(op_base, OP_USBCMD, usbcmd & !CMD_RUN);

    // Wait for halt
    for _ in 0..100 {
        if mmio_read32(op_base, OP_USBSTS) & STS_HALTED != 0 {
            break;
        }
        delay_ms(1);
    }

    // HC reset
    mmio_write32(op_base, OP_USBCMD, CMD_HCRESET);
    for _ in 0..100 {
        if mmio_read32(op_base, OP_USBCMD) & CMD_HCRESET == 0 {
            break;
        }
        delay_ms(1);
    }

    // Clear status
    mmio_write32(op_base, OP_USBSTS, 0x3F);

    // Allocate periodic frame list (1 page = 4 KiB = 1024 entries)
    let frame_list_phys = match physical::alloc_contiguous(1) {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_println!("  EHCI: failed to allocate frame list");
            return;
        }
    };

    // Allocate QH + qTD pool (1 page)
    let qh_page_phys = match physical::alloc_contiguous(1) {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_println!("  EHCI: failed to allocate QH/qTD pool");
            return;
        }
    };

    // Allocate data buffer (1 page)
    let data_buf_phys = match physical::alloc_contiguous(1) {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_println!("  EHCI: failed to allocate data buffer");
            return;
        }
    };

    // QH at start of page (aligned to 64), qTDs at +256
    let async_qh_phys = qh_page_phys;
    let td_pool_phys = qh_page_phys + 256;

    // Initialize async QH (self-linked, head of reclamation list)
    unsafe {
        let qh = async_qh_phys as *mut EhciQh;
        core::ptr::write_bytes(qh, 0, 1);
        (*qh).horiz_link = (async_qh_phys as u32) | QH_TYPE_QH; // circular
        (*qh).characteristics = (1 << 15) | (64 << 16); // H=1, MaxPacket=64
        (*qh).capabilities = 1 << 30; // Mult=1
        (*qh).next_qtd = LP_T;
        (*qh).alt_next_qtd = LP_T;
    }

    // Initialize periodic frame list (all terminate)
    unsafe {
        let fl = frame_list_phys as *mut u32;
        for i in 0..1024 {
            *fl.add(i) = LP_T;
        }
    }

    // Zero data buffer
    unsafe {
        core::ptr::write_bytes(data_buf_phys as *mut u8, 0, 4096);
    }

    // Set periodic frame list base
    mmio_write32(op_base, OP_PERIODICLISTBASE, frame_list_phys as u32);

    // Set async list address
    mmio_write32(op_base, OP_ASYNCLISTADDR, async_qh_phys as u32);

    // Set frame index to 0
    mmio_write32(op_base, OP_FRINDEX, 0);

    // Disable interrupts (we poll)
    mmio_write32(op_base, OP_USBINTR, 0);

    // Set CONFIGFLAG = 1 (route all ports to EHCI)
    mmio_write32(op_base, OP_CONFIGFLAG, 1);

    // Start controller with async schedule enabled
    mmio_write32(op_base, OP_USBCMD, CMD_RUN | CMD_ASYNC_ENABLE | CMD_ITC_8);

    // Wait for controller to start
    delay_ms(10);

    let sts = mmio_read32(op_base, OP_USBSTS);
    if sts & STS_HALTED != 0 {
        crate::serial_println!("  EHCI: controller failed to start (STS={:#010x})", sts);
        return;
    }

    // Allow ports to settle after CONFIGFLAG
    delay_ms(100);

    crate::serial_println!("  EHCI: controller running");

    let mut ctrl = EhciController {
        mmio_base,
        op_base,
        n_ports,
        async_qh_phys,
        td_pool_phys,
        data_buf_phys,
        port_connected: [false; EHCI_MAX_PORTS],
    };

    scan_ports(&ctrl);

    // Record which ports have devices connected
    for i in 0..n_ports as usize {
        let port_offset = OP_PORTSC_BASE + (i as u32) * 4;
        let portsc = mmio_read32(op_base, port_offset);
        ctrl.port_connected[i] = portsc & PORTSC_CCS != 0;
    }

    // Store controller state for hot-plug polling
    *EHCI_CTRL.lock() = Some(ctrl);
}

/// Poll EHCI ports for hot-plug events. Called periodically from the USB poll thread.
pub fn poll_ports() {
    let mut guard = EHCI_CTRL.lock();
    let ctrl = match guard.as_mut() {
        Some(c) => c,
        None => return,
    };

    for i in 0..ctrl.n_ports as usize {
        let port_offset = OP_PORTSC_BASE + (i as u32) * 4;
        let portsc = mmio_read32(ctrl.op_base, port_offset);
        let connected = portsc & PORTSC_CCS != 0;
        let was_connected = ctrl.port_connected[i];

        if connected && !was_connected {
            // New device connected
            crate::serial_println!("  EHCI: hot-plug — device connected on port {}", i + 1);
            ctrl.port_connected[i] = true;

            // Clear status change bits (write-1-to-clear)
            mmio_write32(ctrl.op_base, port_offset, portsc | PORTSC_CSC | PORTSC_PEC);

            if let Some(speed) = reset_port(ctrl, i as u8) {
                delay_ms(10);
                enumerate_device(ctrl, (i + 1) as u8, speed);
            }
        } else if !connected && was_connected {
            // Device disconnected
            crate::serial_println!("  EHCI: hot-unplug — device removed from port {}", i + 1);
            ctrl.port_connected[i] = false;

            // Clear status change bits
            mmio_write32(ctrl.op_base, port_offset, portsc | PORTSC_CSC | PORTSC_PEC);
        }
    }
}

/// Public control transfer for HID polling. Locks EHCI_CTRL internally.
/// Returns the data read from the device on success.
pub fn hid_control_transfer(
    dev_addr: u8,
    speed: UsbSpeed,
    max_packet: u16,
    setup: &SetupPacket,
    data_in: bool,
    data_len: u16,
) -> Result<alloc::vec::Vec<u8>, &'static str> {
    let guard = EHCI_CTRL.lock();
    let ctrl = guard.as_ref().ok_or("EHCI not initialized")?;
    let bytes = control_transfer(ctrl, dev_addr, speed, max_packet, setup, data_in, data_len)?;
    if data_in && bytes > 0 {
        let mut buf = alloc::vec![0u8; bytes];
        read_transfer_data(ctrl, &mut buf, bytes);
        Ok(buf)
    } else {
        Ok(alloc::vec::Vec::new())
    }
}

//! UHCI (Universal Host Controller Interface) driver — USB 1.x.
//!
//! I/O port based controller. Uses BAR4 for register access.
//! Frame list (1024 entries) provides 1ms scheduling granularity.
//! QH/TD structures in identity-mapped DMA memory.

use crate::arch::x86::pit::delay_ms;
use crate::arch::x86::port::{inw, outw, inl, outl};
use crate::drivers::pci::{PciDevice, pci_config_read32, pci_config_write32};
use crate::memory::address::PhysAddr;
use crate::memory::physical;
use super::*;

// ── UHCI I/O Registers (offsets from BAR4) ───────

const REG_USBCMD: u16 = 0x00;
const REG_USBSTS: u16 = 0x02;
const REG_USBINTR: u16 = 0x04;
const REG_FRNUM: u16 = 0x06;
const REG_FRBASEADD: u16 = 0x08;
const REG_PORTSC1: u16 = 0x10;
const REG_PORTSC2: u16 = 0x12;

// USBCMD bits
const CMD_RUN: u16 = 1 << 0;
const CMD_HCRESET: u16 = 1 << 1;
const CMD_GRESET: u16 = 1 << 2;
const CMD_MAXP: u16 = 1 << 7; // Max packet = 64 bytes

// USBSTS bits
const STS_HALTED: u16 = 1 << 5;
const STS_INT: u16 = 1 << 0;
const STS_ERR_INT: u16 = 1 << 1;

// PORTSC bits
const PORT_CCS: u16 = 1 << 0;   // Current Connect Status
const PORT_CSC: u16 = 1 << 1;   // Connect Status Change
const PORT_PE: u16 = 1 << 2;    // Port Enabled
const PORT_PEC: u16 = 1 << 3;   // Port Enable Change
const PORT_PR: u16 = 1 << 9;    // Port Reset
const PORT_LSDA: u16 = 1 << 8;  // Low-Speed Device Attached

// TD ctrl_status bits
const TD_ACTIVE: u32 = 1 << 23;
const TD_IOC: u32 = 1 << 24;
const TD_SPD: u32 = 1 << 29;
const TD_ERR_MASK: u32 = 0x007E_0000; // error bits 22-17

// Link pointer flags
const LP_TERMINATE: u32 = 1 << 0;
const LP_QH: u32 = 1 << 1;
const LP_DEPTH: u32 = 1 << 2; // Depth-first: process linked TD in same frame

// ── DMA Structures ──────────────────────────────

/// UHCI Transfer Descriptor (32 bytes).
#[repr(C)]
struct UhciTd {
    link_ptr: u32,
    ctrl_status: u32,
    token: u32,
    buffer_ptr: u32,
    _sw: [u32; 4], // pad to 32 bytes
}

/// UHCI Queue Head (16 bytes).
#[repr(C)]
struct UhciQh {
    head_link: u32,
    element_link: u32,
    _sw: [u32; 2], // pad to 16 bytes
}

// ── Controller State ────────────────────────────

struct UhciController {
    io_base: u16,
    frame_list_phys: u64,
    qh_phys: u64,
    td_pool_phys: u64,
    data_buf_phys: u64,
    port_connected: [bool; 2],
}

static UHCI_CTRL: crate::sync::spinlock::Spinlock<Option<UhciController>> =
    crate::sync::spinlock::Spinlock::new(None);

fn reg_read16(base: u16, offset: u16) -> u16 {
    unsafe { inw(base + offset) }
}

fn reg_write16(base: u16, offset: u16, val: u16) {
    unsafe { outw(base + offset, val) }
}

fn reg_read32(base: u16, offset: u16) -> u32 {
    unsafe { inl(base + offset) }
}

fn reg_write32(base: u16, offset: u16, val: u32) {
    unsafe { outl(base + offset, val) }
}

// ── Token Builder ───────────────────────────────

fn make_token(pid: u8, dev_addr: u8, endpoint: u8, toggle: u8, max_len: u16) -> u32 {
    let max_len_field = if max_len == 0 { 0x7FF } else { (max_len - 1) as u32 & 0x7FF };
    (max_len_field << 21)
        | ((toggle as u32 & 1) << 19)
        | ((endpoint as u32 & 0xF) << 15)
        | ((dev_addr as u32 & 0x7F) << 8)
        | (pid as u32)
}

// ── Control Transfer ────────────────────────────

/// Execute a control transfer. Returns number of bytes received in `data_buf`.
fn control_transfer(
    ctrl: &UhciController,
    dev_addr: u8,
    setup: &SetupPacket,
    data_in: bool,
    data_len: u16,
) -> Result<usize, &'static str> {
    let td_base = ctrl.td_pool_phys;
    let data_buf = ctrl.data_buf_phys;

    // We have room for up to 8 TDs in a page
    // TD 0 = Setup, TD 1..N = Data, TD N+1 = Status

    let setup_bytes: [u8; 8] = unsafe { core::mem::transmute_copy(setup) };

    // Copy setup packet to data buffer (first 8 bytes)
    let setup_buf_phys = data_buf;
    unsafe {
        let ptr = setup_buf_phys as *mut [u8; 8];
        core::ptr::write_volatile(ptr, setup_bytes);
    }

    // Data buffer starts at offset 64
    let data_phys = data_buf + 64;

    // If data_out, copy data is handled by caller before calling this function
    // For data_in, we'll read after transfer completes

    // Clear any pending status bits before starting
    reg_write16(ctrl.io_base, REG_USBSTS, 0xFFFF);

    let mut td_count = 0u32;
    let max_packet = 8u16; // Default for endpoint 0 before we know better

    // Setup TD
    let setup_td = td_base;
    let next_td_phys = td_base + 32;
    unsafe {
        let td = setup_td as *mut UhciTd;
        (*td).link_ptr = next_td_phys as u32 | LP_DEPTH;
        (*td).ctrl_status = TD_ACTIVE | (3 << 27); // Active, 3 error retries
        (*td).token = make_token(PID_SETUP, dev_addr, 0, 0, 8);
        (*td).buffer_ptr = setup_buf_phys as u32;
    }
    td_count += 1;

    // Data TDs
    let mut remaining = data_len;
    let mut offset = 0u16;
    let mut toggle = 1u8;
    let pid = if data_in { PID_IN } else { PID_OUT };

    while remaining > 0 {
        let chunk = remaining.min(max_packet);
        let this_td = td_base + (td_count as u64) * 32;
        let next = if remaining <= chunk {
            td_base + ((td_count + 1) as u64) * 32 // status TD
        } else {
            td_base + ((td_count + 1) as u64) * 32
        };

        unsafe {
            let td = this_td as *mut UhciTd;
            (*td).link_ptr = next as u32 | LP_DEPTH;
            (*td).ctrl_status = TD_ACTIVE | (3 << 27);
            if data_in { (*td).ctrl_status |= TD_SPD; }
            (*td).token = make_token(pid, dev_addr, 0, toggle, chunk);
            (*td).buffer_ptr = (data_phys + offset as u64) as u32;
        }

        toggle ^= 1;
        offset += chunk;
        remaining -= chunk;
        td_count += 1;
    }

    // Status TD: PID is opposite of data direction.
    // For IN transfers (data_in=true): status = OUT
    // For OUT transfers (data_in=false) or no-data (SET_ADDRESS): status = IN
    let status_pid = if data_in { PID_OUT } else { PID_IN };
    let status_td = td_base + (td_count as u64) * 32;
    unsafe {
        let td = status_td as *mut UhciTd;
        (*td).link_ptr = LP_TERMINATE;
        (*td).ctrl_status = TD_ACTIVE | TD_IOC | (3 << 27);
        (*td).token = make_token(status_pid, dev_addr, 0, 1, 0); // zero-length status
        (*td).buffer_ptr = 0;
    }
    td_count += 1;

    // Point QH to first TD
    unsafe {
        let qh = ctrl.qh_phys as *mut UhciQh;
        (*qh).element_link = setup_td as u32;
    }

    // Wait for completion (poll status TD)
    let timeout = 500u32; // ms
    let start = crate::arch::x86::pit::get_ticks();

    loop {
        let status_ctrl = unsafe {
            let td = status_td as *mut UhciTd;
            core::ptr::read_volatile(&(*td).ctrl_status)
        };

        if status_ctrl & TD_ACTIVE == 0 {
            // Check for errors
            if status_ctrl & TD_ERR_MASK != 0 {
                return Err("USB transfer error");
            }
            break;
        }

        if crate::arch::x86::pit::get_ticks().wrapping_sub(start) > timeout {
            // Deactivate
            unsafe {
                let qh = ctrl.qh_phys as *mut UhciQh;
                (*qh).element_link = LP_TERMINATE;
            }
            return Err("USB transfer timeout");
        }

        core::hint::spin_loop();
    }

    // Deactivate QH
    unsafe {
        let qh = ctrl.qh_phys as *mut UhciQh;
        (*qh).element_link = LP_TERMINATE;
    }

    // Calculate actual bytes transferred from data TDs
    let mut total = 0usize;
    for i in 1..td_count.saturating_sub(1) {
        let td_phys = td_base + (i as u64) * 32;
        let ctrl_status = unsafe {
            let td = td_phys as *mut UhciTd;
            core::ptr::read_volatile(&(*td).ctrl_status)
        };
        let actual = ((ctrl_status + 1) & 0x7FF) as usize;
        total += actual;
    }

    Ok(total)
}

/// Read data from data buffer after a control transfer.
fn read_transfer_data(ctrl: &UhciController, buf: &mut [u8], len: usize) {
    let data_phys = ctrl.data_buf_phys + 64;
    let to_copy = len.min(buf.len());
    unsafe {
        let src = data_phys as *const u8;
        core::ptr::copy_nonoverlapping(src, buf.as_mut_ptr(), to_copy);
    }
}

// ── Device Enumeration ──────────────────────────

fn enumerate_device(ctrl: &UhciController, port: u8, speed: UsbSpeed) {
    // Step 1: GET_DESCRIPTOR (first 8 bytes) to address 0
    let setup = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_DEVICE,
        w_index: 0,
        w_length: 8,
    };

    match control_transfer(ctrl, 0, &setup, true, 8) {
        Ok(n) if n >= 8 => {}
        Ok(_) => {
            crate::serial_println!("  UHCI: port {} — short device descriptor", port);
            return;
        }
        Err(e) => {
            crate::serial_println!("  UHCI: port {} — GET_DESCRIPTOR(8) failed: {}", port, e);
            return;
        }
    }

    let mut desc_buf = [0u8; 18];
    read_transfer_data(ctrl, &mut desc_buf, 8);
    let max_packet = desc_buf[7] as u16;
    if max_packet == 0 {
        crate::serial_println!("  UHCI: port {} — invalid max packet size 0", port);
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

    if let Err(e) = control_transfer(ctrl, 0, &setup_addr, false, 0) {
        crate::serial_println!("  UHCI: port {} — SET_ADDRESS failed: {}", port, e);
        return;
    }
    delay_ms(20); // Device needs time to change address (QEMU requires longer)

    // Step 3: GET_DESCRIPTOR (full 18 bytes) at new address
    let setup_full = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_DEVICE,
        w_index: 0,
        w_length: 18,
    };

    match control_transfer(ctrl, new_addr, &setup_full, true, 18) {
        Ok(n) if n >= 18 => {}
        Ok(n) => {
            crate::serial_println!("  UHCI: device {} — short descriptor ({} bytes)", new_addr, n);
            return;
        }
        Err(e) => {
            crate::serial_println!("  UHCI: device {} — GET_DESCRIPTOR(18) failed: {}", new_addr, e);
            return;
        }
    }

    read_transfer_data(ctrl, &mut desc_buf, 18);
    let dev_desc: DeviceDescriptor = unsafe { core::ptr::read_unaligned(desc_buf.as_ptr() as *const _) };

    // Step 4: GET_DESCRIPTOR (config, header first)
    let setup_cfg = SetupPacket {
        bm_request_type: DIR_DEVICE_TO_HOST,
        b_request: REQ_GET_DESCRIPTOR,
        w_value: DESC_CONFIG,
        w_index: 0,
        w_length: 9,
    };

    let total_len = match control_transfer(ctrl, new_addr, &setup_cfg, true, 9) {
        Ok(n) if n >= 9 => {
            let mut hdr = [0u8; 9];
            read_transfer_data(ctrl, &mut hdr, 9);
            u16::from_le_bytes([hdr[2], hdr[3]])
        }
        _ => {
            crate::serial_println!("  UHCI: device {} — config descriptor header failed", new_addr);
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
    match control_transfer(ctrl, new_addr, &setup_cfg_full, true, config_len) {
        Ok(n) if n >= 9 => {
            read_transfer_data(ctrl, &mut config_buf, n);
        }
        _ => {
            crate::serial_println!("  UHCI: device {} — full config descriptor failed", new_addr);
            return;
        }
    }

    let interfaces = parse_config(&config_buf[..config_len as usize]);

    // Step 6: SET_CONFIGURATION
    let config_val = config_buf[5]; // bConfigurationValue
    let setup_setcfg = SetupPacket {
        bm_request_type: DIR_HOST_TO_DEVICE,
        b_request: REQ_SET_CONFIGURATION,
        w_value: config_val as u16,
        w_index: 0,
        w_length: 0,
    };

    if let Err(e) = control_transfer(ctrl, new_addr, &setup_setcfg, false, 0) {
        crate::serial_println!("  UHCI: device {} — SET_CONFIGURATION failed: {}", new_addr, e);
        return;
    }

    // Determine device class from device descriptor or first interface
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

    // Register USB device
    let usb_dev = UsbDevice {
        address: new_addr,
        speed,
        port,
        controller: ControllerType::Uhci,
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

// ── Port Reset + Scan ───────────────────────────

fn reset_port(io_base: u16, port_reg: u16) -> bool {
    // Set port reset
    let val = reg_read16(io_base, port_reg);
    reg_write16(io_base, port_reg, val | PORT_PR);
    delay_ms(50);

    // Clear port reset
    let val = reg_read16(io_base, port_reg);
    reg_write16(io_base, port_reg, val & !PORT_PR);
    delay_ms(10);

    // Clear status change bits
    let val = reg_read16(io_base, port_reg);
    reg_write16(io_base, port_reg, val | PORT_CSC | PORT_PEC);

    // Enable port
    let val = reg_read16(io_base, port_reg);
    if val & PORT_PE == 0 {
        reg_write16(io_base, port_reg, val | PORT_PE);
        delay_ms(10);
    }

    let val = reg_read16(io_base, port_reg);
    val & PORT_PE != 0
}

fn scan_ports(ctrl: &UhciController) {
    let ports = [REG_PORTSC1, REG_PORTSC2];

    for (i, &port_reg) in ports.iter().enumerate() {
        let status = reg_read16(ctrl.io_base, port_reg);

        if status & PORT_CCS == 0 {
            crate::serial_println!("  UHCI: port {} — no device", i + 1);
            continue;
        }

        let speed = if status & PORT_LSDA != 0 {
            UsbSpeed::Low
        } else {
            UsbSpeed::Full
        };

        crate::serial_println!(
            "  UHCI: port {} — device connected ({})",
            i + 1, if speed == UsbSpeed::Low { "Low-Speed" } else { "Full-Speed" }
        );

        if !reset_port(ctrl.io_base, port_reg) {
            crate::serial_println!("  UHCI: port {} — reset failed (port not enabled)", i + 1);
            continue;
        }

        delay_ms(10);
        enumerate_device(ctrl, (i + 1) as u8, speed);
    }
}

// ── Controller Init ─────────────────────────────

pub fn init_controller(pci: &PciDevice) {
    // BAR4 = I/O base for UHCI
    let bar4 = pci.bars[4];
    if bar4 == 0 {
        crate::serial_println!("  UHCI: BAR4 is zero, cannot initialize");
        return;
    }
    let io_base = (bar4 & 0xFFFC) as u16;

    crate::serial_println!("  UHCI: controller at I/O {:#06x}, IRQ {}", io_base, pci.interrupt_line);

    // Enable bus mastering + I/O space
    let cmd = pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x05);

    // Disable BIOS legacy support (PCI config offset 0xC0 = LEGSUP)
    pci_config_write32(pci.bus, pci.device, pci.function, 0xC0, 0x8F00);

    // Stop controller
    reg_write16(io_base, REG_USBCMD, 0);
    delay_ms(1);

    // Wait for halt
    for _ in 0..100 {
        if reg_read16(io_base, REG_USBSTS) & STS_HALTED != 0 {
            break;
        }
        delay_ms(1);
    }

    // Global reset
    reg_write16(io_base, REG_USBCMD, CMD_GRESET);
    delay_ms(50);
    reg_write16(io_base, REG_USBCMD, 0);
    delay_ms(10);

    // HC reset
    reg_write16(io_base, REG_USBCMD, CMD_HCRESET);
    for _ in 0..100 {
        if reg_read16(io_base, REG_USBCMD) & CMD_HCRESET == 0 {
            break;
        }
        delay_ms(1);
    }

    // Clear status
    reg_write16(io_base, REG_USBSTS, 0xFFFF);

    // Allocate frame list (1 page = 4 KiB = 1024 entries)
    let frame_list_phys = match physical::alloc_contiguous(1) {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_println!("  UHCI: failed to allocate frame list");
            return;
        }
    };

    // Allocate QH + TD pool (1 page for QH + TDs)
    let qh_page_phys = match physical::alloc_contiguous(1) {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_println!("  UHCI: failed to allocate QH/TD pool");
            return;
        }
    };

    // Allocate data buffer (1 page)
    let data_buf_phys = match physical::alloc_contiguous(1) {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_println!("  UHCI: failed to allocate data buffer");
            return;
        }
    };

    // QH at start of page, TDs at +256
    let qh_phys = qh_page_phys;
    let td_pool_phys = qh_page_phys + 256;

    // Initialize QH
    unsafe {
        let qh = qh_phys as *mut UhciQh;
        (*qh).head_link = LP_TERMINATE;
        (*qh).element_link = LP_TERMINATE;
    }

    // Initialize frame list: all entries point to QH
    unsafe {
        let fl = frame_list_phys as *mut u32;
        for i in 0..1024 {
            *fl.add(i) = (qh_phys as u32) | LP_QH;
        }
    }

    // Zero data buffer
    unsafe {
        let ptr = data_buf_phys as *mut u8;
        core::ptr::write_bytes(ptr, 0, 4096);
    }

    // Set frame list base
    reg_write32(io_base, REG_FRBASEADD, frame_list_phys as u32);

    // Set frame number to 0
    reg_write16(io_base, REG_FRNUM, 0);

    // Disable interrupts (we poll)
    reg_write16(io_base, REG_USBINTR, 0);

    // Start controller: run + max packet 64
    reg_write16(io_base, REG_USBCMD, CMD_RUN | CMD_MAXP);

    // Wait for controller to start
    delay_ms(10);

    let sts = reg_read16(io_base, REG_USBSTS);
    if sts & STS_HALTED != 0 {
        crate::serial_println!("  UHCI: controller failed to start (STS={:#06x})", sts);
        return;
    }

    crate::serial_println!("  UHCI: controller running");

    let mut ctrl = UhciController {
        io_base,
        frame_list_phys,
        qh_phys,
        td_pool_phys,
        data_buf_phys,
        port_connected: [false; 2],
    };

    // Scan ports for connected devices and record initial state
    scan_ports(&ctrl);

    // Record which ports have devices connected
    let ports = [REG_PORTSC1, REG_PORTSC2];
    for (i, &port_reg) in ports.iter().enumerate() {
        let status = reg_read16(io_base, port_reg);
        ctrl.port_connected[i] = status & PORT_CCS != 0;
    }

    // Store controller state for hot-plug polling
    *UHCI_CTRL.lock() = Some(ctrl);
}

/// Poll UHCI ports for hot-plug events. Called periodically from the USB poll thread.
pub fn poll_ports() {
    let mut guard = UHCI_CTRL.lock();
    let ctrl = match guard.as_mut() {
        Some(c) => c,
        None => return,
    };

    let ports = [REG_PORTSC1, REG_PORTSC2];
    for (i, &port_reg) in ports.iter().enumerate() {
        let status = reg_read16(ctrl.io_base, port_reg);
        let connected = status & PORT_CCS != 0;
        let was_connected = ctrl.port_connected[i];

        if connected && !was_connected {
            // New device connected
            crate::serial_println!("  UHCI: hot-plug — device connected on port {}", i + 1);
            ctrl.port_connected[i] = true;

            // Clear status change bits
            reg_write16(ctrl.io_base, port_reg, status | PORT_CSC | PORT_PEC);

            let speed = if status & PORT_LSDA != 0 {
                UsbSpeed::Low
            } else {
                UsbSpeed::Full
            };

            if reset_port(ctrl.io_base, port_reg) {
                delay_ms(10);
                enumerate_device(ctrl, (i + 1) as u8, speed);
            }
        } else if !connected && was_connected {
            // Device disconnected
            crate::serial_println!("  UHCI: hot-unplug — device removed from port {}", i + 1);
            ctrl.port_connected[i] = false;

            // Clear status change bits
            reg_write16(ctrl.io_base, port_reg, status | PORT_CSC | PORT_PEC);

            // TODO: remove_device() when we track port→address mapping
        }
    }
}

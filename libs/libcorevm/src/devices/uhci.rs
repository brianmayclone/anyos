//! UHCI (Universal Host Controller Interface) USB 1.1 controller emulation.
//!
//! Implements a minimal Intel PIIX3/PIIX4 UHCI controller (PCI device
//! 8086:7020) with an integrated USB HID tablet device for absolute
//! mouse positioning.
//!
//! The UHCI controller uses I/O port-based access (BAR4, 32 bytes).
//! USB transfers happen via a Frame List in guest RAM, with Transfer
//! Descriptors (TDs) and Queue Heads (QHs) linked in memory.
//!
//! For the tablet device, we intercept IN transfers on endpoint 1 and
//! return HID reports with absolute X/Y coordinates and button state.

use alloc::vec;
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use crate::io::IoHandler;
use crate::error::Result;

// ── UHCI Registers (I/O port offsets) ────────────────────────────────

const USBCMD: u16    = 0x00; // USB Command (R/W)
const USBSTS: u16    = 0x02; // USB Status (R/WC)
const USBINTR: u16   = 0x04; // USB Interrupt Enable (R/W)
const FRNUM: u16     = 0x06; // Frame Number (R/W)
const FLBASEADD: u16 = 0x08; // Frame List Base Address (R/W, 32-bit)
const SOFMOD: u16    = 0x0C; // Start of Frame Modify (R/W)
const PORTSC1: u16   = 0x10; // Port 1 Status/Control (R/WC)
const PORTSC2: u16   = 0x12; // Port 2 Status/Control (R/WC)

// USBCMD bits
const CMD_RS: u16    = 0x0001; // Run/Stop
const CMD_HCRESET: u16 = 0x0002; // Host Controller Reset
const CMD_GRESET: u16 = 0x0004; // Global Reset
const CMD_MAXP: u16  = 0x0080; // Max Packet (0=32, 1=64)

// USBSTS bits
const STS_USBINT: u16  = 0x0001; // USB Interrupt
const STS_USBERR: u16  = 0x0002; // USB Error Interrupt
const STS_RD: u16      = 0x0004; // Resume Detect
const STS_HSE: u16     = 0x0008; // Host System Error
const STS_HCPE: u16    = 0x0010; // Host Controller Process Error
const STS_HCH: u16     = 0x0020; // HC Halted

// PORTSC bits
const PORT_CCS: u16    = 0x0001; // Current Connect Status
const PORT_CSC: u16    = 0x0002; // Connect Status Change
const PORT_PE: u16     = 0x0004; // Port Enable
const PORT_PEC: u16    = 0x0008; // Port Enable Change
const PORT_LSDA: u16   = 0x0100; // Low Speed Device Attached
const PORT_PR: u16     = 0x0200; // Port Reset

// Transfer Descriptor status bits
const TD_STATUS_ACTIVE: u32 = 1 << 23;
const TD_STATUS_IOC: u32    = 1 << 24;

// ── USB HID Tablet Report ────────────────────────────────────────────

/// 6-byte HID report: buttons(1) + x_lo(1) + x_hi(1) + y_lo(1) + y_hi(1) + wheel(1)
/// X/Y are absolute 0-32767.
#[derive(Clone, Copy)]
struct TabletReport {
    buttons: u8,
    x: u16,   // 0..32767
    y: u16,   // 0..32767
}

impl TabletReport {
    fn to_bytes(&self) -> [u8; 6] {
        [
            self.buttons,
            self.x as u8,
            (self.x >> 8) as u8,
            self.y as u8,
            (self.y >> 8) as u8,
            0, // wheel
        ]
    }
}

// ── USB Descriptors ──────────────────────────────────────────────────

// Device descriptor (18 bytes)
const USB_DEVICE_DESC: [u8; 18] = [
    18,          // bLength
    0x01,        // bDescriptorType = DEVICE
    0x10, 0x01,  // bcdUSB = 1.10
    0x00,        // bDeviceClass
    0x00,        // bDeviceSubClass
    0x00,        // bDeviceProtocol
    0x08,        // bMaxPacketSize0 = 8
    0x27, 0x06,  // idVendor = 0x0627 (QEMU tablet)
    0x01, 0x00,  // idProduct = 0x0001
    0x00, 0x01,  // bcdDevice = 1.00
    0x01,        // iManufacturer = 1
    0x02,        // iProduct = 2
    0x00,        // iSerialNumber = 0
    0x01,        // bNumConfigurations = 1
];

// Configuration descriptor (9) + Interface descriptor (9) + HID descriptor (9) + Endpoint descriptor (7) = 34 bytes
const USB_CONFIG_DESC: [u8; 34] = [
    // Configuration descriptor
    9,           // bLength
    0x02,        // bDescriptorType = CONFIGURATION
    34, 0x00,    // wTotalLength = 34
    0x01,        // bNumInterfaces = 1
    0x01,        // bConfigurationValue = 1
    0x00,        // iConfiguration = 0
    0xA0,        // bmAttributes = bus-powered, remote wakeup
    50,          // bMaxPower = 100mA
    // Interface descriptor
    9,           // bLength
    0x04,        // bDescriptorType = INTERFACE
    0x00,        // bInterfaceNumber = 0
    0x00,        // bAlternateSetting = 0
    0x01,        // bNumEndpoints = 1
    0x03,        // bInterfaceClass = HID
    0x00,        // bInterfaceSubClass = 0 (no boot)
    0x00,        // bInterfaceProtocol = 0
    0x00,        // iInterface = 0
    // HID descriptor
    9,           // bLength
    0x21,        // bDescriptorType = HID
    0x01, 0x01,  // bcdHID = 1.01
    0x00,        // bCountryCode = 0
    0x01,        // bNumDescriptors = 1
    0x22,        // bDescriptorType = REPORT
    // wDescriptorLength = length of HID_REPORT_DESC (below)
    HID_REPORT_DESC_LEN as u8, 0x00,
    // Endpoint descriptor (IN, endpoint 1, interrupt)
    7,           // bLength
    0x05,        // bDescriptorType = ENDPOINT
    0x81,        // bEndpointAddress = 1 IN
    0x03,        // bmAttributes = Interrupt
    0x06, 0x00,  // wMaxPacketSize = 6
    10,          // bInterval = 10ms
];

const HID_REPORT_DESC_LEN: usize = 53;

// HID Report Descriptor for absolute tablet (X/Y 0-32767, 3 buttons)
const HID_REPORT_DESC: [u8; HID_REPORT_DESC_LEN] = [
    0x05, 0x01,       // Usage Page (Generic Desktop)
    0x09, 0x02,       // Usage (Mouse)
    0xA1, 0x01,       // Collection (Application)
    0x09, 0x01,       //   Usage (Pointer)
    0xA1, 0x00,       //   Collection (Physical)
    // Buttons (3)
    0x05, 0x09,       //     Usage Page (Buttons)
    0x19, 0x01,       //     Usage Minimum (1)
    0x29, 0x03,       //     Usage Maximum (3)
    0x15, 0x00,       //     Logical Minimum (0)
    0x25, 0x01,       //     Logical Maximum (1)
    0x95, 0x03,       //     Report Count (3)
    0x75, 0x01,       //     Report Size (1)
    0x81, 0x02,       //     Input (Data, Variable, Absolute)
    // Padding (5 bits)
    0x95, 0x01,       //     Report Count (1)
    0x75, 0x05,       //     Report Size (5)
    0x81, 0x01,       //     Input (Constant)
    // X (16 bit, 0..32767)
    0x05, 0x01,       //     Usage Page (Generic Desktop)
    0x09, 0x30,       //     Usage (X)
    0x15, 0x00,       //     Logical Minimum (0)
    0x26, 0xFF, 0x7F, //     Logical Maximum (32767)
    0x75, 0x10,       //     Report Size (16)
    0x95, 0x01,       //     Report Count (1)
    0x81, 0x02,       //     Input (Data, Variable, Absolute)
    // Y (16 bit, 0..32767)
    0x09, 0x31,       //     Usage (Y)
    0x81, 0x02,       //     Input (Data, Variable, Absolute)
    0xC0,             //   End Collection
    0xC0,             // End Collection
];

// String descriptor 0 (language)
const USB_STRING_0: [u8; 4] = [4, 0x03, 0x09, 0x04]; // English US

// ── USB Setup Packet ─────────────────────────────────────────────────

#[derive(Default)]
struct SetupPacket {
    bm_request_type: u8,
    b_request: u8,
    w_value: u16,
    w_index: u16,
    w_length: u16,
}

impl SetupPacket {
    fn from_bytes(data: &[u8]) -> Self {
        if data.len() < 8 { return Self::default(); }
        SetupPacket {
            bm_request_type: data[0],
            b_request: data[1],
            w_value: u16::from_le_bytes([data[2], data[3]]),
            w_index: u16::from_le_bytes([data[4], data[5]]),
            w_length: u16::from_le_bytes([data[6], data[7]]),
        }
    }
}

// ── UHCI Controller ──────────────────────────────────────────────────

pub struct Uhci {
    // HC registers
    usbcmd: u16,
    usbsts: u16,
    usbintr: u16,
    frnum: u16,
    flbaseadd: u32,
    sofmod: u8,
    portsc: [u16; 2],

    // USB tablet state
    current_report: TabletReport,
    report_pending: bool,
    device_address: u8,      // assigned by SET_ADDRESS
    configured: bool,        // SET_CONFIGURATION received

    // Guest RAM access for DMA
    ram_ptr: *const u8,
    ram_size: usize,

    // IRQ pending flag (checked by poll_irqs)
    pub irq_pending: bool,

    // Frame processing counter
    frame_counter: u32,
}

unsafe impl Send for Uhci {}

impl Uhci {
    pub fn new() -> Self {
        Uhci {
            usbcmd: 0,
            usbsts: STS_HCH, // halted initially
            usbintr: 0,
            frnum: 0,
            flbaseadd: 0,
            sofmod: 64,
            portsc: [
                PORT_CCS, // Port 1: device connected (tablet)
                0,        // Port 2: empty
            ],
            current_report: TabletReport { buttons: 0, x: 0, y: 0 },
            report_pending: false,
            device_address: 0,
            configured: false,
            ram_ptr: core::ptr::null(),
            ram_size: 0,
            irq_pending: false,
            frame_counter: 0,
        }
    }

    pub fn set_guest_memory(&mut self, ptr: *mut u8, size: usize) {
        self.ram_ptr = ptr as *const u8;
        self.ram_size = size;
    }

    /// Update the tablet's absolute position and button state.
    /// x, y are in range 0..32767 (absolute screen coordinates).
    pub fn tablet_move(&mut self, x: u16, y: u16, buttons: u8) {
        self.current_report = TabletReport {
            buttons: buttons & 0x07,
            x: x.min(32767),
            y: y.min(32767),
        };
        self.report_pending = true;
    }

    /// Process one USB frame. Call at ~1kHz (every 1ms) or at least
    /// periodically. Walks the frame list and processes TDs.
    /// Returns true if an interrupt should be raised (IRQ pending).
    pub fn process_frame(&mut self) -> bool {
        if self.usbcmd & CMD_RS == 0 {
            return false; // controller halted
        }
        if self.flbaseadd == 0 || self.ram_ptr.is_null() {
            return false;
        }

        // Read frame list pointer for current frame
        let frame_idx = (self.frnum as u32) & 0x3FF;
        let fl_entry = self.read_guest_u32(self.flbaseadd as u64 + frame_idx as u64 * 4);

        if fl_entry & 1 != 0 {
            // Terminate bit set — no work for this frame
        } else {
            let is_qh = (fl_entry >> 1) & 1 != 0;
            let ptr = fl_entry & 0xFFFFFFF0;

            if is_qh {
                self.process_qh(ptr);
            } else {
                self.process_td(ptr);
            }
        }

        // Advance frame number
        self.frnum = self.frnum.wrapping_add(1) & 0x7FF;

        // Check if we should fire interrupt
        if self.usbsts & STS_USBINT != 0 && self.usbintr & 0x01 != 0 {
            self.irq_pending = true;
            return true;
        }
        false
    }

    fn process_qh(&mut self, qh_addr: u32) {
        if qh_addr == 0 || qh_addr as usize >= self.ram_size { return; }

        // QH: [0..3] = horizontal link, [4..7] = element link (first TD)
        let element_link = self.read_guest_u32(qh_addr as u64 + 4);
        if element_link & 1 != 0 { return; } // terminate

        let td_addr = element_link & 0xFFFFFFF0;
        self.process_td(td_addr);
    }

    fn process_td(&mut self, td_addr: u32) {
        if td_addr == 0 || td_addr as usize + 16 > self.ram_size { return; }

        // TD layout (16 bytes):
        // [0..3]  Link pointer
        // [4..7]  Control & Status
        // [8..11] Token
        // [12..15] Buffer pointer

        let ctrl_status = self.read_guest_u32(td_addr as u64 + 4);
        let token = self.read_guest_u32(td_addr as u64 + 8);
        let buffer_ptr = self.read_guest_u32(td_addr as u64 + 12);

        // Only process active TDs
        if ctrl_status & TD_STATUS_ACTIVE == 0 { return; }

        let pid = (token & 0xFF) as u8;
        let dev_addr = ((token >> 8) & 0x7F) as u8;
        let endpoint = ((token >> 15) & 0xF) as u8;
        let max_len = ((token >> 21) & 0x7FF) as u16;
        let actual_len = if max_len == 0x7FF { 0 } else { max_len + 1 };

        match pid {
            0x69 => { // IN token
                let response = self.handle_in(dev_addr, endpoint, actual_len);
                if let Some(data) = response {
                    // Write response data to buffer
                    let write_len = data.len().min(actual_len as usize);
                    self.write_guest(buffer_ptr as u64, &data[..write_len]);

                    // Update TD: clear active, set actual length
                    let new_status = (ctrl_status & !TD_STATUS_ACTIVE)
                        & !0x7FF // clear actual length field
                        | if write_len > 0 { (write_len as u32 - 1) & 0x7FF } else { 0x7FF };
                    self.write_guest_u32(td_addr as u64 + 4, new_status);

                    if ctrl_status & TD_STATUS_IOC != 0 {
                        self.usbsts |= STS_USBINT;
                    }
                }
            }
            0xE1 => { // OUT token
                if actual_len > 0 && buffer_ptr != 0 {
                    let mut data = vec![0u8; actual_len as usize];
                    self.read_guest(buffer_ptr as u64, &mut data);
                    self.handle_out(dev_addr, endpoint, &data);
                }
                // Clear active bit
                let new_status = ctrl_status & !TD_STATUS_ACTIVE & !0x7FF | 0x7FF;
                self.write_guest_u32(td_addr as u64 + 4, new_status);

                if ctrl_status & TD_STATUS_IOC != 0 {
                    self.usbsts |= STS_USBINT;
                }
            }
            0x2D => { // SETUP token
                if actual_len >= 8 && buffer_ptr != 0 {
                    let mut setup_data = [0u8; 8];
                    self.read_guest(buffer_ptr as u64, &mut setup_data);
                    self.handle_setup(dev_addr, &setup_data);
                }
                // Clear active bit
                let new_status = ctrl_status & !TD_STATUS_ACTIVE & !0x7FF | 0x7FF;
                self.write_guest_u32(td_addr as u64 + 4, new_status);
            }
            _ => {
                // Unknown PID — mark as error, clear active
                let new_status = ctrl_status & !TD_STATUS_ACTIVE;
                self.write_guest_u32(td_addr as u64 + 4, new_status);
            }
        }
    }

    fn handle_setup(&mut self, dev_addr: u8, data: &[u8]) {
        let pkt = SetupPacket::from_bytes(data);

        match (pkt.bm_request_type, pkt.b_request) {
            (0x00, 0x05) => {
                // SET_ADDRESS
                self.device_address = pkt.w_value as u8;
            }
            (0x00, 0x09) => {
                // SET_CONFIGURATION
                self.configured = pkt.w_value != 0;
            }
            (0x21, 0x0A) => {
                // SET_IDLE (HID class) — ignore
            }
            (0x21, 0x0B) => {
                // SET_PROTOCOL (HID class) — ignore
            }
            _ => {}
        }
    }

    fn handle_in(&mut self, dev_addr: u8, endpoint: u8, max_len: u16) -> Option<Vec<u8>> {
        // Only respond if addressed to our device (or address 0 during enumeration)
        if dev_addr != self.device_address && dev_addr != 0 && self.device_address != 0 {
            return None;
        }

        match endpoint {
            0 => {
                // Control endpoint — return pending descriptor response
                // (handled via setup_response mechanism)
                None
            }
            1 => {
                // Interrupt IN — HID tablet report
                if !self.configured { return None; }
                let report = self.current_report.to_bytes();
                self.report_pending = false;
                Some(report.to_vec())
            }
            _ => None,
        }
    }

    fn handle_out(&mut self, _dev_addr: u8, _endpoint: u8, _data: &[u8]) {
        // Tablet doesn't accept OUT data
    }

    fn reset(&mut self) {
        self.usbcmd = 0;
        self.usbsts = STS_HCH;
        self.usbintr = 0;
        self.frnum = 0;
        self.flbaseadd = 0;
        self.sofmod = 64;
        // Keep device connected on port 1
        self.portsc = [PORT_CCS, 0];
        self.device_address = 0;
        self.configured = false;
        self.irq_pending = false;
    }

    // ── Guest memory access ─────────────────────────────────────────

    fn read_guest_u32(&self, addr: u64) -> u32 {
        if self.ram_ptr.is_null() || addr as usize + 4 > self.ram_size { return 0; }
        let ptr = unsafe { self.ram_ptr.add(addr as usize) };
        unsafe { (ptr as *const u32).read_unaligned() }
    }

    fn write_guest_u32(&self, addr: u64, val: u32) {
        if self.ram_ptr.is_null() || addr as usize + 4 > self.ram_size { return; }
        let ptr = unsafe { (self.ram_ptr as *mut u8).add(addr as usize) };
        unsafe { (ptr as *mut u32).write_unaligned(val); }
    }

    fn read_guest(&self, addr: u64, buf: &mut [u8]) {
        if self.ram_ptr.is_null() || addr as usize + buf.len() > self.ram_size {
            buf.fill(0);
            return;
        }
        let src = unsafe { self.ram_ptr.add(addr as usize) };
        unsafe { core::ptr::copy_nonoverlapping(src, buf.as_mut_ptr(), buf.len()); }
    }

    fn write_guest(&self, addr: u64, data: &[u8]) {
        if self.ram_ptr.is_null() || addr as usize + data.len() > self.ram_size { return; }
        let dst = unsafe { (self.ram_ptr as *mut u8).add(addr as usize) };
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len()); }
    }
}

// ── I/O Handler (port-based access) ──────────────────────────────────

impl IoHandler for Uhci {
    fn read(&mut self, port: u16, size: u8) -> Result<u32> {
        let offset = port;
        let val = match size {
            2 => match offset {
                USBCMD    => self.usbcmd as u32,
                USBSTS    => self.usbsts as u32,
                USBINTR   => self.usbintr as u32,
                FRNUM     => self.frnum as u32,
                PORTSC1   => self.portsc[0] as u32,
                PORTSC2   => self.portsc[1] as u32,
                _ => 0,
            },
            4 => match offset {
                FLBASEADD => self.flbaseadd,
                _ => 0,
            },
            1 => match offset {
                SOFMOD    => self.sofmod as u32,
                _ => 0,
            },
            _ => 0,
        };
        Ok(val)
    }

    fn write(&mut self, port: u16, size: u8, val: u32) -> Result<()> {
        let offset = port;
        match size {
            2 => {
                let v = val as u16;
                match offset {
                    USBCMD => {
                        if v & CMD_HCRESET != 0 || v & CMD_GRESET != 0 {
                            self.reset();
                            return Ok(());
                        }
                        self.usbcmd = v;
                        if v & CMD_RS != 0 {
                            self.usbsts &= !STS_HCH;
                        } else {
                            self.usbsts |= STS_HCH;
                        }
                    }
                    USBSTS => {
                        self.usbsts &= !(v & 0x001F);
                        if self.usbsts & STS_USBINT == 0 {
                            self.irq_pending = false;
                        }
                    }
                    USBINTR => { self.usbintr = v; }
                    FRNUM   => { self.frnum = v & 0x7FF; }
                    PORTSC1 | 0x11 => { self.handle_portsc_write_16(0, v); }
                    PORTSC2 | 0x13 => { self.handle_portsc_write_16(1, v); }
                    _ => {}
                }
            }
            4 => {
                match offset {
                    FLBASEADD => { self.flbaseadd = val & 0xFFFFF000; }
                    _ => {}
                }
            }
            1 => {
                match offset {
                    SOFMOD => { self.sofmod = val as u8; }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl Uhci {
    fn handle_portsc_write_16(&mut self, port_idx: usize, val: u16) {
        let old = self.portsc[port_idx];

        // Handle port reset
        if val & PORT_PR != 0 && old & PORT_PR == 0 {
            self.portsc[port_idx] = (old & PORT_CCS) | PORT_PE | PORT_PEC;
            if port_idx == 0 {
                self.portsc[port_idx] |= PORT_CCS;
            }
            return;
        }

        // Write-to-clear CSC and PEC bits
        let clear_bits = val & (PORT_CSC | PORT_PEC);
        let mut new_val = old & !clear_bits;
        // PE is writable
        new_val = (new_val & !PORT_PE) | (val & PORT_PE);
        // CCS is read-only
        new_val = (new_val & !PORT_CCS) | (old & PORT_CCS);
        self.portsc[port_idx] = new_val;
    }
}

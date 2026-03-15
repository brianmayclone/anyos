//! UHCI (Universal Host Controller Interface) USB 1.1 controller emulation.
//!
//! Implements a minimal Intel PIIX3/PIIX4 UHCI controller with an integrated
//! USB HID tablet device for absolute mouse positioning.

use alloc::vec;
use alloc::vec::Vec;
use crate::io::IoHandler;
use crate::error::Result;

// ── UHCI Register offsets ────────────────────────────────────────────

const REG_USBCMD: u16    = 0x00;
const REG_USBSTS: u16    = 0x02;
const REG_USBINTR: u16   = 0x04;
const REG_FRNUM: u16     = 0x06;
const REG_FLBASEADD: u16 = 0x08;
const REG_SOFMOD: u16    = 0x0C;
const REG_PORTSC1: u16   = 0x10;
const REG_PORTSC2: u16   = 0x12;

const CMD_RS: u16      = 0x0001;
const CMD_HCRESET: u16 = 0x0002;
const CMD_GRESET: u16  = 0x0004;

const STS_USBINT: u16 = 0x0001;
const STS_HCH: u16    = 0x0020;

const PORT_CCS: u16 = 0x0001;
const PORT_CSC: u16 = 0x0002;
const PORT_PE: u16  = 0x0004;
const PORT_PEC: u16 = 0x0008;
const PORT_PR: u16  = 0x0200;
const PORT_ALWAYS1: u16 = 0x0080; // Bit 7: reserved, always reads as 1

const TD_ACTIVE: u32 = 1 << 23;
const TD_IOC: u32    = 1 << 24;

const PID_SETUP: u8 = 0x2D;
const PID_IN: u8    = 0x69;
const PID_OUT: u8   = 0xE1;

// ── USB HID Tablet ──────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct TabletReport {
    buttons: u8,
    x: u16,
    y: u16,
}

impl TabletReport {
    fn to_bytes(&self) -> [u8; 6] {
        [
            self.buttons,
            self.x as u8, (self.x >> 8) as u8,
            self.y as u8, (self.y >> 8) as u8,
            0, // wheel
        ]
    }
}

// ── USB Descriptors ──────────────────────────────────────────────────

const DEV_DESC: [u8; 18] = [
    18, 0x01, 0x10, 0x01, 0x00, 0x00, 0x00, 0x08,
    0x27, 0x06, 0x01, 0x00, 0x00, 0x01,
    0x01, 0x02, 0x00, 0x01,
];

const HID_REPORT_DESC: [u8; 53] = [
    0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x09, 0x01, 0xA1, 0x00,
    0x05, 0x09, 0x19, 0x01, 0x29, 0x03, 0x15, 0x00, 0x25, 0x01,
    0x95, 0x03, 0x75, 0x01, 0x81, 0x02,
    0x95, 0x01, 0x75, 0x05, 0x81, 0x01,
    0x05, 0x01, 0x09, 0x30, 0x15, 0x00, 0x26, 0xFF, 0x7F,
    0x75, 0x10, 0x95, 0x01, 0x81, 0x02,
    0x09, 0x31, 0x81, 0x02,
    0xC0, 0xC0,
];

const CFG_DESC: [u8; 34] = [
    // Config
    9, 0x02, 34, 0x00, 0x01, 0x01, 0x00, 0xA0, 50,
    // Interface
    9, 0x04, 0x00, 0x00, 0x01, 0x03, 0x00, 0x00, 0x00,
    // HID
    9, 0x21, 0x01, 0x01, 0x00, 0x01, 0x22, 53, 0x00,
    // Endpoint IN 1, interrupt, 6 bytes, 10ms
    7, 0x05, 0x81, 0x03, 0x06, 0x00, 10,
];

// ── UHCI Controller ──────────────────────────────────────────────────

pub struct Uhci {
    usbcmd: u16,
    usbsts: u16,
    usbintr: u16,
    frnum: u16,
    flbaseadd: u32,
    sofmod: u8,
    portsc: [u16; 2],

    // Tablet state
    current_report: TabletReport,
    device_address: u8,
    configured: bool,

    // Control transfer state: pending response for EP0 IN after SETUP
    ctrl_response: Vec<u8>,
    ctrl_response_offset: usize,

    // Guest RAM
    ram_ptr: *const u8,
    ram_size: usize,

    pub irq_pending: bool,
    debug_counter: u32,
}

unsafe impl Send for Uhci {}

impl Uhci {
    pub fn new() -> Self {
        Uhci {
            usbcmd: 0,
            usbsts: STS_HCH,
            usbintr: 0,
            frnum: 0,
            flbaseadd: 0,
            sofmod: 64,
            portsc: [PORT_CCS | PORT_ALWAYS1, PORT_ALWAYS1], // port 1: tablet connected
            current_report: TabletReport { buttons: 0, x: 0, y: 0 },
            device_address: 0,
            configured: false,
            ctrl_response: Vec::new(),
            ctrl_response_offset: 0,
            ram_ptr: core::ptr::null(),
            ram_size: 0,
            irq_pending: false,
            debug_counter: 0,
        }
    }

    pub fn set_guest_memory(&mut self, ptr: *mut u8, size: usize) {
        self.ram_ptr = ptr as *const u8;
        self.ram_size = size;
    }

    pub fn tablet_move(&mut self, x: u16, y: u16, buttons: u8) {
        self.current_report = TabletReport {
            buttons: buttons & 0x07,
            x: x.min(32767),
            y: y.min(32767),
        };
    }

    /// Process one USB frame. Call periodically (~1kHz).
    pub fn process_frame(&mut self) -> bool {
        self.debug_counter += 1;
        if self.usbcmd & CMD_RS == 0 { return false; }
        if self.flbaseadd == 0 || self.ram_ptr.is_null() { return false; }

        let frame_idx = (self.frnum & 0x3FF) as u64;
        let fl_entry = self.read32(self.flbaseadd as u64 + frame_idx * 4);

        if fl_entry & 1 == 0 {
            let is_qh = (fl_entry >> 1) & 1 != 0;
            let ptr = fl_entry & 0xFFFFFFF0;
            if is_qh {
                self.process_qh_chain(ptr);
            } else {
                self.process_td_chain(ptr);
            }
        }

        self.frnum = self.frnum.wrapping_add(1) & 0x7FF;

        if self.usbsts & STS_USBINT != 0 && self.usbintr & 0x01 != 0 {
            self.irq_pending = true;
            true
        } else {
            false
        }
    }

    fn process_qh_chain(&mut self, start_qh: u32) {
        let mut qh_addr = start_qh;
        let mut depth = 0u32;

        loop {
            if qh_addr == 0 || (qh_addr as usize) + 8 > self.ram_size || depth > 32 {
                return;
            }

            let h_link = self.read32(qh_addr as u64);
            let element = self.read32(qh_addr as u64 + 4);

            // Process element TDs if not terminated
            if element & 1 == 0 {
                let td_addr = element & 0xFFFFFFF0;
                let next_td = self.process_td_chain(td_addr);
                self.write32(qh_addr as u64 + 4, next_td);
            }

            // Follow horizontal link to next QH
            if h_link & 1 != 0 { return; } // h_link terminated
            let is_qh = (h_link >> 1) & 1 != 0;
            if !is_qh { return; } // h_link points to TD, not QH — stop

            let next_qh = h_link & 0xFFFFFFF0;
            if next_qh == start_qh || next_qh == qh_addr { return; } // loop detection
            qh_addr = next_qh;
            depth += 1;
        }
    }

    /// Process a chain of TDs. Returns the link pointer of the first
    /// unprocessed (still active) TD, or 0x00000001 (terminate) if all done.
    fn process_td_chain(&mut self, mut td_addr: u32) -> u32 {
        let mut count = 0u32;
        loop {
            if td_addr == 0 || (td_addr as usize) + 16 > self.ram_size || count > 128 {
                return 0x00000001; // terminate
            }

            let link = self.read32(td_addr as u64);
            let ctrl = self.read32(td_addr as u64 + 4);
            let token = self.read32(td_addr as u64 + 8);
            let buf_ptr = self.read32(td_addr as u64 + 12);

            if ctrl & TD_ACTIVE == 0 {
                // Already processed — follow link
                if link & 1 != 0 { return 0x00000001; }
                td_addr = link & 0xFFFFFFF0;
                count += 1;
                continue;
            }

            let pid = (token & 0xFF) as u8;
            let dev = ((token >> 8) & 0x7F) as u8;
            let ep = ((token >> 15) & 0xF) as u8;
            let maxlen_field = ((token >> 21) & 0x7FF) as u16;
            let xfer_len = if maxlen_field == 0x7FF { 0u16 } else { maxlen_field + 1 };

            // Only handle our device (address 0 during enum, or assigned address)
            if dev != 0 && dev != self.device_address {
                // Not our device — NAK (leave active, guest will retry)
                return td_addr | 0x00000000;
            }

            let (completed, actual_bytes) = match pid {
                PID_SETUP => {
                    if xfer_len >= 8 && buf_ptr != 0 {
                        let mut setup = [0u8; 8];
                        self.read_mem(buf_ptr as u64, &mut setup);
                        self.handle_setup(&setup);
                    }
                    (true, 0u16)
                }
                PID_IN => {
                    if ep == 0 {
                        // Control IN — return pending descriptor data
                        let remain = self.ctrl_response.len() - self.ctrl_response_offset;
                        let send = remain.min(xfer_len as usize);
                        if send > 0 && buf_ptr != 0 {
                            let off = self.ctrl_response_offset;
                            self.write_mem(buf_ptr as u64, &self.ctrl_response[off..off + send]);
                            self.ctrl_response_offset += send;
                        }
                        (true, send as u16)
                    } else if ep == 1 && self.configured {
                        // Interrupt IN — HID report
                        let report = self.current_report.to_bytes();
                        let send = report.len().min(xfer_len as usize);
                        if buf_ptr != 0 {
                            self.write_mem(buf_ptr as u64, &report[..send]);
                        }
                        (true, send as u16)
                    } else {
                        (true, 0)
                    }
                }
                PID_OUT => {
                    // Status phase OUT (zero-length) or data OUT — just ACK
                    (true, 0)
                }
                _ => (true, 0),
            };

            if completed {
                // Clear active, set actual length
                let act_len_field = if actual_bytes == 0 { 0x7FFu32 } else { (actual_bytes as u32 - 1) & 0x7FF };
                let new_ctrl = (ctrl & !(TD_ACTIVE | 0x7FF)) | act_len_field;
                self.write32(td_addr as u64 + 4, new_ctrl);

                if ctrl & TD_IOC != 0 {
                    self.usbsts |= STS_USBINT;
                }
            }

            // Follow link to next TD
            if link & 1 != 0 { return 0x00000001; }
            td_addr = link & 0xFFFFFFF0;
            count += 1;
        }
    }

    fn handle_setup(&mut self, data: &[u8]) {
        let bm = data[0];
        let req = data[1];
        let w_value = u16::from_le_bytes([data[2], data[3]]);
        let _w_index = u16::from_le_bytes([data[4], data[5]]);
        let w_length = u16::from_le_bytes([data[6], data[7]]);

        // Reset control transfer state
        self.ctrl_response.clear();
        self.ctrl_response_offset = 0;

        match (bm, req) {
            (0x00, 0x05) => {
                self.device_address = w_value as u8;
                #[cfg(feature = "std")]
                eprintln!("[uhci] SET_ADDRESS({})", self.device_address);
            }
            (0x00, 0x09) => {
                self.configured = w_value != 0;
                #[cfg(feature = "std")]
                eprintln!("[uhci] SET_CONFIGURATION({})", w_value);
            }
            (0x80, 0x06) => {
                // GET_DESCRIPTOR
                let desc_type = (w_value >> 8) as u8;
                let desc_idx = (w_value & 0xFF) as u8;
                let max = w_length as usize;

                match desc_type {
                    0x01 => {
                        // Device descriptor
                        let len = DEV_DESC.len().min(max);
                        self.ctrl_response.extend_from_slice(&DEV_DESC[..len]);
                    }
                    0x02 => {
                        // Configuration descriptor
                        let len = CFG_DESC.len().min(max);
                        self.ctrl_response.extend_from_slice(&CFG_DESC[..len]);
                    }
                    0x22 => {
                        // HID Report descriptor
                        let len = HID_REPORT_DESC.len().min(max);
                        self.ctrl_response.extend_from_slice(&HID_REPORT_DESC[..len]);
                    }
                    0x03 => {
                        // String descriptor
                        match desc_idx {
                            0 => self.ctrl_response.extend_from_slice(&[4, 0x03, 0x09, 0x04]),
                            1 => {
                                let s = "CoreVM";
                                let mut d = vec![0u8; 2 + s.len() * 2];
                                d[0] = d.len() as u8;
                                d[1] = 0x03;
                                for (i, c) in s.encode_utf16().enumerate() {
                                    d[2 + i * 2] = c as u8;
                                    d[3 + i * 2] = (c >> 8) as u8;
                                }
                                let len = d.len().min(max);
                                self.ctrl_response.extend_from_slice(&d[..len]);
                            }
                            2 => {
                                let s = "USB Tablet";
                                let mut d = vec![0u8; 2 + s.len() * 2];
                                d[0] = d.len() as u8;
                                d[1] = 0x03;
                                for (i, c) in s.encode_utf16().enumerate() {
                                    d[2 + i * 2] = c as u8;
                                    d[3 + i * 2] = (c >> 8) as u8;
                                }
                                let len = d.len().min(max);
                                self.ctrl_response.extend_from_slice(&d[..len]);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            (0x81, 0x06) => {
                // GET_DESCRIPTOR (interface) — HID report descriptor
                let desc_type = (w_value >> 8) as u8;
                if desc_type == 0x22 {
                    let max = w_length as usize;
                    let len = HID_REPORT_DESC.len().min(max);
                    self.ctrl_response.extend_from_slice(&HID_REPORT_DESC[..len]);
                }
            }
            (0x21, 0x0A) | (0x21, 0x0B) => {
                // SET_IDLE / SET_PROTOCOL — no data phase
            }
            (0x80, 0x00) => {
                // GET_STATUS — return 0x0000 (self-powered, no remote wakeup)
                self.ctrl_response.extend_from_slice(&[0x00, 0x00]);
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.usbcmd = 0;
        self.usbsts = STS_HCH;
        self.usbintr = 0;
        self.frnum = 0;
        self.flbaseadd = 0;
        self.sofmod = 64;
        // After reset: device connected on port 1, CSC set to trigger enumeration
        self.portsc = [PORT_CCS | PORT_CSC | PORT_ALWAYS1, PORT_ALWAYS1];
        self.device_address = 0;
        self.configured = false;
        self.ctrl_response.clear();
        self.ctrl_response_offset = 0;
        self.irq_pending = false;
    }

    fn read32(&self, addr: u64) -> u32 {
        if self.ram_ptr.is_null() || addr as usize + 4 > self.ram_size { return 0; }
        unsafe { (self.ram_ptr.add(addr as usize) as *const u32).read_unaligned() }
    }

    fn write32(&self, addr: u64, val: u32) {
        if self.ram_ptr.is_null() || addr as usize + 4 > self.ram_size { return; }
        unsafe { ((self.ram_ptr as *mut u8).add(addr as usize) as *mut u32).write_unaligned(val); }
    }

    fn read_mem(&self, addr: u64, buf: &mut [u8]) {
        if self.ram_ptr.is_null() || addr as usize + buf.len() > self.ram_size {
            buf.fill(0); return;
        }
        unsafe { core::ptr::copy_nonoverlapping(self.ram_ptr.add(addr as usize), buf.as_mut_ptr(), buf.len()); }
    }

    fn write_mem(&self, addr: u64, data: &[u8]) {
        if self.ram_ptr.is_null() || addr as usize + data.len() > self.ram_size { return; }
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), (self.ram_ptr as *mut u8).add(addr as usize), data.len()); }
    }

    fn handle_portsc_write_16(&mut self, port_idx: usize, val: u16) {
        let old = self.portsc[port_idx];

        if val & PORT_PR != 0 {
            self.portsc[port_idx] = PORT_PR | (old & PORT_CCS) | PORT_ALWAYS1;
            return;
        }

        if old & PORT_PR != 0 && val & PORT_PR == 0 {
            let mut new_val = PORT_PE | PORT_CSC | PORT_PEC | PORT_ALWAYS1;
            if port_idx == 0 {
                new_val |= PORT_CCS;
            }
            self.portsc[port_idx] = new_val;
            return;
        }

        // Normal write: WC bits (CSC, PEC), writable bits (PE)
        let clear_bits = val & (PORT_CSC | PORT_PEC);
        let mut new_val = old & !clear_bits;
        new_val = (new_val & !PORT_PE) | (val & PORT_PE);
        // CCS is read-only
        new_val = (new_val & !PORT_CCS) | (old & PORT_CCS);
        // Bit 7 always reads 1
        new_val |= PORT_ALWAYS1;
        self.portsc[port_idx] = new_val;
    }
}

// ── I/O Handler ──────────────────────────────────────────────────────

impl IoHandler for Uhci {
    fn read(&mut self, port: u16, size: u8) -> Result<u32> {
        // Process pending frames on EVERY read — SeaBIOS polls in tight
        // loops and expects TDs to complete between I/O accesses.
        for _ in 0..32 {
            self.process_frame();
        }

        let val = match size {
            2 => match port {
                REG_USBCMD  => self.usbcmd as u32,
                REG_USBSTS  => self.usbsts as u32,
                REG_USBINTR => self.usbintr as u32,
                REG_FRNUM   => self.frnum as u32,
                REG_PORTSC1 => self.portsc[0] as u32,
                REG_PORTSC2 => self.portsc[1] as u32,
                _ => 0xFFFF,
            },
            4 => match port {
                REG_FLBASEADD => self.flbaseadd,
                _ => 0xFFFFFFFF,
            },
            1 => match port {
                REG_SOFMOD => self.sofmod as u32,
                _ => 0xFF,
            },
            _ => 0,
        };
        Ok(val)
    }

    fn write(&mut self, port: u16, size: u8, val: u32) -> Result<()> {
        // Process frames before handling the write
        for _ in 0..32 {
            self.process_frame();
        }
        match size {
            2 => {
                let v = val as u16;
                match port {
                    REG_USBCMD => {
                        if v & (CMD_HCRESET | CMD_GRESET) != 0 { self.reset(); return Ok(()); }
                        self.usbcmd = v;
                        if v & CMD_RS != 0 {
                            self.usbsts &= !STS_HCH;
                            #[cfg(feature = "std")]
                            eprintln!("[uhci] RUN (flbase={:#x})", self.flbaseadd);
                        } else {
                            self.usbsts |= STS_HCH;
                        }
                    }
                    REG_USBSTS => {
                        self.usbsts &= !(v & 0x001F);
                        if self.usbsts & STS_USBINT == 0 { self.irq_pending = false; }
                    }
                    REG_USBINTR => { self.usbintr = v; }
                    REG_FRNUM   => { self.frnum = v & 0x7FF; }
                    REG_PORTSC1 => { self.handle_portsc_write_16(0, v); }
                    REG_PORTSC2 => { self.handle_portsc_write_16(1, v); }
                    _ => {}
                }
            }
            4 => {
                if port == REG_FLBASEADD { self.flbaseadd = val & 0xFFFFF000; }
            }
            1 => {
                if port == REG_SOFMOD { self.sofmod = val as u8; }
            }
            _ => {}
        }
        Ok(())
    }
}

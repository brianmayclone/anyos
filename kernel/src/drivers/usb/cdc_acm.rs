//! USB CDC-ACM (Abstract Control Model) serial driver.
//!
//! Supports USB serial adapters (FTDI, CP2102, CH340 in CDC mode, etc.)
//! using Communication Device Class with ACM subclass (class 0x02, subclass 0x02).
//! Registers as `/dev/ttyUSB{N}` character device via HAL.

use super::{ControllerType, SetupPacket, UsbDevice, UsbInterface, UsbSpeed};
use crate::memory::physical;
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

// ── CDC-ACM Constants ────────────────────────────

const SET_LINE_CODING: u8 = 0x20;
const SET_CONTROL_LINE_STATE: u8 = 0x22;

// ── Ring Buffer ──────────────────────────────────

const RX_BUF_SIZE: usize = 4096;

struct RxRingBuffer {
    buf: [u8; RX_BUF_SIZE],
    head: usize,
    tail: usize,
}

impl RxRingBuffer {
    const fn new() -> Self {
        RxRingBuffer {
            buf: [0; RX_BUF_SIZE],
            head: 0,
            tail: 0,
        }
    }

    fn push(&mut self, byte: u8) -> bool {
        let next = (self.head + 1) % RX_BUF_SIZE;
        if next == self.tail {
            return false; // full
        }
        self.buf[self.head] = byte;
        self.head = next;
        true
    }

    fn read_into(&mut self, buf: &mut [u8]) -> usize {
        let mut count = 0;
        while count < buf.len() && self.tail != self.head {
            buf[count] = self.buf[self.tail];
            self.tail = (self.tail + 1) % RX_BUF_SIZE;
            count += 1;
        }
        count
    }
}

// ── Device State ─────────────────────────────────

struct CdcAcmDevice {
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
    bounce_phys: u64,
    rx_ring: RxRingBuffer,
    tty_index: u8,
}

static CDC_ACM_DEVICES: Spinlock<Vec<CdcAcmDevice>> = Spinlock::new(Vec::new());
static NEXT_TTY_INDEX: Spinlock<u8> = Spinlock::new(0);

fn alloc_tty_index() -> u8 {
    let mut idx = NEXT_TTY_INDEX.lock();
    let i = *idx;
    *idx = idx.wrapping_add(1);
    i
}

// ── CDC Control Requests ─────────────────────────

fn set_line_coding(dev: &CdcAcmDevice) -> Result<(), &'static str> {
    // Line coding: 115200 baud, 1 stop bit, no parity, 8 data bits (7 bytes)
    let payload: [u8; 7] = [
        0x00, 0xC2, 0x01, 0x00, // 115200 LE32
        0x00,                    // 1 stop bit
        0x00,                    // no parity
        0x08,                    // 8 data bits
    ];
    // Copy payload to DMA buffer
    unsafe {
        core::ptr::copy_nonoverlapping(payload.as_ptr(), dev.bounce_phys as *mut u8, 7);
    }

    // SET_LINE_CODING is a data-OUT control transfer; we use hid_control_transfer
    // which handles the data phase via the controller's data buffer.
    // For no-data, w_length=0 is fine. For data-OUT to work correctly,
    // send as no-data (most CDC devices accept SET_LINE_CODING without data phase
    // if wLength=0, or we rely on the default).
    let setup = SetupPacket {
        bm_request_type: 0x21, // Host-to-device, Class, Interface
        b_request: SET_LINE_CODING,
        w_value: 0,
        w_index: dev.comm_iface as u16,
        w_length: 0, // Some devices ignore line coding entirely
    };
    let _ = super::hid_control_transfer(
        dev.usb_addr, dev.controller, dev.speed, dev.max_packet,
        &setup, false, 0,
    );
    Ok(())
}

fn set_control_line_state(dev: &CdcAcmDevice, dtr: bool, rts: bool) -> Result<(), &'static str> {
    let w_value = (dtr as u16) | ((rts as u16) << 1);
    let setup = SetupPacket {
        bm_request_type: 0x21,
        b_request: SET_CONTROL_LINE_STATE,
        w_value,
        w_index: dev.comm_iface as u16,
        w_length: 0,
    };
    let _ = super::hid_control_transfer(
        dev.usb_addr, dev.controller, dev.speed, dev.max_packet,
        &setup, false, 0,
    );
    Ok(())
}

// ── Read/Write ───────────────────────────────────

/// Read available data from a CDC-ACM device's RX buffer.
pub fn read_data(tty_index: u8, buf: &mut [u8]) -> Result<usize, &'static str> {
    let mut devs = CDC_ACM_DEVICES.lock();
    let dev = devs.iter_mut().find(|d| d.tty_index == tty_index)
        .ok_or("CDC-ACM device not found")?;
    Ok(dev.rx_ring.read_into(buf))
}

/// Write data to a CDC-ACM device via bulk OUT.
pub fn write_data(tty_index: u8, data: &[u8]) -> Result<usize, &'static str> {
    let mut devs = CDC_ACM_DEVICES.lock();
    let dev = devs.iter_mut().find(|d| d.tty_index == tty_index)
        .ok_or("CDC-ACM device not found")?;

    let to_send = data.len().min(4096);
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), dev.bounce_phys as *mut u8, to_send);
    }
    super::bulk_transfer(
        dev.usb_addr, dev.controller, dev.speed,
        dev.ep_bulk_out, dev.max_packet_out,
        &mut dev.toggle_out,
        dev.bounce_phys, to_send,
    )
}

// ── Probe ────────────────────────────────────────

/// Called when a CDC Communication Interface (class=0x02, subclass=0x02) is detected.
pub fn probe(dev: &UsbDevice, iface: &UsbInterface) {
    crate::serial_println!(
        "  CDC-ACM: probing (addr={}, iface={})",
        dev.address, iface.number
    );

    // Find the companion Data Interface (class 0x0A)
    let data_iface = match dev.interfaces.iter().find(|i| i.class == 0x0A) {
        Some(di) => di,
        None => {
            crate::serial_println!("  CDC-ACM: no Data Interface (class 0x0A) found");
            return;
        }
    };

    // Find bulk IN and bulk OUT endpoints on the Data Interface
    let bulk_in = data_iface.endpoints.iter().find(|ep| {
        (ep.attributes & 0x03) == 2 && (ep.address & 0x80) != 0
    });
    let bulk_out = data_iface.endpoints.iter().find(|ep| {
        (ep.attributes & 0x03) == 2 && (ep.address & 0x80) == 0
    });

    let (ep_in, ep_out) = match (bulk_in, bulk_out) {
        (Some(i), Some(o)) => {
            crate::serial_println!(
                "  CDC-ACM: bulk IN ep={:#04x} (max={}), bulk OUT ep={:#04x} (max={})",
                i.address, i.max_packet_size, o.address, o.max_packet_size
            );
            (i, o)
        }
        _ => {
            crate::serial_println!("  CDC-ACM: missing bulk endpoints on Data Interface");
            return;
        }
    };

    // Allocate DMA bounce buffer
    let bounce_phys = match physical::alloc_frame() {
        Some(f) => f.as_u64(),
        None => {
            crate::serial_println!("  CDC-ACM: failed to allocate DMA page");
            return;
        }
    };
    unsafe { core::ptr::write_bytes(bounce_phys as *mut u8, 0, 4096); }

    let tty_index = alloc_tty_index();

    let acm_dev = CdcAcmDevice {
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
        bounce_phys,
        rx_ring: RxRingBuffer::new(),
        tty_index,
    };

    // Send initial CDC control requests
    let _ = set_line_coding(&acm_dev);
    let _ = set_control_line_state(&acm_dev, true, true); // DTR + RTS

    crate::serial_println!("  CDC-ACM: registered as /dev/ttyUSB{}", tty_index);

    // Register as HAL character device
    let path = alloc::format!("/dev/ttyUSB{}", tty_index);
    crate::drivers::hal::register_device(
        &path,
        alloc::boxed::Box::new(CdcAcmHalDriver { tty_index }),
        None,
    );

    CDC_ACM_DEVICES.lock().push(acm_dev);
}

// ── Polling ──────────────────────────────────────

/// Poll all CDC-ACM devices for incoming data on bulk IN.
pub fn poll_all() {
    let mut devs = CDC_ACM_DEVICES.lock();
    for dev in devs.iter_mut() {
        match super::bulk_transfer(
            dev.usb_addr, dev.controller, dev.speed,
            dev.ep_bulk_in, dev.max_packet_in,
            &mut dev.toggle_in,
            dev.bounce_phys, dev.max_packet_in as usize,
        ) {
            Ok(n) if n > 0 => {
                unsafe {
                    let src = dev.bounce_phys as *const u8;
                    for i in 0..n {
                        let byte = core::ptr::read_volatile(src.add(i));
                        dev.rx_ring.push(byte);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Disconnect handler for hot-unplug.
pub fn disconnect(port: u8, controller: ControllerType) {
    let mut devs = CDC_ACM_DEVICES.lock();
    if let Some(idx) = devs.iter().position(|d| d.port == port && d.controller == controller) {
        let dev = devs.remove(idx);
        crate::serial_println!("  CDC-ACM: /dev/ttyUSB{} removed", dev.tty_index);
    }
}

// ── HAL Integration ──────────────────────────────

use crate::drivers::hal::{Driver, DriverType, DriverError};

struct CdcAcmHalDriver {
    tty_index: u8,
}

impl Driver for CdcAcmHalDriver {
    fn name(&self) -> &str { "USB CDC-ACM Serial" }
    fn driver_type(&self) -> DriverType { DriverType::Char }
    fn init(&mut self) -> Result<(), DriverError> { Ok(()) }

    fn read(&self, _offset: usize, buf: &mut [u8]) -> Result<usize, DriverError> {
        read_data(self.tty_index, buf).map_err(|_| DriverError::IoError)
    }

    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        write_data(self.tty_index, buf).map_err(|_| DriverError::IoError)
    }

    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

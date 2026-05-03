// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! VirtIO Serial (Console) PCI driver — multiport.
//!
//! Negotiates VIRTIO_CONSOLE_F_MULTIPORT so QEMU's `virtserialport` channels
//! (e.g. SPICE vdagent's `com.redhat.spice.0`) become accessible. Each port is
//! exposed as a character device:
//!
//! - `/dev/vportN`              — by index (always present, port 0 .. max_nr_ports-1)
//! - `/dev/virtio-ports/<name>` — by host-assigned name (PORT_NAME control msg)
//! - `/dev/vport-<alias>`       — short alias for well-known names
//!   - `com.redhat.spice.0`  → `/dev/vport-spice`     (vdagent)
//!   - `org.spice-space.webdav.0` → `/dev/vport-webdav`
//!
//! VirtIO PCI device IDs:
//! - 0x1003 (transitional) / 0x1043 (modern) — virtio-serial / virtio-console
//!
//! Reference: VirtIO Spec v1.2 §5.3 (Console Device).

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use core::fmt::Write as _;

use crate::drivers::hal::{self, Driver, DriverError, DriverType};
use crate::drivers::pci::PciDevice;
use crate::drivers::virtio::virtqueue::VirtQueue;
use crate::drivers::virtio::{self, VirtioDevice, VIRTIO_F_VERSION_1};
use crate::memory::physical;
use crate::sync::spinlock::Spinlock;

// ── Constants ────────────────────────────────────────────────────────────────

/// Maximum ports we expose. Real-world setups use 1–3 ports; cap to keep the
/// per-port memory footprint bounded.
const MAX_PORTS: usize = 8;

/// One 4 KiB page per RX/TX buffer is plenty for clipboard-sized traffic.
const BUF_PAGES: usize = 1;
const BUF_SIZE: usize = BUF_PAGES * 4096;

/// Number of RX descriptors we keep posted on the control queue. QEMU drops
/// control messages silently if no descriptor is available (see
/// `send_control_msg` in QEMU's virtio-serial-bus.c) — so we must have
/// several in flight to catch back-to-back PORT_NAME / PORT_OPEN events.
const CTRL_RX_SLOTS: usize = 8;

// ── Feature bits (virtio-console) ───────────────────────────────────────────

const VIRTIO_CONSOLE_F_SIZE: u64 = 1 << 0; // we ignore size
const VIRTIO_CONSOLE_F_MULTIPORT: u64 = 1 << 1;

// ── Control message events (virtio-v1.2 §5.3.6) ─────────────────────────────

const VIRTIO_CONSOLE_DEVICE_READY: u16 = 0;
const VIRTIO_CONSOLE_DEVICE_ADD: u16 = 1;
const VIRTIO_CONSOLE_DEVICE_REMOVE: u16 = 2;
const VIRTIO_CONSOLE_PORT_READY: u16 = 3;
const VIRTIO_CONSOLE_CONSOLE_PORT: u16 = 4;
const VIRTIO_CONSOLE_RESIZE: u16 = 5;
const VIRTIO_CONSOLE_PORT_OPEN: u16 = 6;
const VIRTIO_CONSOLE_PORT_NAME: u16 = 7;

const CTRL_HDR_SIZE: usize = 8; // u32 id + u16 event + u16 value

// ── Queue index helpers ──────────────────────────────────────────────────────

/// Per virtio-console spec: port 0 uses queues 0/1, control uses 2/3,
/// port N>=1 uses queues 2*(N+1) (rx) and 2*(N+1)+1 (tx).
#[inline]
fn rx_queue_idx(port: u32) -> u16 {
    if port == 0 {
        0
    } else {
        (2 * (port + 1)) as u16
    }
}
#[inline]
fn tx_queue_idx(port: u32) -> u16 {
    if port == 0 {
        1
    } else {
        (2 * (port + 1) + 1) as u16
    }
}
const CONTROL_RX_QIDX: u16 = 2;
const CONTROL_TX_QIDX: u16 = 3;

/// ISR address of the (single) virtio-serial bus, used by the shared IRQ
/// handler. Without this, we share IRQ 10 with virtio-input and never
/// acknowledge the device — IOAPIC level-triggered IRQs storm endlessly.
static SERIAL_ISR_ADDR: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

// ── Bus state (one per PCI device — typically only one in a guest) ──────────

/// Per-port runtime state owned by the bus. Ports aren't separate `Driver`
/// instances so we don't have to share the `VirtioDevice` across owners.
struct Port {
    rx_queue: VirtQueue,
    tx_queue: VirtQueue,
    rx_buf_phys: u64,
    tx_buf_phys: u64,
    /// Pending bytes from a completed RX descriptor that haven't been read yet.
    rx_pending: [u8; BUF_SIZE],
    rx_pending_len: usize,
    rx_pending_pos: usize,
    rx_posted: bool,
    /// Set after we've sent PORT_READY for this port.
    ready_acked: bool,
    /// Host-assigned port name (from PORT_NAME control message).
    name: Option<String>,
    /// Set after PORT_OPEN(value=1) was sent.
    opened: bool,
}

struct SerialBus {
    vdev: VirtioDevice,
    /// Control queues are only present when MULTIPORT was negotiated.
    control_rx: Option<VirtQueue>,
    control_tx: Option<VirtQueue>,
    /// 8 separate RX buffers, posted simultaneously. QEMU drops control
    /// messages silently if no descriptor is available, so we keep a pool.
    ctrl_rx_phys: [u64; CTRL_RX_SLOTS],
    /// `ctrl_rx_in_flight[slot] = Some(desc_id)` while the device owns slot.
    ctrl_rx_in_flight: [Option<u16>; CTRL_RX_SLOTS],
    ctrl_tx_phys: u64,
    /// Negotiated maximum port count from device config (offset 4, u32).
    /// Capped at MAX_PORTS internally.
    max_nr_ports: u32,
    ports: [Option<Port>; MAX_PORTS],
}

impl SerialBus {
    fn has_control(&self) -> bool {
        self.control_rx.is_some() && self.control_tx.is_some()
    }

    /// Refill the RX queue so all CTRL_RX_SLOTS buffers are posted.
    fn post_ctrl_rx(&mut self) {
        let crx = match self.control_rx.as_mut() {
            Some(q) => q,
            None => return,
        };
        let mut posted_any = false;
        for slot in 0..CTRL_RX_SLOTS {
            if self.ctrl_rx_in_flight[slot].is_some() {
                continue;
            }
            let phys = self.ctrl_rx_phys[slot];
            if phys == 0 {
                continue;
            }
            let writable = [(phys, BUF_SIZE as u32)];
            match crx.push(&[], &writable) {
                Some(id) => {
                    self.ctrl_rx_in_flight[slot] = Some(id);
                    posted_any = true;
                }
                None => break, // queue full
            }
        }
        if posted_any {
            self.vdev.notify_queue(CONTROL_RX_QIDX);
        }
    }

    /// Send a control message to the device (no extra payload).
    fn send_ctrl(&mut self, port_id: u32, event: u16, value: u16) {
        if self.control_tx.is_none() {
            return;
        }
        let buf = self.ctrl_tx_phys as *mut u8;
        unsafe {
            // u32 id (LE) + u16 event (LE) + u16 value (LE)
            core::ptr::write_volatile(buf as *mut u32, port_id);
            core::ptr::write_volatile(buf.add(4) as *mut u16, event);
            core::ptr::write_volatile(buf.add(6) as *mut u16, value);
        }
        let readable = [(self.ctrl_tx_phys, CTRL_HDR_SIZE as u32)];
        // Borrow ctx and vdev disjointly via raw pointer to avoid the closure
        // capturing &self alongside &mut self.control_tx.
        let vdev_ptr: *const VirtioDevice = &self.vdev;
        let ctx = self.control_tx.as_mut().unwrap();
        let result = ctx.execute_sync(&readable, &[], || unsafe {
            (*vdev_ptr).notify_queue(CONTROL_TX_QIDX);
        });
        match result {
            Some(_) => crate::serial_println!(
                "[virtio-serial] TX ctrl port={} event={} value={} OK",
                port_id, event, value
            ),
            None => crate::serial_println!(
                "[virtio-serial] TX ctrl port={} event={} value={} FAILED",
                port_id, event, value
            ),
        }
    }

    /// Drain any completed control messages and act on them.
    fn poll_control(&mut self) {
        if !self.has_control() {
            return;
        }
        while let Some((id, len)) = self
            .control_rx
            .as_mut()
            .and_then(|q| q.poll_used())
        {
            // Find which slot this descriptor belongs to.
            let mut slot_found = None;
            for slot in 0..CTRL_RX_SLOTS {
                if self.ctrl_rx_in_flight[slot] == Some(id) {
                    slot_found = Some(slot);
                    break;
                }
            }
            let slot = match slot_found {
                Some(s) => s,
                None => {
                    crate::serial_println!(
                        "[virtio-serial] RX ctrl: unknown desc id={} — skipping",
                        id
                    );
                    continue;
                }
            };
            self.ctrl_rx_in_flight[slot] = None;

            let phys = self.ctrl_rx_phys[slot];
            let bytes = (len as usize).min(BUF_SIZE);
            let mut hdr = [0u8; BUF_SIZE];
            unsafe {
                core::ptr::copy_nonoverlapping(phys as *const u8, hdr.as_mut_ptr(), bytes);
            }

            // Re-post immediately so the device always has buffers available.
            self.post_ctrl_rx();

            crate::serial_println!(
                "[virtio-serial] RX ctrl slot={} len={} bytes={:02x}{:02x}{:02x}{:02x} {:02x}{:02x} {:02x}{:02x}",
                slot,
                len,
                hdr[0], hdr[1], hdr[2], hdr[3],
                hdr[4], hdr[5],
                hdr[6], hdr[7]
            );
            if bytes >= CTRL_HDR_SIZE {
                let port_id = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
                let event = u16::from_le_bytes([hdr[4], hdr[5]]);
                let value = u16::from_le_bytes([hdr[6], hdr[7]]);
                let payload = &hdr[CTRL_HDR_SIZE..bytes];
                self.handle_ctrl(port_id, event, value, payload);
            }
        }
        self.post_ctrl_rx();
    }

    fn handle_ctrl(&mut self, port_id: u32, event: u16, value: u16, payload: &[u8]) {
        match event {
            VIRTIO_CONSOLE_DEVICE_ADD => {
                if (port_id as usize) < MAX_PORTS && self.ports[port_id as usize].is_some() {
                    // Confirm we know about this port and are ready to use it.
                    let needs_ack = !self.ports[port_id as usize].as_ref().unwrap().ready_acked;
                    if needs_ack {
                        self.send_ctrl(port_id, VIRTIO_CONSOLE_PORT_READY, 1);
                        // Send PORT_OPEN preemptively too — some QEMU versions
                        // delay PORT_NAME until the port is opened by the guest.
                        self.send_ctrl(port_id, VIRTIO_CONSOLE_PORT_OPEN, 1);
                        if let Some(p) = self.ports[port_id as usize].as_mut() {
                            p.ready_acked = true;
                            p.opened = true;
                        }
                        crate::serial_println!(
                            "[virtio-serial] PORT_ADD port={} ack (sent PORT_READY + PORT_OPEN)",
                            port_id
                        );
                    }
                }
            }
            VIRTIO_CONSOLE_PORT_NAME => {
                if (port_id as usize) < MAX_PORTS {
                    // Name is a non-NUL-terminated UTF-8 string in the payload.
                    let trimmed_end = payload
                        .iter()
                        .position(|&b| b == 0)
                        .unwrap_or(payload.len());
                    let name = core::str::from_utf8(&payload[..trimmed_end])
                        .unwrap_or("")
                        .to_string();
                    if !name.is_empty() {
                        // Just record the name on the port. HAL registration
                        // happens later in probe(), outside the BUS spinlock —
                        // calling hal::register_device (which heap-allocates)
                        // while holding our lock can deadlock or corrupt other
                        // subsystems' allocator state.
                        if let Some(p) = self.ports[port_id as usize].as_mut() {
                            p.name = Some(name.clone());
                        }
                        crate::serial_println!(
                            "[virtio-serial] PORT_NAME port={} name={}",
                            port_id,
                            name
                        );
                    }
                }
            }
            VIRTIO_CONSOLE_PORT_OPEN => {
                crate::serial_verbose_println!(
                    "  virtio-serial: host PORT_OPEN port={} value={}",
                    port_id,
                    value
                );
            }
            VIRTIO_CONSOLE_DEVICE_REMOVE => {
                if (port_id as usize) < MAX_PORTS {
                    self.ports[port_id as usize] = None;
                }
            }
            VIRTIO_CONSOLE_CONSOLE_PORT
            | VIRTIO_CONSOLE_RESIZE
            | VIRTIO_CONSOLE_DEVICE_READY
            | VIRTIO_CONSOLE_PORT_READY => {
                // Informational — no action needed.
            }
            _ => {}
        }
    }

    fn ensure_rx_posted(&mut self, port_id: u32) {
        let idx = port_id as usize;
        if idx >= MAX_PORTS {
            return;
        }
        let phys;
        {
            let p = match self.ports[idx].as_mut() {
                Some(p) => p,
                None => return,
            };
            if p.rx_posted {
                return;
            }
            phys = p.rx_buf_phys;
            let writable = [(phys, BUF_SIZE as u32)];
            if p.rx_queue.push(&[], &writable).is_some() {
                p.rx_posted = true;
            } else {
                return;
            }
        }
        self.vdev.notify_queue(rx_queue_idx(port_id));
    }

    fn poll_port_rx(&mut self, port_id: u32) {
        let idx = port_id as usize;
        if idx >= MAX_PORTS {
            return;
        }
        let p = match self.ports[idx].as_mut() {
            Some(p) => p,
            None => return,
        };
        if let Some((_id, len)) = p.rx_queue.poll_used() {
            p.rx_posted = false;
            let bytes = (len as usize).min(BUF_SIZE);
            if bytes > 0 {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        p.rx_buf_phys as *const u8,
                        p.rx_pending.as_mut_ptr(),
                        bytes,
                    );
                }
                p.rx_pending_len = bytes;
                p.rx_pending_pos = 0;
            }
        }
        self.ensure_rx_posted(port_id);
    }

    fn read_port(&mut self, port_id: u32, buf: &mut [u8]) -> usize {
        // Always service control queue + RX before reading.
        self.poll_control();
        self.poll_port_rx(port_id);

        let idx = port_id as usize;
        let p = match self.ports[idx].as_mut() {
            Some(p) => p,
            None => return 0,
        };
        if p.rx_pending_pos < p.rx_pending_len {
            let avail = p.rx_pending_len - p.rx_pending_pos;
            let n = avail.min(buf.len());
            buf[..n].copy_from_slice(&p.rx_pending[p.rx_pending_pos..p.rx_pending_pos + n]);
            p.rx_pending_pos += n;
            n
        } else {
            0
        }
    }

    fn write_port(&mut self, port_id: u32, buf: &[u8]) -> Result<usize, DriverError> {
        // Service control queue while we're at it.
        self.poll_control();

        if buf.is_empty() {
            return Ok(0);
        }
        let idx = port_id as usize;
        let send_len = buf.len().min(BUF_SIZE);
        let phys;
        {
            let p = match self.ports[idx].as_mut() {
                Some(p) => p,
                None => return Err(DriverError::NoDevice),
            };
            phys = p.tx_buf_phys;
            unsafe {
                core::ptr::copy_nonoverlapping(buf.as_ptr(), phys as *mut u8, send_len);
            }
            let readable = [(phys, send_len as u32)];
            let qidx = tx_queue_idx(port_id);
            let result = p
                .tx_queue
                .execute_sync(&readable, &[], || self.vdev.notify_queue(qidx));
            if result.is_none() {
                return Err(DriverError::IoError);
            }
        }
        Ok(send_len)
    }
}

// SAFETY: SerialBus contains VirtQueue (which is itself `Send` via its own
// unsafe impl) and raw phys-address u64s. Access is serialized through BUS.
unsafe impl Send for SerialBus {}

// ── Bus Singleton ────────────────────────────────────────────────────────────

static BUS: Spinlock<Option<SerialBus>> = Spinlock::new(None);

fn with_bus<R>(f: impl FnOnce(&mut SerialBus) -> R) -> Option<R> {
    let mut guard = BUS.lock();
    let bus = guard.as_mut()?;
    Some(f(bus))
}

// ── Per-Port Char Device Driver ─────────────────────────────────────────────

pub struct VirtioSerialPortDriver {
    port_id: u32,
}

impl Driver for VirtioSerialPortDriver {
    fn name(&self) -> &str {
        "VirtIO Serial Port"
    }
    fn driver_type(&self) -> DriverType {
        DriverType::Char
    }
    fn init(&mut self) -> Result<(), DriverError> {
        Ok(())
    }
    fn read(&self, _offset: usize, buf: &mut [u8]) -> Result<usize, DriverError> {
        let n = with_bus(|bus| bus.read_port(self.port_id, buf)).unwrap_or(0);
        Ok(n)
    }
    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        with_bus(|bus| bus.write_port(self.port_id, buf)).unwrap_or(Err(DriverError::NoDevice))
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

// ── IRQ handler ──────────────────────────────────────────────────────────────

/// Reads the device ISR (which clears the pending bit) and pumps the control
/// queue if anything was waiting. Crucial when virtio-serial shares IRQ lines
/// with other devices like virtio-input — without acking our ISR the IOAPIC
/// would stay asserted and re-fire endlessly.
fn serial_irq_handler(_irq: u8) {
    let isr_addr = SERIAL_ISR_ADDR.load(core::sync::atomic::Ordering::Relaxed);
    if isr_addr == 0 {
        return;
    }
    // Reading the ISR clears its pending bit (virtio-pci spec §4.1.4.5).
    let isr = virtio::mmio_read8(isr_addr);
    if isr & 1 == 0 {
        return;
    }
    let _ = with_bus(|bus| bus.poll_control());
}

// ── Aliases for well-known port names ───────────────────────────────────────

/// Return a short well-known alias for a virtio-serial port name, or None.
fn well_known_alias(name: &str) -> Option<&'static str> {
    match name {
        "com.redhat.spice.0" => Some("spice"),
        "org.spice-space.webdav.0" => Some("webdav"),
        "org.qemu.guest_agent.0" => Some("qga"),
        _ => None,
    }
}

fn register_named_aliases(port_id: u32, name: &str) {
    // Canonical: /dev/virtio-ports/<name>
    let mut canonical = String::from("/dev/virtio-ports/");
    canonical.push_str(name);
    let canonical_path = canonical.clone();
    hal::register_device(
        &canonical,
        Box::new(VirtioSerialPortDriver { port_id }),
        None,
    );
    crate::serial_verbose_println!(
        "  virtio-serial: aliased port {} as {}",
        port_id,
        canonical_path
    );

    // Short alias for well-known names: /dev/vport-<alias>
    if let Some(alias) = well_known_alias(name) {
        let mut short = String::from("/dev/vport-");
        short.push_str(alias);
        hal::register_device(
            &short,
            Box::new(VirtioSerialPortDriver { port_id }),
            None,
        );
        crate::serial_verbose_println!(
            "  virtio-serial: aliased port {} as {}",
            port_id,
            short
        );
    }
}

// ── Probe Function ───────────────────────────────────────────────────────────

/// Probe and initialize a VirtIO Serial device from the PCI bus.
///
/// Called by the PCI driver table when a matching device is found.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn Driver>> {
    crate::serial_println!(
        "[virtio-serial] probing PCI {:04x}:{:04x}",
        pci.vendor_id,
        pci.device_id
    );

    // Refuse a second virtio-serial PCI device — single-bus design.
    {
        let g = BUS.lock();
        if g.is_some() {
            crate::serial_println!("[virtio-serial] already initialized, skipping");
            return None;
        }
    }

    // 1. Find VirtIO PCI capabilities.
    let caps = match virtio::find_capabilities(pci) {
        Some(c) => c,
        None => {
            crate::serial_println!(
                "[virtio-serial] no modern PCI capabilities — abandoning probe"
            );
            return None;
        }
    };

    // 2. Create device handle (maps BARs).
    let vdev = VirtioDevice::new(pci, &caps);

    // 3. Negotiate features — request MULTIPORT so we see named ports.
    let desired_features = VIRTIO_F_VERSION_1 | VIRTIO_CONSOLE_F_MULTIPORT;
    let negotiated = match vdev.init_device(desired_features) {
        Ok(f) => f,
        Err(e) => {
            crate::serial_println!("[virtio-serial] feature negotiation failed: {}", e);
            return None;
        }
    };
    let multiport = negotiated & VIRTIO_CONSOLE_F_MULTIPORT != 0;

    // 4. Read max_nr_ports from device config (offset 4 in multiport mode).
    let max_nr_ports: u32 = if multiport && vdev.device_cfg != 0 {
        // u32 at device_cfg+4 (cols+rows are u16 each in front).
        unsafe { core::ptr::read_volatile((vdev.device_cfg + 4) as *const u32) }
    } else {
        1
    };
    let port_count = (max_nr_ports as usize).min(MAX_PORTS).max(1);
    crate::serial_println!(
        "[virtio-serial] multiport={} max_nr_ports={} (using {})",
        multiport,
        max_nr_ports,
        port_count
    );

    // 5. Set up port queues. Tolerate per-port failures so a partial port
    //    map still yields a working /dev/vport0 instead of nothing.
    let mut ports: [Option<Port>; MAX_PORTS] = core::array::from_fn(|_| None);
    let mut configured_ports = 0u32;
    for port_id in 0..port_count as u32 {
        let rxq = match vdev.setup_queue(rx_queue_idx(port_id)) {
            Some(q) => q,
            None => {
                crate::serial_println!(
                    "[virtio-serial] port {} rx queue {} unavailable — stopping at {} ports",
                    port_id,
                    rx_queue_idx(port_id),
                    configured_ports
                );
                break;
            }
        };
        let txq = match vdev.setup_queue(tx_queue_idx(port_id)) {
            Some(q) => q,
            None => {
                crate::serial_println!(
                    "[virtio-serial] port {} tx queue {} unavailable — stopping at {} ports",
                    port_id,
                    tx_queue_idx(port_id),
                    configured_ports
                );
                break;
            }
        };
        let rx_phys = match physical::alloc_contiguous(BUF_PAGES) {
            Some(p) => p.as_u64(),
            None => {
                crate::serial_println!("[virtio-serial] OOM allocating port {} rx buffer", port_id);
                break;
            }
        };
        let tx_phys = match physical::alloc_contiguous(BUF_PAGES) {
            Some(p) => p.as_u64(),
            None => {
                crate::serial_println!("[virtio-serial] OOM allocating port {} tx buffer", port_id);
                break;
            }
        };
        unsafe {
            core::ptr::write_bytes(rx_phys as *mut u8, 0, BUF_SIZE);
            core::ptr::write_bytes(tx_phys as *mut u8, 0, BUF_SIZE);
        }
        ports[port_id as usize] = Some(Port {
            rx_queue: rxq,
            tx_queue: txq,
            rx_buf_phys: rx_phys,
            tx_buf_phys: tx_phys,
            rx_pending: [0u8; BUF_SIZE],
            rx_pending_len: 0,
            rx_pending_pos: 0,
            rx_posted: false,
            ready_acked: false,
            name: None,
            opened: false,
        });
        configured_ports += 1;
    }
    if configured_ports == 0 {
        crate::serial_println!("[virtio-serial] no ports could be configured — giving up");
        return None;
    }

    // 6. Control queues (only when MULTIPORT was negotiated). Failure here is
    //    not fatal — we still expose port 0 in single-port mode.
    let mut ctrl_rx_phys: [u64; CTRL_RX_SLOTS] = [0; CTRL_RX_SLOTS];
    let (control_rx, control_tx, ctrl_tx_phys) = if multiport {
        match (
            vdev.setup_queue(CONTROL_RX_QIDX),
            vdev.setup_queue(CONTROL_TX_QIDX),
            physical::alloc_contiguous(BUF_PAGES),
        ) {
            (Some(crx), Some(ctx), Some(txp)) => {
                let tx_p = txp.as_u64();
                unsafe {
                    core::ptr::write_bytes(tx_p as *mut u8, 0, BUF_SIZE);
                }
                // Allocate CTRL_RX_SLOTS separate RX buffers so back-to-back
                // PORT_NAME / PORT_OPEN events from QEMU never get dropped.
                let mut all_ok = true;
                for slot in 0..CTRL_RX_SLOTS {
                    match physical::alloc_contiguous(BUF_PAGES) {
                        Some(p) => {
                            let phys = p.as_u64();
                            unsafe {
                                core::ptr::write_bytes(phys as *mut u8, 0, BUF_SIZE);
                            }
                            ctrl_rx_phys[slot] = phys;
                        }
                        None => {
                            crate::serial_println!(
                                "[virtio-serial] OOM allocating ctrl rx slot {}",
                                slot
                            );
                            all_ok = false;
                            break;
                        }
                    }
                }
                if all_ok {
                    (Some(crx), Some(ctx), tx_p)
                } else {
                    (None, None, 0u64)
                }
            }
            _ => {
                crate::serial_println!(
                    "[virtio-serial] control queue setup failed — running without named-port discovery"
                );
                (None, None, 0u64)
            }
        }
    } else {
        crate::serial_println!(
            "[virtio-serial] device did not negotiate MULTIPORT — single-port mode"
        );
        (None, None, 0u64)
    };

    // 7. Mark device ready.
    vdev.set_driver_ok();

    // 8. Build bus and install singleton.
    let has_control = control_rx.is_some() && control_tx.is_some();
    let mut bus = SerialBus {
        vdev,
        control_rx,
        control_tx,
        ctrl_rx_phys,
        ctrl_rx_in_flight: [None; CTRL_RX_SLOTS],
        ctrl_tx_phys,
        max_nr_ports,
        ports,
    };
    if has_control {
        bus.post_ctrl_rx();
        bus.send_ctrl(0, VIRTIO_CONSOLE_DEVICE_READY, 1);
    }

    // Save ISR address for the shared IRQ handler before installing the bus.
    let isr_addr = bus.vdev.isr_addr;

    {
        let mut guard = BUS.lock();
        *guard = Some(bus);
    }

    // Register IRQ handler *before* the device can plausibly start firing —
    // we already sent DEVICE_READY above, so QEMU may already be queueing
    // PORT_ADD events that would trigger interrupts. With the handler in
    // place we ack our ISR and prevent storms on shared IRQ lines.
    if has_control {
        SERIAL_ISR_ADDR.store(isr_addr, core::sync::atomic::Ordering::Relaxed);
        let irq = pci.interrupt_line;
        crate::serial_println!("[virtio-serial] registering IRQ handler on line {}", irq);
        crate::arch::x86::irq::register_irq(irq, serial_irq_handler);
    }

    // 9. Register /dev/vportN per-port up-front so userspace can open them
    //    even before the host sends PORT_NAME. Named aliases are added
    //    asynchronously when the control channel surfaces them.
    for port_id in 0..configured_ports {
        let mut path = String::from("/dev/vport");
        let _ = write!(path, "{}", port_id);
        hal::register_device(
            &path,
            Box::new(VirtioSerialPortDriver { port_id }),
            if port_id == 0 { Some(pci.clone()) } else { None },
        );
        crate::serial_println!("[virtio-serial] registered {}", path);
    }

    // 10. Drain the control queue with real wall-time waits between polls so
    //     QEMU's iothread has a chance to deliver the back-and-forth:
    //     DEVICE_READY → PORT_ADD → PORT_READY → PORT_NAME → PORT_OPEN.
    //     Each step is one iothread round-trip (~1 ms in KVM, more under TCG).
    //     We poll for ~30 ms — enough for QEMU under KVM to deliver every
    //     event for any port that has a name. Ports without names just stay
    //     anonymous; they'll still respond to /dev/vportN reads.
    if has_control {
        for _ in 0..30 {
            let _ = with_bus(|bus| bus.poll_control());
            crate::arch::hal::delay_ms(1);
        }
    }

    // 11. Now register named-port aliases in HAL — this allocates heap and
    //     touches HAL's lock, so we do it AFTER releasing the BUS spinlock.
    let named_ports: alloc::vec::Vec<(u32, String)> = with_bus(|bus| {
        let mut v = alloc::vec::Vec::new();
        for (i, slot) in bus.ports.iter().enumerate() {
            if let Some(p) = slot {
                if let Some(name) = &p.name {
                    v.push((i as u32, name.clone()));
                }
            }
        }
        v
    })
    .unwrap_or_default();
    for (port_id, name) in named_ports {
        register_named_aliases(port_id, &name);
    }

    // We've already registered devices via HAL with custom paths — return None
    // so the caller doesn't double-register us under /dev/charN.
    let _ = VIRTIO_CONSOLE_F_SIZE;
    None
}

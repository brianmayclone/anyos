//! VirtIO Network PCI driver.
//!
//! Provides Ethernet connectivity in QEMU/KVM VMs using the VirtIO modern
//! transport. Registers as a [`NetworkDriver`] for the kernel network stack.
//!
//! VirtIO device IDs:
//! - 0x1000 (transitional) / 0x1041 (modern) — virtio-net

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::drivers::pci::PciDevice;
use crate::drivers::virtio::{self, VirtioDevice, VIRTIO_F_VERSION_1};
use crate::drivers::virtio::virtqueue::VirtQueue;
use crate::memory::physical;
use crate::sync::spinlock::Spinlock;

// ── Constants ────────────────────────────────────────────────────────────────

/// Number of RX/TX buffers.
const NUM_BUFFERS: usize = 32;

/// Max Ethernet frame size + VirtIO net header.
const RX_BUF_SIZE: usize = 1526 + 12; // MTU 1500 + Ethernet overhead + virtio-net header

/// VirtIO net header size (without mergeable buffers).
const VIRTIO_NET_HDR_SIZE: usize = 12;

/// Feature: device has given MAC address.
const VIRTIO_NET_F_MAC: u64 = 1 << 5;

// ── Driver State ─────────────────────────────────────────────────────────────

struct VirtioNet {
    vdev: VirtioDevice,
    receiveq: VirtQueue,
    transmitq: VirtQueue,
    mac: [u8; 6],
    rx_bufs_phys: [u64; NUM_BUFFERS],
    tx_buf_phys: u64,
    rx_queue: VecDeque<Vec<u8>>,
    rx_posted: usize,
}

// VirtioNet is only accessed under the STATE spinlock.
unsafe impl Send for VirtioNet {}

static STATE: Spinlock<Option<VirtioNet>> = Spinlock::new(None);

// ── Internal ─────────────────────────────────────────────────────────────────

impl VirtioNet {
    /// Post all RX buffers to the receive queue.
    fn post_rx_buffers(&mut self) {
        while self.rx_posted < NUM_BUFFERS {
            let buf_phys = self.rx_bufs_phys[self.rx_posted];
            let writable = [(buf_phys, RX_BUF_SIZE as u32)];
            if self.receiveq.push(&[], &writable).is_some() {
                self.vdev.notify_queue(0);
                self.rx_posted += 1;
            } else {
                break;
            }
        }
    }

    /// Poll the receive queue for completed packets.
    fn poll_rx(&mut self) {
        while let Some((id, len)) = self.receiveq.poll_used() {
            let bytes = len as usize;
            let buf_idx = id as usize;

            // Skip the virtio-net header, copy only the Ethernet frame.
            if bytes > VIRTIO_NET_HDR_SIZE && bytes <= RX_BUF_SIZE && buf_idx < NUM_BUFFERS {
                let frame_len = bytes - VIRTIO_NET_HDR_SIZE;
                let buf_phys = self.rx_bufs_phys[buf_idx];
                let mut packet = Vec::with_capacity(frame_len);
                unsafe {
                    packet.set_len(frame_len);
                    core::ptr::copy_nonoverlapping(
                        (buf_phys + VIRTIO_NET_HDR_SIZE as u64) as *const u8,
                        packet.as_mut_ptr(),
                        frame_len,
                    );
                }
                if self.rx_queue.len() < 1024 {
                    self.rx_queue.push_back(packet);
                }
            }

            // Re-post this buffer immediately so the device can reuse it.
            if buf_idx < NUM_BUFFERS {
                let buf_phys = self.rx_bufs_phys[buf_idx];
                let writable = [(buf_phys, RX_BUF_SIZE as u32)];
                if self.receiveq.push(&[], &writable).is_some() {
                    self.vdev.notify_queue(0);
                }
            }
        }
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Transmit an Ethernet frame via VirtIO Net.
pub fn transmit(data: &[u8]) -> bool {
    let mut state = STATE.lock();
    let net = match state.as_mut() {
        Some(n) => n,
        None => return false,
    };

    if data.len() + VIRTIO_NET_HDR_SIZE > 4096 {
        return false;
    }

    // Write virtio-net header (all zeros = no offload) + frame to TX buffer.
    unsafe {
        core::ptr::write_bytes(net.tx_buf_phys as *mut u8, 0, VIRTIO_NET_HDR_SIZE);
        core::ptr::copy_nonoverlapping(
            data.as_ptr(),
            (net.tx_buf_phys + VIRTIO_NET_HDR_SIZE as u64) as *mut u8,
            data.len(),
        );
    }

    let total_len = (VIRTIO_NET_HDR_SIZE + data.len()) as u32;
    let readable = [(net.tx_buf_phys, total_len)];
    let result = net.transmitq.execute_sync(
        &readable,
        &[],
        || net.vdev.notify_queue(1),
    );
    result.is_some()
}

/// Get MAC address.
pub fn get_mac() -> Option<[u8; 6]> {
    STATE.lock().as_ref().map(|n| n.mac)
}

/// Check if link is up (VirtIO Net is always "up" when probed).
pub fn is_link_up() -> bool {
    STATE.lock().is_some()
}

/// Poll for received packets and feed them to the network stack.
pub fn poll_rx() {
    let mut state = STATE.lock();
    if let Some(net) = state.as_mut() {
        net.poll_rx();
    }
}

/// Dequeue a single received packet.
pub fn recv_packet() -> Option<Vec<u8>> {
    let mut state = STATE.lock();
    let net = state.as_mut()?;
    net.rx_queue.pop_front()
}

// ── NetworkDriver Trait ─────────────────────────────────────────────────────

pub struct VirtioNetDriver;

impl super::NetworkDriver for VirtioNetDriver {
    fn name(&self) -> &str { "VirtIO Net" }
    fn transmit(&mut self, data: &[u8]) -> bool { transmit(data) }
    fn get_mac(&self) -> [u8; 6] { get_mac().unwrap_or([0; 6]) }
    fn link_up(&self) -> bool { is_link_up() }
}

// ── Probe Function ──────────────────────────────────────────────────────────

/// Probe and initialize a VirtIO Net device from the PCI bus.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    crate::serial_verbose_println!("VirtIO Net: probing PCI {:04x}:{:04x}",
        pci.vendor_id, pci.device_id);

    // 1. Find VirtIO PCI capabilities.
    let caps = virtio::find_capabilities(pci)?;

    // 2. Create device handle.
    let vdev = VirtioDevice::new(pci, &caps);

    // 3. Negotiate features.
    let desired_features = VIRTIO_F_VERSION_1 | VIRTIO_NET_F_MAC;
    let negotiated = match vdev.init_device(desired_features) {
        Ok(f) => f,
        Err(e) => {
            crate::serial_verbose_println!("VirtIO Net: init failed: {}", e);
            return None;
        }
    };

    // 4. Read MAC address from device config.
    let mac = if negotiated & VIRTIO_NET_F_MAC != 0 && vdev.device_cfg != 0 {
        let mut m = [0u8; 6];
        for i in 0..6 {
            m[i] = virtio::mmio_read8(vdev.device_cfg + i as u64);
        }
        m
    } else {
        // Generate a locally-administered MAC if device doesn't provide one.
        [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]
    };

    crate::serial_verbose_println!("VirtIO Net: MAC = {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);

    // 5. Set up virtqueues: receiveq (0) and transmitq (1).
    let receiveq = vdev.setup_queue(0)?;
    let transmitq = vdev.setup_queue(1)?;

    // 6. Mark device ready.
    vdev.set_driver_ok();

    // 7. Allocate RX buffers (one page per buffer).
    let mut rx_bufs_phys = [0u64; NUM_BUFFERS];
    for i in 0..NUM_BUFFERS {
        let phys = physical::alloc_contiguous(1)?.as_u64();
        unsafe { core::ptr::write_bytes(phys as *mut u8, 0, 4096); }
        rx_bufs_phys[i] = phys;
    }

    // 8. Allocate TX buffer (one page).
    let tx_buf_phys = physical::alloc_contiguous(1)?.as_u64();
    unsafe { core::ptr::write_bytes(tx_buf_phys as *mut u8, 0, 4096); }

    // 9. Initialize state.
    let mut net = VirtioNet {
        vdev,
        receiveq,
        transmitq,
        mac,
        rx_bufs_phys,
        tx_buf_phys,
        rx_queue: VecDeque::new(),
        rx_posted: 0,
    };

    // Post initial RX buffers.
    net.post_rx_buffers();

    *STATE.lock() = Some(net);

    // Register with the network subsystem.
    super::register(Box::new(VirtioNetDriver));

    crate::serial_verbose_println!("VirtIO Net: initialized");

    super::create_hal_driver("VirtIO Net")
}

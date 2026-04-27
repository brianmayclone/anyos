//! VirtIO Network PCI driver.
//!
//! Provides Ethernet connectivity in QEMU/KVM VMs using the VirtIO modern
//! transport. Registers as a [`NetworkDriver`] for the kernel network stack.
//!
//! Uses interrupt-driven RX via the VirtIO ISR status register for low-latency
//! packet reception. Packets are processed directly in the IRQ handler and
//! fed into the TCP/IP stack.
//!
//! VirtIO device IDs:
//! - 0x1000 (transitional) / 0x1041 (modern) — virtio-net

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::drivers::pci::PciDevice;
use crate::drivers::virtio::virtqueue::VirtQueue;
use crate::drivers::virtio::{self, VirtioDevice, VIRTIO_F_VERSION_1};
use crate::memory::physical;
use crate::sync::spinlock::Spinlock;

// ── Constants ────────────────────────────────────────────────────────────────

/// Number of RX/TX buffers.
const NUM_BUFFERS: usize = 128;
/// Number of dedicated TX buffers kept in flight concurrently.
const NUM_TX_BUFFERS: usize = 64;
const TX_DESC_MAP_SIZE: usize = NUM_BUFFERS;
const INVALID_TX_SLOT: u16 = u16::MAX;
const TX_WAIT_SPINS: u32 = 100_000;

/// Max Ethernet frame size + VirtIO net header.
const RX_BUF_SIZE: usize = 1526 + 12; // MTU 1500 + Ethernet overhead + virtio-net header

/// VirtIO net header size.
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
    tx_bufs_phys: [u64; NUM_TX_BUFFERS],
    tx_free_slots: VecDeque<u16>,
    tx_desc_to_slot: [u16; TX_DESC_MAP_SIZE],
    tx_in_flight: usize,
    rx_queue: VecDeque<Vec<u8>>,
    rx_posted: usize,
}

// VirtioNet is only accessed under the STATE spinlock.
unsafe impl Send for VirtioNet {}

static STATE: Spinlock<Option<VirtioNet>> = Spinlock::new(None);

// ── Internal ─────────────────────────────────────────────────────────────────

impl VirtioNet {
    /// Reclaim completed TX descriptors so their buffers can be reused.
    fn reap_tx_completions(&mut self) {
        while let Some((id, _len)) = self.transmitq.poll_used() {
            let desc_idx = id as usize;
            if desc_idx < self.tx_desc_to_slot.len() {
                let slot = self.tx_desc_to_slot[desc_idx];
                if slot != INVALID_TX_SLOT {
                    self.tx_desc_to_slot[desc_idx] = INVALID_TX_SLOT;
                    self.tx_free_slots.push_back(slot);
                }
            }
            if self.tx_in_flight > 0 {
                self.tx_in_flight -= 1;
            }
        }
    }

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
        let mut reposted = 0usize;
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
                if self.rx_queue.len() < 2048 {
                    self.rx_queue.push_back(packet);
                }
            }

            // Re-post this buffer immediately so the device can reuse it.
            if buf_idx < NUM_BUFFERS {
                let buf_phys = self.rx_bufs_phys[buf_idx];
                let writable = [(buf_phys, RX_BUF_SIZE as u32)];
                if self.receiveq.push(&[], &writable).is_some() {
                    reposted += 1;
                }
            }
        }

        if reposted > 0 {
            self.vdev.notify_queue(0);
        }
    }

    fn wait_for_tx_slot(&mut self) -> Option<usize> {
        for _ in 0..TX_WAIT_SPINS {
            self.reap_tx_completions();
            if let Some(slot) = self.tx_free_slots.pop_front() {
                return Some(slot as usize);
            }
            core::hint::spin_loop();
        }
        None
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

    net.reap_tx_completions();

    let slot = match net.wait_for_tx_slot() {
        Some(slot) => slot,
        None => return false,
    };
    let buf_phys = net.tx_bufs_phys[slot];

    // Write virtio-net header (all zeros = no offload) + frame to a dedicated TX buffer.
    unsafe {
        core::ptr::write_bytes(buf_phys as *mut u8, 0, VIRTIO_NET_HDR_SIZE);
        core::ptr::copy_nonoverlapping(
            data.as_ptr(),
            (buf_phys + VIRTIO_NET_HDR_SIZE as u64) as *mut u8,
            data.len(),
        );
    }

    let total_len = (VIRTIO_NET_HDR_SIZE + data.len()) as u32;
    let readable = [(buf_phys, total_len)];
    let desc_id = match net.transmitq.push(&readable, &[]) {
        Some(id) => id,
        None => {
            net.tx_free_slots.push_front(slot as u16);
            return false;
        }
    };

    if (desc_id as usize) >= net.tx_desc_to_slot.len() {
        net.tx_free_slots.push_front(slot as u16);
        return false;
    }

    net.tx_desc_to_slot[desc_id as usize] = slot as u16;
    net.tx_in_flight += 1;
    net.vdev.notify_queue(1);
    true
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
        net.reap_tx_completions();
        net.poll_rx();
    }
}

/// Dequeue a single received packet.
pub fn recv_packet() -> Option<Vec<u8>> {
    let mut state = STATE.lock();
    let net = state.as_mut()?;
    net.rx_queue.pop_front()
}

// ── IRQ Handler ─────────────────────────────────────────────────────────────

/// ISR address for the VirtIO-Net device (set during probe).
static ISR_ADDR: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

fn virtio_net_irq_handler(_irq: u8) {
    // Read and acknowledge the ISR status register.
    // Reading this register automatically clears the interrupt.
    let isr_addr = ISR_ADDR.load(core::sync::atomic::Ordering::Relaxed);
    if isr_addr == 0 {
        return;
    }
    let isr_status = virtio::mmio_read8(isr_addr);

    if isr_status & 1 == 0 {
        return; // Not our interrupt (shared IRQ line)
    }

    // Queue interrupt: drain received packets from the used ring.
    // Use try_lock to avoid deadlock if we're already holding the lock.
    let mut has_rx = false;
    if let Some(mut state) = STATE.try_lock() {
        if let Some(net) = state.as_mut() {
            net.reap_tx_completions();
            net.poll_rx();
            has_rx = !net.rx_queue.is_empty();
        }
    }

    // Process received packets through the network stack outside the lock.
    // This feeds them into Ethernet → IP → TCP and wakes blocked threads.
    if has_rx {
        while let Some(packet) = recv_packet() {
            crate::net::ethernet::handle_frame(&packet);
        }
    }
}

// ── NetworkDriver Trait ─────────────────────────────────────────────────────

pub struct VirtioNetDriver;

impl super::NetworkDriver for VirtioNetDriver {
    fn name(&self) -> &str {
        "VirtIO Net"
    }
    fn transmit(&mut self, data: &[u8]) -> bool {
        transmit(data)
    }
    fn get_mac(&self) -> [u8; 6] {
        get_mac().unwrap_or([0; 6])
    }
    fn link_up(&self) -> bool {
        is_link_up()
    }
}

// ── Probe Function ──────────────────────────────────────────────────────────

/// Probe and initialize a VirtIO Net device from the PCI bus.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    crate::serial_verbose_println!(
        "VirtIO Net: probing PCI {:04x}:{:04x}",
        pci.vendor_id,
        pci.device_id
    );

    // 1. Find VirtIO PCI capabilities.
    let caps = virtio::find_capabilities(pci)?;

    // 2. Create device handle.
    let vdev = VirtioDevice::new(pci, &caps);

    // 3. Store ISR address for the IRQ handler.
    ISR_ADDR.store(vdev.isr_addr, core::sync::atomic::Ordering::Relaxed);

    // 4. Negotiate features.
    let desired_features = VIRTIO_F_VERSION_1 | VIRTIO_NET_F_MAC;
    let negotiated = match vdev.init_device(desired_features) {
        Ok(f) => f,
        Err(e) => {
            crate::serial_verbose_println!("VirtIO Net: init failed: {}", e);
            return None;
        }
    };

    // 5. Read MAC address from device config.
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

    crate::serial_verbose_println!(
        "VirtIO Net: MAC = {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5]
    );

    // 6. Set up virtqueues: receiveq (0) and transmitq (1).
    let mut receiveq = vdev.setup_queue(0)?;
    let transmitq = vdev.setup_queue(1)?;

    // 7. Enable interrupts on the RX queue.
    // This clears VIRTQ_AVAIL_F_NO_INTERRUPT so the device will raise
    // an interrupt when packets arrive, instead of requiring polling.
    receiveq.enable_interrupts();

    // 8. Mark device ready.
    vdev.set_driver_ok();

    // 9. Allocate RX buffers (one page per buffer).
    let mut rx_bufs_phys = [0u64; NUM_BUFFERS];
    for i in 0..NUM_BUFFERS {
        let phys = physical::alloc_contiguous(1)?.as_u64();
        unsafe {
            core::ptr::write_bytes(phys as *mut u8, 0, 4096);
        }
        rx_bufs_phys[i] = phys;
    }

    // 10. Allocate TX buffer (one page).
    let mut tx_bufs_phys = [0u64; NUM_TX_BUFFERS];
    for slot in tx_bufs_phys.iter_mut() {
        let phys = physical::alloc_contiguous(1)?.as_u64();
        unsafe {
            core::ptr::write_bytes(phys as *mut u8, 0, 4096);
        }
        *slot = phys;
    }

    // 11. Initialize state.
    let mut net = VirtioNet {
        vdev,
        receiveq,
        transmitq,
        mac,
        rx_bufs_phys,
        tx_bufs_phys,
        tx_free_slots: (0..NUM_TX_BUFFERS as u16).collect(),
        tx_desc_to_slot: [INVALID_TX_SLOT; TX_DESC_MAP_SIZE],
        tx_in_flight: 0,
        rx_queue: VecDeque::new(),
        rx_posted: 0,
    };

    // Post initial RX buffers.
    net.post_rx_buffers();

    *STATE.lock() = Some(net);

    // 12. Register IRQ handler for interrupt-driven RX.
    let irq = pci.interrupt_line;
    crate::serial_verbose_println!("VirtIO Net: IRQ = {}", irq);
    crate::arch::x86::irq::register_irq(irq, virtio_net_irq_handler);
    if crate::arch::x86::apic::is_initialized() {
        crate::arch::x86::ioapic::unmask_irq(irq);
    } else {
        crate::arch::x86::pic::unmask(irq);
    }

    // Register with the network subsystem.
    super::register(Box::new(VirtioNetDriver));

    crate::serial_verbose_println!(
        "VirtIO Net: initialized (IRQ-driven RX, {} buffers)",
        NUM_BUFFERS
    );

    super::create_hal_driver("VirtIO Net")
}

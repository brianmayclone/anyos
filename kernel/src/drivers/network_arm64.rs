//! ARM64 network subsystem with VirtIO-MMIO support.
//!
//! This mirrors the common `crate::drivers::network::*` facade that the rest of
//! the kernel already uses on x86, but backs it with an ARM64-native VirtIO Net
//! implementation for QEMU `virt`.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::ptr;

use crate::memory::physical;
use crate::sync::spinlock::Spinlock;

use crate::drivers::arm::VirtioMmioDevice;
use crate::drivers::arm::virtqueue::{VirtQueue, VRING_DESC_F_WRITE, DEFAULT_QUEUE_SIZE};

const RX_QUEUE_SIZE: usize = DEFAULT_QUEUE_SIZE as usize;
const RX_BUF_SIZE: usize = 2048;
const TX_BUF_SIZE: usize = 4096;
const VIRTIO_NET_HDR_SIZE: usize = 12;
const VIRTIO_NET_F_MAC: u32 = 1 << 5;

#[inline]
fn phys_to_virt(phys: u64) -> usize {
    (phys + 0xFFFF_0000_4000_0000) as usize
}

pub trait NetworkDriver: Send {
    fn name(&self) -> &str;
    fn transmit(&mut self, data: &[u8]) -> bool;
    fn get_mac(&self) -> [u8; 6];
    fn link_up(&self) -> bool;
    fn set_enabled(&mut self, _enabled: bool) {}
    fn is_enabled(&self) -> bool { true }
    fn driver_name(&self) -> &str { self.name() }
}

static NET: Spinlock<Option<Box<dyn NetworkDriver>>> = Spinlock::new(None);

pub fn register(driver: Box<dyn NetworkDriver>) {
    crate::serial_verbose_println!("  Network: registered '{}'", driver.name());
    *NET.lock() = Some(driver);
}

pub fn with_net<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut dyn NetworkDriver) -> R,
{
    let mut net = NET.lock();
    let driver = net.as_mut()?;
    Some(f(driver.as_mut()))
}

pub fn transmit(data: &[u8]) -> bool {
    with_net(|d| d.transmit(data)).unwrap_or(false)
}

pub fn get_mac() -> Option<[u8; 6]> {
    with_net(|d| d.get_mac())
}

pub fn is_available() -> bool {
    NET.lock().is_some()
}

pub fn link_up() -> bool {
    with_net(|d| d.link_up()).unwrap_or(false)
}

pub fn set_enabled(enabled: bool) {
    let _ = with_net(|d| d.set_enabled(enabled));
}

pub fn is_enabled() -> bool {
    with_net(|d| d.is_enabled()).unwrap_or(false)
}

pub fn driver_name() -> Option<String> {
    with_net(|d| String::from(d.driver_name()))
}

pub fn with_wifi<F, R>(_f: F) -> Option<R>
where
    F: FnOnce(&mut dyn NetworkDriver) -> R,
{
    None
}

pub fn wifi_available() -> bool {
    false
}

struct VirtioNet {
    base: usize,
    irq: u32,
    receiveq: VirtQueue,
    transmitq: VirtQueue,
    mac: [u8; 6],
    enabled: bool,
    rx_bufs_phys: [u64; RX_QUEUE_SIZE],
    tx_buf_phys: u64,
    rx_queue: VecDeque<Vec<u8>>,
}

unsafe impl Send for VirtioNet {}

static STATE: Spinlock<Option<VirtioNet>> = Spinlock::new(None);

impl VirtioNet {
    fn post_rx_buffers(&mut self) -> bool {
        for &buf_phys in &self.rx_bufs_phys {
            if self.receiveq.push_buf(buf_phys, RX_BUF_SIZE as u32, VRING_DESC_F_WRITE).is_none() {
                return false;
            }
        }
        self.notify_rx();
        true
    }

    fn notify_rx(&self) {
        unsafe {
            core::arch::asm!("dsb sy", options(nostack, preserves_flags));
            ptr::write_volatile((self.base + 0x050) as *mut u32, 0);
        }
    }

    fn notify_tx(&self) {
        unsafe {
            core::arch::asm!("dsb sy", options(nostack, preserves_flags));
            ptr::write_volatile((self.base + 0x050) as *mut u32, 1);
        }
    }

    fn ack_interrupt(&self) -> u32 {
        let status = unsafe { ptr::read_volatile((self.base + 0x060) as *const u32) };
        unsafe { ptr::write_volatile((self.base + 0x064) as *mut u32, status); }
        status
    }

    fn poll_rx(&mut self) {
        while let Some((id, len)) = self.receiveq.pop_used() {
            let buf_idx = id as usize;
            if buf_idx < RX_QUEUE_SIZE {
                let bytes = len as usize;
                if bytes > VIRTIO_NET_HDR_SIZE && bytes <= RX_BUF_SIZE {
                    let payload_len = bytes - VIRTIO_NET_HDR_SIZE;
                    let buf_virt = phys_to_virt(self.rx_bufs_phys[buf_idx]);
                    let mut packet = Vec::with_capacity(payload_len);
                    unsafe {
                        packet.set_len(payload_len);
                        ptr::copy_nonoverlapping(
                            (buf_virt + VIRTIO_NET_HDR_SIZE) as *const u8,
                            packet.as_mut_ptr(),
                            payload_len,
                        );
                    }
                    if self.rx_queue.len() < 2048 {
                        self.rx_queue.push_back(packet);
                    }
                }

                let buf_phys = self.rx_bufs_phys[buf_idx];
                if self.receiveq.push_buf(buf_phys, RX_BUF_SIZE as u32, VRING_DESC_F_WRITE).is_some() {
                    self.notify_rx();
                }
            }
        }
    }

    fn transmit(&mut self, data: &[u8]) -> bool {
        if !self.enabled || data.len() + VIRTIO_NET_HDR_SIZE > TX_BUF_SIZE {
            return false;
        }

        let tx_virt = phys_to_virt(self.tx_buf_phys);
        unsafe {
            ptr::write_bytes(tx_virt as *mut u8, 0, VIRTIO_NET_HDR_SIZE);
            ptr::copy_nonoverlapping(
                data.as_ptr(),
                (tx_virt + VIRTIO_NET_HDR_SIZE) as *mut u8,
                data.len(),
            );
        }

        let total_len = (VIRTIO_NET_HDR_SIZE + data.len()) as u32;
        if self.transmitq.push_buf(self.tx_buf_phys, total_len, 0).is_none() {
            return false;
        }
        self.notify_tx();

        let start_tick = crate::arch::hal::timer_current_ticks();
        let max_wait_ticks = (crate::arch::hal::timer_frequency_hz() / 2).max(1) as u32;
        while !self.transmitq.has_used() {
            core::hint::spin_loop();
            self.poll_rx();
            let now = crate::arch::hal::timer_current_ticks();
            if now.wrapping_sub(start_tick) >= max_wait_ticks {
                crate::serial_verbose_println!("  virtio-net(arm64): TX timeout len={}", data.len());
                return false;
            }
        }

        self.transmitq.pop_used().is_some()
    }
}

struct VirtioNetDriver;

impl NetworkDriver for VirtioNetDriver {
    fn name(&self) -> &str { "VirtIO Net" }

    fn transmit(&mut self, data: &[u8]) -> bool {
        let mut state = STATE.lock();
        match state.as_mut() {
            Some(net) => net.transmit(data),
            None => false,
        }
    }

    fn get_mac(&self) -> [u8; 6] {
        STATE.lock().as_ref().map(|net| net.mac).unwrap_or([0; 6])
    }

    fn link_up(&self) -> bool {
        STATE.lock().as_ref().map(|net| net.enabled).unwrap_or(false)
    }

    fn set_enabled(&mut self, enabled: bool) {
        if let Some(net) = STATE.lock().as_mut() {
            net.enabled = enabled;
        }
    }

    fn is_enabled(&self) -> bool {
        STATE.lock().as_ref().map(|net| net.enabled).unwrap_or(false)
    }

    fn driver_name(&self) -> &str { "virtio-net" }
}

pub fn init_mmio(dev: &VirtioMmioDevice) {
    if dev.init_device(VIRTIO_NET_F_MAC).is_none() {
        crate::serial_verbose_println!("  virtio-net(arm64): feature negotiation failed");
        return;
    }

    let mut mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    for (idx, byte) in mac.iter_mut().enumerate() {
        *byte = dev.read_config_u8(idx);
    }

    let receiveq = match VirtQueue::new(0, DEFAULT_QUEUE_SIZE) {
        Some(q) => q,
        None => {
            crate::serial_verbose_println!("  virtio-net(arm64): failed to allocate RX queue");
            return;
        }
    };
    let transmitq = match VirtQueue::new(1, DEFAULT_QUEUE_SIZE) {
        Some(q) => q,
        None => {
            crate::serial_verbose_println!("  virtio-net(arm64): failed to allocate TX queue");
            return;
        }
    };

    let (rx_desc, rx_avail, rx_used) = receiveq.phys_addrs();
    if !dev.setup_queue_raw(0, DEFAULT_QUEUE_SIZE, rx_desc, rx_avail, rx_used) {
        crate::serial_verbose_println!("  virtio-net(arm64): failed to setup RX queue");
        return;
    }

    let (tx_desc, tx_avail, tx_used) = transmitq.phys_addrs();
    if !dev.setup_queue_raw(1, DEFAULT_QUEUE_SIZE, tx_desc, tx_avail, tx_used) {
        crate::serial_verbose_println!("  virtio-net(arm64): failed to setup TX queue");
        return;
    }

    let mut rx_bufs_phys = [0u64; RX_QUEUE_SIZE];
    for slot in &mut rx_bufs_phys {
        let frame = match physical::alloc_frame() {
            Some(frame) => frame,
            None => {
                crate::serial_verbose_println!("  virtio-net(arm64): out of memory for RX buffers");
                return;
            }
        };
        *slot = frame.0;
        unsafe {
            ptr::write_bytes(phys_to_virt(frame.0) as *mut u8, 0, RX_BUF_SIZE);
        }
    }

    let tx_frame = match physical::alloc_frame() {
        Some(frame) => frame,
        None => {
            crate::serial_verbose_println!("  virtio-net(arm64): out of memory for TX buffer");
            return;
        }
    };
    unsafe {
        ptr::write_bytes(phys_to_virt(tx_frame.0) as *mut u8, 0, TX_BUF_SIZE);
    }

    let mut net = VirtioNet {
        base: dev.base(),
        irq: dev.irq(),
        receiveq,
        transmitq,
        mac,
        enabled: true,
        rx_bufs_phys,
        tx_buf_phys: tx_frame.0,
        rx_queue: VecDeque::new(),
    };

    if !net.post_rx_buffers() {
        crate::serial_verbose_println!("  virtio-net(arm64): failed to post RX buffers");
        return;
    }

    dev.driver_ok();
    crate::arch::arm64::gic::enable_irq(dev.irq());
    crate::arch::arm64::exceptions::register_irq(dev.irq(), virtio_net_irq_handler);

    *STATE.lock() = Some(net);
    register(Box::new(VirtioNetDriver));

    crate::serial_println!(
        "  virtio-net(arm64): MAC={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, IRQ {}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], dev.irq()
    );
}

pub fn poll_rx() {
    let mut state = STATE.lock();
    if let Some(net) = state.as_mut() {
        net.poll_rx();
    }
}

pub fn recv_packet() -> Option<Vec<u8>> {
    let mut state = STATE.lock();
    let net = state.as_mut()?;
    net.rx_queue.pop_front()
}

fn virtio_net_irq_handler() {
    let mut state = match STATE.try_lock() {
        Some(state) => state,
        None => return,
    };
    let net = match state.as_mut() {
        Some(net) => net,
        None => return,
    };
    let status = net.ack_interrupt();
    if status == 0 {
        return;
    }
    net.poll_rx();
}

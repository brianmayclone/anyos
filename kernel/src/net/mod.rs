//! Network stack coordinator.
//! Provides global network configuration, packet polling, and sub-module access.

pub mod types;
pub mod checksum;
pub mod ethernet;
pub mod arp;
pub mod ipv4;
pub mod icmp;
pub mod udp;
pub mod dhcp;
pub mod dns;
pub mod tcp;
pub mod interfaces;

use alloc::vec::Vec;
use types::{Ipv4Addr, MacAddr, NetConfig};
use crate::sync::spinlock::Spinlock;
use core::sync::atomic::{AtomicU32, Ordering};

/// Global network configuration protected by a spinlock.
static NET_CONFIG: Spinlock<NetConfig> = Spinlock::new(NetConfig::new());

/// Initialize the network stack. Call after E1000 driver is initialized.
pub fn init() {
    // Get MAC from E1000
    let mac_bytes = crate::drivers::network::e1000::get_mac().unwrap_or([0; 6]);
    let mac = MacAddr(mac_bytes);

    {
        let mut cfg = NET_CONFIG.lock();
        cfg.mac = mac;
    }

    arp::init();
    icmp::init();
    udp::init();
    tcp::init();

    crate::serial_println!("[OK] Network stack initialized (MAC={})", mac);
}

/// Get a snapshot of the current network config.
pub fn config() -> NetConfig {
    let cfg = NET_CONFIG.lock();
    NetConfig {
        ip: cfg.ip,
        mask: cfg.mask,
        gateway: cfg.gateway,
        dns: cfg.dns,
        mac: cfg.mac,
    }
}

/// Update network configuration (e.g. after DHCP).
pub fn set_config(ip: Ipv4Addr, mask: Ipv4Addr, gateway: Ipv4Addr, dns: Ipv4Addr) {
    let mut cfg = NET_CONFIG.lock();
    cfg.ip = ip;
    cfg.mask = mask;
    cfg.gateway = gateway;
    cfg.dns = dns;
}

/// Load network configuration files from disk (hosts + interfaces).
/// Call after VFS is initialized and the root filesystem is mounted.
pub fn load_config_files() {
    dns::load_hosts();
    interfaces::load_interfaces();
}

/// Tick counter for rate-limiting retransmission checks.
static LAST_RETRANSMIT_CHECK: AtomicU32 = AtomicU32::new(0);

/// Poll for incoming packets and dispatch them through the protocol stack.
/// Call this from any context that needs to process network traffic.
pub fn poll() {
    poll_rx();

    // Rate-limit retransmission checks (every 10 ticks = 100ms).
    // This avoids expensive TCP_CONNECTIONS lock acquisition on every poll.
    let now = crate::arch::x86::pit::get_ticks();
    let last = LAST_RETRANSMIT_CHECK.load(Ordering::Relaxed);
    if now.wrapping_sub(last) >= 10 {
        LAST_RETRANSMIT_CHECK.store(now, Ordering::Relaxed);
        tcp::check_retransmissions();
    }
}

/// Fast path: process incoming packets only, no retransmission checks.
/// Used by recv/send hot paths and IRQ handler for maximum throughput.
pub fn poll_rx() {
    // Batch-drain E1000 rx_queue (single lock acquisition)
    let mut packets: Vec<Vec<u8>> = Vec::new();
    crate::drivers::network::e1000::recv_all_packets(&mut packets);
    for packet in packets.iter() {
        ethernet::handle_frame(packet);
    }

    // Poll hardware RX ring in case IRQs were missed, then drain again
    crate::drivers::network::e1000::poll_rx();
    packets.clear();
    crate::drivers::network::e1000::recv_all_packets(&mut packets);
    for packet in packets.iter() {
        ethernet::handle_frame(packet);
    }

    // Process CDC-ECM (USB Ethernet) RX packets
    while let Some(packet) = crate::drivers::usb::cdc_ecm::recv_packet() {
        ethernet::handle_frame(&packet);
    }
}

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

use types::{Ipv4Addr, MacAddr, NetConfig};
use crate::sync::spinlock::Spinlock;

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

/// Poll for incoming packets and dispatch them through the protocol stack.
/// Call this from any context that needs to process network traffic.
pub fn poll() {
    // Process all pending RX packets
    while let Some(packet) = crate::drivers::network::e1000::recv_packet() {
        ethernet::handle_frame(&packet);
    }

    // Also do a hardware RX ring poll in case IRQs were missed
    crate::drivers::network::e1000::poll_rx();
    while let Some(packet) = crate::drivers::network::e1000::recv_packet() {
        ethernet::handle_frame(&packet);
    }

    // Process CDC-ECM (USB Ethernet) RX packets
    while let Some(packet) = crate::drivers::usb::cdc_ecm::recv_packet() {
        ethernet::handle_frame(&packet);
    }

    // TCP retransmission and TIME_WAIT cleanup
    tcp::check_retransmissions();
}

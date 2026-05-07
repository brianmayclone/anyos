//! Network stack coordinator.
//! Provides global network configuration, packet polling, and sub-module access.

pub mod arp;
pub mod checksum;
pub mod dhcp;
pub mod dns;
pub mod ethernet;
pub mod icmp;
pub mod icmpv6;
pub mod interfaces;
pub mod ipv4;
pub mod ipv6;
pub mod ndp;
pub mod tcp;
pub mod trace;
pub mod types;
pub mod udp;
pub mod wifi;

use crate::sync::spinlock::Spinlock;
#[allow(unused_imports)]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use types::{Ipv4Addr, Ipv6Addr, MacAddr, NetConfig};

/// Global network configuration protected by a spinlock.
static NET_CONFIG: Spinlock<NetConfig> = Spinlock::new(NetConfig::new());

/// Initialize the network stack. Call after NIC driver is initialized.
pub fn init() {
    // Get MAC from the registered NIC driver (E1000, VirtIO Net, etc.)
    let mac_bytes = crate::drivers::network::get_mac().unwrap_or([0; 6]);
    let mac = MacAddr(mac_bytes);

    // Generate IPv6 link-local address from MAC (EUI-64)
    let link_local = Ipv6Addr::from_mac_link_local(&mac);

    {
        let mut cfg = NET_CONFIG.lock();
        cfg.mac = mac;
        cfg.ipv6_link_local = link_local;
    }

    arp::init();
    icmp::init();
    udp::init();
    tcp::init();

    crate::serial_verbose_println!(
        "[OK] Network stack initialized (MAC={}, IPv6 LL={})",
        mac,
        link_local
    );
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
        ipv6_link_local: cfg.ipv6_link_local,
        ipv6_addr: cfg.ipv6_addr,
        ipv6_prefix_len: cfg.ipv6_prefix_len,
        ipv6_gateway: cfg.ipv6_gateway,
        ipv6_dns: cfg.ipv6_dns,
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

/// Update IPv6 network configuration (e.g. after SLAAC or DHCPv6).
pub fn set_config_v6(addr: Ipv6Addr, prefix_len: u8, gateway: Ipv6Addr, dns: Ipv6Addr) {
    let mut cfg = NET_CONFIG.lock();
    cfg.ipv6_addr = addr;
    cfg.ipv6_prefix_len = prefix_len;
    cfg.ipv6_gateway = gateway;
    cfg.ipv6_dns = dns;
}

/// Load network configuration files from disk (hosts + interfaces).
/// Call after VFS is initialized and the root filesystem is mounted.
pub fn load_config_files() {
    dns::load_hosts();
    interfaces::load_interfaces();

    // Send Router Solicitation to discover IPv6 routers and trigger SLAAC
    ndp::send_router_solicitation();
}

/// Tick counter for rate-limiting retransmission checks.
static LAST_RETRANSMIT_CHECK: AtomicU32 = AtomicU32::new(0);

/// Poll for incoming packets and dispatch them through the protocol stack.
/// Call this from any context that needs to process network traffic.
pub fn poll() {
    poll_rx();

    // Rate-limit retransmission checks (every 2 ticks = 20ms).
    // Must match DELAYED_ACK_TICKS so delayed ACKs are flushed on time.
    let now = crate::arch::hal::timer_current_ticks();
    let last = LAST_RETRANSMIT_CHECK.load(Ordering::Relaxed);
    if now.wrapping_sub(last) >= 2 {
        LAST_RETRANSMIT_CHECK.store(now, Ordering::Relaxed);
        tcp::check_retransmissions();
    }
}

/// Fast path: process incoming packets only, no retransmission checks.
/// Used by recv/send hot paths and IRQ handler for maximum throughput.
pub fn poll_rx() {
    #[cfg(target_arch = "x86_64")]
    {
        let mut packets: Vec<Vec<u8>> = Vec::new();

        // E1000: batch-drain rx_queue + hardware ring in one driver lock hold.
        crate::drivers::network::e1000::poll_rx_into(&mut packets);
        for packet in packets.iter() {
            ethernet::handle_frame(packet);
        }

        // VirtIO Net: batch-drain rx_queue + used ring in one driver lock hold.
        packets.clear();
        crate::drivers::network::virtio_net::poll_rx_into(&mut packets);
        for packet in packets.iter() {
            ethernet::handle_frame(packet);
        }

        // CDC-ECM (USB Ethernet)
        while let Some(packet) = crate::drivers::usb::cdc_ecm::recv_packet() {
            ethernet::handle_frame(&packet);
        }

        // RTL8188EU WiFi
        while let Some(packet) = crate::drivers::network::rtl8188eu::recv_packet() {
            ethernet::handle_frame(&packet);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        crate::drivers::network::poll_rx();
        while let Some(packet) = crate::drivers::network::recv_packet() {
            ethernet::handle_frame(&packet);
        }
    }
}

/// Yield briefly between polling attempts in blocking network helpers.
///
/// Normal syscall/kernel-thread callers should not busy-spin while waiting for
/// ARP/NDP/DNS/ICMP/DHCP progress. If interrupts are currently disabled, we may
/// be in an IRQ or another non-preemptible section, so falling back to a CPU
/// pause is safer than entering the scheduler.
pub(crate) fn wait_for_poll_progress() {
    if crate::arch::hal::interrupts_enabled() {
        let wake_at = crate::arch::hal::timer_current_ticks().wrapping_add(1);
        crate::task::scheduler::sleep_until(wake_at);
    } else {
        core::hint::spin_loop();
    }
}

//! Generic network backend abstraction.
//!
//! A [`NetBackend`] moves Ethernet frames between the E1000 device model and
//! some host-side transport (TAP, SLIRP, vhost-net, …).  The trait is
//! intentionally minimal so new backends can be added with little effort.

#[cfg(feature = "std")]
extern crate alloc;
#[cfg(feature = "std")]
use alloc::vec::Vec;
#[cfg(feature = "std")]
use alloc::collections::VecDeque;

/// Network backend mode — selectable from VMManager / config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetMode {
    /// No host networking — packets are silently dropped.
    None,
    /// User-mode networking (built-in NAT + DHCP, no root required).
    UserMode,
    /// TAP device bridged to a host interface (requires CAP_NET_ADMIN).
    Tap,
}

impl Default for NetMode {
    fn default() -> Self { NetMode::None }
}

/// Trait that every network backend must implement.
///
/// The VM loop calls [`send`] with Ethernet frames from the guest (TX path)
/// and [`recv`] to collect frames destined for the guest (RX path).
/// Both methods must be non-blocking.
#[cfg(feature = "std")]
pub trait NetBackend: Send {
    /// Send an Ethernet frame from guest to the network.
    /// Must not block. Drop silently if the backend cannot accept right now.
    fn send(&mut self, frame: &[u8]);

    /// Receive pending Ethernet frames destined for the guest.
    /// Returns frames collected since the last call.  Must not block.
    fn recv(&mut self) -> Vec<Vec<u8>>;

    /// Tick / poll — called periodically from the VM loop.
    /// Backends that need periodic work (e.g. SLIRP timer expiry) do it here.
    fn poll(&mut self) {}

    /// Human-readable description for diagnostics.
    fn description(&self) -> &str;
}

/// Null backend — silently drops all traffic.
#[cfg(feature = "std")]
pub struct NullNet;

#[cfg(feature = "std")]
impl NetBackend for NullNet {
    fn send(&mut self, _frame: &[u8]) {}
    fn recv(&mut self) -> Vec<Vec<u8>> { Vec::new() }
    fn description(&self) -> &str { "none (disconnected)" }
}

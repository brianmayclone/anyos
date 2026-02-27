//! Network interface configuration — parses and applies `/System/etc/network/interfaces`.
//!
//! Supports DHCP and static IPv4 configuration per interface.  The parsed
//! configuration is cached in memory so userspace can query it via syscall.

use alloc::string::String;
use alloc::vec::Vec;
use super::types::Ipv4Addr;
use crate::sync::spinlock::Spinlock;

const INTERFACES_PATH: &str = "/System/etc/network/interfaces";
const MAX_INTERFACES: usize = 8;

/// Configuration method for a network interface.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IfaceMethod {
    Dhcp,
    Static,
}

/// Parsed configuration for a single network interface.
#[derive(Clone)]
pub struct IfaceConfig {
    /// Interface name (e.g. "eth0").
    pub name: String,
    /// Configuration method.
    pub method: IfaceMethod,
    /// Static IP address (only meaningful when method == Static).
    pub address: Ipv4Addr,
    /// Subnet mask (only meaningful when method == Static).
    pub netmask: Ipv4Addr,
    /// Default gateway (only meaningful when method == Static).
    pub gateway: Ipv4Addr,
    /// DNS server (only meaningful when method == Static).
    pub dns: Ipv4Addr,
}

impl IfaceConfig {
    /// Create a default DHCP interface configuration.
    fn new_dhcp(name: &str) -> Self {
        IfaceConfig {
            name: String::from(name),
            method: IfaceMethod::Dhcp,
            address: Ipv4Addr::ZERO,
            netmask: Ipv4Addr::ZERO,
            gateway: Ipv4Addr::ZERO,
            dns: Ipv4Addr::ZERO,
        }
    }
}

/// Cached interface configurations, loaded from disk.
static IFACE_CONFIGS: Spinlock<Vec<IfaceConfig>> = Spinlock::new(Vec::new());

/// Load (or reload) the interfaces configuration file from disk.
/// Called at boot after VFS init, and on demand via syscall.
pub fn load_interfaces() {
    let data = match crate::fs::vfs::read_file_to_vec(INTERFACES_PATH) {
        Ok(d) => d,
        Err(_) => {
            crate::serial_println!("[NET] interfaces file not found: {}", INTERFACES_PATH);
            return;
        }
    };

    let text = match core::str::from_utf8(&data) {
        Ok(t) => t,
        Err(_) => {
            crate::serial_println!("[NET] interfaces file is not valid UTF-8");
            return;
        }
    };

    let configs = parse_interfaces(text);
    let count = configs.len();
    {
        let mut table = IFACE_CONFIGS.lock();
        *table = configs;
    }
    crate::serial_println!("[OK] Loaded {} interface configs from {}", count, INTERFACES_PATH);
}

/// Parse the interfaces config text into a list of interface configurations.
fn parse_interfaces(text: &str) -> Vec<IfaceConfig> {
    let mut configs: Vec<IfaceConfig> = Vec::new();

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let keyword = match parts.next() {
            Some(k) => k,
            None => continue,
        };

        match keyword {
            "iface" => {
                if configs.len() >= MAX_INTERFACES {
                    break;
                }
                let name = match parts.next() {
                    Some(n) => n,
                    None => continue,
                };
                let method_str = match parts.next() {
                    Some(m) => m,
                    None => continue,
                };
                let method = match method_str {
                    "dhcp" => IfaceMethod::Dhcp,
                    "static" => IfaceMethod::Static,
                    _ => continue,
                };
                configs.push(IfaceConfig {
                    name: String::from(name),
                    method,
                    address: Ipv4Addr::ZERO,
                    netmask: Ipv4Addr::ZERO,
                    gateway: Ipv4Addr::ZERO,
                    dns: Ipv4Addr::ZERO,
                });
            }
            "address" | "netmask" | "gateway" | "dns" => {
                // These are sub-keys for the most recent iface block
                if let Some(cfg) = configs.last_mut() {
                    if let Some(val_str) = parts.next() {
                        if let Some(addr) = Ipv4Addr::parse(val_str) {
                            match keyword {
                                "address" => cfg.address = addr,
                                "netmask" => cfg.netmask = addr,
                                "gateway" => cfg.gateway = addr,
                                "dns" => cfg.dns = addr,
                                _ => {}
                            }
                        }
                    }
                }
            }
            _ => {} // Ignore unknown keywords
        }
    }

    configs
}

/// Get a snapshot of the current interface configurations.
pub fn get_configs() -> Vec<IfaceConfig> {
    IFACE_CONFIGS.lock().clone()
}

/// Serialize interface configs into a byte buffer for userspace consumption.
///
/// Format per entry (64 bytes):
///   - `[0]`:    method (0=dhcp, 1=static)
///   - `[1]`:    name length
///   - `[2..18]`: name bytes (max 16 chars, null-padded)
///   - `[18..22]`: address
///   - `[22..26]`: netmask
///   - `[26..30]`: gateway
///   - `[30..34]`: dns
///   - `[34..64]`: reserved (zeroed)
///
/// Returns the number of entries written.
pub fn serialize_configs(buf: &mut [u8]) -> u32 {
    let configs = IFACE_CONFIGS.lock();
    let max_entries = buf.len() / 64;
    let count = configs.len().min(max_entries);

    for (i, cfg) in configs.iter().enumerate() {
        if i >= count {
            break;
        }
        let off = i * 64;

        // Zero out the entry
        for b in &mut buf[off..off + 64] {
            *b = 0;
        }

        buf[off] = match cfg.method {
            IfaceMethod::Dhcp => 0,
            IfaceMethod::Static => 1,
        };

        let name_bytes = cfg.name.as_bytes();
        let name_len = name_bytes.len().min(16);
        buf[off + 1] = name_len as u8;
        buf[off + 2..off + 2 + name_len].copy_from_slice(&name_bytes[..name_len]);

        buf[off + 18..off + 22].copy_from_slice(&cfg.address.0);
        buf[off + 22..off + 26].copy_from_slice(&cfg.netmask.0);
        buf[off + 26..off + 30].copy_from_slice(&cfg.gateway.0);
        buf[off + 30..off + 34].copy_from_slice(&cfg.dns.0);
    }

    count as u32
}

/// Deserialize interface configs from a userspace buffer and save to disk.
///
/// Uses the same 64-byte-per-entry format as `serialize_configs`.
/// After parsing, writes the new config to `/System/etc/network/interfaces`
/// and updates the in-memory cache.
pub fn apply_and_save(buf: &[u8], count: u32) -> u32 {
    let count = count as usize;
    if count == 0 || buf.len() < count * 64 {
        return u32::MAX;
    }

    let mut configs: Vec<IfaceConfig> = Vec::new();

    for i in 0..count {
        let off = i * 64;
        let method = match buf[off] {
            0 => IfaceMethod::Dhcp,
            1 => IfaceMethod::Static,
            _ => continue,
        };

        let name_len = (buf[off + 1] as usize).min(16);
        let name = match core::str::from_utf8(&buf[off + 2..off + 2 + name_len]) {
            Ok(s) => String::from(s),
            Err(_) => continue,
        };

        let address = Ipv4Addr([buf[off + 18], buf[off + 19], buf[off + 20], buf[off + 21]]);
        let netmask = Ipv4Addr([buf[off + 22], buf[off + 23], buf[off + 24], buf[off + 25]]);
        let gateway = Ipv4Addr([buf[off + 26], buf[off + 27], buf[off + 28], buf[off + 29]]);
        let dns = Ipv4Addr([buf[off + 30], buf[off + 31], buf[off + 32], buf[off + 33]]);

        configs.push(IfaceConfig {
            name,
            method,
            address,
            netmask,
            gateway,
            dns,
        });
    }

    // Generate config file text
    let mut text = String::from("# anyOS network interfaces configuration\n");
    text.push_str("#\n");
    text.push_str("# Syntax:\n");
    text.push_str("#   iface <name> dhcp\n");
    text.push_str("#\n");
    text.push_str("#   iface <name> static\n");
    text.push_str("#     address <ip>\n");
    text.push_str("#     netmask <mask>\n");
    text.push_str("#     gateway <gw>\n");
    text.push_str("#     dns <dns>\n");
    text.push('\n');

    for cfg in &configs {
        match cfg.method {
            IfaceMethod::Dhcp => {
                text.push_str("iface ");
                text.push_str(&cfg.name);
                text.push_str(" dhcp\n");
            }
            IfaceMethod::Static => {
                text.push_str("iface ");
                text.push_str(&cfg.name);
                text.push_str(" static\n");
                push_ip_line(&mut text, "  address ", cfg.address);
                push_ip_line(&mut text, "  netmask ", cfg.netmask);
                push_ip_line(&mut text, "  gateway ", cfg.gateway);
                push_ip_line(&mut text, "  dns ", cfg.dns);
            }
        }
    }

    // Write to disk
    let write_ok = write_file(INTERFACES_PATH, text.as_bytes());

    // Update in-memory cache
    {
        let mut table = IFACE_CONFIGS.lock();
        *table = configs.clone();
    }

    // Apply the first interface config to the active network stack
    if let Some(cfg) = configs.first() {
        if cfg.method == IfaceMethod::Static {
            super::set_config(cfg.address, cfg.netmask, cfg.gateway, cfg.dns);
            crate::serial_println!("[NET] Applied static config for {}: {}", cfg.name, cfg.address);
        }
        // DHCP is handled by the dhcp binary at boot; no immediate action here
    }

    if write_ok { 0 } else { u32::MAX }
}

/// Helper: append "  <prefix><ip>\n" to a string.
fn push_ip_line(text: &mut String, prefix: &str, ip: Ipv4Addr) {
    use core::fmt::Write;
    text.push_str(prefix);
    let _ = write!(text, "{}", ip);
    text.push('\n');
}

/// Write bytes to a file, creating/truncating as needed.
fn write_file(path: &str, data: &[u8]) -> bool {
    use crate::fs::vfs;
    use crate::fs::file::FileFlags;
    let fd = match vfs::open(path, FileFlags::CREATE_WRITE) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let ok = vfs::write(fd, data).is_ok();
    let _ = vfs::close(fd);
    ok
}

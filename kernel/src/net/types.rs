//! Network types: MAC address, IPv4 address, and global network configuration.

/// A 6-byte IEEE 802 MAC (hardware) address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    /// The broadcast MAC address (ff:ff:ff:ff:ff:ff).
    pub const BROADCAST: MacAddr = MacAddr([0xFF; 6]);
    /// The zero MAC address (00:00:00:00:00:00).
    pub const ZERO: MacAddr = MacAddr([0; 6]);

    /// Returns the underlying 6-byte array.
    pub fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }

    /// Convert a multicast IP to its corresponding multicast MAC (RFC 1112).
    /// Low 23 bits of IP mapped to 01:00:5E:xx:xx:xx.
    pub fn from_multicast_ip(ip: Ipv4Addr) -> MacAddr {
        MacAddr([0x01, 0x00, 0x5E, ip.0[1] & 0x7F, ip.0[2], ip.0[3]])
    }
}

impl core::fmt::Display for MacAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5])
    }
}

/// A 16-byte IPv6 address in network byte order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv6Addr(pub [u8; 16]);

impl Ipv6Addr {
    /// The unspecified address (::).
    pub const UNSPECIFIED: Ipv6Addr = Ipv6Addr([0; 16]);
    /// The loopback address (::1).
    pub const LOOPBACK: Ipv6Addr = Ipv6Addr([0,0,0,0, 0,0,0,0, 0,0,0,0, 0,0,0,1]);
    /// All-nodes multicast (ff02::1).
    pub const ALL_NODES: Ipv6Addr = Ipv6Addr([0xff,0x02,0,0, 0,0,0,0, 0,0,0,0, 0,0,0,1]);
    /// All-routers multicast (ff02::2).
    pub const ALL_ROUTERS: Ipv6Addr = Ipv6Addr([0xff,0x02,0,0, 0,0,0,0, 0,0,0,0, 0,0,0,2]);

    /// Construct from 16 bytes.
    pub fn new(bytes: [u8; 16]) -> Self {
        Ipv6Addr(bytes)
    }

    /// Returns the underlying 16-byte array.
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Check if this is the unspecified address (::).
    pub fn is_unspecified(&self) -> bool {
        self.0 == [0; 16]
    }

    /// Check if this is the loopback address (::1).
    pub fn is_loopback(&self) -> bool {
        *self == Self::LOOPBACK
    }

    /// Check if this is a multicast address (ff00::/8).
    pub fn is_multicast(&self) -> bool {
        self.0[0] == 0xff
    }

    /// Check if this is a link-local address (fe80::/10).
    pub fn is_link_local(&self) -> bool {
        self.0[0] == 0xfe && (self.0[1] & 0xc0) == 0x80
    }

    /// Compute the solicited-node multicast address (ff02::1:ffXX:XXXX).
    /// Used by NDP Neighbor Solicitation.
    pub fn solicited_node_multicast(&self) -> Ipv6Addr {
        let mut addr = [0u8; 16];
        addr[0] = 0xff;
        addr[1] = 0x02;
        // bytes 2..11 = 0
        addr[11] = 0x01;
        addr[12] = 0xff;
        addr[13] = self.0[13];
        addr[14] = self.0[14];
        addr[15] = self.0[15];
        Ipv6Addr(addr)
    }

    /// Convert a multicast IPv6 address to its Ethernet multicast MAC (RFC 2464).
    /// Maps to 33:33:xx:xx:xx:xx using the low 32 bits.
    pub fn multicast_mac(&self) -> MacAddr {
        MacAddr([0x33, 0x33, self.0[12], self.0[13], self.0[14], self.0[15]])
    }

    /// Generate a link-local address from a MAC address (EUI-64, RFC 4291 Appendix A).
    pub fn from_mac_link_local(mac: &MacAddr) -> Ipv6Addr {
        let mut addr = [0u8; 16];
        addr[0] = 0xfe;
        addr[1] = 0x80;
        // bytes 2..7 = 0 (prefix padding)
        // EUI-64: insert ff:fe in middle, flip U/L bit
        addr[8] = mac.0[0] ^ 0x02; // flip Universal/Local bit
        addr[9] = mac.0[1];
        addr[10] = mac.0[2];
        addr[11] = 0xff;
        addr[12] = 0xfe;
        addr[13] = mac.0[3];
        addr[14] = mac.0[4];
        addr[15] = mac.0[5];
        Ipv6Addr(addr)
    }

    /// Parse an IPv6 address string (supports :: shorthand).
    pub fn parse(s: &str) -> Option<Ipv6Addr> {
        // Split on "::" to handle zero-compression
        let bytes = s.as_bytes();

        // Find "::" position
        let mut double_colon_pos: Option<usize> = None;
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == b':' && bytes[i + 1] == b':' {
                double_colon_pos = Some(i);
                break;
            }
            i += 1;
        }

        let mut groups = [[0u8; 2]; 8];
        let mut group_count = 0;

        if let Some(dc_pos) = double_colon_pos {
            // Parse groups before ::
            let before = &s[..dc_pos];
            if !before.is_empty() {
                for part in before.split(':') {
                    if group_count >= 8 { return None; }
                    let val = parse_hex16(part)?;
                    groups[group_count] = [(val >> 8) as u8, (val & 0xff) as u8];
                    group_count += 1;
                }
            }

            let before_count = group_count;

            // Parse groups after ::
            let after = &s[dc_pos + 2..];
            let mut after_groups = [[0u8; 2]; 8];
            let mut after_count = 0;
            if !after.is_empty() {
                for part in after.split(':') {
                    if after_count >= 8 { return None; }
                    let val = parse_hex16(part)?;
                    after_groups[after_count] = [(val >> 8) as u8, (val & 0xff) as u8];
                    after_count += 1;
                }
            }

            let total = before_count + after_count;
            if total > 8 { return None; }

            // Fill zeros in the middle
            let zeros = 8 - total;
            for _ in 0..zeros {
                if group_count >= 8 { return None; }
                groups[group_count] = [0, 0];
                group_count += 1;
            }
            for j in 0..after_count {
                if group_count >= 8 { return None; }
                groups[group_count] = after_groups[j];
                group_count += 1;
            }
        } else {
            // No :: — must have exactly 8 groups
            for part in s.split(':') {
                if group_count >= 8 { return None; }
                let val = parse_hex16(part)?;
                groups[group_count] = [(val >> 8) as u8, (val & 0xff) as u8];
                group_count += 1;
            }
        }

        if group_count != 8 { return None; }

        let mut addr = [0u8; 16];
        for g in 0..8 {
            addr[g * 2] = groups[g][0];
            addr[g * 2 + 1] = groups[g][1];
        }
        Some(Ipv6Addr(addr))
    }
}

/// Parse a hex string (1-4 chars) into a u16 value.
fn parse_hex16(s: &str) -> Option<u16> {
    if s.is_empty() || s.len() > 4 { return None; }
    let mut val: u16 = 0;
    for b in s.bytes() {
        val = val << 4;
        val |= match b {
            b'0'..=b'9' => (b - b'0') as u16,
            b'a'..=b'f' => (b - b'a' + 10) as u16,
            b'A'..=b'F' => (b - b'A' + 10) as u16,
            _ => return None,
        };
    }
    Some(val)
}

impl core::fmt::Display for Ipv6Addr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Find longest run of zero groups for :: compression
        let mut groups = [0u16; 8];
        for i in 0..8 {
            groups[i] = ((self.0[i * 2] as u16) << 8) | self.0[i * 2 + 1] as u16;
        }

        // Find longest run of zeros
        let mut best_start = 8usize;
        let mut best_len = 0usize;
        let mut cur_start = 0;
        let mut cur_len = 0;
        for i in 0..8 {
            if groups[i] == 0 {
                if cur_len == 0 { cur_start = i; }
                cur_len += 1;
                if cur_len > best_len {
                    best_start = cur_start;
                    best_len = cur_len;
                }
            } else {
                cur_len = 0;
            }
        }
        if best_len < 2 {
            best_start = 8; // Don't compress single zero
            best_len = 0;
        }

        let mut first = true;
        let mut i = 0;
        while i < 8 {
            if i == best_start {
                if i == 0 { write!(f, ":")?; }
                write!(f, ":")?;
                i += best_len;
                first = i >= 8;
                continue;
            }
            if !first { write!(f, ":")?; }
            write!(f, "{:x}", groups[i])?;
            first = false;
            i += 1;
        }
        Ok(())
    }
}

/// A version-independent IP address (either IPv4 or IPv6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpAddr {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

impl IpAddr {
    /// Check if this is the loopback address (127.x.x.x or ::1).
    pub fn is_loopback(&self) -> bool {
        match self {
            IpAddr::V4(v4) => v4.0[0] == 127,
            IpAddr::V6(v6) => v6.is_loopback(),
        }
    }

    /// Check if this is a multicast address.
    pub fn is_multicast(&self) -> bool {
        match self {
            IpAddr::V4(v4) => v4.is_multicast(),
            IpAddr::V6(v6) => v6.is_multicast(),
        }
    }

    /// Check if this is the unspecified/zero address.
    pub fn is_unspecified(&self) -> bool {
        match self {
            IpAddr::V4(v4) => *v4 == Ipv4Addr::ZERO,
            IpAddr::V6(v6) => v6.is_unspecified(),
        }
    }
}

impl core::fmt::Display for IpAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            IpAddr::V4(v4) => write!(f, "{}", v4),
            IpAddr::V6(v6) => write!(f, "{}", v6),
        }
    }
}

impl From<Ipv4Addr> for IpAddr {
    fn from(v4: Ipv4Addr) -> Self { IpAddr::V4(v4) }
}

impl From<Ipv6Addr> for IpAddr {
    fn from(v6: Ipv6Addr) -> Self { IpAddr::V6(v6) }
}

/// A 4-byte IPv4 address in network byte order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    /// The all-zeros address (0.0.0.0).
    pub const ZERO: Ipv4Addr = Ipv4Addr([0, 0, 0, 0]);
    /// The limited broadcast address (255.255.255.255).
    pub const BROADCAST: Ipv4Addr = Ipv4Addr([255, 255, 255, 255]);

    /// Construct an IPv4 address from four octets.
    pub fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Ipv4Addr([a, b, c, d])
    }

    /// Returns the underlying 4-byte array.
    pub fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }

    /// Convert to a big-endian `u32` for arithmetic operations.
    pub fn to_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    /// Construct from a big-endian `u32`.
    pub fn from_u32(val: u32) -> Self {
        Ipv4Addr(val.to_be_bytes())
    }

    /// Parse "a.b.c.d" string to Ipv4Addr
    pub fn parse(s: &str) -> Option<Ipv4Addr> {
        let mut parts = [0u8; 4];
        let mut idx = 0;
        let mut num: u32 = 0;
        let mut has_digit = false;

        for b in s.bytes() {
            match b {
                b'0'..=b'9' => {
                    num = num * 10 + (b - b'0') as u32;
                    if num > 255 { return None; }
                    has_digit = true;
                }
                b'.' => {
                    if !has_digit || idx >= 3 { return None; }
                    parts[idx] = num as u8;
                    idx += 1;
                    num = 0;
                    has_digit = false;
                }
                _ => return None,
            }
        }

        if !has_digit || idx != 3 { return None; }
        parts[3] = num as u8;
        Some(Ipv4Addr(parts))
    }

    /// Check if this is a multicast address (224.0.0.0 - 239.255.255.255).
    pub fn is_multicast(&self) -> bool {
        self.0[0] >= 224 && self.0[0] <= 239
    }

    /// Check if this is a broadcast address for the given subnet
    pub fn is_broadcast_for(&self, mask: Ipv4Addr) -> bool {
        let ip = self.to_u32();
        let m = mask.to_u32();
        // Broadcast: all host bits are 1
        ip | m == 0xFFFFFFFF
    }
}

impl core::fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

/// Global network configuration
pub struct NetConfig {
    pub ip: Ipv4Addr,
    pub mask: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub dns: Ipv4Addr,
    pub mac: MacAddr,
    /// IPv6 link-local address (auto-generated from MAC via EUI-64).
    pub ipv6_link_local: Ipv6Addr,
    /// IPv6 global/ULA address (from SLAAC, DHCPv6, or static config).
    pub ipv6_addr: Ipv6Addr,
    /// IPv6 prefix length (typically 64).
    pub ipv6_prefix_len: u8,
    /// IPv6 default gateway (link-local address of the router).
    pub ipv6_gateway: Ipv6Addr,
    /// IPv6 DNS server.
    pub ipv6_dns: Ipv6Addr,
}

impl NetConfig {
    /// Create a zeroed-out default network configuration.
    pub const fn new() -> Self {
        NetConfig {
            ip: Ipv4Addr::ZERO,
            mask: Ipv4Addr::ZERO,
            gateway: Ipv4Addr::ZERO,
            dns: Ipv4Addr::ZERO,
            mac: MacAddr::ZERO,
            ipv6_link_local: Ipv6Addr::UNSPECIFIED,
            ipv6_addr: Ipv6Addr::UNSPECIFIED,
            ipv6_prefix_len: 64,
            ipv6_gateway: Ipv6Addr::UNSPECIFIED,
            ipv6_dns: Ipv6Addr::UNSPECIFIED,
        }
    }

    /// Check if an IPv4 address is in the local subnet
    pub fn is_local(&self, target: Ipv4Addr) -> bool {
        let local_net = self.ip.to_u32() & self.mask.to_u32();
        let target_net = target.to_u32() & self.mask.to_u32();
        local_net == target_net
    }

    /// Check if an IPv6 address is in the local prefix.
    pub fn is_local_v6(&self, target: Ipv6Addr) -> bool {
        if target.is_link_local() {
            return true; // link-local is always on-link
        }
        if self.ipv6_addr.is_unspecified() {
            return false;
        }
        let prefix_bytes = (self.ipv6_prefix_len / 8) as usize;
        let prefix_bits = self.ipv6_prefix_len % 8;
        // Compare full bytes
        for i in 0..prefix_bytes.min(16) {
            if self.ipv6_addr.0[i] != target.0[i] {
                return false;
            }
        }
        // Compare remaining bits
        if prefix_bits > 0 && prefix_bytes < 16 {
            let mask = 0xFFu8 << (8 - prefix_bits);
            if (self.ipv6_addr.0[prefix_bytes] & mask) != (target.0[prefix_bytes] & mask) {
                return false;
            }
        }
        true
    }
}

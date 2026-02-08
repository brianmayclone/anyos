/// Network types: MAC address, IPv4 address, and global network configuration.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    pub const BROADCAST: MacAddr = MacAddr([0xFF; 6]);
    pub const ZERO: MacAddr = MacAddr([0; 6]);

    pub fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }
}

impl core::fmt::Display for MacAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    pub const ZERO: Ipv4Addr = Ipv4Addr([0, 0, 0, 0]);
    pub const BROADCAST: Ipv4Addr = Ipv4Addr([255, 255, 255, 255]);

    pub fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Ipv4Addr([a, b, c, d])
    }

    pub fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }

    pub fn to_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

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
}

impl NetConfig {
    pub const fn new() -> Self {
        NetConfig {
            ip: Ipv4Addr::ZERO,
            mask: Ipv4Addr::ZERO,
            gateway: Ipv4Addr::ZERO,
            dns: Ipv4Addr::ZERO,
            mac: MacAddr::ZERO,
        }
    }

    /// Check if an IP is in the local subnet
    pub fn is_local(&self, target: Ipv4Addr) -> bool {
        let local_net = self.ip.to_u32() & self.mask.to_u32();
        let target_net = target.to_u32() & self.mask.to_u32();
        local_net == target_net
    }
}

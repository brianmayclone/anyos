//! Unit tests for network type parsing and IPv4 packet parsing.
//!
//! All functions are pure (no hardware, heap, or scheduler needed).

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::net::ipv4;
use crate::net::types::{Ipv4Addr, MacAddr, NetConfig};

pub static SUITE: TestSuite = TestSuite {
    name: "net::types",
    cases: &[
        // Ipv4Addr
        TestCase { name: "ipv4_new_and_bytes",       run: test_ipv4_new },
        TestCase { name: "ipv4_to_from_u32",         run: test_ipv4_u32 },
        TestCase { name: "ipv4_parse_valid",          run: test_ipv4_parse_valid },
        TestCase { name: "ipv4_parse_invalid",        run: test_ipv4_parse_invalid },
        TestCase { name: "ipv4_is_multicast",         run: test_multicast },
        TestCase { name: "ipv4_is_broadcast",         run: test_broadcast },
        TestCase { name: "ipv4_constants",            run: test_ipv4_constants },
        // MacAddr
        TestCase { name: "mac_broadcast",             run: test_mac_broadcast },
        TestCase { name: "mac_zero",                  run: test_mac_zero },
        TestCase { name: "mac_multicast_from_ip",     run: test_mac_multicast },
        // NetConfig
        TestCase { name: "netconfig_new",             run: test_netconfig_new },
        TestCase { name: "netconfig_is_local",        run: test_netconfig_local },
        // IPv4 packet parsing
        TestCase { name: "ipv4_parse_valid_packet",   run: test_parse_valid_packet },
        TestCase { name: "ipv4_parse_short_packet",   run: test_parse_short },
        TestCase { name: "ipv4_parse_wrong_version",  run: test_parse_wrong_version },
        TestCase { name: "ipv4_parse_truncated",      run: test_parse_truncated },
    ],
};

// ── Ipv4Addr ──────────────────────────────────────────────────────────────────

fn test_ipv4_new(ctx: &mut TestContext) {
    let ip = Ipv4Addr::new(192, 168, 1, 100);
    ctx.expect_eq(ip.as_bytes(), &[192u8, 168, 1, 100], "new() stores bytes correctly");
}

fn test_ipv4_u32(ctx: &mut TestContext) {
    let ip = Ipv4Addr::new(192, 168, 1, 1);
    let n = ip.to_u32();
    let ip2 = Ipv4Addr::from_u32(n);
    ctx.expect_eq(ip2.as_bytes(), ip.as_bytes(), "to_u32 / from_u32 round-trip");

    // 1.2.3.4 → 0x01020304
    ctx.expect_eq(Ipv4Addr::new(1, 2, 3, 4).to_u32(), 0x01020304u32, "1.2.3.4 == 0x01020304");
    ctx.expect_eq(
        Ipv4Addr::from_u32(0x7F000001).as_bytes(),
        &[127u8, 0, 0, 1],
        "from_u32(0x7F000001) == 127.0.0.1"
    );
}

fn test_ipv4_parse_valid(ctx: &mut TestContext) {
    let cases = &[
        ("0.0.0.0",         [0u8, 0, 0, 0]),
        ("127.0.0.1",       [127, 0, 0, 1]),
        ("192.168.1.1",     [192, 168, 1, 1]),
        ("255.255.255.255", [255, 255, 255, 255]),
        ("10.0.0.1",        [10, 0, 0, 1]),
    ];
    for (s, expected) in cases {
        let parsed = Ipv4Addr::parse(s);
        if let Some(ip) = ctx.expect_some(parsed, "valid IP parses to Some") {
            ctx.expect_eq(ip.as_bytes(), expected, "parsed bytes match");
        }
    }
}

fn test_ipv4_parse_invalid(ctx: &mut TestContext) {
    let invalids = &[
        "", "abc", "1.2.3", "1.2.3.4.5", "256.0.0.1", "1.2.3.-1",
        "1.2.3.4a", "999.999.999.999",
    ];
    for s in invalids {
        ctx.expect_none(Ipv4Addr::parse(s), "invalid IP parses to None");
    }
}

fn test_multicast(ctx: &mut TestContext) {
    // 224.0.0.0/4 is multicast.
    ctx.expect_true(Ipv4Addr::new(224, 0, 0, 1).is_multicast(),  "224.0.0.1 is multicast");
    ctx.expect_true(Ipv4Addr::new(239, 255, 255, 255).is_multicast(), "239.x.x.x is multicast");
    ctx.expect_false(Ipv4Addr::new(192, 168, 1, 1).is_multicast(), "192.168.1.1 not multicast");
    ctx.expect_false(Ipv4Addr::new(223, 0, 0, 1).is_multicast(),   "223.x.x.x not multicast");
}

fn test_broadcast(ctx: &mut TestContext) {
    let mask = Ipv4Addr::new(255, 255, 255, 0);
    ctx.expect_true(
        Ipv4Addr::new(192, 168, 1, 255).is_broadcast_for(mask),
        "192.168.1.255 is /24 broadcast"
    );
    ctx.expect_false(
        Ipv4Addr::new(192, 168, 1, 254).is_broadcast_for(mask),
        "192.168.1.254 is not /24 broadcast"
    );
}

fn test_ipv4_constants(ctx: &mut TestContext) {
    ctx.expect_eq(Ipv4Addr::ZERO.as_bytes(),      &[0u8, 0, 0, 0],           "ZERO");
    ctx.expect_eq(Ipv4Addr::BROADCAST.as_bytes(), &[255u8, 255, 255, 255],   "BROADCAST");
}

// ── MacAddr ───────────────────────────────────────────────────────────────────

fn test_mac_broadcast(ctx: &mut TestContext) {
    ctx.expect_eq(MacAddr::BROADCAST.as_bytes(), &[0xFFu8; 6], "broadcast MAC is FF:FF:FF:FF:FF:FF");
}

fn test_mac_zero(ctx: &mut TestContext) {
    ctx.expect_eq(MacAddr::ZERO.as_bytes(), &[0u8; 6], "zero MAC is 00:00:00:00:00:00");
}

fn test_mac_multicast(ctx: &mut TestContext) {
    // IPv4 multicast MAC: 01:00:5E + low 23 bits of IP.
    // 224.1.2.3 → 01:00:5E:01:02:03
    let ip = Ipv4Addr::new(224, 1, 2, 3);
    let mac = MacAddr::from_multicast_ip(ip);
    let b = mac.as_bytes();
    ctx.expect_eq(b[0], 0x01u8, "multicast MAC byte 0 == 0x01");
    ctx.expect_eq(b[1], 0x00u8, "multicast MAC byte 1 == 0x00");
    ctx.expect_eq(b[2], 0x5Eu8, "multicast MAC byte 2 == 0x5E");
    ctx.expect_eq(b[3], 0x01u8, "multicast MAC byte 3 == low bits of IP octet 1");
    ctx.expect_eq(b[4], 0x02u8, "multicast MAC byte 4 == IP octet 2");
    ctx.expect_eq(b[5], 0x03u8, "multicast MAC byte 5 == IP octet 3");
}

// ── NetConfig ────────────────────────────────────────────────────────────────

fn test_netconfig_new(ctx: &mut TestContext) {
    let cfg = NetConfig::new();
    // A freshly created config has all-zero IP/MAC.
    ctx.expect_eq(cfg.ip.as_bytes(),  &[0u8; 4], "default IP is 0.0.0.0");
    ctx.expect_eq(cfg.mac.as_bytes(), &[0u8; 6], "default MAC is zero");
}

fn test_netconfig_local(ctx: &mut TestContext) {
    let mut cfg = NetConfig::new();
    cfg.ip   = Ipv4Addr::new(10, 0, 0, 5);
    cfg.mask = Ipv4Addr::new(255, 255, 255, 0);

    ctx.expect_true(
        cfg.is_local(Ipv4Addr::new(10, 0, 0, 200)),
        "10.0.0.200 is local (same /24 subnet)"
    );
    ctx.expect_false(
        cfg.is_local(Ipv4Addr::new(10, 0, 1, 1)),
        "10.0.1.1 is not local (/24 different subnet)"
    );
    ctx.expect_false(
        cfg.is_local(Ipv4Addr::new(192, 168, 1, 1)),
        "192.168.1.1 is not local"
    );
}

// ── IPv4 packet parsing ───────────────────────────────────────────────────────

fn test_parse_valid_packet(ctx: &mut TestContext) {
    // Minimal valid IPv4/TCP header (no options, 20-byte IP header).
    let pkt: &[u8] = &[
        0x45, 0x00, 0x00, 0x28,  // ver=4, ihl=5, dscp, total=40
        0x00, 0x01, 0x40, 0x00,  // id, DF flag, frag offset=0
        0x40, 0x06, 0x00, 0x00,  // ttl=64, proto=TCP(6), checksum
        0xC0, 0xA8, 0x01, 0x01,  // src 192.168.1.1
        0xC0, 0xA8, 0x01, 0x02,  // dst 192.168.1.2
        // 20 bytes of dummy TCP payload
        0x00, 0x50, 0x01, 0xBB, 0x00,0x00,0x00,0x00,
        0x00,0x00,0x00,0x00, 0x50,0x02, 0x20,0x00, 0x00,0x00, 0x00,0x00,
    ];

    let parsed = ipv4::parse(pkt);
    if let Some(p) = ctx.expect_some(parsed, "valid packet parses to Some") {
        ctx.expect_eq(p.src.as_bytes(), &[192u8, 168, 1, 1], "src IP");
        ctx.expect_eq(p.dst.as_bytes(), &[192u8, 168, 1, 2], "dst IP");
        ctx.expect_eq(p.protocol, ipv4::PROTO_TCP, "protocol == TCP");
        ctx.expect_eq(p.ttl, 64u8, "TTL == 64");
        ctx.expect_eq(p.header_len, 20usize, "header_len == 20");
        ctx.expect_eq(p.payload.len(), 20usize, "payload is 20 bytes");
    }
}

fn test_parse_short(ctx: &mut TestContext) {
    // Fewer than 20 bytes — cannot contain a complete IP header.
    for len in [0usize, 1, 5, 19] {
        let buf = alloc::vec![0u8; len];
        ctx.expect_none(ipv4::parse(&buf), "too-short packet returns None");
    }
}

fn test_parse_wrong_version(ctx: &mut TestContext) {
    let mut pkt = [0u8; 20];
    pkt[0] = 0x65; // version=6 (IPv6), ihl=5
    ctx.expect_none(ipv4::parse(&pkt), "IPv6 version byte returns None from IPv4 parser");

    pkt[0] = 0x35; // version=3
    ctx.expect_none(ipv4::parse(&pkt), "version=3 returns None");
}

fn test_parse_truncated(ctx: &mut TestContext) {
    // total_len field claims 40 bytes but buffer is only 20 — truncated.
    let pkt: &[u8] = &[
        0x45, 0x00, 0x00, 0x28,  // total_len = 40
        0x00, 0x01, 0x00, 0x00,
        0x40, 0x11, 0x00, 0x00,
        0xC0, 0xA8, 0x01, 0x01,
        0xC0, 0xA8, 0x01, 0x02,
        // payload claimed but missing — only 20 bytes provided
    ];
    // Parser should either return None or a packet with empty/truncated payload.
    // Either way it must not panic.
    let _ = ipv4::parse(pkt);
    ctx.expect_true(true, "truncated packet does not panic");
}

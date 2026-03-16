//! User-mode networking (SLIRP-style) backend.
//!
//! Provides NAT + DHCP + DNS for guest VMs without requiring root or TAP
//! devices.  The guest sees a virtual 10.0.2.0/24 network:
//!
//! - Gateway / NAT router:  10.0.2.2
//! - DHCP server:           10.0.2.2
//! - DNS relay:             10.0.2.3
//! - Guest (DHCP-assigned): 10.0.2.15
//!
//! Architecture:
//! - ARP: answered locally (no real ARP needed)
//! - DHCP: minimal DHCP server (DISCOVER→OFFER, REQUEST→ACK)
//! - DNS: UDP relay to host resolver
//! - TCP: per-connection host socket, non-blocking
//! - UDP: per-flow host socket, non-blocking
//! - ICMP: silently dropped (would need raw sockets / root)

use alloc::vec;
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use alloc::collections::BTreeMap;
use alloc::string::String;
use std::net::{TcpStream, UdpSocket, SocketAddr, Ipv4Addr, TcpListener};
use std::io::{Read, Write, ErrorKind};
use std::time::Instant;
use super::net::NetBackend;

// ── Network configuration ────────────────────────────────────────────────────

const NET_PREFIX: [u8; 3] = [10, 0, 2];
const GATEWAY_IP: [u8; 4] = [10, 0, 2, 2];
const DNS_IP: [u8; 4]     = [10, 0, 2, 3];
const GUEST_IP: [u8; 4]   = [10, 0, 2, 15];
const NETMASK: [u8; 4]    = [255, 255, 255, 0];
const BROADCAST: [u8; 4]  = [10, 0, 2, 255];

/// MAC address of the virtual gateway.
const GW_MAC: [u8; 6] = [0x52, 0x55, 0x0A, 0x00, 0x02, 0x02];

/// Maximum Ethernet frame we handle.
const MAX_FRAME: usize = 1514;

// ── Ethernet / IP / TCP / UDP helpers ────────────────────────────────────────

const ETH_HDR: usize = 14;
const IP_HDR_MIN: usize = 20;

const ETHERTYPE_ARP: u16 = 0x0806;
const ETHERTYPE_IPV4: u16 = 0x0800;

const IP_PROTO_ICMP: u8 = 1;
const IP_PROTO_TCP: u8 = 6;
const IP_PROTO_UDP: u8 = 17;

fn u16be(b: &[u8], off: usize) -> u16 {
    ((b[off] as u16) << 8) | b[off + 1] as u16
}

fn u32be(b: &[u8], off: usize) -> u32 {
    ((b[off] as u32) << 24) | ((b[off+1] as u32) << 16) |
    ((b[off+2] as u32) << 8) | b[off+3] as u32
}

fn put_u16be(b: &mut [u8], off: usize, v: u16) {
    b[off] = (v >> 8) as u8;
    b[off + 1] = v as u8;
}

fn put_u32be(b: &mut [u8], off: usize, v: u32) {
    b[off]   = (v >> 24) as u8;
    b[off+1] = (v >> 16) as u8;
    b[off+2] = (v >> 8) as u8;
    b[off+3] = v as u8;
}

fn ip_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += ((data[i] as u32) << 8) | data[i + 1] as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !sum as u16
}

/// Build an Ethernet header.
fn eth_header(dst: &[u8; 6], src: &[u8; 6], ethertype: u16) -> [u8; 14] {
    let mut h = [0u8; 14];
    h[0..6].copy_from_slice(dst);
    h[6..12].copy_from_slice(src);
    put_u16be(&mut h, 12, ethertype);
    h
}

/// Build a minimal IPv4 header (no options).
fn ip_header(proto: u8, src: [u8; 4], dst: [u8; 4], payload_len: u16, id: u16) -> [u8; 20] {
    let total_len = 20 + payload_len;
    let mut h = [0u8; 20];
    h[0] = 0x45; // version=4, ihl=5
    put_u16be(&mut h, 2, total_len);
    put_u16be(&mut h, 4, id);
    h[6] = 0x40; // DF flag
    h[8] = 64;   // TTL
    h[9] = proto;
    h[12..16].copy_from_slice(&src);
    h[16..20].copy_from_slice(&dst);
    let cksum = ip_checksum(&h);
    put_u16be(&mut h, 10, cksum);
    h
}

/// Build a UDP header (without checksum — optional for IPv4).
fn udp_header(src_port: u16, dst_port: u16, payload_len: u16) -> [u8; 8] {
    let total = 8 + payload_len;
    let mut h = [0u8; 8];
    put_u16be(&mut h, 0, src_port);
    put_u16be(&mut h, 2, dst_port);
    put_u16be(&mut h, 4, total);
    // checksum 0 = not computed (valid for IPv4 UDP)
    h
}

// ── TCP connection tracking ──────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TcpFlowKey {
    guest_port: u16,
    remote_ip: [u8; 4],
    remote_port: u16,
}

#[derive(PartialEq, Eq)]
enum TcpState {
    SynReceived,
    Established,
    FinWait,
    Closed,
}

struct TcpConnection {
    stream: TcpStream,
    state: TcpState,
    /// Our (gateway) sequence number.
    our_seq: u32,
    /// Last ACKed guest sequence number.
    guest_seq: u32,
    /// Initial guest sequence from SYN.
    guest_isn: u32,
    /// Read buffer for data from host socket.
    read_buf: [u8; 4096],
}

// ── UDP flow tracking ────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct UdpFlowKey {
    guest_port: u16,
    remote_ip: [u8; 4],
    remote_port: u16,
}

struct UdpFlow {
    socket: UdpSocket,
    last_active: Instant,
}

// ── SLIRP backend ────────────────────────────────────────────────────────────

pub struct SlirpNet {
    /// Guest MAC address (learned from first frame or DHCP).
    guest_mac: [u8; 6],
    /// Frames ready to be delivered to the guest.
    rx_queue: VecDeque<Vec<u8>>,
    /// Active TCP connections.
    tcp_conns: BTreeMap<TcpFlowKey, TcpConnection>,
    /// Active UDP flows.
    udp_flows: BTreeMap<UdpFlowKey, UdpFlow>,
    /// DNS relay socket (shared for all DNS queries).
    dns_socket: Option<UdpSocket>,
    /// Pending DNS replies keyed by (guest_src_port, dns_txid).
    dns_pending: BTreeMap<u16, u16>, // txid → guest_src_port
    /// IP identification counter.
    ip_id: u16,
    /// Host DNS server address.
    host_dns: SocketAddr,
    /// DHCP state — true once guest has been offered an address.
    dhcp_offered: bool,
}

impl SlirpNet {
    pub fn new() -> Self {
        // Detect host DNS resolver
        let host_dns = detect_host_dns();

        SlirpNet {
            guest_mac: [0; 6],
            rx_queue: VecDeque::new(),
            tcp_conns: BTreeMap::new(),
            udp_flows: BTreeMap::new(),
            dns_socket: None,
            dns_pending: BTreeMap::new(),
            ip_id: 1,
            host_dns,
            dhcp_offered: false,
        }
    }

    fn next_ip_id(&mut self) -> u16 {
        let id = self.ip_id;
        self.ip_id = self.ip_id.wrapping_add(1);
        id
    }

    /// Process an incoming Ethernet frame from the guest.
    fn process_frame(&mut self, frame: &[u8]) {
        if frame.len() < ETH_HDR { return; }

        // Learn guest MAC from source
        self.guest_mac.copy_from_slice(&frame[6..12]);

        let ethertype = u16be(frame, 12);
        match ethertype {
            ETHERTYPE_ARP => self.handle_arp(frame),
            ETHERTYPE_IPV4 => self.handle_ipv4(frame),
            _ => {} // drop unknown
        }
    }

    // ── ARP ──────────────────────────────────────────────────────────────

    fn handle_arp(&mut self, frame: &[u8]) {
        if frame.len() < ETH_HDR + 28 { return; }
        let arp = &frame[ETH_HDR..];
        let op = u16be(arp, 6);
        if op != 1 { return; } // only handle ARP Request

        let target_ip = &arp[24..28];
        // Reply for any IP in our subnet (gateway, DNS)
        if target_ip[0] != NET_PREFIX[0] || target_ip[1] != NET_PREFIX[1] || target_ip[2] != NET_PREFIX[2] {
            return;
        }

        let mut reply = vec![0u8; ETH_HDR + 28];
        // Ethernet header
        reply[0..6].copy_from_slice(&self.guest_mac);
        reply[6..12].copy_from_slice(&GW_MAC);
        put_u16be(&mut reply, 12, ETHERTYPE_ARP);
        // ARP reply
        let r = &mut reply[ETH_HDR..];
        put_u16be(r, 0, 1);    // HTYPE = Ethernet
        put_u16be(r, 2, 0x0800); // PTYPE = IPv4
        r[4] = 6; // HLEN
        r[5] = 4; // PLEN
        put_u16be(r, 6, 2);    // OPER = Reply
        r[8..14].copy_from_slice(&GW_MAC); // sender MAC
        r[14..18].copy_from_slice(target_ip); // sender IP = requested IP
        r[18..24].copy_from_slice(&self.guest_mac); // target MAC
        r[24..28].copy_from_slice(&arp[14..18]); // target IP = requester's IP

        self.rx_queue.push_back(reply);
    }

    // ── IPv4 ─────────────────────────────────────────────────────────────

    fn handle_ipv4(&mut self, frame: &[u8]) {
        if frame.len() < ETH_HDR + IP_HDR_MIN { return; }
        let ip = &frame[ETH_HDR..];
        let ihl = ((ip[0] & 0x0F) as usize) * 4;
        if ip.len() < ihl { return; }
        let total_len = u16be(ip, 2) as usize;
        if ip.len() < total_len { return; }

        let proto = ip[9];
        let src_ip: [u8; 4] = [ip[12], ip[13], ip[14], ip[15]];
        let dst_ip: [u8; 4] = [ip[16], ip[17], ip[18], ip[19]];
        let payload = &ip[ihl..total_len];

        match proto {
            IP_PROTO_UDP => self.handle_udp(src_ip, dst_ip, payload),
            IP_PROTO_TCP => self.handle_tcp(src_ip, dst_ip, payload),
            IP_PROTO_ICMP => self.handle_icmp(src_ip, dst_ip, payload),
            _ => {}
        }
    }

    // ── ICMP ─────────────────────────────────────────────────────────────

    fn handle_icmp(&mut self, src_ip: [u8; 4], dst_ip: [u8; 4], payload: &[u8]) {
        if payload.len() < 8 { return; }
        let icmp_type = payload[0];
        if icmp_type != 8 { return; } // only Echo Request

        // Build Echo Reply
        let mut icmp_reply = payload.to_vec();
        icmp_reply[0] = 0; // Echo Reply
        icmp_reply[2] = 0; icmp_reply[3] = 0; // clear checksum
        let cksum = ip_checksum(&icmp_reply);
        put_u16be(&mut icmp_reply, 2, cksum);

        self.send_ip_packet(IP_PROTO_ICMP, dst_ip, src_ip, &icmp_reply);
    }

    // ── UDP ──────────────────────────────────────────────────────────────

    fn handle_udp(&mut self, src_ip: [u8; 4], dst_ip: [u8; 4], payload: &[u8]) {
        if payload.len() < 8 { return; }
        let src_port = u16be(payload, 0);
        let dst_port = u16be(payload, 2);
        let udp_data = &payload[8..];

        // DHCP (guest → broadcast or gateway, port 67)
        if dst_port == 67 {
            self.handle_dhcp(src_ip, udp_data, src_port);
            return;
        }

        // DNS (destination = DNS_IP:53 or gateway:53)
        if dst_port == 53 && (dst_ip == DNS_IP || dst_ip == GATEWAY_IP) {
            self.handle_dns(src_port, udp_data);
            return;
        }

        // General UDP — NAT to host
        self.handle_udp_nat(src_ip, src_port, dst_ip, dst_port, udp_data);
    }

    fn handle_udp_nat(&mut self, _src_ip: [u8; 4], src_port: u16, dst_ip: [u8; 4], dst_port: u16, data: &[u8]) {
        let key = UdpFlowKey { guest_port: src_port, remote_ip: dst_ip, remote_port: dst_port };

        // Create flow if new
        if !self.udp_flows.contains_key(&key) {
            let sock = match UdpSocket::bind("0.0.0.0:0") {
                Ok(s) => s,
                Err(_) => return,
            };
            let _ = sock.set_nonblocking(true);
            self.udp_flows.insert(key, UdpFlow {
                socket: sock,
                last_active: Instant::now(),
            });
        }

        if let Some(flow) = self.udp_flows.get_mut(&key) {
            let dst = SocketAddr::new(
                Ipv4Addr::new(dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3]).into(),
                dst_port,
            );
            let _ = flow.socket.send_to(data, dst);
            flow.last_active = Instant::now();
        }
    }

    fn poll_udp(&mut self) {
        let mut responses: Vec<(UdpFlowKey, Vec<u8>)> = Vec::new();

        for (key, flow) in &mut self.udp_flows {
            let mut buf = [0u8; 2048];
            match flow.socket.recv_from(&mut buf) {
                Ok((n, _addr)) => {
                    flow.last_active = Instant::now();
                    responses.push((*key, buf[..n].to_vec()));
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(_) => {}
            }
        }

        for (key, data) in responses {
            // Send response back to guest
            let udp_hdr = udp_header(key.remote_port, key.guest_port, data.len() as u16);
            let mut payload = Vec::with_capacity(8 + data.len());
            payload.extend_from_slice(&udp_hdr);
            payload.extend_from_slice(&data);
            self.send_ip_packet(IP_PROTO_UDP, key.remote_ip, GUEST_IP, &payload);
        }

        // Expire old flows (>60s idle)
        let now = Instant::now();
        self.udp_flows.retain(|_, flow| now.duration_since(flow.last_active).as_secs() < 60);
    }

    // ── DNS relay ────────────────────────────────────────────────────────

    fn handle_dns(&mut self, guest_src_port: u16, data: &[u8]) {
        if data.len() < 12 { return; }
        let txid = u16be(data, 0);

        // Lazy-init shared DNS socket
        if self.dns_socket.is_none() {
            if let Ok(s) = UdpSocket::bind("0.0.0.0:0") {
                let _ = s.set_nonblocking(true);
                self.dns_socket = Some(s);
            }
        }

        if let Some(ref sock) = self.dns_socket {
            let _ = sock.send_to(data, self.host_dns);
            self.dns_pending.insert(txid, guest_src_port);
        }
    }

    fn poll_dns(&mut self) {
        // Collect replies first, then send (avoids borrow conflict on self)
        let mut replies: Vec<(u16, Vec<u8>)> = Vec::new(); // (guest_port, dns_data)
        if let Some(ref sock) = self.dns_socket {
            let mut buf = [0u8; 2048];
            loop {
                match sock.recv_from(&mut buf) {
                    Ok((n, _)) => {
                        if n < 12 { continue; }
                        let txid = u16be(&buf, 0);
                        if let Some(guest_port) = self.dns_pending.remove(&txid) {
                            replies.push((guest_port, buf[..n].to_vec()));
                        }
                    }
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
        }
        for (guest_port, data) in replies {
            let udp_hdr = udp_header(53, guest_port, data.len() as u16);
            let mut payload = Vec::with_capacity(8 + data.len());
            payload.extend_from_slice(&udp_hdr);
            payload.extend_from_slice(&data);
            self.send_ip_packet(IP_PROTO_UDP, DNS_IP, GUEST_IP, &payload);
        }
    }

    // ── DHCP server ──────────────────────────────────────────────────────

    fn handle_dhcp(&mut self, _src_ip: [u8; 4], data: &[u8], _src_port: u16) {
        // Minimal DHCP: parse enough to distinguish DISCOVER vs REQUEST
        if data.len() < 240 { return; }
        let msg_type = data[0]; // op: 1=BOOTREQUEST
        if msg_type != 1 { return; }

        let xid = &data[4..8];
        let chaddr = &data[28..34]; // client hardware address (first 6 bytes)

        // Find DHCP message type in options (after magic cookie at offset 236)
        let magic = &data[236..240];
        if magic != [99, 130, 83, 99] { return; } // DHCP magic cookie

        let options = &data[240..];
        let dhcp_msg_type = find_dhcp_option(options, 53);
        let dhcp_msg_type = match dhcp_msg_type {
            Some(t) if !t.is_empty() => t[0],
            _ => return,
        };

        match dhcp_msg_type {
            1 => self.send_dhcp_offer(xid, chaddr),  // DISCOVER
            3 => self.send_dhcp_ack(xid, chaddr),     // REQUEST
            _ => {}
        }
    }

    fn send_dhcp_offer(&mut self, xid: &[u8], chaddr: &[u8]) {
        self.dhcp_offered = true;
        self.build_dhcp_reply(xid, chaddr, 2); // DHCPOFFER
    }

    fn send_dhcp_ack(&mut self, xid: &[u8], chaddr: &[u8]) {
        self.build_dhcp_reply(xid, chaddr, 5); // DHCPACK
    }

    fn build_dhcp_reply(&mut self, xid: &[u8], chaddr: &[u8], msg_type: u8) {
        let mut reply = vec![0u8; 576]; // minimum DHCP packet
        reply[0] = 2; // op = BOOTREPLY
        reply[1] = 1; // htype = Ethernet
        reply[2] = 6; // hlen
        reply[4..8].copy_from_slice(xid);
        reply[16..20].copy_from_slice(&GUEST_IP); // yiaddr
        reply[20..24].copy_from_slice(&GATEWAY_IP); // siaddr (next server)
        // chaddr
        let mac_len = chaddr.len().min(16);
        reply[28..28 + mac_len].copy_from_slice(&chaddr[..mac_len]);

        // DHCP magic cookie
        reply[236..240].copy_from_slice(&[99, 130, 83, 99]);

        // Options
        let mut off = 240;
        // 53: DHCP Message Type
        reply[off] = 53; reply[off+1] = 1; reply[off+2] = msg_type; off += 3;
        // 54: Server Identifier
        reply[off] = 54; reply[off+1] = 4; reply[off+2..off+6].copy_from_slice(&GATEWAY_IP); off += 6;
        // 51: Lease Time (86400 = 24h)
        reply[off] = 51; reply[off+1] = 4; put_u32be(&mut reply, off+2, 86400); off += 6;
        // 1: Subnet Mask
        reply[off] = 1; reply[off+1] = 4; reply[off+2..off+6].copy_from_slice(&NETMASK); off += 6;
        // 3: Router
        reply[off] = 3; reply[off+1] = 4; reply[off+2..off+6].copy_from_slice(&GATEWAY_IP); off += 6;
        // 6: DNS Server
        reply[off] = 6; reply[off+1] = 4; reply[off+2..off+6].copy_from_slice(&DNS_IP); off += 6;
        // 28: Broadcast Address
        reply[off] = 28; reply[off+1] = 4; reply[off+2..off+6].copy_from_slice(&BROADCAST); off += 6;
        // 255: End
        reply[off] = 255;

        // Wrap in UDP: src=67, dst=68
        let udp_hdr = udp_header(67, 68, reply.len() as u16);
        let mut udp_payload = Vec::with_capacity(8 + reply.len());
        udp_payload.extend_from_slice(&udp_hdr);
        udp_payload.extend_from_slice(&reply);

        // Build IP packet: src=GATEWAY, dst=BROADCAST (255.255.255.255)
        let id = self.next_ip_id();
        let ip_hdr = ip_header(IP_PROTO_UDP, GATEWAY_IP, [255, 255, 255, 255], udp_payload.len() as u16, id);

        // Build Ethernet frame: dst=broadcast
        let eth = eth_header(&[0xFF; 6], &GW_MAC, ETHERTYPE_IPV4);

        let mut frame = Vec::with_capacity(ETH_HDR + 20 + udp_payload.len());
        frame.extend_from_slice(&eth);
        frame.extend_from_slice(&ip_hdr);
        frame.extend_from_slice(&udp_payload);

        self.rx_queue.push_back(frame);
    }

    // ── TCP NAT ──────────────────────────────────────────────────────────

    fn handle_tcp(&mut self, src_ip: [u8; 4], dst_ip: [u8; 4], payload: &[u8]) {
        if payload.len() < 20 { return; }
        let src_port = u16be(payload, 0);
        let dst_port = u16be(payload, 2);
        let seq = u32be(payload, 4);
        let ack = u32be(payload, 8);
        let data_offset = ((payload[12] >> 4) as usize) * 4;
        let flags = payload[13];
        let tcp_data = if data_offset < payload.len() { &payload[data_offset..] } else { &[] };

        let syn = flags & 0x02 != 0;
        let ack_flag = flags & 0x10 != 0;
        let fin = flags & 0x01 != 0;
        let rst = flags & 0x04 != 0;

        let key = TcpFlowKey { guest_port: src_port, remote_ip: dst_ip, remote_port: dst_port };

        if rst {
            // Guest sent RST — close connection
            self.tcp_conns.remove(&key);
            return;
        }

        if syn && !ack_flag {
            // New connection: SYN
            // Connect to real host
            let addr = SocketAddr::new(
                Ipv4Addr::new(dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3]).into(),
                dst_port,
            );
            match TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(5)) {
                Ok(stream) => {
                    let _ = stream.set_nonblocking(true);
                    let _ = stream.set_nodelay(true);
                    let our_seq: u32 = 0x1000_0000; // fixed ISN for simplicity
                    let conn = TcpConnection {
                        stream,
                        state: TcpState::SynReceived,
                        our_seq,
                        guest_seq: seq.wrapping_add(1), // SYN counts as 1 byte
                        guest_isn: seq,
                        read_buf: [0u8; 4096],
                    };
                    self.tcp_conns.insert(key, conn);
                    // Send SYN-ACK
                    self.send_tcp_flags(key, 0x12, our_seq, seq.wrapping_add(1), &[]); // SYN+ACK
                }
                Err(_) => {
                    // Connection refused — send RST
                    self.send_tcp_flags(key, 0x14, 0, seq.wrapping_add(1), &[]); // RST+ACK
                }
            }
            return;
        }

        // Existing connection — extract values to avoid borrow conflicts
        {
            let conn = match self.tcp_conns.get_mut(&key) {
                Some(c) => c,
                None => return,
            };

            if ack_flag && conn.state == TcpState::SynReceived {
                conn.state = TcpState::Established;
                conn.our_seq = conn.our_seq.wrapping_add(1); // SYN consumed
            }

            // Data from guest → write to host socket
            if !tcp_data.is_empty() && conn.state == TcpState::Established {
                let _ = conn.stream.write_all(tcp_data);
                conn.guest_seq = seq.wrapping_add(tcp_data.len() as u32);
            }

            if fin {
                conn.guest_seq = conn.guest_seq.wrapping_add(1); // FIN counts as 1
            }
        }

        // Now send TCP responses outside the mutable borrow of tcp_conns
        if let Some(conn) = self.tcp_conns.get(&key) {
            let our_seq = conn.our_seq;
            let guest_seq = conn.guest_seq;

            if !tcp_data.is_empty() {
                self.send_tcp_flags(key, 0x10, our_seq, guest_seq, &[]); // ACK
            }

            if fin {
                self.send_tcp_flags(key, 0x10, our_seq, guest_seq, &[]); // ACK FIN
                self.send_tcp_flags(key, 0x11, our_seq, guest_seq, &[]); // FIN+ACK
                if let Some(c) = self.tcp_conns.get_mut(&key) {
                    c.state = TcpState::Closed;
                }
            }
        }
    }

    fn poll_tcp(&mut self) {
        // Read data from host sockets and inject into guest
        let mut data_to_send: Vec<(TcpFlowKey, Vec<u8>)> = Vec::new();
        let mut closed: Vec<TcpFlowKey> = Vec::new();

        for (key, conn) in &mut self.tcp_conns {
            if conn.state != TcpState::Established { continue; }
            match conn.stream.read(&mut conn.read_buf) {
                Ok(0) => {
                    // EOF — host closed connection
                    closed.push(*key);
                }
                Ok(n) => {
                    data_to_send.push((*key, conn.read_buf[..n].to_vec()));
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(_) => {
                    closed.push(*key);
                }
            }
        }

        for (key, data) in data_to_send {
            if let Some(conn) = self.tcp_conns.get(&key) {
                let seq = conn.our_seq;
                let ack = conn.guest_seq;
                self.send_tcp_flags(key, 0x18, seq, ack, &data); // PSH+ACK
                // Advance our_seq
                if let Some(conn) = self.tcp_conns.get_mut(&key) {
                    conn.our_seq = conn.our_seq.wrapping_add(data.len() as u32);
                }
            }
        }

        for key in closed {
            if let Some(conn) = self.tcp_conns.get(&key) {
                let seq = conn.our_seq;
                let ack = conn.guest_seq;
                self.send_tcp_flags(key, 0x11, seq, ack, &[]); // FIN+ACK
            }
            self.tcp_conns.remove(&key);
        }
    }

    fn send_tcp_flags(&mut self, key: TcpFlowKey, flags: u8, seq: u32, ack: u32, data: &[u8]) {
        let hdr_len: u8 = 20;
        let mut tcp = vec![0u8; hdr_len as usize + data.len()];
        put_u16be(&mut tcp, 0, key.remote_port); // src port (from gateway perspective)
        put_u16be(&mut tcp, 2, key.guest_port);  // dst port
        put_u32be(&mut tcp, 4, seq);
        put_u32be(&mut tcp, 8, ack);
        tcp[12] = (hdr_len / 4) << 4; // data offset
        tcp[13] = flags;
        put_u16be(&mut tcp, 14, 65535); // window size
        if !data.is_empty() {
            tcp[20..].copy_from_slice(data);
        }

        // TCP checksum (pseudo-header + TCP header + data)
        let src = key.remote_ip;
        let dst = GUEST_IP;
        let tcp_len = tcp.len() as u16;
        let mut pseudo = vec![0u8; 12 + tcp.len()];
        pseudo[0..4].copy_from_slice(&src);
        pseudo[4..8].copy_from_slice(&dst);
        pseudo[9] = IP_PROTO_TCP;
        put_u16be(&mut pseudo, 10, tcp_len);
        pseudo[12..].copy_from_slice(&tcp);
        let cksum = ip_checksum(&pseudo);
        put_u16be(&mut tcp, 16, cksum);

        self.send_ip_packet(IP_PROTO_TCP, key.remote_ip, GUEST_IP, &tcp);
    }

    // ── Common packet builder ────────────────────────────────────────────

    fn send_ip_packet(&mut self, proto: u8, src: [u8; 4], dst: [u8; 4], payload: &[u8]) {
        let id = self.next_ip_id();
        let ip_hdr = ip_header(proto, src, dst, payload.len() as u16, id);
        let eth = eth_header(&self.guest_mac, &GW_MAC, ETHERTYPE_IPV4);

        let mut frame = Vec::with_capacity(ETH_HDR + 20 + payload.len());
        frame.extend_from_slice(&eth);
        frame.extend_from_slice(&ip_hdr);
        frame.extend_from_slice(payload);

        self.rx_queue.push_back(frame);
    }
}

impl NetBackend for SlirpNet {
    fn send(&mut self, frame: &[u8]) {
        self.process_frame(frame);
    }

    fn recv(&mut self) -> Vec<Vec<u8>> {
        let mut out = Vec::with_capacity(self.rx_queue.len());
        while let Some(f) = self.rx_queue.pop_front() {
            out.push(f);
        }
        out
    }

    fn poll(&mut self) {
        self.poll_dns();
        self.poll_udp();
        self.poll_tcp();
    }

    fn description(&self) -> &str {
        "user-mode NAT (10.0.2.0/24)"
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn find_dhcp_option<'a>(options: &'a [u8], code: u8) -> Option<&'a [u8]> {
    let mut i = 0;
    while i < options.len() {
        let opt = options[i];
        if opt == 255 { break; } // end
        if opt == 0 { i += 1; continue; } // pad
        if i + 1 >= options.len() { break; }
        let len = options[i + 1] as usize;
        if i + 2 + len > options.len() { break; }
        if opt == code {
            return Some(&options[i + 2..i + 2 + len]);
        }
        i += 2 + len;
    }
    None
}

fn detect_host_dns() -> SocketAddr {
    // Try to read /etc/resolv.conf
    if let Ok(contents) = std::fs::read_to_string("/etc/resolv.conf") {
        for line in contents.lines() {
            let line = line.trim();
            if line.starts_with("nameserver") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(ip) = parts[1].parse::<Ipv4Addr>() {
                        // Skip loopback (systemd-resolved uses 127.0.0.53)
                        // — it works fine for us since we forward from host context
                        return SocketAddr::new(ip.into(), 53);
                    }
                }
            }
        }
    }
    // Fallback: Google DNS
    SocketAddr::new(Ipv4Addr::new(8, 8, 8, 8).into(), 53)
}

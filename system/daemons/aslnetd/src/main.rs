#![cfg_attr(not(target_os = "linux"), no_std)]
#![cfg_attr(not(target_os = "linux"), no_main)]

#[cfg(target_os = "linux")]
extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use anyos_std::{fs, ipc, net, println, process, sys};
use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};
use libsvc::ServiceLifecycle;

#[cfg(not(target_os = "linux"))]
anyos_std::entry!(main);

const PIPE_NAME: &str = "aslnetd";
const STATUS_PATH: &str = "/System/var/asl/aslnetd.status";

const ASLNETD_DIRS: &[&str] = &["config"];
const ASLNETD_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_bool("config/nat_enabled", true),
    default_bool("config/dns_broker_enabled", true),
    default_string("config/default_listen_address", "127.0.0.1"),
    default_int("config/max_forward_rules", 256),
];
const ASLNETD_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/aslnetd",
    RegistryScope::System,
    1,
    ASLNETD_DIRS,
    ASLNETD_DEFAULTS,
    &[],
);
const ASLNETD_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("aslnetd", &ASLNETD_MANIFEST);

#[derive(Clone)]
struct ForwardRule {
    distro: String,
    id: String,
    listen_address: String,
    listen_port: u16,
    guest_port: u16,
    protocol: String,
}

struct PacketFrame {
    distro: String,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct TcpNatConn {
    distro: String,
    guest_mac: [u8; 6],
    guest_ip: [u8; 4],
    guest_port: u16,
    remote_ip: [u8; 4],
    remote_port: u16,
    socket_id: u32,
    guest_next_seq: u32,
    host_next_seq: u32,
    established: bool,
    closing: bool,
}

#[derive(Clone)]
struct UdpNatFlow {
    distro: String,
    guest_mac: [u8; 6],
    guest_ip: [u8; 4],
    guest_port: u16,
    remote_ip: [u8; 4],
    remote_port: u16,
    local_port: u16,
    last_seen_ms: u32,
}

struct BrokerState {
    rules: Vec<ForwardRule>,
    rx_queue: Vec<PacketFrame>,
    tcp_nat: Vec<TcpNatConn>,
    udp_nat: Vec<UdpNatFlow>,
    apply_count: u32,
    rejected_count: u32,
    last_apply_ms: u32,
    tx_packets: u64,
    tx_bytes: u64,
    rx_packets: u64,
    rx_bytes: u64,
    control_packets: u64,
    nat_packets: u64,
    dropped_packets: u64,
    /// `config/dns_broker_enabled` from the confd schema (default true).
    /// When false, guest DNS queries are not answered by the broker —
    /// the guest must use a different resolver (e.g. resolv.conf pointing
    /// elsewhere). Toggleable at runtime via `SET_DNS_BROKER 0|1`.
    dns_broker_enabled: bool,
    /// Counter exposed in the status file for observability — how many
    /// guest DNS queries were dropped because the broker was disabled.
    dns_disabled_drops: u64,
}

impl BrokerState {
    fn new() -> Self {
        Self {
            rules: Vec::new(),
            rx_queue: Vec::new(),
            tcp_nat: Vec::new(),
            udp_nat: Vec::new(),
            apply_count: 0,
            rejected_count: 0,
            last_apply_ms: 0,
            tx_packets: 0,
            tx_bytes: 0,
            rx_packets: 0,
            rx_bytes: 0,
            control_packets: 0,
            nat_packets: 0,
            dropped_packets: 0,
            dns_broker_enabled: true, // matches default_bool("config/dns_broker_enabled", true)
            dns_disabled_drops: 0,
        }
    }
}

const ASL_HOST_MAC: [u8; 6] = [0x02, 0x41, 0x53, 0x4c, 0x00, 0xfe];
const ASL_GUEST_IP: [u8; 4] = [172, 30, 0, 2];
const ASL_GATEWAY_IP: [u8; 4] = [172, 30, 0, 1];
const ASL_NETMASK: [u8; 4] = [255, 255, 255, 0];
const ASL_BROADCAST_IP: [u8; 4] = [172, 30, 0, 255];
const IPV4_BROADCAST: [u8; 4] = [255, 255, 255, 255];
const ETH_BROADCAST: [u8; 6] = [0xff; 6];
const TCP_SYN: u16 = 0x02;
const TCP_RST: u16 = 0x04;
const TCP_PSH: u16 = 0x08;
const TCP_ACK: u16 = 0x10;
const TCP_FIN: u16 = 0x01;
const TCP_RECV_EOF: u32 = u32::MAX - 1;
const UDP_NAT_PORT_BASE: u16 = 40960;
const UDP_NAT_MAX_FLOWS: usize = 128;

fn main() {
    println!("aslnetd: starting");
    let _ = ASLNETD_SCHEMA.register();

    let mut lifecycle = ServiceLifecycle::connect("aslnetd").ok();
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_starting();
        let _ = lifecycle.set_health("starting");
    }

    let old_pipe = ipc::pipe_open(PIPE_NAME);
    if old_pipe != 0 {
        ipc::pipe_close(old_pipe);
    }
    let pipe_id = ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        println!("aslnetd: failed to create '{}' pipe", PIPE_NAME);
        if let Some(lifecycle) = lifecycle.as_mut() {
            let _ = lifecycle.notify_failed("pipe_create_failed");
        }
        return;
    }

    let mut state = BrokerState::new();
    write_status(&state, "ready");
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_ready();
        let _ = lifecycle.set_health("ready");
    }
    println!("aslnetd: ready (pipe='{}')", PIPE_NAME);

    let mut pending = String::new();
    let mut buf = [0u8; 1024];
    loop {
        if handle_requests(pipe_id, &mut pending, &mut state, &mut buf) {
            process::sleep(20);
        } else {
            process::sleep(100);
        }
    }
}

fn handle_requests(
    pipe_id: u32,
    pending: &mut String,
    state: &mut BrokerState,
    buf: &mut [u8],
) -> bool {
    let n = ipc::pipe_read(pipe_id, buf);
    if n == 0 || n == u32::MAX {
        return false;
    }
    let Ok(text) = core::str::from_utf8(&buf[..n as usize]) else {
        return true;
    };
    pending.push_str(text);
    while let Some(pos) = pending.find('\n') {
        let mut line = pending[..pos].to_string();
        pending.drain(..=pos);
        if line.ends_with('\r') {
            line.pop();
        }
        if !line.is_empty() {
            handle_line(state, &line);
        }
    }
    true
}

fn handle_line(state: &mut BrokerState, line: &str) {
    let Some(tab_pos) = line.find('\t') else {
        return;
    };
    let Some(tid) = parse_u32(&line[..tab_pos]) else {
        return;
    };
    let cmd = line[tab_pos + 1..].trim();
    let response = dispatch(state, cmd);
    let reply_name = format!("aslnetd-{}", tid);
    let reply_pipe = ipc::pipe_open(&reply_name);
    if reply_pipe != 0 {
        ipc::pipe_write(reply_pipe, response.as_bytes());
    }
}

fn dispatch(state: &mut BrokerState, cmd: &str) -> String {
    let (verb, rest) = split_first_word(cmd);
    match verb {
        "STATUS" | "status" => status_response(state),
        "CLEAR" | "clear" => {
            let distro = rest.trim();
            if distro.is_empty() {
                return err("invalid_distro");
            }
            let before = state.rules.len();
            state.rules.retain(|rule| rule.distro != distro);
            clear_nat_for_distro(state, distro);
            state.apply_count = state.apply_count.wrapping_add(1);
            state.last_apply_ms = sys::uptime_ms();
            write_status(state, "ready");
            ok_lines(alloc::vec![
                format!("distro\t{}", distro),
                format!("removed\t{}", before.saturating_sub(state.rules.len())),
            ])
        }
        "APPLY" | "apply" => apply_rule(state, rest),
        "VALIDATE" | "validate" => validate_response(rest),
        "TX" | "tx" => tx_frame(state, rest),
        "RX_POLL" | "rx_poll" => rx_poll(state, rest),
        "INJECT" | "inject" => inject_frame(state, rest),
        "SET_DNS_BROKER" | "set_dns_broker" => set_dns_broker(state, rest),
        _ => err("unknown_command"),
    }
}

/// Toggle the DNS broker at runtime. Argument: `0` or `1`. Mirrors the
/// `config/dns_broker_enabled` schema default. Returns the new state.
fn set_dns_broker(state: &mut BrokerState, rest: &str) -> String {
    let arg = rest.trim();
    let new_value = match arg {
        "1" | "true" | "on" | "enable" | "enabled" => true,
        "0" | "false" | "off" | "disable" | "disabled" => false,
        _ => return err("invalid_dns_broker_arg"),
    };
    state.dns_broker_enabled = new_value;
    write_status(state, "ready");
    ok_lines(alloc::vec![format!("dns_broker_enabled\t{}", new_value)])
}

fn apply_rule(state: &mut BrokerState, rest: &str) -> String {
    let fields = split_tab_fields(rest);
    if fields.len() < 6 {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_apply");
    }
    let Some(listen_port) = parse_u16(fields[3]) else {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_listen_port");
    };
    let Some(guest_port) = parse_u16(fields[4]) else {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_guest_port");
    };
    if let Err(message) = validate_rule(fields[1], fields[2], listen_port, guest_port, fields[5]) {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err(message);
    }

    let new_rule = ForwardRule {
        distro: String::from(fields[0]),
        id: String::from(fields[1]),
        listen_address: String::from(fields[2]),
        listen_port,
        guest_port,
        protocol: String::from(fields[5]),
    };
    if state.rules.iter().any(|rule| conflicts(rule, &new_rule)) {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("listener_conflict");
    }
    state
        .rules
        .retain(|rule| !(rule.distro == new_rule.distro && rule.id == new_rule.id));
    state.rules.push(new_rule);
    state.apply_count = state.apply_count.wrapping_add(1);
    state.last_apply_ms = sys::uptime_ms();
    write_status(state, "ready");
    ok_lines(alloc::vec![
        format!("rules\t{}", state.rules.len()),
        String::from("applied\ttrue"),
    ])
}

fn validate_response(rest: &str) -> String {
    let fields = split_tab_fields(rest);
    if fields.len() < 5 {
        return err("invalid_validate");
    }
    let Some(listen_port) = parse_u16(fields[2]) else {
        return err("invalid_listen_port");
    };
    let Some(guest_port) = parse_u16(fields[3]) else {
        return err("invalid_guest_port");
    };
    match validate_rule(fields[0], fields[1], listen_port, guest_port, fields[4]) {
        Ok(()) => ok_lines(alloc::vec![String::from("valid\ttrue")]),
        Err(message) => ok_lines(alloc::vec![
            String::from("valid\tfalse"),
            format!("message\t{}", message),
        ]),
    }
}

fn tx_frame(state: &mut BrokerState, rest: &str) -> String {
    let fields = split_tab_fields(rest);
    if fields.len() < 2 || fields[0].is_empty() {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_tx");
    }
    let Some(frame) = decode_hex(fields[1]) else {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_frame");
    };
    if frame.is_empty() || frame.len() > 1518 {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_frame_size");
    }

    state.tx_packets = state.tx_packets.wrapping_add(1);
    state.tx_bytes = state.tx_bytes.wrapping_add(frame.len() as u64);
    let rx_generated = handle_guest_frame(state, fields[0], &frame);
    state.last_apply_ms = sys::uptime_ms();
    write_status(state, "ready");
    ok_lines(alloc::vec![
        format!("distro\t{}", fields[0]),
        format!("tx_bytes\t{}", frame.len()),
        format!("rx_generated\t{}", rx_generated),
    ])
}

fn rx_poll(state: &mut BrokerState, rest: &str) -> String {
    let fields = split_tab_fields(rest);
    if fields.is_empty() || fields[0].is_empty() {
        return err("invalid_rx_poll");
    }
    let max_bytes = fields
        .get(1)
        .and_then(|value| parse_u32(value))
        .unwrap_or(1518)
        .clamp(64, 1518) as usize;

    poll_nat_for_distro(state, fields[0]);

    let Some(index) = state
        .rx_queue
        .iter()
        .position(|frame| frame.distro == fields[0] && frame.bytes.len() <= max_bytes)
    else {
        return ok_lines(alloc::vec![
            String::from("packet\tfalse"),
            String::from("bytes\t0"),
        ]);
    };
    let frame = state.rx_queue.remove(index);
    state.rx_packets = state.rx_packets.wrapping_add(1);
    state.rx_bytes = state.rx_bytes.wrapping_add(frame.bytes.len() as u64);
    state.last_apply_ms = sys::uptime_ms();
    write_status(state, "ready");
    ok_lines(alloc::vec![
        String::from("packet\ttrue"),
        format!("bytes\t{}", frame.bytes.len()),
        format!("data\t{}", encode_hex(&frame.bytes)),
    ])
}

fn inject_frame(state: &mut BrokerState, rest: &str) -> String {
    let fields = split_tab_fields(rest);
    if fields.len() < 2 || fields[0].is_empty() {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_inject");
    }
    let Some(frame) = decode_hex(fields[1]) else {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_frame");
    };
    if frame.is_empty() || frame.len() > 1518 {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_frame_size");
    }
    if state.rx_queue.len() >= 64 {
        state.dropped_packets = state.dropped_packets.wrapping_add(1);
        return err("rx_queue_full");
    }

    state.rx_queue.push(PacketFrame {
        distro: String::from(fields[0]),
        bytes: frame,
    });
    write_status(state, "ready");
    ok_lines(alloc::vec![format!("queued\t{}", state.rx_queue.len())])
}

fn handle_guest_frame(state: &mut BrokerState, distro: &str, frame: &[u8]) -> usize {
    let mut generated = 0usize;
    if let Some(reply) = arp_reply(frame) {
        generated += enqueue_control_frame(state, distro, reply) as usize;
    }
    if let Some(reply) = dhcp_reply(frame) {
        generated += enqueue_control_frame(state, distro, reply) as usize;
    }
    // ADR-0003 / config/dns_broker_enabled: only answer DNS when the
    // broker is enabled. When disabled we count the drop so operators
    // can see it in the status file, and the guest gets no reply
    // (resolv.conf-driven fallback can take over).
    if state.dns_broker_enabled {
        if let Some(reply) = dns_reply(frame) {
            generated += enqueue_control_frame(state, distro, reply) as usize;
        }
    } else if is_dns_query(frame) {
        state.dns_disabled_drops = state.dns_disabled_drops.wrapping_add(1);
    }
    if let Some(reply) = icmp_echo_reply(frame) {
        generated += enqueue_control_frame(state, distro, reply) as usize;
    }
    generated += handle_udp_nat_frame(state, distro, frame);
    generated += handle_tcp_nat_frame(state, distro, frame);
    generated
}

fn enqueue_control_frame(state: &mut BrokerState, distro: &str, bytes: Vec<u8>) -> bool {
    if bytes.is_empty() || bytes.len() > 1518 {
        state.dropped_packets = state.dropped_packets.wrapping_add(1);
        return false;
    }
    if state.rx_queue.len() >= 64 {
        state.dropped_packets = state.dropped_packets.wrapping_add(1);
        return false;
    }
    state.control_packets = state.control_packets.wrapping_add(1);
    state.rx_queue.push(PacketFrame {
        distro: String::from(distro),
        bytes,
    });
    true
}

fn enqueue_nat_frame(state: &mut BrokerState, distro: &str, bytes: Vec<u8>) -> bool {
    if bytes.is_empty() || bytes.len() > 1518 {
        state.dropped_packets = state.dropped_packets.wrapping_add(1);
        return false;
    }
    if state.rx_queue.len() >= 64 {
        state.dropped_packets = state.dropped_packets.wrapping_add(1);
        return false;
    }
    state.nat_packets = state.nat_packets.wrapping_add(1);
    state.rx_queue.push(PacketFrame {
        distro: String::from(distro),
        bytes,
    });
    true
}

fn arp_reply(frame: &[u8]) -> Option<Vec<u8>> {
    if frame.len() < 42 || read_be16(frame, 12) != 0x0806 {
        return None;
    }
    if read_be16(frame, 14) != 1 || read_be16(frame, 16) != 0x0800 {
        return None;
    }
    if frame[18] != 6 || frame[19] != 4 || read_be16(frame, 20) != 1 {
        return None;
    }
    let target_ip = read_ipv4(frame, 38)?;
    if target_ip != ASL_GATEWAY_IP {
        return None;
    }
    let guest_mac = read_mac(frame, 22)?;
    let guest_ip = read_ipv4(frame, 28)?;

    let mut out = Vec::new();
    out.extend_from_slice(&guest_mac);
    out.extend_from_slice(&ASL_HOST_MAC);
    out.extend_from_slice(&0x0806u16.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&0x0800u16.to_be_bytes());
    out.push(6);
    out.push(4);
    out.extend_from_slice(&2u16.to_be_bytes());
    out.extend_from_slice(&ASL_HOST_MAC);
    out.extend_from_slice(&ASL_GATEWAY_IP);
    out.extend_from_slice(&guest_mac);
    out.extend_from_slice(&guest_ip);
    Some(out)
}

fn dhcp_reply(frame: &[u8]) -> Option<Vec<u8>> {
    let ipv4 = parse_ipv4(frame)?;
    if ipv4.protocol != 17 {
        return None;
    }
    let udp = parse_udp(frame, ipv4.payload_offset, ipv4.payload_len)?;
    if udp.src_port != 68 || udp.dst_port != 67 {
        return None;
    }
    let bootp = frame.get(udp.payload_offset..udp.payload_offset + udp.payload_len)?;
    if bootp.len() < 240 || bootp[236..240] != [99, 130, 83, 99] {
        return None;
    }

    let message_type = dhcp_option(bootp, 53).and_then(|value| value.first().copied())?;
    let reply_type = match message_type {
        1 => 2,
        3 => 5,
        _ => return None,
    };
    let guest_mac = read_mac(bootp, 28)?;
    let flags = read_be16(bootp, 10);
    let dst_mac = if flags & 0x8000 != 0 {
        ETH_BROADCAST
    } else {
        guest_mac
    };

    let mut payload = alloc::vec![0u8; 236];
    payload[0] = 2;
    payload[1] = 1;
    payload[2] = 6;
    payload[3] = 0;
    payload[4..8].copy_from_slice(&bootp[4..8]);
    payload[8..12].copy_from_slice(&bootp[8..12]);
    payload[16..20].copy_from_slice(&ASL_GUEST_IP);
    payload[20..24].copy_from_slice(&ASL_GATEWAY_IP);
    payload[28..44].copy_from_slice(&bootp[28..44]);
    payload.extend_from_slice(&[99, 130, 83, 99]);
    push_dhcp_option(&mut payload, 53, &[reply_type]);
    push_dhcp_option(&mut payload, 54, &ASL_GATEWAY_IP);
    push_dhcp_option(&mut payload, 51, &86400u32.to_be_bytes());
    push_dhcp_option(&mut payload, 1, &ASL_NETMASK);
    push_dhcp_option(&mut payload, 3, &ASL_GATEWAY_IP);
    push_dhcp_option(&mut payload, 6, &ASL_GATEWAY_IP);
    push_dhcp_option(&mut payload, 28, &ASL_BROADCAST_IP);
    push_dhcp_option(&mut payload, 58, &43200u32.to_be_bytes());
    push_dhcp_option(&mut payload, 59, &75600u32.to_be_bytes());
    push_dhcp_option(&mut payload, 15, b"asl");
    payload.push(255);
    while payload.len() < 300 {
        payload.push(0);
    }

    Some(build_udp_ipv4_frame(
        dst_mac,
        ASL_HOST_MAC,
        ASL_GATEWAY_IP,
        IPV4_BROADCAST,
        67,
        68,
        &payload,
    ))
}

/// True if `frame` looks like a DNS query (UDP/53 with non-empty
/// question section). Lightweight check — does not do full parse.
/// Used by the disabled-DNS drop counter so we don't count unrelated
/// UDP traffic.
fn is_dns_query(frame: &[u8]) -> bool {
    let Some(ipv4) = parse_ipv4(frame) else {
        return false;
    };
    if ipv4.protocol != 17 {
        return false;
    }
    let Some(udp) = parse_udp(frame, ipv4.payload_offset, ipv4.payload_len) else {
        return false;
    };
    if udp.dst_port != 53 {
        return false;
    }
    let Some(query) = frame.get(udp.payload_offset..udp.payload_offset + udp.payload_len) else {
        return false;
    };
    query.len() >= 12 && read_be16(query, 4) != 0
}

fn dns_reply(frame: &[u8]) -> Option<Vec<u8>> {
    let ipv4 = parse_ipv4(frame)?;
    if ipv4.protocol != 17 {
        return None;
    }
    let udp = parse_udp(frame, ipv4.payload_offset, ipv4.payload_len)?;
    if udp.dst_port != 53 {
        return None;
    }
    let query = frame.get(udp.payload_offset..udp.payload_offset + udp.payload_len)?;
    if query.len() < 12 || read_be16(query, 4) == 0 {
        return None;
    }
    let (question_end, name) = parse_dns_question(query, 12)?;
    let resolved = resolve_dns_name(&name);

    let mut payload = Vec::new();
    payload.extend_from_slice(&query[0..2]);
    let flags = if resolved.is_some() {
        0x8180u16
    } else {
        0x8183u16
    };
    payload.extend_from_slice(&flags.to_be_bytes());
    payload.extend_from_slice(&1u16.to_be_bytes());
    payload.extend_from_slice(&(resolved.is_some() as u16).to_be_bytes());
    payload.extend_from_slice(&0u16.to_be_bytes());
    payload.extend_from_slice(&0u16.to_be_bytes());
    payload.extend_from_slice(&query[12..question_end]);
    if let Some(ip) = resolved {
        payload.extend_from_slice(&0xc00cu16.to_be_bytes());
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.extend_from_slice(&60u32.to_be_bytes());
        payload.extend_from_slice(&4u16.to_be_bytes());
        payload.extend_from_slice(&ip);
    }

    Some(build_udp_ipv4_frame(
        ipv4.src_mac,
        ASL_HOST_MAC,
        ASL_GATEWAY_IP,
        ipv4.src_ip,
        53,
        udp.src_port,
        &payload,
    ))
}

fn icmp_echo_reply(frame: &[u8]) -> Option<Vec<u8>> {
    let ipv4 = parse_ipv4(frame)?;
    if ipv4.protocol != 1 || ipv4.dst_ip != ASL_GATEWAY_IP {
        return None;
    }
    let payload = frame.get(ipv4.payload_offset..ipv4.payload_offset + ipv4.payload_len)?;
    if payload.len() < 8 || payload[0] != 8 {
        return None;
    }
    let mut icmp = payload.to_vec();
    icmp[0] = 0;
    icmp[2] = 0;
    icmp[3] = 0;
    let checksum = internet_checksum(&icmp);
    icmp[2..4].copy_from_slice(&checksum.to_be_bytes());
    Some(build_ipv4_frame(
        ipv4.src_mac,
        ASL_HOST_MAC,
        ASL_GATEWAY_IP,
        ipv4.src_ip,
        1,
        &icmp,
    ))
}

fn handle_udp_nat_frame(state: &mut BrokerState, distro: &str, frame: &[u8]) -> usize {
    let Some(ipv4) = parse_ipv4(frame) else {
        return 0;
    };
    if ipv4.protocol != 17 || is_control_ipv4(ipv4.dst_ip) {
        return 0;
    }
    let Some(udp) = parse_udp(frame, ipv4.payload_offset, ipv4.payload_len) else {
        return 0;
    };
    if matches!(udp.dst_port, 53 | 67 | 68) {
        return 0;
    }
    let Some(payload) = frame.get(udp.payload_offset..udp.payload_offset + udp.payload_len) else {
        return 0;
    };
    let now = sys::uptime_ms();
    let local_port = ensure_udp_nat_flow(
        state,
        distro,
        ipv4.src_mac,
        ipv4.src_ip,
        udp.src_port,
        ipv4.dst_ip,
        udp.dst_port,
        now,
    );
    if local_port == 0 {
        state.dropped_packets = state.dropped_packets.wrapping_add(1);
        return 0;
    }

    let sent = udp_sendto(&ipv4.dst_ip, udp.dst_port, local_port, payload, 0);
    if sent == u32::MAX {
        state.dropped_packets = state.dropped_packets.wrapping_add(1);
        return 0;
    }
    0
}

fn ensure_udp_nat_flow(
    state: &mut BrokerState,
    distro: &str,
    guest_mac: [u8; 6],
    guest_ip: [u8; 4],
    guest_port: u16,
    remote_ip: [u8; 4],
    remote_port: u16,
    now: u32,
) -> u16 {
    if let Some(flow) = state.udp_nat.iter_mut().find(|flow| {
        flow.distro == distro
            && flow.guest_ip == guest_ip
            && flow.guest_port == guest_port
            && flow.remote_ip == remote_ip
            && flow.remote_port == remote_port
    }) {
        flow.guest_mac = guest_mac;
        flow.last_seen_ms = now;
        return flow.local_port;
    }

    if state.udp_nat.len() >= UDP_NAT_MAX_FLOWS {
        state.udp_nat.remove(0);
    }
    let local_port = allocate_udp_nat_port(state);
    if udp_bind(local_port) == u32::MAX {
        return 0;
    }
    let _ = udp_set_opt(local_port, 2, 0);
    state.udp_nat.push(UdpNatFlow {
        distro: String::from(distro),
        guest_mac,
        guest_ip,
        guest_port,
        remote_ip,
        remote_port,
        local_port,
        last_seen_ms: now,
    });
    local_port
}

fn allocate_udp_nat_port(state: &BrokerState) -> u16 {
    for offset in 0..UDP_NAT_MAX_FLOWS {
        let candidate = UDP_NAT_PORT_BASE + offset as u16;
        if !state
            .udp_nat
            .iter()
            .any(|flow| flow.local_port == candidate)
        {
            return candidate;
        }
    }
    UDP_NAT_PORT_BASE
}

fn handle_tcp_nat_frame(state: &mut BrokerState, distro: &str, frame: &[u8]) -> usize {
    let Some(ipv4) = parse_ipv4(frame) else {
        return 0;
    };
    if ipv4.protocol != 6 || is_control_ipv4(ipv4.dst_ip) {
        return 0;
    }
    let Some(tcp) = parse_tcp(frame, ipv4.payload_offset, ipv4.payload_len) else {
        return 0;
    };
    if tcp.flags & TCP_RST != 0 {
        close_tcp_nat_flow(
            state,
            distro,
            ipv4.src_ip,
            tcp.src_port,
            ipv4.dst_ip,
            tcp.dst_port,
        );
        return 0;
    }

    if tcp.flags & TCP_SYN != 0 && tcp.flags & TCP_ACK == 0 {
        return handle_tcp_syn(state, distro, &ipv4, &tcp) as usize;
    }

    let Some(index) = find_tcp_nat_flow(
        state,
        distro,
        ipv4.src_ip,
        tcp.src_port,
        ipv4.dst_ip,
        tcp.dst_port,
    ) else {
        return 0;
    };

    state.tcp_nat[index].guest_mac = ipv4.src_mac;
    if tcp.flags & TCP_ACK != 0 {
        state.tcp_nat[index].established = true;
    }

    let mut generated = 0usize;
    if !tcp.payload.is_empty() {
        let socket_id = state.tcp_nat[index].socket_id;
        let sent = net::tcp_send(socket_id, tcp.payload);
        if sent == u32::MAX {
            let reply = tcp_reset_for(&state.tcp_nat[index]);
            close_tcp_nat_by_index(state, index);
            return enqueue_nat_frame(state, distro, reply) as usize;
        }
        let sent_len = (sent as usize).min(tcp.payload.len());
        state.tcp_nat[index].guest_next_seq = tcp.seq.wrapping_add(sent_len as u32);
        let reply = tcp_ack_for(&state.tcp_nat[index]);
        generated += enqueue_nat_frame(state, distro, reply) as usize;
    }

    if tcp.flags & TCP_FIN != 0 {
        state.tcp_nat[index].guest_next_seq = tcp.seq.wrapping_add(tcp.payload.len() as u32 + 1);
        let reply = tcp_fin_ack_for(&state.tcp_nat[index]);
        close_tcp_nat_by_index(state, index);
        generated += enqueue_nat_frame(state, distro, reply) as usize;
    }

    generated
}

fn handle_tcp_syn(
    state: &mut BrokerState,
    distro: &str,
    ipv4: &Ipv4Packet,
    tcp: &TcpPacket<'_>,
) -> bool {
    close_tcp_nat_flow(
        state,
        distro,
        ipv4.src_ip,
        tcp.src_port,
        ipv4.dst_ip,
        tcp.dst_port,
    );
    let socket_id = net::tcp_connect(&ipv4.dst_ip, tcp.dst_port, 5000);
    if socket_id == u32::MAX {
        let reply = build_tcp_ipv4_frame(
            ipv4.src_mac,
            ASL_HOST_MAC,
            ipv4.dst_ip,
            ipv4.src_ip,
            tcp.dst_port,
            tcp.src_port,
            0,
            tcp.seq.wrapping_add(1),
            TCP_RST | TCP_ACK,
            &[],
        );
        return enqueue_nat_frame(state, distro, reply);
    }

    let host_seq = initial_tcp_seq(ipv4.dst_ip, tcp.dst_port, ipv4.src_ip, tcp.src_port);
    let conn = TcpNatConn {
        distro: String::from(distro),
        guest_mac: ipv4.src_mac,
        guest_ip: ipv4.src_ip,
        guest_port: tcp.src_port,
        remote_ip: ipv4.dst_ip,
        remote_port: tcp.dst_port,
        socket_id,
        guest_next_seq: tcp.seq.wrapping_add(1),
        host_next_seq: host_seq.wrapping_add(1),
        established: false,
        closing: false,
    };
    let reply = build_tcp_ipv4_frame(
        conn.guest_mac,
        ASL_HOST_MAC,
        conn.remote_ip,
        conn.guest_ip,
        conn.remote_port,
        conn.guest_port,
        host_seq,
        conn.guest_next_seq,
        TCP_SYN | TCP_ACK,
        &[],
    );
    state.tcp_nat.push(conn);
    enqueue_nat_frame(state, distro, reply)
}

fn poll_nat_for_distro(state: &mut BrokerState, distro: &str) {
    poll_udp_nat_for_distro(state, distro);
    poll_tcp_nat_for_distro(state, distro);
}

fn poll_udp_nat_for_distro(state: &mut BrokerState, distro: &str) {
    let flows = state.udp_nat.clone();
    let mut rx = [0u8; 1536];
    for flow in flows.iter().filter(|flow| flow.distro == distro) {
        loop {
            let n = udp_recvfrom(flow.local_port, &mut rx);
            if n == 0 || n == u32::MAX || n < 8 {
                break;
            }
            let n = n as usize;
            let src_ip = [rx[0], rx[1], rx[2], rx[3]];
            let src_port = u16::from_le_bytes([rx[4], rx[5]]);
            let payload_len = u16::from_le_bytes([rx[6], rx[7]]) as usize;
            if payload_len == 0 || 8 + payload_len > n {
                break;
            }
            let payload = &rx[8..8 + payload_len];
            let frame = build_udp_ipv4_frame(
                flow.guest_mac,
                ASL_HOST_MAC,
                src_ip,
                flow.guest_ip,
                src_port,
                flow.guest_port,
                payload,
            );
            if !enqueue_nat_frame(state, distro, frame) {
                break;
            }
        }
    }
}

fn poll_tcp_nat_for_distro(state: &mut BrokerState, distro: &str) {
    let mut index = 0usize;
    while index < state.tcp_nat.len() {
        if state.tcp_nat[index].distro != distro || state.tcp_nat[index].closing {
            index += 1;
            continue;
        }
        let available = net::tcp_recv_available(state.tcp_nat[index].socket_id);
        if available == TCP_RECV_EOF {
            let frame = tcp_fin_ack_for(&state.tcp_nat[index]);
            close_tcp_nat_by_index(state, index);
            let _ = enqueue_nat_frame(state, distro, frame);
            continue;
        }
        if available == 0 || available == u32::MAX {
            index += 1;
            continue;
        }

        let mut buf = alloc::vec![0u8; (available as usize).min(1400)];
        let n = net::tcp_recv(state.tcp_nat[index].socket_id, &mut buf);
        if n == 0 || n == u32::MAX {
            index += 1;
            continue;
        }
        buf.truncate((n as usize).min(buf.len()));
        let frame = tcp_data_for(&mut state.tcp_nat[index], &buf);
        let _ = enqueue_nat_frame(state, distro, frame);
        index += 1;
    }
}

fn clear_nat_for_distro(state: &mut BrokerState, distro: &str) {
    let mut index = 0usize;
    while index < state.tcp_nat.len() {
        if state.tcp_nat[index].distro == distro {
            close_tcp_nat_by_index(state, index);
        } else {
            index += 1;
        }
    }
    let mut udp_index = 0usize;
    while udp_index < state.udp_nat.len() {
        if state.udp_nat[udp_index].distro == distro {
            let flow = state.udp_nat.remove(udp_index);
            let _ = udp_unbind(flow.local_port);
        } else {
            udp_index += 1;
        }
    }
}

fn close_tcp_nat_flow(
    state: &mut BrokerState,
    distro: &str,
    guest_ip: [u8; 4],
    guest_port: u16,
    remote_ip: [u8; 4],
    remote_port: u16,
) {
    if let Some(index) =
        find_tcp_nat_flow(state, distro, guest_ip, guest_port, remote_ip, remote_port)
    {
        close_tcp_nat_by_index(state, index);
    }
}

fn close_tcp_nat_by_index(state: &mut BrokerState, index: usize) {
    let conn = state.tcp_nat.remove(index);
    let _ = net::tcp_close(conn.socket_id);
}

fn find_tcp_nat_flow(
    state: &BrokerState,
    distro: &str,
    guest_ip: [u8; 4],
    guest_port: u16,
    remote_ip: [u8; 4],
    remote_port: u16,
) -> Option<usize> {
    state.tcp_nat.iter().position(|conn| {
        conn.distro == distro
            && conn.guest_ip == guest_ip
            && conn.guest_port == guest_port
            && conn.remote_ip == remote_ip
            && conn.remote_port == remote_port
    })
}

fn tcp_ack_for(conn: &TcpNatConn) -> Vec<u8> {
    build_tcp_ipv4_frame(
        conn.guest_mac,
        ASL_HOST_MAC,
        conn.remote_ip,
        conn.guest_ip,
        conn.remote_port,
        conn.guest_port,
        conn.host_next_seq,
        conn.guest_next_seq,
        TCP_ACK,
        &[],
    )
}

fn tcp_reset_for(conn: &TcpNatConn) -> Vec<u8> {
    build_tcp_ipv4_frame(
        conn.guest_mac,
        ASL_HOST_MAC,
        conn.remote_ip,
        conn.guest_ip,
        conn.remote_port,
        conn.guest_port,
        conn.host_next_seq,
        conn.guest_next_seq,
        TCP_RST | TCP_ACK,
        &[],
    )
}

fn tcp_fin_ack_for(conn: &TcpNatConn) -> Vec<u8> {
    build_tcp_ipv4_frame(
        conn.guest_mac,
        ASL_HOST_MAC,
        conn.remote_ip,
        conn.guest_ip,
        conn.remote_port,
        conn.guest_port,
        conn.host_next_seq,
        conn.guest_next_seq,
        TCP_FIN | TCP_ACK,
        &[],
    )
}

fn tcp_data_for(conn: &mut TcpNatConn, payload: &[u8]) -> Vec<u8> {
    let seq = conn.host_next_seq;
    conn.host_next_seq = conn.host_next_seq.wrapping_add(payload.len() as u32);
    build_tcp_ipv4_frame(
        conn.guest_mac,
        ASL_HOST_MAC,
        conn.remote_ip,
        conn.guest_ip,
        conn.remote_port,
        conn.guest_port,
        seq,
        conn.guest_next_seq,
        TCP_PSH | TCP_ACK,
        payload,
    )
}

fn initial_tcp_seq(
    remote_ip: [u8; 4],
    remote_port: u16,
    guest_ip: [u8; 4],
    guest_port: u16,
) -> u32 {
    let mut value = 0x4153_4c00u32;
    for byte in remote_ip.iter().chain(guest_ip.iter()) {
        value = value.rotate_left(5) ^ (*byte as u32);
    }
    value ^ ((remote_port as u32) << 16) ^ guest_port as u32
}

fn is_control_ipv4(ip: [u8; 4]) -> bool {
    matches!(ip, ASL_GATEWAY_IP | ASL_BROADCAST_IP | IPV4_BROADCAST)
}

fn status_response(state: &BrokerState) -> String {
    let mut lines = alloc::vec![
        String::from("mode\tnat"),
        format!(
            "dns\t{}",
            if state.dns_broker_enabled {
                "host-broker"
            } else {
                "disabled"
            }
        ),
        format!("dns_broker_enabled\t{}", state.dns_broker_enabled),
        format!("dns_disabled_drops\t{}", state.dns_disabled_drops),
        format!("rules\t{}", state.rules.len()),
        format!("rx_queue\t{}", state.rx_queue.len()),
        format!("tx_packets\t{}", state.tx_packets),
        format!("tx_bytes\t{}", state.tx_bytes),
        format!("rx_packets\t{}", state.rx_packets),
        format!("rx_bytes\t{}", state.rx_bytes),
        format!("control_packets\t{}", state.control_packets),
        format!("nat_packets\t{}", state.nat_packets),
        format!("tcp_nat\t{}", state.tcp_nat.len()),
        format!("udp_nat\t{}", state.udp_nat.len()),
        format!("dropped_packets\t{}", state.dropped_packets),
        format!("apply_count\t{}", state.apply_count),
        format!("rejected_count\t{}", state.rejected_count),
        format!("last_apply_ms\t{}", state.last_apply_ms),
    ];
    if let Some(rule) = state.rules.last() {
        lines.push(format!(
            "last_rule\t{}\t{}\t{}:{}->{}\t{}",
            rule.distro,
            rule.id,
            rule.listen_address,
            rule.listen_port,
            rule.guest_port,
            rule.protocol
        ));
    }
    ok_lines(lines)
}

struct Ipv4Packet {
    src_mac: [u8; 6],
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    protocol: u8,
    payload_offset: usize,
    payload_len: usize,
}

struct UdpPacket {
    src_port: u16,
    dst_port: u16,
    payload_offset: usize,
    payload_len: usize,
}

struct TcpPacket<'a> {
    src_port: u16,
    dst_port: u16,
    seq: u32,
    flags: u16,
    payload: &'a [u8],
}

fn parse_ipv4(frame: &[u8]) -> Option<Ipv4Packet> {
    if frame.len() < 34 || read_be16(frame, 12) != 0x0800 {
        return None;
    }
    let ihl = ((frame[14] & 0x0f) as usize) * 4;
    if frame[14] >> 4 != 4 || ihl < 20 {
        return None;
    }
    let total_len = read_be16(frame, 16) as usize;
    if total_len < ihl || frame.len() < 14 + total_len {
        return None;
    }
    Some(Ipv4Packet {
        src_mac: read_mac(frame, 6)?,
        src_ip: read_ipv4(frame, 26)?,
        dst_ip: read_ipv4(frame, 30)?,
        protocol: frame[23],
        payload_offset: 14 + ihl,
        payload_len: total_len - ihl,
    })
}

fn parse_tcp(frame: &[u8], offset: usize, max_len: usize) -> Option<TcpPacket<'_>> {
    if max_len < 20 || frame.len() < offset + 20 {
        return None;
    }
    let data_offset = ((frame[offset + 12] >> 4) as usize) * 4;
    if data_offset < 20 || data_offset > max_len || frame.len() < offset + data_offset {
        return None;
    }
    Some(TcpPacket {
        src_port: read_be16(frame, offset),
        dst_port: read_be16(frame, offset + 2),
        seq: read_be32(frame, offset + 4),
        flags: (((frame[offset + 12] & 0x01) as u16) << 8) | frame[offset + 13] as u16,
        payload: &frame[offset + data_offset..offset + max_len],
    })
}

fn parse_udp(frame: &[u8], offset: usize, max_len: usize) -> Option<UdpPacket> {
    if max_len < 8 || frame.len() < offset + 8 {
        return None;
    }
    let len = read_be16(frame, offset + 4) as usize;
    if len < 8 || len > max_len || frame.len() < offset + len {
        return None;
    }
    Some(UdpPacket {
        src_port: read_be16(frame, offset),
        dst_port: read_be16(frame, offset + 2),
        payload_offset: offset + 8,
        payload_len: len - 8,
    })
}

fn build_udp_ipv4_frame(
    dst_mac: [u8; 6],
    src_mac: [u8; 6],
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut udp = Vec::new();
    udp.extend_from_slice(&src_port.to_be_bytes());
    udp.extend_from_slice(&dst_port.to_be_bytes());
    udp.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    udp.extend_from_slice(&0u16.to_be_bytes());
    udp.extend_from_slice(payload);
    build_ipv4_frame(dst_mac, src_mac, src_ip, dst_ip, 17, &udp)
}

fn build_tcp_ipv4_frame(
    dst_mac: [u8; 6],
    src_mac: [u8; 6],
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u16,
    payload: &[u8],
) -> Vec<u8> {
    let tcp_len = 20 + payload.len();
    let mut tcp = Vec::new();
    tcp.extend_from_slice(&src_port.to_be_bytes());
    tcp.extend_from_slice(&dst_port.to_be_bytes());
    tcp.extend_from_slice(&seq.to_be_bytes());
    tcp.extend_from_slice(&ack.to_be_bytes());
    tcp.push(0x50 | ((flags >> 8) as u8 & 0x01));
    tcp.push(flags as u8);
    tcp.extend_from_slice(&64240u16.to_be_bytes());
    tcp.extend_from_slice(&0u16.to_be_bytes());
    tcp.extend_from_slice(&0u16.to_be_bytes());
    tcp.extend_from_slice(payload);

    let mut pseudo = Vec::new();
    pseudo.extend_from_slice(&src_ip);
    pseudo.extend_from_slice(&dst_ip);
    pseudo.push(0);
    pseudo.push(6);
    pseudo.extend_from_slice(&(tcp_len as u16).to_be_bytes());
    pseudo.extend_from_slice(&tcp);
    let checksum = internet_checksum(&pseudo);
    tcp[16..18].copy_from_slice(&checksum.to_be_bytes());

    build_ipv4_frame(dst_mac, src_mac, src_ip, dst_ip, 6, &tcp)
}

fn build_ipv4_frame(
    dst_mac: [u8; 6],
    src_mac: [u8; 6],
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    protocol: u8,
    payload: &[u8],
) -> Vec<u8> {
    let total_len = 20 + payload.len();
    let mut out = Vec::new();
    out.extend_from_slice(&dst_mac);
    out.extend_from_slice(&src_mac);
    out.extend_from_slice(&0x0800u16.to_be_bytes());
    out.push(0x45);
    out.push(0);
    out.extend_from_slice(&(total_len as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&0x4000u16.to_be_bytes());
    out.push(64);
    out.push(protocol);
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&src_ip);
    out.extend_from_slice(&dst_ip);
    let checksum = internet_checksum(&out[14..34]);
    out[24..26].copy_from_slice(&checksum.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn push_dhcp_option(out: &mut Vec<u8>, code: u8, value: &[u8]) {
    if value.len() > u8::MAX as usize {
        return;
    }
    out.push(code);
    out.push(value.len() as u8);
    out.extend_from_slice(value);
}

fn dhcp_option<'a>(bootp: &'a [u8], code: u8) -> Option<&'a [u8]> {
    let mut index = 240usize;
    while index < bootp.len() {
        let option = bootp[index];
        index += 1;
        match option {
            0 => continue,
            255 => return None,
            _ => {
                if index >= bootp.len() {
                    return None;
                }
                let len = bootp[index] as usize;
                index += 1;
                if index + len > bootp.len() {
                    return None;
                }
                if option == code {
                    return Some(&bootp[index..index + len]);
                }
                index += len;
            }
        }
    }
    None
}

fn parse_dns_question(query: &[u8], mut offset: usize) -> Option<(usize, String)> {
    let mut name = String::new();
    loop {
        let len = *query.get(offset)? as usize;
        offset += 1;
        if len == 0 {
            break;
        }
        if len & 0xc0 != 0 || len > 63 || offset + len > query.len() {
            return None;
        }
        if !name.is_empty() {
            name.push('.');
        }
        for byte in &query[offset..offset + len] {
            if !byte.is_ascii_alphanumeric() && !matches!(*byte, b'-' | b'_') {
                return None;
            }
            name.push(byte.to_ascii_lowercase() as char);
        }
        offset += len;
    }
    if offset + 4 > query.len() {
        return None;
    }
    let qtype = read_be16(query, offset);
    let qclass = read_be16(query, offset + 2);
    if qtype != 1 || qclass != 1 {
        return None;
    }
    Some((offset + 4, name))
}

fn resolve_dns_name(name: &str) -> Option<[u8; 4]> {
    match name {
        "gateway.asl" | "host.asl" | "dns.asl" => Some(ASL_GATEWAY_IP),
        _ => {
            let mut ip = [0u8; 4];
            if net::dns(name, &mut ip) == 0 {
                Some(ip)
            } else {
                None
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn udp_bind(port: u16) -> u32 {
    net::udp_bind(port)
}

#[cfg(target_os = "linux")]
fn udp_bind(_port: u16) -> u32 {
    u32::MAX
}

#[cfg(not(target_os = "linux"))]
fn udp_unbind(port: u16) -> u32 {
    net::udp_unbind(port)
}

#[cfg(target_os = "linux")]
fn udp_unbind(_port: u16) -> u32 {
    0
}

#[cfg(not(target_os = "linux"))]
fn udp_sendto(dst_ip: &[u8; 4], dst_port: u16, src_port: u16, data: &[u8], flags: u32) -> u32 {
    net::udp_sendto(dst_ip, dst_port, src_port, data, flags)
}

#[cfg(target_os = "linux")]
fn udp_sendto(_dst_ip: &[u8; 4], _dst_port: u16, _src_port: u16, _data: &[u8], _flags: u32) -> u32 {
    u32::MAX
}

#[cfg(not(target_os = "linux"))]
fn udp_recvfrom(port: u16, buf: &mut [u8]) -> u32 {
    net::udp_recvfrom(port, buf)
}

#[cfg(target_os = "linux")]
fn udp_recvfrom(_port: u16, _buf: &mut [u8]) -> u32 {
    0
}

#[cfg(not(target_os = "linux"))]
fn udp_set_opt(port: u16, opt: u32, value: u32) -> u32 {
    net::udp_set_opt(port, opt, value)
}

#[cfg(target_os = "linux")]
fn udp_set_opt(_port: u16, _opt: u32, _value: u32) -> u32 {
    0
}

fn read_be16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_be32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_mac(bytes: &[u8], offset: usize) -> Option<[u8; 6]> {
    if offset + 6 > bytes.len() {
        return None;
    }
    Some([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
    ])
}

fn read_ipv4(bytes: &[u8], offset: usize) -> Option<[u8; 4]> {
    if offset + 4 > bytes.len() {
        return None;
    }
    Some([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn internet_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        sum = sum.wrapping_add(u16::from_be_bytes([bytes[index], bytes[index + 1]]) as u32);
        index += 2;
    }
    if index < bytes.len() {
        sum = sum.wrapping_add((bytes[index] as u32) << 8);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn validate_rule(
    id: &str,
    listen_address: &str,
    listen_port: u16,
    guest_port: u16,
    protocol: &str,
) -> Result<(), &'static str> {
    if id.is_empty() || !id.bytes().all(valid_id_byte) {
        return Err("invalid_rule_id");
    }
    if !valid_listen_address(listen_address) {
        return Err("invalid_listen_address");
    }
    if listen_port == 0 || guest_port == 0 {
        return Err("invalid_port");
    }
    if protocol != "tcp" {
        return Err("invalid_protocol");
    }
    Ok(())
}

fn conflicts(left: &ForwardRule, right: &ForwardRule) -> bool {
    if left.protocol != right.protocol || left.listen_port != right.listen_port {
        return false;
    }
    let left_addr = normalize_listen_address(&left.listen_address);
    let right_addr = normalize_listen_address(&right.listen_address);
    left_addr == right_addr
        || (left_addr == "0.0.0.0" && valid_ipv4(right_addr))
        || (right_addr == "0.0.0.0" && valid_ipv4(left_addr))
}

fn normalize_listen_address(address: &str) -> &str {
    match address {
        "localhost" => "127.0.0.1",
        "*" => "0.0.0.0",
        other => other,
    }
}

fn valid_listen_address(address: &str) -> bool {
    matches!(address, "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "*") || valid_ipv4(address)
}

fn valid_ipv4(address: &str) -> bool {
    let mut parts = 0usize;
    for part in address.split('.') {
        parts += 1;
        if parts > 4 || part.is_empty() || part.len() > 3 {
            return false;
        }
        let mut value = 0u16;
        for b in part.bytes() {
            if !b.is_ascii_digit() {
                return false;
            }
            value = match value
                .checked_mul(10)
                .and_then(|v| v.checked_add((b - b'0') as u16))
            {
                Some(value) => value,
                None => return false,
            };
        }
        if value > 255 {
            return false;
        }
    }
    parts == 4
}

fn valid_id_byte(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_')
}

fn write_status(state: &BrokerState, health: &str) {
    let _ = fs::mkdir("/System/var");
    let _ = fs::mkdir("/System/var/asl");
    let dns_label = if state.dns_broker_enabled {
        "host-broker"
    } else {
        "disabled"
    };
    let mut text = format!(
        "health={}\nmode=nat\ndns={}\ndns_broker_enabled={}\ndns_disabled_drops={}\nrules={}\nrx_queue={}\ntx_packets={}\ntx_bytes={}\nrx_packets={}\nrx_bytes={}\ncontrol_packets={}\nnat_packets={}\ntcp_nat={}\nudp_nat={}\ndropped_packets={}\napply_count={}\nrejected_count={}\nlast_apply_ms={}\n",
        health,
        dns_label,
        state.dns_broker_enabled,
        state.dns_disabled_drops,
        state.rules.len(),
        state.rx_queue.len(),
        state.tx_packets,
        state.tx_bytes,
        state.rx_packets,
        state.rx_bytes,
        state.control_packets,
        state.nat_packets,
        state.tcp_nat.len(),
        state.udp_nat.len(),
        state.dropped_packets,
        state.apply_count,
        state.rejected_count,
        state.last_apply_ms
    );
    if let Some(rule) = state.rules.last() {
        text.push_str(&format!(
            "last_rule={}:{}:{}:{}->{}:{}\n",
            rule.distro,
            rule.id,
            rule.listen_address,
            rule.listen_port,
            rule.guest_port,
            rule.protocol
        ));
    }
    let _ = fs::write_bytes(STATUS_PATH, text.as_bytes());
}

fn ok_lines(lines: Vec<String>) -> String {
    let mut out = format!("OK\t{}\n", lines.len());
    for line in lines {
        out.push_str(&line);
        out.push('\n');
    }
    out.push('\n');
    out
}

fn err(code: &str) -> String {
    format!("ERR\t{}\t{}\n\n", code, code)
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::new();
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn decode_hex(input: &str) -> Option<Vec<u8>> {
    if input.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let high = hex_nibble(bytes[index])?;
        let low = hex_nibble(bytes[index + 1])?;
        out.push((high << 4) | low);
        index += 2;
    }
    Some(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn split_first_word(s: &str) -> (&str, &str) {
    let trimmed = s.trim();
    if let Some(pos) = trimmed.find(char::is_whitespace) {
        (&trimmed[..pos], trimmed[pos + 1..].trim())
    } else {
        (trimmed, "")
    }
}

fn split_tab_fields(rest: &str) -> Vec<&str> {
    rest.split('\t').collect()
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut value = 0u32;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(value)
}

fn parse_u16(s: &str) -> Option<u16> {
    let value = parse_u32(s)?;
    u16::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_tx_status_and_rx_queue_roundtrip() {
        let mut state = BrokerState::new();
        let tx = dispatch(&mut state, "TX ubuntu-dev\t001122334455");
        assert!(tx.contains("tx_bytes\t6"));

        let status = dispatch(&mut state, "STATUS");
        assert!(status.contains("tx_packets\t1"));
        assert!(status.contains("tx_bytes\t6"));

        let empty = dispatch(&mut state, "RX_POLL ubuntu-dev\t1518");
        assert!(empty.contains("packet\tfalse"));

        let injected = dispatch(&mut state, "INJECT ubuntu-dev\tdeadbeef");
        assert!(injected.contains("queued\t1"));

        let rx = dispatch(&mut state, "RX_POLL ubuntu-dev\t1518");
        assert!(rx.contains("packet\ttrue"));
        assert!(rx.contains("bytes\t4"));
        assert!(rx.contains("data\tdeadbeef"));
    }

    #[test]
    fn rejects_invalid_packet_hex() {
        let mut state = BrokerState::new();
        let response = dispatch(&mut state, "TX ubuntu-dev\tabc");
        assert!(response.starts_with("ERR\tinvalid_frame"));
        assert_eq!(state.rejected_count, 1);
    }

    #[test]
    fn guest_arp_request_generates_gateway_reply() {
        let mut state = BrokerState::new();
        let frame = arp_request();
        let tx = dispatch(
            &mut state,
            &format!("TX ubuntu-dev\t{}", encode_hex(&frame)),
        );
        assert!(tx.contains("rx_generated\t1"));

        let rx = dispatch(&mut state, "RX_POLL ubuntu-dev\t1518");
        let reply = response_frame(&rx);
        assert_eq!(&reply[0..6], &GUEST_MAC);
        assert_eq!(&reply[6..12], &ASL_HOST_MAC);
        assert_eq!(read_be16(&reply, 12), 0x0806);
        assert_eq!(read_be16(&reply, 20), 2);
        assert_eq!(read_ipv4(&reply, 28).unwrap(), ASL_GATEWAY_IP);
    }

    #[test]
    fn guest_dhcp_discover_generates_offer() {
        let mut state = BrokerState::new();
        let frame = dhcp_discover();
        let tx = dispatch(
            &mut state,
            &format!("TX ubuntu-dev\t{}", encode_hex(&frame)),
        );
        assert!(tx.contains("rx_generated\t1"));

        let rx = dispatch(&mut state, "RX_POLL ubuntu-dev\t1518");
        let reply = response_frame(&rx);
        assert_eq!(read_be16(&reply, 12), 0x0800);
        let ipv4 = parse_ipv4(&reply).unwrap();
        let udp = parse_udp(&reply, ipv4.payload_offset, ipv4.payload_len).unwrap();
        assert_eq!(udp.src_port, 67);
        assert_eq!(udp.dst_port, 68);
        let bootp = &reply[udp.payload_offset..udp.payload_offset + udp.payload_len];
        assert_eq!(&bootp[16..20], &ASL_GUEST_IP);
        assert_eq!(dhcp_option(bootp, 53).unwrap(), &[2]);
        assert_eq!(dhcp_option(bootp, 54).unwrap(), &ASL_GATEWAY_IP);
    }

    #[test]
    fn guest_dns_query_for_gateway_asl_generates_answer() {
        let mut state = BrokerState::new();
        let frame = dns_query("gateway.asl");
        let tx = dispatch(
            &mut state,
            &format!("TX ubuntu-dev\t{}", encode_hex(&frame)),
        );
        assert!(tx.contains("rx_generated\t1"));

        let rx = dispatch(&mut state, "RX_POLL ubuntu-dev\t1518");
        let reply = response_frame(&rx);
        let ipv4 = parse_ipv4(&reply).unwrap();
        let udp = parse_udp(&reply, ipv4.payload_offset, ipv4.payload_len).unwrap();
        assert_eq!(udp.src_port, 53);
        let dns = &reply[udp.payload_offset..udp.payload_offset + udp.payload_len];
        assert_eq!(read_be16(dns, 2), 0x8180);
        assert_eq!(read_be16(dns, 6), 1);
        assert!(dns.windows(4).any(|window| window == ASL_GATEWAY_IP));
    }

    #[test]
    fn dns_broker_default_state_is_enabled() {
        let state = BrokerState::new();
        assert!(state.dns_broker_enabled);
        assert_eq!(state.dns_disabled_drops, 0);
    }

    #[test]
    fn set_dns_broker_disables_and_reenables() {
        let mut state = BrokerState::new();

        let off = dispatch(&mut state, "SET_DNS_BROKER 0");
        assert!(off.starts_with("OK\t"), "off response: {off:?}");
        assert!(off.contains("dns_broker_enabled\tfalse"));
        assert!(!state.dns_broker_enabled);

        let on = dispatch(&mut state, "SET_DNS_BROKER 1");
        assert!(on.contains("dns_broker_enabled\ttrue"));
        assert!(state.dns_broker_enabled);

        // Word aliases keep operator UX consistent with the rest of the
        // CLI surface (matches asld bool-arg parsing).
        let off_word = dispatch(&mut state, "SET_DNS_BROKER off");
        assert!(off_word.contains("dns_broker_enabled\tfalse"));
        let on_word = dispatch(&mut state, "SET_DNS_BROKER enable");
        assert!(on_word.contains("dns_broker_enabled\ttrue"));
    }

    #[test]
    fn set_dns_broker_rejects_garbage() {
        let mut state = BrokerState::new();
        let resp = dispatch(&mut state, "SET_DNS_BROKER yes-please");
        assert!(resp.starts_with("ERR"), "expected ERR got: {resp:?}");
        // State must not be touched when the arg is rejected.
        assert!(state.dns_broker_enabled);
    }

    #[test]
    fn dns_query_is_dropped_when_broker_disabled() {
        let mut state = BrokerState::new();
        // Disable the broker.
        let _ = dispatch(&mut state, "SET_DNS_BROKER 0");

        let frame = dns_query("gateway.asl");
        let tx = dispatch(
            &mut state,
            &format!("TX ubuntu-dev\t{}", encode_hex(&frame)),
        );
        // No reply was generated for this DNS query.
        assert!(
            tx.contains("rx_generated\t0"),
            "expected rx_generated\t0 got: {tx:?}"
        );

        // Drop counter advanced — observable in status.
        assert_eq!(state.dns_disabled_drops, 1);

        // Status response surfaces the disabled state and drop count.
        let status = dispatch(&mut state, "STATUS");
        assert!(status.contains("dns_broker_enabled\tfalse"));
        assert!(status.contains("dns\tdisabled"));
        assert!(status.contains("dns_disabled_drops\t1"));
    }

    #[test]
    fn dns_query_works_again_after_re_enabling_broker() {
        let mut state = BrokerState::new();
        // Off, query dropped.
        let _ = dispatch(&mut state, "SET_DNS_BROKER 0");
        let frame = dns_query("gateway.asl");
        let _ = dispatch(
            &mut state,
            &format!("TX ubuntu-dev\t{}", encode_hex(&frame)),
        );
        assert_eq!(state.dns_disabled_drops, 1);

        // On, query answered.
        let _ = dispatch(&mut state, "SET_DNS_BROKER 1");
        let tx = dispatch(
            &mut state,
            &format!("TX ubuntu-dev\t{}", encode_hex(&frame)),
        );
        assert!(tx.contains("rx_generated\t1"));
        // Drop count is sticky — does not reset on re-enable. Operators
        // see the cumulative number until the daemon restarts.
        assert_eq!(state.dns_disabled_drops, 1);
    }

    #[test]
    fn is_dns_query_recognises_only_real_dns_traffic() {
        // Real DNS query.
        let dns = dns_query("gateway.asl");
        assert!(is_dns_query(&dns));

        // DHCP frame: UDP/67 — not DNS.
        let dhcp = dhcp_discover();
        assert!(!is_dns_query(&dhcp));

        // ARP request: not even IPv4.
        let arp = arp_request();
        assert!(!is_dns_query(&arp));
    }

    #[test]
    fn tcp_nat_data_frame_uses_flow_tuple_and_advances_sequence() {
        let mut conn = TcpNatConn {
            distro: String::from("ubuntu-dev"),
            guest_mac: GUEST_MAC,
            guest_ip: ASL_GUEST_IP,
            guest_port: 49152,
            remote_ip: [93, 184, 216, 34],
            remote_port: 80,
            socket_id: 7,
            guest_next_seq: 1001,
            host_next_seq: 2001,
            established: true,
            closing: false,
        };

        let frame = tcp_data_for(&mut conn, b"HTTP");
        assert_eq!(conn.host_next_seq, 2005);
        let ipv4 = parse_ipv4(&frame).unwrap();
        assert_eq!(ipv4.src_ip, [93, 184, 216, 34]);
        assert_eq!(ipv4.dst_ip, ASL_GUEST_IP);
        let tcp = parse_tcp(&frame, ipv4.payload_offset, ipv4.payload_len).unwrap();
        assert_eq!(tcp.src_port, 80);
        assert_eq!(tcp.dst_port, 49152);
        assert_eq!(tcp.seq, 2001);
        assert_eq!(tcp.flags & (TCP_PSH | TCP_ACK), TCP_PSH | TCP_ACK);
        assert_eq!(tcp.payload, b"HTTP");
        assert!(tcp_checksum_valid(&frame, &ipv4));
    }

    #[test]
    fn tcp_nat_syn_ack_frame_is_parseable() {
        let conn = TcpNatConn {
            distro: String::from("ubuntu-dev"),
            guest_mac: GUEST_MAC,
            guest_ip: ASL_GUEST_IP,
            guest_port: 49153,
            remote_ip: [1, 1, 1, 1],
            remote_port: 443,
            socket_id: 9,
            guest_next_seq: 42,
            host_next_seq: 9001,
            established: false,
            closing: false,
        };

        let frame = build_tcp_ipv4_frame(
            conn.guest_mac,
            ASL_HOST_MAC,
            conn.remote_ip,
            conn.guest_ip,
            conn.remote_port,
            conn.guest_port,
            conn.host_next_seq - 1,
            conn.guest_next_seq,
            TCP_SYN | TCP_ACK,
            &[],
        );
        let ipv4 = parse_ipv4(&frame).unwrap();
        let tcp = parse_tcp(&frame, ipv4.payload_offset, ipv4.payload_len).unwrap();
        assert_eq!(tcp.flags & (TCP_SYN | TCP_ACK), TCP_SYN | TCP_ACK);
        assert_eq!(tcp.seq, 9000);
        assert!(tcp_checksum_valid(&frame, &ipv4));
    }

    const GUEST_MAC: [u8; 6] = [0x02, 0x41, 0x53, 0x4c, 0x00, 0x01];

    fn response_frame(response: &str) -> Vec<u8> {
        for line in response.lines() {
            if let Some(data) = line.strip_prefix("data\t") {
                return decode_hex(data).unwrap();
            }
        }
        panic!("missing response frame: {response}");
    }

    fn arp_request() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&ETH_BROADCAST);
        out.extend_from_slice(&GUEST_MAC);
        out.extend_from_slice(&0x0806u16.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&0x0800u16.to_be_bytes());
        out.push(6);
        out.push(4);
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&GUEST_MAC);
        out.extend_from_slice(&ASL_GUEST_IP);
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&ASL_GATEWAY_IP);
        out
    }

    fn dhcp_discover() -> Vec<u8> {
        let mut payload = alloc::vec![0u8; 236];
        payload[0] = 1;
        payload[1] = 1;
        payload[2] = 6;
        payload[4..8].copy_from_slice(&0x1234_5678u32.to_be_bytes());
        payload[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
        payload[28..34].copy_from_slice(&GUEST_MAC);
        payload.extend_from_slice(&[99, 130, 83, 99]);
        push_dhcp_option(&mut payload, 53, &[1]);
        payload.push(255);
        build_udp_ipv4_frame(
            ETH_BROADCAST,
            GUEST_MAC,
            [0, 0, 0, 0],
            IPV4_BROADCAST,
            68,
            67,
            &payload,
        )
    }

    fn dns_query(name: &str) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0x1200u16.to_be_bytes());
        payload.extend_from_slice(&0x0100u16.to_be_bytes());
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.extend_from_slice(&0u16.to_be_bytes());
        payload.extend_from_slice(&0u16.to_be_bytes());
        payload.extend_from_slice(&0u16.to_be_bytes());
        for label in name.split('.') {
            payload.push(label.len() as u8);
            payload.extend_from_slice(label.as_bytes());
        }
        payload.push(0);
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.extend_from_slice(&1u16.to_be_bytes());
        build_udp_ipv4_frame(
            ASL_HOST_MAC,
            GUEST_MAC,
            ASL_GUEST_IP,
            ASL_GATEWAY_IP,
            49152,
            53,
            &payload,
        )
    }

    fn tcp_checksum_valid(frame: &[u8], ipv4: &Ipv4Packet) -> bool {
        let tcp = &frame[ipv4.payload_offset..ipv4.payload_offset + ipv4.payload_len];
        let mut pseudo = Vec::new();
        pseudo.extend_from_slice(&ipv4.src_ip);
        pseudo.extend_from_slice(&ipv4.dst_ip);
        pseudo.push(0);
        pseudo.push(6);
        pseudo.extend_from_slice(&(tcp.len() as u16).to_be_bytes());
        pseudo.extend_from_slice(tcp);
        internet_checksum(&pseudo) == 0
    }
}
